use super::*;

/// Resolve a loader dependency from active files and comparable receipts. A
/// Thunderstore pack version is not interchangeable with its loader's native
/// version or with another author's pack version.
pub(super) fn availability(
    profile: &GameProfile,
    dependency: &DependencySpec,
    runtime: &str,
    records: &[InstalledModRecord],
    target: Option<&InstallTargetOption>,
) -> DependencyAvailability {
    let package_ref = dependency
        .provider
        .eq_ignore_ascii_case("thunderstore")
        .then(|| parse_thunderstore_ref(dependency).ok())
        .flatten();
    let in_target = |record: &&InstalledModRecord| {
        record.profile_id == profile.id
            && record.last_status != "removed"
            && target.is_none_or(|target| record_payload_covers_install_target(record, target))
    };
    let runtime_records = records
        .iter()
        .filter(in_target)
        .filter(|record| {
            record
                .runtime_id
                .as_deref()
                .is_none_or(|known| known.eq_ignore_ascii_case(runtime))
                && (runtime_record_matches(record, runtime)
                    || package_ref
                        .as_ref()
                        .is_some_and(|package| record_matches_package(record, package)))
        })
        .collect::<Vec<_>>();
    let active_records = runtime_records
        .iter()
        .copied()
        .filter(|record| record.enabled)
        .collect::<Vec<_>>();
    let conflicting_loader = matches!(runtime, "bepinex" | "bepinex-il2cpp")
        && active_records.is_empty()
        && records.iter().filter(in_target).any(|record| {
            record.enabled
                && record.runtime_id.as_deref().is_some_and(|known| {
                    matches!(known, "bepinex" | "bepinex-il2cpp")
                        && !known.eq_ignore_ascii_case(runtime)
                })
        });
    let present = !conflicting_loader
        && target
            .map(|target| runtime_installed_for_target(profile, runtime, target))
            .unwrap_or_else(|| {
                package_ref
                    .as_ref()
                    .map(|package| thunderstore_runtime_available(profile, package))
                    .unwrap_or_else(|| runtime_installed(profile, runtime))
            });

    if let Some(required) = dependency.version.as_deref() {
        let comparable = active_records
            .iter()
            .copied()
            .filter(|record| {
                package_ref
                    .as_ref()
                    .map(|package| record_matches_package(record, package))
                    .unwrap_or_else(|| native_version_source_matches(record, dependency))
            })
            .filter_map(|record| record.package_version.as_deref())
            .collect::<Vec<_>>();
        if !comparable.is_empty() {
            return DependencyAvailability {
                available: present
                    && comparable
                        .iter()
                        .any(|installed| version_requirement_matches(required, installed)),
                version_verified: true,
            };
        }
    }

    // A disabled receipt is not proof that its component is active, even if
    // leftover files happen to resemble the loader. A different active receipt
    // can still provide the component in this target.
    let disabled = !runtime_records.is_empty() && active_records.is_empty();
    DependencyAvailability {
        available: present && !disabled,
        version_verified: dependency.version.is_none(),
    }
}

fn native_version_source_matches(record: &InstalledModRecord, dependency: &DependencySpec) -> bool {
    let Some(source) = dependency.source.as_deref() else {
        return false;
    };
    let Some(recorded_source) = record.package_id.as_deref() else {
        return false;
    };
    let provider_identity = |value: &str| -> Option<String> {
        let (provider, location) = value.split_once(':')?;
        let provider = match provider {
            "github" | "github-release" => "github-release",
            "bepinbuilds" => "bepinbuilds",
            _ => return None,
        };
        let location = location.split('#').next()?.split('@').next()?;
        (!location.is_empty()).then(|| format!("{provider}:{location}").to_ascii_lowercase())
    };
    match (
        provider_identity(source),
        provider_identity(recorded_source),
    ) {
        (Some(expected), Some(installed)) => expected == installed,
        _ => false,
    }
}

fn record_matches_package(
    record: &InstalledModRecord,
    package_ref: &ThunderstorePackageRef,
) -> bool {
    record
        .package_id
        .as_deref()
        .is_some_and(|id| id.eq_ignore_ascii_case(&thunderstore_package_id(package_ref)))
        || record
            .dependency_string
            .as_deref()
            .and_then(|token| parse_thunderstore_token(token, None))
            .is_some_and(|installed| {
                installed
                    .namespace
                    .eq_ignore_ascii_case(&package_ref.namespace)
                    && installed.name.eq_ignore_ascii_case(&package_ref.name)
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        store: PathBuf,
        profile: GameProfile,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "mod-garrison-runtime-dependencies-{}",
                Uuid::new_v4()
            ));
            let game = root.join("game");
            fs::create_dir_all(&game).unwrap();
            fs::write(game.join("FutureGame.exe"), b"inert executable marker").unwrap();
            let profile = serde_json::from_value(serde_json::json!({
                "id": "future-game", "name": "Future Game", "gamePath": game,
                "engine": "unity-mono", "loader": "bepinex", "createdAt": "", "updatedAt": ""
            }))
            .unwrap();
            Self {
                store: root.join("store"),
                root,
                profile,
            }
        }

        fn loader(&self, prefix: &str) {
            let game = PathBuf::from(&self.profile.game_path);
            for path in [
                "BepInEx/core/BepInEx.dll",
                "winhttp.dll",
                "doorstop_config.ini",
                "FutureGame.exe",
            ] {
                let file = game.join(prefix).join(path);
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, b"inert runtime marker").unwrap();
            }
        }

        fn target(&self, prefix: &str) -> InstallTargetOption {
            let path = join_install_route(prefix, "BepInEx");
            InstallTargetOption {
                id: install_route_key("bepinex", &path),
                label: path.clone(),
                relative_path: path.clone(),
                scope: install_target_scope_from_path(&path),
                recommended: true,
            }
        }

        fn record(&self, package: &str, version: &str, prefix: &str) -> InstalledModRecord {
            let core = join_install_route(prefix, "BepInEx/core/BepInEx.dll");
            let plan = InstallPlan {
                adapter_id: "bepinex".to_string(),
                adapter_name: "BepInEx".to_string(),
                confidence: 1.0,
                summary: "Inert runtime fixture".to_string(),
                mappings: vec![mapping(&core, "game", &core, "Inert runtime fixture")],
                dependencies: Vec::new(),
                warnings: Vec::new(),
                requires_confirmation: false,
            };
            serde_json::from_value(serde_json::json!({
                "id": "managed-runtime", "profileId": self.profile.id,
                "archivePath": self.root.join("inert.zip"), "archiveName": "Inert runtime",
                "packageId": package, "packageVersion": version, "runtimeId": "bepinex",
                "adapterId": "bepinex", "summary": "Inert fixture", "installedAt": "",
                "filesWritten": [PathBuf::from(&self.profile.game_path).join(&core)],
                "backupsWritten": [], "dependencies": [], "enabled": true, "lastStatus": "installed", "plan": plan
            })).unwrap()
        }

        fn store_records(&self, records: Vec<InstalledModRecord>) {
            write_store(
                &installed_mods_path(&self.store),
                &StoreFile {
                    version: 1,
                    items: records,
                },
            )
            .unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.root.parent() == Some(std::env::temp_dir().as_path())
                && self.root.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .starts_with("mod-garrison-runtime-dependencies-")
                })
            {
                let _ = fs::remove_dir_all(&self.root);
            }
        }
    }

    #[test]
    fn future_game_alias_reuses_equivalent_github_loader_without_claiming_pack_version() {
        let fixture = Fixture::new();
        fixture.loader("");
        fixture.store_records(vec![fixture.record(
            "github-release:BepInEx/BepInEx",
            "v5.4.23.5",
            "",
        )]);
        let dependency = parse_thunderstore_dependency("BepInEx-BepInExPack-5.4.2100");
        let target = fixture.target("");
        assert_eq!(
            runtime_id_for_dependency(&fixture.profile, &dependency).as_deref(),
            Some("bepinex")
        );
        let general = dependency_availability(&fixture.store, &fixture.profile, &dependency);
        let targeted = dependency_availability_for_target(
            &fixture.store,
            &fixture.profile,
            &dependency,
            &target,
        );
        assert!(general.available && targeted.available);
        assert!(!general.version_verified && !targeted.version_verified);
        let mut plan = fixture
            .record("github-release:BepInEx/BepInEx", "v5.4.23.5", "")
            .plan
            .unwrap();
        plan.dependencies = vec![dependency];
        ensure_install_target_dependencies_available(
            &fixture.store,
            &fixture.profile,
            &plan,
            &target,
        )
        .unwrap();
    }

    #[test]
    fn known_old_pack_cannot_be_satisfied_by_existing_loader_files() {
        let fixture = Fixture::new();
        fixture.loader("");
        fixture.store_records(vec![fixture.record(
            "thunderstore:BepInEx/BepInExPack",
            "5.4.2100",
            "",
        )]);
        let dependency = parse_thunderstore_dependency("BepInEx-BepInExPack-5.4.2304");
        let general = dependency_availability(&fixture.store, &fixture.profile, &dependency);
        let targeted = dependency_availability_for_target(
            &fixture.store,
            &fixture.profile,
            &dependency,
            &fixture.target(""),
        );
        assert!(!general.available && !targeted.available);
        assert!(general.version_verified && targeted.version_verified);
        let matching = parse_thunderstore_dependency("BepInEx-BepInExPack-5.4.2100");
        assert!(dependency_availability(&fixture.store, &fixture.profile, &matching).available);
        assert!(
            dependency_availability_for_target(
                &fixture.store,
                &fixture.profile,
                &matching,
                &fixture.target("")
            )
            .available
        );
    }

    #[test]
    fn native_runtime_requirement_never_compares_thunderstore_pack_version() {
        let fixture = Fixture::new();
        fixture.loader("");
        let mut dependency = known_runtime_dependency(&fixture.profile, "bepinex");
        dependency.version = Some(">=5.4.22".to_string());
        fixture.store_records(vec![fixture.record(
            "thunderstore:BepInEx/BepInExPack",
            "5.4.2100",
            "",
        )]);
        for target in [None, Some(fixture.target(""))] {
            let available = target
                .as_ref()
                .map(|target| {
                    dependency_availability_for_target(
                        &fixture.store,
                        &fixture.profile,
                        &dependency,
                        target,
                    )
                })
                .unwrap_or_else(|| {
                    dependency_availability(&fixture.store, &fixture.profile, &dependency)
                });
            assert!(available.available);
            assert!(!available.version_verified);
        }
        // Legacy native receipts without packageProvider are still comparable
        // when their GitHub source identity proves the version domain.
        fixture.store_records(vec![fixture.record(
            "github:BepInEx/BepInEx",
            "v5.4.21",
            "",
        )]);
        let old = dependency_availability(&fixture.store, &fixture.profile, &dependency);
        assert!(!old.available && old.version_verified);
        fixture.store_records(vec![fixture.record(
            "github-release:BepInEx/BepInEx#BepInEx_win_x64_*.zip",
            "v5.4.23",
            "",
        )]);
        let current = dependency_availability_for_target(
            &fixture.store,
            &fixture.profile,
            &dependency,
            &fixture.target(""),
        );
        assert!(current.available && current.version_verified);
    }

    #[test]
    fn loader_in_other_server_root_does_not_satisfy_client_target() {
        let fixture = Fixture::new();
        fixture.loader("Server");
        fixture.store_records(vec![fixture.record(
            "github-release:BepInEx/BepInEx",
            "v5.4.23.5",
            "Server",
        )]);
        let dependency = parse_thunderstore_dependency("BepInEx-BepInExPack-5.4.2100");
        assert!(
            !dependency_availability_for_target(
                &fixture.store,
                &fixture.profile,
                &dependency,
                &fixture.target("")
            )
            .available
        );
        assert!(
            dependency_availability_for_target(
                &fixture.store,
                &fixture.profile,
                &dependency,
                &fixture.target("Server")
            )
            .available
        );
    }

    #[test]
    fn disabled_or_missing_loader_is_not_satisfied_by_receipt_alone() {
        let fixture = Fixture::new();
        fixture.loader("");
        let mut record = fixture.record("thunderstore:BepInEx/BepInExPack", "5.4.2100", "");
        let dependency = parse_thunderstore_dependency("BepInEx-BepInExPack-5.4.2100");
        record.enabled = false;
        fixture.store_records(vec![record.clone()]);
        assert!(
            !dependency_availability_for_target(
                &fixture.store,
                &fixture.profile,
                &dependency,
                &fixture.target("")
            )
            .available
        );
        assert!(!dependency_availability(&fixture.store, &fixture.profile, &dependency).available);
        record.enabled = true;
        fixture.store_records(vec![record]);
        fs::remove_file(PathBuf::from(&fixture.profile.game_path).join("BepInEx/core/BepInEx.dll"))
            .unwrap();
        assert!(
            !dependency_availability_for_target(
                &fixture.store,
                &fixture.profile,
                &dependency,
                &fixture.target("")
            )
            .available
        );
        assert!(!dependency_availability(&fixture.store, &fixture.profile, &dependency).available);
    }

    #[test]
    fn registry_alias_selects_the_game_runtime_and_rejects_conflicting_managed_kind() {
        let mut fixture = Fixture::new();
        fixture.loader("");
        fixture.store_records(vec![fixture.record(
            "github-release:BepInEx/BepInEx",
            "v5.4.23.5",
            "",
        )]);
        let dependency = parse_thunderstore_dependency("Some_Team-BepInExPack_Future_Game-1.2.3");
        assert_eq!(
            runtime_id_for_dependency(&fixture.profile, &dependency).as_deref(),
            Some("bepinex")
        );
        fixture.profile.engine = "unity-il2cpp".to_string();
        fixture.profile.loader = "bepinex-il2cpp".to_string();
        assert_eq!(
            runtime_id_for_dependency(&fixture.profile, &dependency).as_deref(),
            Some("bepinex-il2cpp")
        );
        assert!(!dependency_availability(&fixture.store, &fixture.profile, &dependency).available);
        assert!(
            !dependency_availability_for_target(
                &fixture.store,
                &fixture.profile,
                &dependency,
                &fixture.target("")
            )
            .available
        );
        fixture.profile.engine = "unknown".to_string();
        fixture.profile.loader = "none".to_string();
        assert!(runtime_id_for_dependency(&fixture.profile, &dependency).is_none());
        let plugin = parse_thunderstore_dependency("Some_Team-BepInExConfigurationManager-1.2.3");
        assert!(runtime_id_for_dependency(&fixture.profile, &plugin).is_none());
    }
}
