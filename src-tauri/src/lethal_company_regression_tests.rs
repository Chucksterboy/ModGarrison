//! Offline reproductions of Lethal Company provider target handoffs.
//! All DLL/executable bytes are inert markers; these tests never run game code.

use super::*;

struct Fixture {
    root: PathBuf,
    store: PathBuf,
    profile: GameProfile,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mod-garrison-lethal-company-regression-{}",
            Uuid::new_v4()
        ));
        let game = root.join("game");
        let store = root.join("store");
        fs::create_dir_all(game.join("Lethal Company_Data")).unwrap();
        fs::create_dir_all(game.join("BepInEx/plugins")).unwrap();
        fs::write(game.join("Lethal Company.exe"), b"inert executable marker").unwrap();
        let profile: GameProfile = serde_json::from_value(serde_json::json!({
            "id": "lethal-company-regression", "name": "Lethal Company", "gamePath": game,
            "gameId": null, "steamAppId": "1966720", "engine": "unity-mono",
            "loader": "bepinex", "createdAt": "", "updatedAt": ""
        }))
        .unwrap();
        write_store(
            &profiles_path(&store),
            &StoreFile {
                version: 1,
                items: vec![profile.clone()],
            },
        )
        .unwrap();
        Self {
            root,
            store,
            profile,
        }
    }

    fn game(&self) -> PathBuf {
        PathBuf::from(&self.profile.game_path)
    }

    fn zip(&self, name: &str, members: &[(&str, &[u8])]) -> PathBuf {
        let path = self.root.join(name);
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        for (name, bytes) in members {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    fn more_company(&self, with_config: bool) -> (PathBuf, InstallPlan) {
        let mut members: Vec<(&str, &[u8])> = vec![
            ("BepInEx/plugins/MoreCompany.dll", b"inert plugin marker"),
            ("icon.png", b"inert icon marker"),
            (
                "manifest.json",
                br#"{"name":"MoreCompany","version_number":"1.14.0","website_url":"https://example.invalid","description":"Offline structural fixture","dependencies":["BepInEx-BepInExPack-5.4.2100"]}"#,
            ),
            ("README.md", b"Offline structural fixture, no third-party code."),
        ];
        if with_config {
            members.push(("BepInEx/config/MoreCompany.cfg", b"value=fixture\n"));
        }
        let source = self.zip("MoreCompany-1.14.0.zip", &members);
        let scanned = scan_import_source(&self.store, &source).unwrap();
        let analysis = analyze_scanned_archive(scanned, &self.profile);
        let plan = analysis.recommended_plan.unwrap_or_else(|| {
            panic!(
                "Expected complete BepInEx plan: {}",
                analysis.compatibility.reason
            )
        });
        assert_eq!(plan.adapter_id, "bepinex");
        assert!(!plan.requires_confirmation);
        (source, plan)
    }

    fn document(&self) -> ProviderRouteDocument {
        ProviderRouteDocument {
            provider: "Nexus Mods".to_string(),
            mod_id: "nexus:lethalcompany/60".to_string(),
            mod_name: "More Company".to_string(),
            text: "Install BepInEx. Place BepInEx/plugins/MoreCompany.dll in your BepInEx/plugins folder."
                .to_string(),
        }
    }

    fn install(
        &self,
        source: &Path,
        plan: &InstallPlan,
        targets: Vec<InstallTargetOption>,
        selected: Option<String>,
    ) -> InstallResult {
        let mut visited = HashSet::new();
        install_archive_impl_with_metadata(
            &self.store,
            &self.profile,
            &source.to_string_lossy(),
            plan,
            InstallOptions {
                metadata: InstallMetadata {
                    package_id: Some("nexus:lethalcompany/60".to_string()),
                    package_provider: Some("nexus".to_string()),
                    package_version: Some("1.14.0".to_string()),
                    install_targets: targets,
                    selected_install_target_id: selected,
                    ..InstallMetadata::default()
                },
                // Routing/lifecycle test: no provider requests or loader execution.
                resolve_dependencies: false,
                visited_dependencies: &mut visited,
                dependency_depth: 0,
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let temporary = std::env::temp_dir();
        if self.root.parent() == Some(temporary.as_path())
            && self.root.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("mod-garrison-lethal-company-regression-")
            })
        {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
fn nexus_more_company_plugin_preflight_resolves_to_sole_loader_root() {
    let fixture = Fixture::new();
    let (_, plan) = fixture.more_company(false);
    let document = fixture.document();
    let described = provider_install_target_options(&fixture.store, &fixture.profile, &document);
    let requested = described
        .iter()
        .find(|target| target.id == "bepinex|bepinex/plugins")
        .expect("Reproduce the pending Nexus handoff's exact leaf target ID");
    let targets =
        provider_plan_install_target_options(&fixture.store, &fixture.profile, &document, &plan);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].id, "bepinex|bepinex");
    let selected = provider_selected_install_target_id(
        &fixture.store,
        &fixture.profile,
        &document,
        &targets,
        Some(&requested.id),
    );
    assert_eq!(selected.as_deref(), Some("bepinex|bepinex"));
    assert_eq!(
        select_install_target_id(&targets, selected.as_deref()).unwrap(),
        selected
    );
}

#[test]
fn unrelated_saved_provider_target_still_fails_closed() {
    let fixture = Fixture::new();
    let (_, plan) = fixture.more_company(false);
    let document = fixture.document();
    let targets =
        provider_plan_install_target_options(&fixture.store, &fixture.profile, &document, &plan);
    for stale in [
        "bepinex|server/bepinex/plugins",
        "reframework|reframework/plugins",
        "bepinex|bepinex/plugins-not-the-same-folder",
    ] {
        let selected = provider_selected_install_target_id(
            &fixture.store,
            &fixture.profile,
            &document,
            &targets,
            Some(stale),
        );
        assert_eq!(selected.as_deref(), Some(stale));
        assert!(select_install_target_id(&targets, selected.as_deref()).is_err());
    }
    assert!(!fixture
        .game()
        .join("BepInEx/plugins/MoreCompany.dll")
        .exists());
}

#[test]
fn canonical_plugin_selection_keeps_config_companion_and_removes_owned_payload() {
    let fixture = Fixture::new();
    let (source, plan) = fixture.more_company(true);
    let document = fixture.document();
    let targets =
        provider_plan_install_target_options(&fixture.store, &fixture.profile, &document, &plan);
    let selected = provider_selected_install_target_id(
        &fixture.store,
        &fixture.profile,
        &document,
        &targets,
        Some("bepinex|bepinex/plugins"),
    );
    let manual = fixture.game().join("BepInEx/plugins/Unrelated.dll");
    fs::write(&manual, b"pre-existing manual plugin").unwrap();
    let result = fixture.install(&source, &plan, targets, selected);
    let plugin = fixture.game().join("BepInEx/plugins/MoreCompany.dll");
    let config = fixture.game().join("BepInEx/config/MoreCompany.cfg");
    assert_eq!(fs::read(&plugin).unwrap(), b"inert plugin marker");
    assert_eq!(fs::read(&config).unwrap(), b"value=fixture\n");
    assert!(!fixture.game().join("icon.png").exists());
    assert!(!fixture.game().join("manifest.json").exists());
    let store = read_store::<InstalledModRecord>(&installed_mods_path(&fixture.store)).unwrap();
    let record = store
        .items
        .iter()
        .find(|record| record.id == result.installed_mod_id)
        .unwrap();
    assert_eq!(record.files_written.len(), 2);
    assert!(record
        .written_file_hashes
        .contains_key(&plugin.to_string_lossy().to_string()));
    assert!(record
        .written_file_hashes
        .contains_key(&config.to_string_lossy().to_string()));
    let deactivation = deactivate_mod_files_with_policy(
        &fixture.store,
        &fixture.profile,
        record,
        ModifiedFilePolicy::RequireRemovalConfirmation,
    )
    .unwrap();
    assert!(deactivation.preserved_files.is_empty());
    commit_removed_installed_mod_record(&fixture.store, &fixture.profile.id, &record.id).unwrap();
    cleanup_install_data(&fixture.store, &fixture.profile.id, &record.id);
    assert!(!plugin.exists());
    assert!(!config.exists());
    assert_eq!(fs::read(manual).unwrap(), b"pre-existing manual plugin");
    assert!(
        read_store::<InstalledModRecord>(&installed_mods_path(&fixture.store))
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn plugin_description_does_not_filter_runtime_bootstrap_or_configuration() {
    let fixture = Fixture::new();
    let source = fixture.zip(
        "BepInEx-runtime.zip",
        &[
            ("BepInEx/core/BepInEx.dll", b"inert runtime marker"),
            ("winhttp.dll", b"inert bootstrap marker"),
            ("doorstop_config.ini", b"enabled=true\n"),
            ("BepInEx/config/BepInEx.cfg", b"fixture=true\n"),
        ],
    );
    let scanned = scan_import_source(&fixture.store, &source).unwrap();
    let plan = analyze_scanned_archive(scanned, &fixture.profile)
        .recommended_plan
        .unwrap();
    assert_eq!(
        runtime_supplied_by_plan(&fixture.profile, &plan).as_deref(),
        Some("bepinex")
    );
    let document = fixture.document();
    let targets =
        provider_plan_install_target_options(&fixture.store, &fixture.profile, &document, &plan);
    let selected = provider_selected_install_target_id(
        &fixture.store,
        &fixture.profile,
        &document,
        &targets,
        Some("bepinex|bepinex/plugins"),
    );
    fixture.install(&source, &plan, targets, selected);
    for file in [
        "BepInEx/core/BepInEx.dll",
        "winhttp.dll",
        "doorstop_config.ini",
        "BepInEx/config/BepInEx.cfg",
    ] {
        assert!(fixture.game().join(file).is_file(), "Missing {file}");
    }
}

#[test]
fn client_leaf_preflight_preserves_distinct_server_variant() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.game().join("Server/Server_Data")).unwrap();
    fs::create_dir_all(fixture.game().join("Server/BepInEx/plugins")).unwrap();
    fs::write(
        fixture.game().join("Server/Server.exe"),
        b"inert server marker",
    )
    .unwrap();
    let source = fixture.zip(
        "client-server.zip",
        &[
            (
                "Client/BepInEx/plugins/MoreCompany.dll",
                b"inert client plugin",
            ),
            (
                "Server/BepInEx/plugins/MoreCompany.dll",
                b"inert server plugin",
            ),
        ],
    );
    let scanned = scan_import_source(&fixture.store, &source).unwrap();
    let mut plan = bepinex_plan(&scanned, &fixture.profile).unwrap();
    let document = fixture.document();
    let targets =
        provider_plan_install_target_options(&fixture.store, &fixture.profile, &document, &plan);
    assert_eq!(targets.len(), 2);
    assert_eq!(
        targets
            .iter()
            .filter(|target| target.scope == "hosted-server")
            .count(),
        1
    );
    let selected = provider_selected_install_target_id(
        &fixture.store,
        &fixture.profile,
        &document,
        &targets,
        Some("bepinex|bepinex/plugins"),
    )
    .unwrap();
    let selected_target = targets.iter().find(|target| target.id == selected).unwrap();
    assert_eq!(selected_target.relative_path, "BepInEx");
    apply_install_target_option(&mut plan, selected_target).unwrap();
    assert_eq!(plan.mappings.len(), 1);
    assert_eq!(
        plan.mappings[0].source_path,
        "Client/BepInEx/plugins/MoreCompany.dll"
    );
    assert_eq!(
        plan.mappings[0].target_relative_path,
        "BepInEx/plugins/MoreCompany.dll"
    );
}
