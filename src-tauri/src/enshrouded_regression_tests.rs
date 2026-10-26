//! Offline regressions for Enshrouded installation dataflow and file lifecycle.
//! Fixtures reproduce package structure, not third-party mod implementations.

use super::*;

struct Fixture {
    root: PathBuf,
    store: PathBuf,
    profile: GameProfile,
}

impl Fixture {
    fn new(server: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "mod-garrison-enshrouded-regression-{}",
            Uuid::new_v4()
        ));
        let game = root.join("game");
        let store = root.join("store");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join(if server {
                "enshrouded_server.exe"
            } else {
                "enshrouded.exe"
            }),
            b"offline fixture executable marker",
        )
        .unwrap();
        for filename in [
            "enshrouded.kfc",
            "enshrouded.kfc_resources",
            "enshrouded_000.dat",
        ] {
            fs::write(
                game.join(filename),
                b"unchanged inert vanilla fixture asset",
            )
            .unwrap();
        }
        let profile: GameProfile = serde_json::from_value(serde_json::json!({
            "id": "enshrouded-regression", "name": "Enshrouded", "gamePath": game,
            "gameId": "enshrouded", "steamAppId": if server { "2278520" } else { "1203620" },
            "engine": "enshrouded", "loader": "none", "createdAt": "", "updatedAt": ""
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

    fn source(&self, version: &str) -> PathBuf {
        let source = self.root.join(format!("source-{version}"));
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(
            source.join("mod.json"),
            serde_json::to_vec(&serde_json::json!({
                "id":"example", "version":version, "caps":["patch"]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(source.join("src/mod.lua"), b"-- inert fixture\n").unwrap();
        fs::write(
            source.join("settings.ini"),
            format!("setting=default-{version}\n"),
        )
        .unwrap();
        source
    }

    fn install_fixture(&self, source: &Path, version: &str) -> InstallResult {
        let plan = ordinary_plan();
        let mut visited = HashSet::new();
        // This test targets journaling/replacement, not network provisioning.
        // No loader is downloaded or executed by the offline fixture.
        install_archive_impl_with_metadata(
            &self.store,
            &self.profile,
            &source.to_string_lossy(),
            &plan,
            InstallOptions {
                metadata: InstallMetadata {
                    package_id: Some("nexus:enshrouded/999999".to_string()),
                    package_provider: Some("nexus".to_string()),
                    package_version: Some(version.to_string()),
                    ..InstallMetadata::default()
                },
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
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn ordinary_plan() -> InstallPlan {
    InstallPlan {
        adapter_id: "enshrouded".to_string(),
        adapter_name: "Enshrouded fixture".to_string(),
        confidence: 1.0,
        summary: "Offline EML package fixture".to_string(),
        mappings: ["mod.json", "src/mod.lua", "settings.ini"]
            .iter()
            .map(|path| {
                mapping(
                    path,
                    "game",
                    &format!("mods/example/{path}"),
                    "Fixture payload.",
                )
            })
            .collect(),
        dependencies: Vec::new(),
        warnings: Vec::new(),
        requires_confirmation: false,
    }
}

fn record(profile: &GameProfile, plan: InstallPlan) -> InstalledModRecord {
    serde_json::from_value(serde_json::json!({
        "id":"fixture-record", "profileId":profile.id, "archivePath":"unused.zip",
        "archiveName":"unused.zip", "adapterId":"enshrouded", "summary":"Fixture",
        "installedAt":"", "filesWritten":[], "backupsWritten":[], "dependencies":[],
        "packageId":"nexus:enshrouded/999999", "plan":plan
    }))
    .unwrap()
}

fn nexus_file(id: u64, name: &str, file_name: &str) -> NexusFileRecord {
    serde_json::from_value(serde_json::json!({
        "file_id":id, "name":name, "file_name":file_name,
        "category_name":"MAIN", "version":"1.0.0", "uploaded_timestamp":id
    }))
    .unwrap()
}

#[test]
fn max_stack_patch_archive_backslash_members_get_complete_eml_plan() {
    let fixture = Fixture::new(false);
    // Structural fixture of Nexus Max Stack #111: no author scripts/assets are
    // copied. The raw backslashes are intentional and reproduce its ZIP layout.
    let archive = fixture.zip(
        "max-stack.zip",
        &[
            (r"max-stack\config.txt", b"max_stack=100\n"),
            (r"max-stack\items.txt", b"fixture-item\n"),
            (
                r"max-stack\mod.json",
                br#"{"id":"max-stack","caps":["patch"]}"#,
            ),
            (r"max-stack\src\mod.lua", b"-- inert structural fixture\n"),
        ],
    );
    let scanned = scan_zip_archive(&archive).unwrap();
    let analysis = analyze_scanned_archive_with_identity(
        scanned,
        &fixture.profile,
        Some(provider_source_identity(
            "nexus",
            "nexus:enshrouded/111".to_string(),
            Some("1.0".to_string()),
            Some("enshrouded".to_string()),
            "Offline Nexus fixture",
        )),
    );
    let plan = analysis
        .recommended_plan
        .expect("Max Stack requires an EML route");
    assert_eq!(plan.adapter_id, "enshrouded");
    let targets = plan
        .mappings
        .iter()
        .map(|mapping| mapping.target_relative_path.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(
        targets,
        HashSet::from([
            "mods/max-stack/config.txt",
            "mods/max-stack/items.txt",
            "mods/max-stack/mod.json",
            "mods/max-stack/src/mod.lua",
        ])
    );
    assert!(plan
        .dependencies
        .iter()
        .any(|dependency| dependency.id == "runtime:enshrouded-eml" && dependency.required));
    assert!(!Path::new(&fixture.profile.game_path).join("mods").exists());
}

#[test]
fn max_stack_cogwheel_settings_edit_owned_text_configs_and_refresh_receipts() {
    let fixture = Fixture::new(false);
    // Match the four public settings and unusual CRCRLF format of Max Stack
    // without copying its script or its explanatory prose.
    let config = concat!(
        "# Structural test settings\r\r\n",
        "enabled=true\r\r\n",
        "\r\r\n",
        "# Stack ceiling fixture comment\r\r\n",
        "maxStackSize=999\r\r\n",
        "stackMultiplier=1.0\r\r\n",
        "patchStackablesOnly=true\r\r\n"
    );
    let items = "# Item override fixture comment\r\n# Fixture_Item=40\r\n";
    let archive = fixture.zip(
        "max-stack-settings.zip",
        &[
            (r"max-stack\config.txt", config.as_bytes()),
            (r"max-stack\items.txt", items.as_bytes()),
            (
                r"max-stack\mod.json",
                br#"{"id":"max-stack","caps":["patch"]}"#,
            ),
            (r"max-stack\src\mod.lua", b"-- inert structural fixture\n"),
        ],
    );
    let source_identity = provider_source_identity(
        "nexus",
        "nexus:enshrouded/111".to_string(),
        Some("1.0.0".to_string()),
        Some("enshrouded".to_string()),
        "Offline Nexus fixture",
    );
    let analysis = analyze_scanned_archive_with_identity(
        scan_zip_archive(&archive).unwrap(),
        &fixture.profile,
        Some(source_identity.clone()),
    );
    let plan = analysis.recommended_plan.unwrap();
    let mut visited = HashSet::new();
    let installed = install_archive_impl_with_metadata(
        &fixture.store,
        &fixture.profile,
        &archive.to_string_lossy(),
        &plan,
        InstallOptions {
            metadata: InstallMetadata {
                package_id: Some("nexus:enshrouded/111".to_string()),
                package_provider: Some("nexus".to_string()),
                package_identity: Some(source_identity),
                ..InstallMetadata::default()
            },
            resolve_dependencies: false,
            visited_dependencies: &mut visited,
            dependency_depth: 0,
        },
    )
    .unwrap();

    let records = read_store::<InstalledModRecord>(&installed_mods_path(&fixture.store))
        .unwrap()
        .items;
    // These are the same helpers used by get_mod_config_details_sync, without a
    // desktop AppHandle or any network/runtime installation in this fixture.
    let discovered = discover_profile_config_files(&fixture.profile);
    let config_paths = resolved_profile_config_files(&fixture.profile, &records, &discovered)
        .remove(&installed.installed_mod_id)
        .unwrap();
    let details = config_paths
        .iter()
        .map(|path| read_mod_config_file(&fixture.store, &fixture.profile, path).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        details
            .iter()
            .map(|detail| detail.file_name.as_str())
            .collect::<HashSet<_>>(),
        HashSet::from(["config.txt", "items.txt"])
    );
    let settings = details
        .iter()
        .find(|detail| detail.file_name == "config.txt")
        .unwrap();
    assert!(settings.warning.is_none());
    assert_eq!(settings.entries.len(), 4);
    let values = settings
        .entries
        .iter()
        .map(|entry| (entry.key.as_str(), entry.value.as_str()))
        .collect::<HashMap<_, _>>();
    assert!(values["enabled"].parse::<bool>().unwrap());
    assert!(values["patchStackablesOnly"].parse::<bool>().unwrap());
    assert_eq!(values["maxStackSize"].parse::<u64>().unwrap(), 999);
    assert_eq!(values["stackMultiplier"].parse::<f64>().unwrap(), 1.0);
    let overrides = details
        .iter()
        .find(|detail| detail.file_name == "items.txt")
        .unwrap();
    assert!(overrides.entries.is_empty());

    for (file_name, key, next) in [
        ("config.txt", "enabled", "false"),
        ("config.txt", "maxStackSize", "1500"),
        ("config.txt", "stackMultiplier", "2.5"),
        ("config.txt", "patchStackablesOnly", "false"),
        ("items.txt", "Fixture_Item", "250"),
        ("items.txt", "Fixture_Item", "500"),
    ] {
        let path = Path::new(&fixture.profile.game_path)
            .join("mods/max-stack")
            .join(file_name);
        let updated = update_mod_config_value_impl(
            &fixture.store,
            &fixture.profile,
            &UpdateModConfigValueInput {
                profile_id: fixture.profile.id.clone(),
                installed_mod_id: installed.installed_mod_id.clone(),
                file_path: path.to_string_lossy().to_string(),
                section: None,
                key: key.to_string(),
                value: next.to_string(),
            },
        )
        .unwrap();
        assert_eq!(
            updated
                .entries
                .iter()
                .find(|entry| entry.key == key)
                .unwrap()
                .value,
            next
        );
        let dirty_cache: serde_json::Value = serde_json::from_slice(
            &fs::read(Path::new(&fixture.profile.game_path).join(".cache/files.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(dirty_cache["build_id"], "");
        assert_eq!(dirty_cache["files"], serde_json::json!({}));
        let stored = read_store::<InstalledModRecord>(&installed_mods_path(&fixture.store))
            .unwrap()
            .items
            .into_iter()
            .find(|record| record.id == installed.installed_mod_id)
            .unwrap();
        let receipt: InstalledModRecord = serde_json::from_slice(
            &fs::read(installed_mod_receipt_path(
                &fixture.store,
                &fixture.profile,
                &installed.installed_mod_id,
            ))
            .unwrap(),
        )
        .unwrap();
        let hash = sha256_file(&path).unwrap();
        assert_eq!(
            managed_record_hash_for_path(&stored, &path),
            Some(hash.as_str())
        );
        assert_eq!(
            managed_record_hash_for_path(&receipt, &path),
            Some(hash.as_str())
        );
    }
    let edited =
        fs::read_to_string(Path::new(&fixture.profile.game_path).join("mods/max-stack/config.txt"))
            .unwrap();
    assert!(edited.contains("# Structural test settings"));
    assert!(edited.contains("# Stack ceiling fixture comment"));
    let edited_items =
        fs::read_to_string(Path::new(&fixture.profile.game_path).join("mods/max-stack/items.txt"))
            .unwrap();
    assert!(edited_items.contains("# Item override fixture comment"));
    assert!(edited_items.contains("\r\nFixture_Item=500\r\n"));
    assert_eq!(parse_key_value_config_entries(&edited_items).len(), 1);
    for (key, value) in [
        ("bad\nkey", "1"),
        ("Fixture_Item", "0"),
        ("Other_Item", "1.5"),
        ("Other_Item", "2147483648"),
    ] {
        assert!(update_max_stack_item_override(&edited_items, None, key, value).is_err());
    }
}

#[test]
fn normalized_zip_lookup_rejects_separator_and_case_alias_collision() {
    let fixture = Fixture::new(false);
    let archive = fixture.zip(
        "collision.zip",
        &[
            (r"Example\mod.json", br#"{"id":"example"}"#),
            ("example/MOD.JSON", br#"{"id":"other"}"#),
        ],
    );
    let mut zip = ZipArchive::new(File::open(archive).unwrap()).unwrap();
    assert!(enshrouded_loaders::zip_member_index(&mut zip, "example/mod.json").is_err());
}

#[test]
fn root_native_archive_with_backslash_companion_paths_is_readable() {
    let fixture = Fixture::new(false);
    let dll = enshrouded_loaders::native_dll_fixture(&["DllMain"]);
    let archive = fixture.zip(
        "spawn.zip",
        &[
            (r"Release\dinput8.dll", dll.as_slice()),
            (r"Release\SpawnTheWorld.ini", b"Key=F12\n"),
        ],
    );
    let scanned = scan_zip_archive(&archive).unwrap();
    let plan = enshrouded::plan(&scanned, &fixture.profile).unwrap();
    assert_eq!(plan.mappings.len(), 2);
    assert!(plan
        .mappings
        .iter()
        .any(|mapping| mapping.target_relative_path == "dinput8.dll"));
}

#[test]
fn ordinary_mod_payload_never_claims_a_registered_enshrouded_loader() {
    let fixture = Fixture::new(false);
    let mut profile = fixture.profile.clone();
    // A selected/registered loader is not evidence that this new package supplies
    // it. Valid already-installed DLLs are additionally covered by the live test.
    for loader in [
        "enshrouded-eml",
        "enshrouded-shroudtopia",
        "enshrouded-proxy",
    ] {
        profile.loader = loader.to_string();
        assert!(runtime_supplied_by_plan(&profile, &ordinary_plan()).is_none());
    }
}

#[test]
fn distinct_nexus_mods_sharing_a_proxy_filename_are_not_name_duplicates() {
    let fixture = Fixture::new(false);
    let mut plan = ordinary_plan();
    plan.mappings = vec![mapping(
        "dinput8.dll",
        "game",
        "dinput8.dll",
        "Structural shared bootstrap fixture.",
    )];
    let mut prior = record(&fixture.profile, plan.clone());
    prior.package_id = Some("nexus:enshrouded/165".to_string());
    prior.package_provider = Some("nexus".to_string());
    prior.archive_name = "dinput8.dll".to_string();
    prior.display_name = Some("dinput8".to_string());
    write_store(
        &installed_mods_path(&fixture.store),
        &StoreFile {
            version: 1,
            items: vec![prior],
        },
    )
    .unwrap();
    let mut metadata = InstallMetadata {
        package_id: Some("nexus:enshrouded/214".to_string()),
        package_provider: Some("nexus".to_string()),
        ..InstallMetadata::default()
    };
    assert!(duplicate_installed_mod_reason(
        &fixture.store,
        &fixture.profile,
        &plan,
        "dinput8.dll",
        &metadata,
    )
    .unwrap()
    .is_none());
    // The authoritative Nexus identity must still prevent a second install of
    // the same mod, regardless of a generic archive/bootstrap filename.
    metadata.package_id = Some("nexus:enshrouded/165".to_string());
    assert!(duplicate_installed_mod_reason(
        &fixture.store,
        &fixture.profile,
        &plan,
        "dinput8.dll",
        &metadata,
    )
    .unwrap()
    .is_some());
}

#[test]
fn automatic_dependency_marker_survives_receipt_serialization() {
    let fixture = Fixture::new(false);
    let mut plan = ordinary_plan();
    enshrouded_integration::mark_automatic(&mut plan, "automatic-install");
    let original = record(&fixture.profile, plan);
    let restored: InstalledModRecord =
        serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert!(enshrouded_integration::automatic_ids(&[restored]).contains(&original.id));
}

#[test]
fn automatic_dependency_marker_survives_replacement_baseline() {
    let fixture = Fixture::new(false);
    let mut prior_plan = ordinary_plan();
    enshrouded_integration::mark_automatic(&mut prior_plan, "prior");
    let prior = record(&fixture.profile, prior_plan);
    let mut replacement_plan = ordinary_plan();
    enshrouded_integration::with_replacement_baseline(&prior, || {
        enshrouded_integration::carry_replacement_baseline(
            &fixture.profile,
            "nexus:enshrouded/999999",
            &mut replacement_plan,
        );
    });
    let replacement = record(&fixture.profile, replacement_plan);
    assert!(enshrouded_integration::automatic_ids(&[replacement]).contains(&prior.id));
}

#[test]
fn update_replaces_mod_manifest_but_preserves_user_settings() {
    let fixture = Fixture::new(false);
    let first = fixture.install_fixture(&fixture.source("1.0.0"), "1.0.0");
    let game = Path::new(&fixture.profile.game_path);
    fs::write(
        game.join("mods/example/settings.ini"),
        b"setting=user-choice\n",
    )
    .unwrap();
    let next_source = fixture.source("2.0.0");
    replace_installed_mod_transaction(
        &fixture.store,
        &fixture.profile,
        Some(&first.installed_mod_id),
        "nexus:enshrouded/999999",
        || Ok(fixture.install_fixture(&next_source, "2.0.0")),
    )
    .unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(game.join("mods/example/mod.json")).unwrap()).unwrap();
    assert_eq!(manifest["version"], "2.0.0");
    assert_eq!(
        fs::read(game.join("mods/example/settings.ini")).unwrap(),
        b"setting=user-choice\n"
    );
    let records = read_store::<InstalledModRecord>(&installed_mods_path(&fixture.store)).unwrap();
    assert_eq!(records.items.len(), 1);
    assert_eq!(records.items[0].package_version.as_deref(), Some("2.0.0"));
    assert!(records.items[0].runtime_id.is_none());
}

#[test]
fn loader_manifests_are_payload_not_user_configuration() {
    assert!(!is_user_editable_config_file(Path::new(
        "mods/example/mod.json"
    )));
    assert!(!is_user_editable_config_file(Path::new(
        "mods/creative/flame.json"
    )));
    assert!(!is_user_editable_config_file(Path::new(
        ".mod-garrison/enshrouded-eml.json"
    )));
    assert!(is_user_editable_config_file(Path::new(
        "mods/example/settings.json"
    )));
}

#[test]
fn client_and_dedicated_nexus_files_follow_the_verified_executable_root() {
    let client = Fixture::new(false);
    let server = Fixture::new(true);
    let files = [
        nexus_file(1, "Creative Mode Client", "CreativeMode-client.zip"),
        nexus_file(
            2,
            "Creative Mode Dedicated Server",
            "CreativeMode-server.zip",
        ),
    ];
    assert_eq!(
        choose_nexus_file_for_profile(&client.profile, &files, None)
            .unwrap()
            .unwrap()
            .file_id,
        1
    );
    assert_eq!(
        choose_nexus_file_for_profile(&server.profile, &files, None)
            .unwrap()
            .unwrap()
            .file_id,
        2
    );
    assert!(choose_nexus_file_for_profile(&client.profile, &files, Some("2")).is_err());
    assert!(choose_nexus_file_for_profile(&server.profile, &files, Some("1")).is_err());
}
