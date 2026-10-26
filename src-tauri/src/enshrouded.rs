//! Enshrouded package routing. A route is only produced for a real Enshrouded
//! executable root and a complete, recognizable package; this module never
//! creates directories or changes files while analysing an archive.

use super::*;

const ROOT_PROXIES: &[&str] = &[
    "dinput8.dll",
    "powrprof.dll",
    "dbghelp.dll",
    "dbgcore.dll",
    "winmm.dll",
    "version.dll",
];

/// The Steam profile is verified by the command layer. Requiring its game id
/// and an executable here also keeps a profile's manually selected engine from
/// turning an unrelated game's folder into an Enshrouded install root.
pub(super) fn is_profile(profile: &GameProfile) -> bool {
    profile.game_id.as_deref() == Some("enshrouded")
        && profile.engine == "enshrouded"
        && ["enshrouded.exe", "enshrouded_server.exe"]
            .iter()
            .any(|name| Path::new(&profile.game_path).join(name).is_file())
}

pub(super) fn plan(scanned: &ScannedArchive, profile: &GameProfile) -> Option<InstallPlan> {
    if !is_profile(profile) {
        return None;
    }
    if let Some(plan) = enshrouded_runtime::verified_runtime_plan(scanned, profile) {
        return Some(plan);
    }
    if let Some(plan) = enshrouded_loaders::shroudtopia_runtime_plan(scanned, profile) {
        return Some(plan);
    }
    if let Some(plan) = enshrouded_loaders::proxy_runtime_plan(scanned, profile) {
        return Some(plan);
    }
    let files = installable_files(&scanned.entries);
    if files.is_empty()
        || files.iter().any(|entry| {
            validate_archive_relative_path(&entry.path).is_err()
                || forbidden_payload(&entry.path)
                || basename(&entry.path).eq_ignore_ascii_case("shroudtopia.dll")
        })
    {
        return None;
    }

    let trusted_game_source = scanned.package_identity.as_ref().is_some_and(|identity| {
        identity.provider.eq_ignore_ascii_case("nexus")
            && identity
                .provider_game_id
                .as_deref()
                .is_some_and(|game| game.eq_ignore_ascii_case("enshrouded"))
    });

    // Work with the original archive paths, not logical_path: the generic
    // scanner can strip a sole `mods` directory as an archive wrapper.
    let mut roots = files
        .iter()
        .filter(|file| {
            ROOT_PROXIES
                .iter()
                .any(|proxy| basename(&file.path).eq_ignore_ascii_case(proxy))
        })
        .map(|file| archive_path_parent(&file.path))
        .filter(|parent| {
            !parent.split('/').any(|part| {
                matches!(
                    part.to_ascii_lowercase().as_str(),
                    "mods" | "greeneye" | "aldrich-modmenu"
                )
            }) && !files.iter().any(|file| {
                basename(&file.path).eq_ignore_ascii_case("mod.json")
                    && archive_path_parent(&file.path).eq_ignore_ascii_case(parent)
            })
        })
        .collect::<Vec<_>>();
    roots.sort_by_key(|root| root.to_ascii_lowercase());
    roots.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    let all_roots = roots.clone();
    roots.retain(|root| {
        !all_roots
            .iter()
            .any(|other| !root.eq_ignore_ascii_case(other) && archive_path_is_within(root, other))
    });

    if roots.len() == 1 {
        let root = &roots[0];
        if files.iter().any(|file| {
            archive_path_parent(&file.path).eq_ignore_ascii_case(root)
                && ROOT_PROXIES
                    .iter()
                    .any(|proxy| basename(&file.path).eq_ignore_ascii_case(proxy))
                && !enshrouded_loaders::native_member_is_x64_dll(scanned, &file.path)
        }) {
            return None;
        }
        let relative_paths = files
            .iter()
            .map(|file| archive_path_relative_to(&file.path, root))
            .collect::<Option<Vec<_>>>()?;
        if !trusted_game_source && !recognizable_root_package(&relative_paths) {
            return None;
        }
        if relative_paths.iter().any(|path| foreign_game_payload(path)) {
            return None;
        }
        let mappings = files
            .iter()
            .zip(&relative_paths)
            .map(|(file, relative)| {
                mapping(
                    &file.path,
                    "game",
                    relative,
                    "Complete Enshrouded root package and companion files.",
                )
            })
            .collect();
        let dependencies = mod_dependencies(scanned, profile, true)?;
        return Some(install_plan(mappings, dependencies));
    }
    // Different proxy roots mean different client/server/optional packages.
    // They must be resolved as variants, not flattened into one game root.
    if !roots.is_empty() {
        return None;
    }

    // A package already containing `mods/<id>/mod.json` retains that tree.
    let manifests = module_manifests(&files);
    if manifests.is_empty() {
        return None;
    }
    let mut explicit_roots = Vec::new();
    for manifest in &manifests {
        let parts = normalize_archive_path(&manifest.path)
            .split('/')
            .map(str::to_string)
            .collect::<Vec<_>>();
        if let Some(index) = parts
            .iter()
            .position(|part| part.eq_ignore_ascii_case("mods"))
        {
            // Enshrouded manifests live exactly one mod directory below mods.
            if parts.len() != index + 3 {
                return None;
            }
            explicit_roots.push(parts[..index].join("/"));
        }
    }
    explicit_roots.sort_by_key(|root| root.to_ascii_lowercase());
    explicit_roots.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    if !explicit_roots.is_empty() {
        if explicit_roots.len() != 1 || explicit_roots.len() > manifests.len() {
            return None;
        }
        let prefix = &explicit_roots[0];
        let relative_paths = files
            .iter()
            .map(|file| archive_path_relative_to(&file.path, prefix))
            .collect::<Option<Vec<_>>>()?;
        if relative_paths.iter().any(|path| {
            !path.to_ascii_lowercase().starts_with("mods/") || foreign_game_payload(path)
        }) {
            return None;
        }
        let mappings = files
            .iter()
            .zip(&relative_paths)
            .map(|(file, relative)| {
                mapping(
                    &file.path,
                    "game",
                    relative,
                    "Enshrouded mod manifest, scripts and complete companion tree.",
                )
            })
            .collect::<Vec<_>>();
        let dependencies = mod_dependencies(scanned, profile, false)?;
        return Some(install_plan(mappings, dependencies));
    }

    // Bare mod folders are common on Nexus. Mount each manifest's own complete
    // subtree in mods rather than putting src/ or mod.json at the game root.
    let mut mappings = Vec::with_capacity(files.len());
    let mut destinations = HashSet::new();
    let mut module_roots = Vec::new();
    for manifest in manifests {
        let source_root = archive_path_parent(&manifest.path);
        if module_roots.iter().any(|(root, _): &(String, String)| {
            archive_path_is_within(&source_root, root) || archive_path_is_within(root, &source_root)
        }) {
            return None;
        }
        let named_folder = basename(&source_root);
        let folder = (!source_root.is_empty() && valid_mod_folder(&named_folder))
            .then_some(named_folder)
            .or_else(|| manifest_folder(scanned, manifest))
            .or_else(|| {
                let folder = archive_stem(&scanned.archive_name);
                valid_mod_folder(&folder).then_some(folder)
            })?;
        if !destinations.insert(folder.to_ascii_lowercase()) {
            return None;
        }
        module_roots.push((source_root, folder));
    }
    for file in &files {
        let (root, folder) = module_roots
            .iter()
            .find(|(root, _)| archive_path_is_within(&file.path, root))?;
        let relative = archive_path_relative_to(&file.path, root)?;
        if foreign_game_payload(&relative) {
            return None;
        }
        mappings.push(mapping(
            &file.path,
            "game",
            &format!("mods/{folder}/{relative}"),
            "Unwrapped Enshrouded mod manifest and its complete payload.",
        ));
    }
    let dependencies = mod_dependencies(scanned, profile, false)?;
    Some(install_plan(mappings, dependencies))
}

fn install_plan(mappings: Vec<InstallMapping>, dependencies: Vec<DependencySpec>) -> InstallPlan {
    InstallPlan {
        adapter_id: "enshrouded".to_string(),
        adapter_name: "Enshrouded".to_string(),
        confidence: 0.98,
        summary: format!(
            "Install {} Enshrouded package file(s), preserving their complete layout.",
            mappings.len()
        ),
        mappings,
        dependencies,
        warnings: Vec::new(),
        requires_confirmation: false,
    }
}

fn mod_dependencies(
    scanned: &ScannedArchive,
    profile: &GameProfile,
    native_host: bool,
) -> Option<Vec<DependencySpec>> {
    let mut dependencies = Vec::new();
    let files = installable_files(&scanned.entries);
    for manifest in module_manifests(&files) {
        let dependency =
            match enshrouded_loaders::classify_manifest(scanned, &manifest.path).ok()? {
                enshrouded_loaders::ManifestLoader::Eml => {
                    known_runtime_dependency(profile, "enshrouded-eml")
                }
                enshrouded_loaders::ManifestLoader::Shroudtopia => {
                    enshrouded_loaders::native_mod_dependency(scanned, &manifest.path)?
                }
                // These mods need their accompanying root proxy or an explicit
                // native host. Dropping a DLL into mods does not load it.
                enshrouded_loaders::ManifestLoader::Flame
                | enshrouded_loaders::ManifestLoader::UnknownNative => {
                    if native_host {
                        continue;
                    }
                    return None;
                }
            };
        append_unique_dependencies(&mut dependencies, [dependency]);
    }
    Some(dependencies)
}

fn module_manifests(files: &[ArchiveEntry]) -> Vec<&ArchiveEntry> {
    let mut candidates = files
        .iter()
        .filter(|file| basename(&file.path).eq_ignore_ascii_case("mod.json"))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|file| file.path.matches('/').count());
    let mut manifests: Vec<&ArchiveEntry> = Vec::new();
    for candidate in candidates {
        if !manifests.iter().any(|manifest| {
            archive_path_is_within(&candidate.path, &archive_path_parent(&manifest.path))
        }) {
            manifests.push(candidate);
        }
    }
    manifests
}

fn recognizable_root_package(paths: &[String]) -> bool {
    paths.iter().any(|path| {
        let lower = path.to_ascii_lowercase();
        lower == "spawntheworld.ini"
            || lower == "chainloader.ini"
            || lower == "shroudtopia.dll"
            || lower == "shroudtopia.json"
            || lower.starts_with("greeneye/")
            || lower.starts_with("aldrich-modmenu/")
            || lower == "mods/creative/creative.dll"
    })
}

fn forbidden_payload(path: &str) -> bool {
    let path = normalize_archive_path(path).to_ascii_lowercase();
    path.ends_with(".exe")
        || path.ends_with(".msi")
        || path.ends_with(".bat")
        || path.ends_with(".cmd")
        || path.ends_with(".ps1")
        || path.ends_with(".kfc")
        || path.ends_with(".kfc_resources")
        || path.ends_with(".ksd")
        || path.starts_with(".mod-garrison/")
        || path.contains("/.mod-garrison/")
        || path.contains("/savegame/")
        || path.starts_with("savegame/")
}

fn foreign_game_payload(path: &str) -> bool {
    let lower = normalize_archive_path(path).to_ascii_lowercase();
    let components = lower.split('/').collect::<Vec<_>>();
    components.iter().any(|component| {
        matches!(
            *component,
            "bepinex" | "ue4ss" | "reframework" | "natives" | "doorstop_libs" | "fomod"
        )
    }) || lower.ends_with(".pak")
        || lower.ends_with(".utoc")
        || lower.ends_with(".ucas")
        || lower.ends_with(".as")
        || lower.ends_with(".ct")
        || basename(&lower).starts_with("enshrouded_") && lower.ends_with(".dat")
}

fn valid_mod_folder(folder: &str) -> bool {
    !folder.is_empty()
        && folder.len() <= 128
        && !folder.contains(['/', '\\'])
        && validate_archive_relative_path(folder).is_ok()
}

fn manifest_folder(scanned: &ScannedArchive, manifest: &ArchiveEntry) -> Option<String> {
    // Reading bounded manifest data is optional: named source folders remain a
    // deterministic fallback when only an analysis listing is available.
    if manifest.size > MAX_CONFIG_READ_BYTES {
        return None;
    }
    let source = Path::new(&scanned.archive_path);
    let contents = if source.is_dir() {
        let path = safe_join(source, &manifest.path).ok()?;
        validate_deployment_containment(source, &path).ok()?;
        let metadata = fs::metadata(&path).ok()?;
        if metadata.len() > MAX_CONFIG_READ_BYTES {
            return None;
        }
        fs::read(path).ok()?
    } else {
        let file = File::open(source).ok()?;
        let mut zip = ZipArchive::new(file).ok()?;
        let index = enshrouded_loaders::zip_member_index(&mut zip, &manifest.path).ok()?;
        let mut member = zip.by_index(index).ok()?;
        if member.size() > MAX_CONFIG_READ_BYTES {
            return None;
        }
        let mut bytes = Vec::new();
        member
            .by_ref()
            .take(MAX_CONFIG_READ_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_CONFIG_READ_BYTES {
            return None;
        }
        bytes
    };
    let value: serde_json::Value = serde_json::from_slice(&contents).ok()?;
    ["id", "name"].iter().find_map(|key| {
        let name = value.get(key)?.as_str()?.trim();
        valid_mod_folder(name).then(|| name.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        profile: GameProfile,
    }

    impl Fixture {
        fn new(server: bool) -> Self {
            let root = std::env::temp_dir()
                .join(format!("mod-garrison-enshrouded-test-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            fs::write(
                root.join(if server {
                    "enshrouded_server.exe"
                } else {
                    "enshrouded.exe"
                }),
                b"test executable marker",
            )
            .unwrap();
            let profile = GameProfile {
                id: "enshrouded-test".to_string(),
                name: "Enshrouded".to_string(),
                game_path: root.to_string_lossy().to_string(),
                game_id: Some("enshrouded".to_string()),
                steam_app_id: Some("1203620".to_string()),
                engine: "enshrouded".to_string(),
                loader: "none".to_string(),
                setup_status: "ready".to_string(),
                setup_warnings: Vec::new(),
                setup_updated_at: None,
                mods_enabled: true,
                created_at: now_string(),
                updated_at: now_string(),
            };
            Self { root, profile }
        }

        fn package(&self, paths: &[&str]) -> ScannedArchive {
            let source = self.root.join(format!("package-{}", Uuid::new_v4()));
            fs::create_dir_all(&source).unwrap();
            for path in paths {
                if validate_archive_relative_path(path).is_err() {
                    continue;
                }
                let destination = source.join(path);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                let content = if basename(path).eq_ignore_ascii_case("mod.json") {
                    br#"{"id":"test-mod","scripts":{"main":"src/mod.lua"}}"#.to_vec()
                } else if path.ends_with("minimap_mod.dll") {
                    enshrouded_loaders::native_dll_fixture(&["CreateModInstance"])
                } else if path.ends_with(".dll") {
                    enshrouded_loaders::native_dll_fixture(&["DllMain"])
                } else {
                    b"fixture".to_vec()
                };
                fs::write(destination, content).unwrap();
            }
            let mut scanned = package(paths);
            scanned.archive_path = source.to_string_lossy().to_string();
            scanned
        }

        fn nexus_package(&self, paths: &[&str]) -> ScannedArchive {
            let mut scanned = self.package(paths);
            scanned.package_identity = Some(provider_source_identity(
                "nexus",
                "nexus:enshrouded/231".to_string(),
                Some("1.0.0".to_string()),
                Some("enshrouded".to_string()),
                "Verified Nexus catalogue",
            ));
            scanned
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn package(paths: &[&str]) -> ScannedArchive {
        ScannedArchive {
            archive_path: "not-on-disk.zip".to_string(),
            archive_name: "Fog.zip".to_string(),
            entries: paths
                .iter()
                .map(|path| ArchiveEntry {
                    path: path.to_string(),
                    logical_path: path.to_string(),
                    size: 1,
                    is_directory: false,
                })
                .collect(),
            manifest: None,
            package_identity: None,
        }
    }

    fn targets(plan: &InstallPlan) -> Vec<String> {
        plan.mappings
            .iter()
            .map(|mapping| mapping.target_relative_path.clone())
            .collect()
    }

    #[test]
    fn enshrouded_creative_preserves_complete_root_payload() {
        let f = Fixture::new(false);
        let scanned = f.package(&[
            "dinput8.dll",
            "mods/creative/mod.json",
            "mods/creative/flame.json",
            "mods/creative/creative.dll",
            "mods/creative/assets/icons.bin",
        ]);
        let plan = plan(&scanned, &f.profile).unwrap();
        assert_eq!(targets(&plan).len(), 5);
        assert!(plan.dependencies.is_empty());
        assert!(targets(&plan).contains(&"mods/creative/flame.json".to_string()));
        assert!(!f.root.join("mods").exists());
    }

    #[test]
    fn enshrouded_root_wrapper_is_removed_without_losing_companions() {
        let f = Fixture::new(false);
        let scanned = f.nexus_package(&[
            "Release v1/dinput8.dll",
            "Release v1/StatSliders.ini",
            "Release v1/data/font.rgba",
        ]);
        let plan = plan(&scanned, &f.profile).unwrap();
        assert_eq!(
            targets(&plan),
            ["dinput8.dll", "StatSliders.ini", "data/font.rgba"]
        );
    }

    #[test]
    fn enshrouded_mod_menu_uses_its_root_folder() {
        let f = Fixture::new(false);
        let plan = plan(
            &f.package(&[
                "POWRPROF.dll",
                "aldrich-modmenu/aldrich_modmenu_layer.dll",
                "aldrich-modmenu/aldrich-modmenu.ini",
            ]),
            &f.profile,
        )
        .unwrap();
        assert!(targets(&plan).contains(&"aldrich-modmenu/aldrich_modmenu_layer.dll".to_string()));
    }

    #[test]
    fn enshrouded_greeneye_stays_outside_mods() {
        let f = Fixture::new(false);
        let plan = plan(
            &f.package(&[
                "dbgcore.dll",
                "greeneye/unlock-map-markers/unlock_map_markers.dll",
                "greeneye/unlock-map-markers/minimap_config.ini",
            ]),
            &f.profile,
        )
        .unwrap();
        assert!(targets(&plan).iter().all(|path| !path.starts_with("mods/")));
    }

    #[test]
    fn enshrouded_shroudtopia_native_mod_keeps_companion_assets() {
        let f = Fixture::new(false);
        let plan = plan(
            &f.package(&[
                "mods/minimap_mod/mod.json",
                "mods/minimap_mod/minimap_mod.dll",
                "mods/minimap_mod/atlas.rgba",
                "mods/minimap_mod/data.bin",
            ]),
            &f.profile,
        )
        .unwrap();
        assert_eq!(plan.mappings.len(), 4);
        assert_eq!(plan.dependencies[0].id, "runtime:enshrouded-shroudtopia");
    }

    #[test]
    fn enshrouded_eml_mod_requires_loader_without_flattening_src() {
        let f = Fixture::new(false);
        let plan = plan(
            &f.package(&[
                "mods/Less Dense Shroud Fog/mod.json",
                "mods/Less Dense Shroud Fog/src/mod.lua",
            ]),
            &f.profile,
        )
        .unwrap();
        assert_eq!(plan.dependencies[0].id, "runtime:enshrouded-eml");
        assert_eq!(targets(&plan)[1], "mods/Less Dense Shroud Fog/src/mod.lua");
    }

    #[test]
    fn enshrouded_bare_mod_folder_gets_mods_prefix() {
        let f = Fixture::new(false);
        let plan = plan(
            &f.package(&["buildZonePlus/mod.json", "buildZonePlus/src/mod.lua"]),
            &f.profile,
        )
        .unwrap();
        assert_eq!(targets(&plan)[1], "mods/buildZonePlus/src/mod.lua");
    }

    #[test]
    fn enshrouded_flat_mod_uses_stable_manifest_id() {
        let f = Fixture::new(false);
        let source = f.root.join("package");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(
            source.join("mod.json"),
            br#"{"id":"stable-fog","name":"Fog Display"}"#,
        )
        .unwrap();
        fs::write(source.join("src/mod.lua"), b"return {}").unwrap();
        let scanned = scan_folder_source(&source, "Fog v1-72.zip".to_string()).unwrap();
        let plan = plan(&scanned, &f.profile).unwrap();
        assert!(targets(&plan).contains(&"mods/stable-fog/mod.json".to_string()));
    }

    #[test]
    fn enshrouded_sole_mods_top_folder_must_not_be_stripped() {
        let f = Fixture::new(false);
        let mut scanned = f.package(&["mods/Fog/mod.json", "mods/Fog/src/mod.lua"]);
        for entry in &mut scanned.entries {
            entry.logical_path = entry.path.strip_prefix("mods/").unwrap().to_string();
        }
        let plan = plan(&scanned, &f.profile).unwrap();
        assert_eq!(targets(&plan)[0], "mods/Fog/mod.json");
    }

    #[test]
    fn enshrouded_multiple_unwrapped_mods_remain_separate() {
        let f = Fixture::new(false);
        let plan = plan(
            &f.package(&[
                "Flight/mod.json",
                "Flight/src/mod.lua",
                "Fog/mod.json",
                "Fog/src/mod.lua",
            ]),
            &f.profile,
        )
        .unwrap();
        assert_eq!(targets(&plan)[0], "mods/Flight/mod.json");
        assert_eq!(targets(&plan)[2], "mods/Fog/mod.json");
    }

    #[test]
    fn enshrouded_generic_root_proxy_requires_verified_game_provenance() {
        let f = Fixture::new(false);
        let paths = ["dinput8.dll", "Other.ini"];
        assert!(plan(&f.package(&paths), &f.profile).is_none());
        assert!(plan(&f.nexus_package(&paths), &f.profile).is_some());
    }

    #[test]
    fn enshrouded_rejects_unrelated_game_payload_even_from_provenance() {
        let f = Fixture::new(false);
        assert!(plan(
            &f.nexus_package(&["dinput8.dll", "reframework/autorun/Example.lua"]),
            &f.profile,
        )
        .is_none());
    }

    #[test]
    fn enshrouded_cannot_route_to_wrong_or_unproven_game_root() {
        let mut f = Fixture::new(false);
        let scanned = f.nexus_package(&["dinput8.dll", "SpawnTheWorld.ini"]);
        f.profile.game_id = Some("re2".to_string());
        assert!(plan(&scanned, &f.profile).is_none());
        f.profile.game_id = Some("enshrouded".to_string());
        fs::remove_file(f.root.join("enshrouded.exe")).unwrap();
        assert!(plan(&scanned, &f.profile).is_none());
    }

    #[test]
    fn enshrouded_dedicated_package_uses_existing_server_root() {
        let f = Fixture::new(true);
        let plan = plan(
            &f.package(&[
                "powrprof.dll",
                "mods/creative/creative.dll",
                "mods/creative/mod.json",
            ]),
            &f.profile,
        )
        .unwrap();
        assert!(targets(&plan).contains(&"powrprof.dll".to_string()));
    }

    #[test]
    fn enshrouded_rejects_mixed_client_and_server_proxy_roots() {
        let f = Fixture::new(false);
        assert!(plan(
            &f.nexus_package(&["Client/dinput8.dll", "Server/powrprof.dll"]),
            &f.profile,
        )
        .is_none());
    }

    #[test]
    fn enshrouded_rejects_tool_executables_and_base_game_patches() {
        let f = Fixture::new(false);
        for unsupported in ["Tool.exe", "enshrouded.kfc", "character0.ksd", "script.cmd"] {
            assert!(plan(&f.nexus_package(&["dinput8.dll", unsupported]), &f.profile).is_none());
        }
    }

    #[test]
    fn enshrouded_rejects_forged_internal_runtime_proof() {
        let f = Fixture::new(false);
        assert!(plan(
            &f.nexus_package(&["dbghelp.dll", ".mod-garrison/enshrouded-eml.json"]),
            &f.profile,
        )
        .is_none());
    }

    #[test]
    fn enshrouded_rejects_unsafe_source_paths() {
        let f = Fixture::new(false);
        assert!(plan(
            &f.nexus_package(&["dinput8.dll", "../outside.ini"]),
            &f.profile,
        )
        .is_none());
    }

    #[test]
    fn enshrouded_unknown_native_manifest_never_guesses_eml_or_shroudtopia() {
        let f = Fixture::new(false);
        let scanned = f.package(&["mods/Example/mod.json", "mods/Example/example.dll"]);
        assert!(plan(&scanned, &f.profile).is_none());
    }

    #[test]
    fn enshrouded_malformed_mod_manifest_is_not_installed() {
        let f = Fixture::new(false);
        let scanned = f.package(&["mods/Example/mod.json", "mods/Example/src/mod.lua"]);
        fs::write(
            Path::new(&scanned.archive_path).join("mods/Example/mod.json"),
            b"{not valid JSON}",
        )
        .unwrap();
        assert!(plan(&scanned, &f.profile).is_none());
    }

    #[test]
    fn enshrouded_manifest_display_name_does_not_rename_supplied_mod_directory() {
        let f = Fixture::new(false);
        let scanned = f.package(&["ActualFolder/mod.json", "ActualFolder/src/mod.lua"]);
        fs::write(
            Path::new(&scanned.archive_path).join("ActualFolder/mod.json"),
            br#"{"id":"actual-id","name":"Fancy Display Name"}"#,
        )
        .unwrap();
        let plan = plan(&scanned, &f.profile).unwrap();
        assert_eq!(targets(&plan)[0], "mods/ActualFolder/mod.json");
    }

    #[test]
    fn enshrouded_incomplete_flame_plugin_without_root_proxy_is_not_successful() {
        let f = Fixture::new(false);
        let scanned = f.package(&[
            "mods/creative/mod.json",
            "mods/creative/flame.json",
            "mods/creative/creative.dll",
        ]);
        assert!(plan(&scanned, &f.profile).is_none());
    }

    #[test]
    fn enshrouded_duplicate_unwrapped_mod_folder_targets_are_rejected() {
        let f = Fixture::new(false);
        let scanned = f.package(&[
            "ReleaseA/Fog/mod.json",
            "ReleaseA/Fog/src/mod.lua",
            "ReleaseB/Fog/mod.json",
            "ReleaseB/Fog/src/mod.lua",
        ]);
        assert!(plan(&scanned, &f.profile).is_none());
    }

    #[test]
    fn enshrouded_other_game_provider_identity_cannot_authorize_a_generic_proxy() {
        let f = Fixture::new(false);
        let mut scanned = f.nexus_package(&["dinput8.dll", "Example.ini"]);
        scanned.package_identity.as_mut().unwrap().provider_game_id = Some("re2".to_string());
        assert!(plan(&scanned, &f.profile).is_none());
    }

    #[test]
    fn enshrouded_asset_named_mod_json_is_not_a_second_loader_manifest() {
        let f = Fixture::new(false);
        let scanned = f.package(&[
            "mods/Fog/mod.json",
            "mods/Fog/src/mod.lua",
            "mods/Fog/assets/mod.json",
        ]);
        fs::write(
            Path::new(&scanned.archive_path).join("mods/Fog/assets/mod.json"),
            b"application data, not a loader manifest",
        )
        .unwrap();
        let plan = plan(&scanned, &f.profile).unwrap();
        assert_eq!(plan.mappings.len(), 3);
        assert_eq!(plan.dependencies.len(), 1);
    }

    #[test]
    fn enshrouded_nested_proxy_companion_does_not_become_another_game_root() {
        let f = Fixture::new(false);
        let scanned = f.nexus_package(&[
            "Release/dinput8.dll",
            "Release/chainloader.ini",
            "Release/mods/XHL/mod.json",
            "Release/mods/XHL/dinput8.dll",
        ]);
        let plan = plan(&scanned, &f.profile).unwrap();
        assert_eq!(targets(&plan)[3], "mods/XHL/dinput8.dll");
    }

    #[test]
    fn enshrouded_malformed_root_proxy_is_not_reported_installed() {
        let f = Fixture::new(false);
        let scanned = f.nexus_package(&["dinput8.dll", "SpawnTheWorld.ini"]);
        fs::write(
            Path::new(&scanned.archive_path).join("dinput8.dll"),
            b"not actually a DLL",
        )
        .unwrap();
        assert!(plan(&scanned, &f.profile).is_none());
    }

    #[test]
    fn enshrouded_root_native_host_does_not_hide_an_eml_script_requirement() {
        let f = Fixture::new(false);
        let scanned = f.nexus_package(&[
            "dinput8.dll",
            "NativeHost.ini",
            "mods/Fog/mod.json",
            "mods/Fog/src/mod.lua",
        ]);
        let plan = plan(&scanned, &f.profile).unwrap();
        assert_eq!(plan.dependencies.len(), 1);
        assert_eq!(plan.dependencies[0].id, "runtime:enshrouded-eml");
    }
}
