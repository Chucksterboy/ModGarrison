//! Shared BepInEx package regressions, independent of any particular game.
//! Fixtures contain inert bytes and never execute loaders or contact providers.

use super::*;

struct Fixture {
    root: PathBuf,
    store: PathBuf,
    profile: GameProfile,
}

impl Fixture {
    fn new(il2cpp: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "mod-garrison-bepinex-regression-{}",
            Uuid::new_v4()
        ));
        let game = root.join("game");
        let store = root.join("store");
        fs::create_dir_all(game.join("FutureGame_Data")).unwrap();
        fs::write(game.join("FutureGame.exe"), b"inert executable marker").unwrap();
        let profile: GameProfile = serde_json::from_value(serde_json::json!({
            "id": "generic-bepinex-regression", "name": "Future Unity Game", "gamePath": game,
            "gameId": null, "steamAppId": null,
            "engine": if il2cpp { "unity-il2cpp" } else { "unity-mono" },
            "loader": if il2cpp { "bepinex-il2cpp" } else { "bepinex" },
            "createdAt": "", "updatedAt": ""
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

    fn analyze(&self, source: &Path) -> ArchiveAnalysis {
        analyze_scanned_archive(
            scan_import_source(&self.store, source).unwrap(),
            &self.profile,
        )
    }

    fn install(&self, source: &Path, plan: &InstallPlan) -> InstallResult {
        let mut visited = HashSet::new();
        install_archive_impl_with_metadata(
            &self.store,
            &self.profile,
            &source.to_string_lossy(),
            plan,
            InstallOptions {
                metadata: InstallMetadata::default(),
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
                    .starts_with("mod-garrison-bepinex-regression-")
            })
        {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

const CANONICAL_PAYLOADS: &[(&str, &[u8])] = &[
    ("plugins/Library/Library.dll", b"inert plugin"),
    ("plugins/Library/data/options.json", b"{}"),
    ("core/LibrarySupport.dll", b"inert core support"),
    ("patchers/Preloader/Preloader.dll", b"inert patcher"),
    ("patchers/Preloader/data/schema.json", b"{}"),
    ("monomod/Fixture.mm.dll", b"inert monomod patch"),
    ("config/Fixture/extra.json", b"{}"),
];

#[test]
fn canonical_bepinex_subfolders_and_companions_work_for_mono_and_il2cpp() {
    for il2cpp in [false, true] {
        let fixture = Fixture::new(il2cpp);
        let source = fixture.zip("canonical.zip", CANONICAL_PAYLOADS);
        let analysis = fixture.analyze(&source);
        assert_eq!(analysis.compatibility.status, "compatible");
        let plan = analysis.recommended_plan.unwrap();
        assert_eq!(plan.adapter_id, "bepinex");
        assert_eq!(plan.mappings.len(), CANONICAL_PAYLOADS.len());
        for (path, _) in CANONICAL_PAYLOADS {
            assert!(
                plan.mappings.iter().any(|mapping| {
                    mapping.source_path == *path
                        && mapping.target_relative_path == format!("BepInEx/{path}")
                }),
                "Missing canonical mapping for {path}"
            );
        }
        assert!(plan.dependencies.iter().any(|dependency| {
            dependency.id
                == if il2cpp {
                    "runtime:bepinex-il2cpp"
                } else {
                    "runtime:bepinex"
                }
        }));
    }
}

#[test]
fn single_semantic_directory_is_not_discarded_as_an_archive_wrapper() {
    for directory in ["core", "patchers", "monomod", "plugins", "config"] {
        let fixture = Fixture::new(false);
        let member = format!(
            "{directory}/Nested/Fixture.{}",
            if directory == "config" { "json" } else { "dll" }
        );
        let source = fixture.zip("single-directory.zip", &[(&member, b"inert assembly")]);
        let analysis = fixture.analyze(&source);
        let plan = analysis.recommended_plan.unwrap();
        assert_eq!(plan.mappings.len(), 1);
        assert_eq!(
            plan.mappings[0].target_relative_path,
            format!("BepInEx/{member}")
        );
        assert_ne!(
            plan.mappings[0].target_relative_path,
            "BepInEx/plugins/Fixture.dll"
        );
    }
}

#[test]
fn wrapped_backslash_patcher_layout_preserves_stage_and_data() {
    for il2cpp in [false, true] {
        let fixture = Fixture::new(il2cpp);
        let source = fixture.zip(
            "wrapped.zip",
            &[
                (r"Wrapper\PATCHERS\Group\Patch.dll", b"inert patcher"),
                (r"Wrapper\PATCHERS\Group\Data\schema.json", b"{}"),
            ],
        );
        let plan = fixture.analyze(&source).recommended_plan.unwrap();
        assert_eq!(plan.mappings.len(), 2);
        assert!(plan
            .mappings
            .iter()
            .any(|mapping| { mapping.target_relative_path == "BepInEx/PATCHERS/Group/Patch.dll" }));
        assert!(plan.mappings.iter().any(|mapping| {
            mapping.target_relative_path == "BepInEx/PATCHERS/Group/Data/schema.json"
        }));
    }
}

#[test]
fn canonical_bepinex_payloads_are_owned_and_removed_without_deleting_manual_files() {
    for il2cpp in [false, true] {
        let fixture = Fixture::new(il2cpp);
        let source = fixture.zip("owned-canonical.zip", CANONICAL_PAYLOADS);
        let plan = fixture.analyze(&source).recommended_plan.unwrap();
        let manual = fixture.game().join("BepInEx/patchers/Manual.dll");
        fs::create_dir_all(manual.parent().unwrap()).unwrap();
        fs::write(&manual, b"pre-existing manual assembly").unwrap();
        let result = fixture.install(&source, &plan);
        let store = read_store::<InstalledModRecord>(&installed_mods_path(&fixture.store)).unwrap();
        let record = store
            .items
            .iter()
            .find(|record| record.id == result.installed_mod_id)
            .unwrap();
        assert!(record.runtime_id.is_none());
        assert_eq!(record.files_written.len(), CANONICAL_PAYLOADS.len());
        for (path, bytes) in CANONICAL_PAYLOADS {
            let destination = fixture.game().join(format!("BepInEx/{path}"));
            assert_eq!(fs::read(&destination).unwrap(), *bytes);
            assert_eq!(
                record
                    .written_file_hashes
                    .get(&destination.to_string_lossy().to_string()),
                Some(&sha256_file(&destination).unwrap()),
            );
        }
        deactivate_mod_files_with_policy(
            &fixture.store,
            &fixture.profile,
            record,
            ModifiedFilePolicy::RequireRemovalConfirmation,
        )
        .unwrap();
        commit_removed_installed_mod_record(&fixture.store, &fixture.profile.id, &record.id)
            .unwrap();
        cleanup_install_data(&fixture.store, &fixture.profile.id, &record.id);
        for (path, _) in CANONICAL_PAYLOADS {
            assert!(!fixture.game().join(format!("BepInEx/{path}")).exists());
        }
        assert_eq!(fs::read(&manual).unwrap(), b"pre-existing manual assembly");
    }
}

#[test]
fn unknown_companion_directories_are_not_guessed_or_partially_installed() {
    let fixture = Fixture::new(false);
    let source = fixture.zip(
        "unknown-data.zip",
        &[
            ("patchers/Fixture.dll", b"inert patcher"),
            (
                "UnknownDestination/important.bin",
                b"unclassified companion",
            ),
        ],
    );
    let analysis = fixture.analyze(&source);
    assert_eq!(analysis.compatibility.status, "blocked");
    assert!(analysis.recommended_plan.is_none());
    assert!(analysis
        .compatibility
        .reason
        .contains("UnknownDestination/important.bin"));
    assert!(!fixture.game().join("BepInEx/patchers/Fixture.dll").exists());
}

#[test]
fn canonical_core_or_patcher_names_do_not_authorize_a_non_bepinex_host() {
    let mut fixture = Fixture::new(false);
    fixture.profile.engine = "unreal".to_string();
    fixture.profile.loader = "none".to_string();
    for member in [
        "core/Fixture.dll",
        "core/data.json",
        "patchers/Fixture.dll",
        "monomod/Fixture.mm.dll",
        "config/Fixture.json",
    ] {
        let source = fixture.zip("unsupported-host.zip", &[(member, b"inert assembly")]);
        let analysis = fixture.analyze(&source);
        assert_eq!(analysis.compatibility.status, "blocked");
        assert!(analysis.recommended_plan.is_none());
    }
}

#[test]
fn loose_plugins_keep_the_profile_runtime_including_game_specific_overrides() {
    for variant in ["mono", "il2cpp", "valheim"] {
        let mut fixture = Fixture::new(variant == "il2cpp");
        if variant == "valheim" {
            fixture.profile.game_id = Some("valheim".to_string());
        }
        let source = fixture.zip("loose-plugin.zip", &[("Fixture.dll", b"inert plugin")]);
        let plan = fixture.analyze(&source).recommended_plan.unwrap();
        assert_eq!(plan.mappings.len(), 1);
        assert_eq!(
            plan.mappings[0].target_relative_path,
            "BepInEx/plugins/Fixture.dll"
        );
        let dependency = &plan.dependencies[0];
        match variant {
            "mono" => assert_eq!(
                dependency.source.as_deref(),
                Some("github-release:BepInEx/BepInEx#BepInEx_win_x64_*.zip")
            ),
            "il2cpp" => assert_eq!(
                dependency.source.as_deref(),
                Some("bepinbuilds:bepinex_be#BepInEx-Unity.IL2CPP-win-x64-*.zip")
            ),
            "valheim" => assert_eq!(
                dependency.source.as_deref(),
                Some("thunderstore:denikson/BepInExPack_Valheim")
            ),
            _ => unreachable!(),
        }
    }
}

#[test]
fn canonical_patchers_preserve_distinct_client_and_server_destinations() {
    let fixture = Fixture::new(false);
    fs::create_dir_all(fixture.game().join("Server/Server_Data")).unwrap();
    fs::write(
        fixture.game().join("Server/Server.exe"),
        b"inert server marker",
    )
    .unwrap();
    let source = fixture.zip(
        "variants.zip",
        &[
            ("Client/patchers/Fixture.dll", b"inert client patcher"),
            ("Server/patchers/Fixture.dll", b"inert server patcher"),
        ],
    );
    let plan = fixture.analyze(&source).recommended_plan.unwrap();
    assert_eq!(plan.mappings.len(), 2);
    assert!(plan.mappings.iter().any(|mapping| {
        mapping.source_path == "Client/patchers/Fixture.dll"
            && mapping.target_relative_path == "BepInEx/patchers/Fixture.dll"
    }));
    assert!(plan.mappings.iter().any(|mapping| {
        mapping.source_path == "Server/patchers/Fixture.dll"
            && mapping.target_relative_path == "Server/BepInEx/patchers/Fixture.dll"
    }));
}

#[test]
fn wrapped_complete_runtime_payload_keeps_all_canonical_directories() {
    for il2cpp in [false, true] {
        let fixture = Fixture::new(il2cpp);
        let core = if il2cpp {
            "Wrapper/BepInEx/core/BepInEx.Unity.IL2CPP.dll"
        } else {
            "Wrapper/BepInEx/core/BepInEx.dll"
        };
        let source = fixture.zip(
            "runtime.zip",
            &[
                (core, b"inert runtime"),
                ("Wrapper/winhttp.dll", b"inert bootstrap"),
                ("Wrapper/doorstop_config.ini", b"enabled=true\n"),
                ("Wrapper/.doorstop_version", b"fixture"),
                ("Wrapper/BepInEx/patchers/Fixture.dll", b"inert patcher"),
                (
                    "Wrapper/BepInEx/monomod/Fixture.mm.dll",
                    b"inert monomod patch",
                ),
                ("Wrapper/BepInEx/config/Fixture.cfg", b"fixture=true\n"),
            ],
        );
        let plan = fixture.analyze(&source).recommended_plan.unwrap();
        assert_eq!(plan.mappings.len(), 7);
        assert_eq!(
            runtime_supplied_by_plan(&fixture.profile, &plan).as_deref(),
            Some(if il2cpp { "bepinex-il2cpp" } else { "bepinex" })
        );
        for mapping in &plan.mappings {
            assert_eq!(
                mapping.target_relative_path,
                mapping.source_path.strip_prefix("Wrapper/").unwrap()
            );
        }
    }
}
