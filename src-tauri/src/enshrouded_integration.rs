//! Hooks into the existing deployment/store journals, not a separate installer.
use super::*;

const AUTOMATIC_SOURCE: &str = "enshrouded-automatic-dependency";

pub(super) fn invalidate_eml_config_cache(
    root: &Path,
    profile: &GameProfile,
    record: &InstalledModRecord,
) -> Result<Option<DeploymentOutcome>, String> {
    if !enshrouded::is_profile(profile) || !enshrouded_patch_state::record_uses_eml(record) {
        return Ok(None);
    }
    // EML's timestamps have one-second precision. An edit must still trigger
    // patching when it lands in the same second as the previous game launch.
    let plan = InstallPlan {
        adapter_id: "enshrouded".to_string(),
        adapter_name: "Enshrouded configuration".to_string(),
        confidence: 1.0,
        summary: "Apply changed EML settings at the next game startup.".to_string(),
        mappings: vec![generated_mapping(
            "enshrouded-eml-config-cache",
            "game",
            ".cache/files.json",
            "Rebuild EML patches after a configuration change.",
            "{\"build_id\":\"\",\"files\":{}}",
        )],
        dependencies: Vec::new(),
        warnings: Vec::new(),
        requires_confirmation: false,
    };
    deploy_plan_transaction(
        root,
        profile,
        &record.id,
        Path::new(&record.archive_path),
        &plan,
        None,
    )
    .map(Some)
}

thread_local! {
    static REPLACEMENT_BASELINES: RefCell<Vec<InstalledModRecord>> = const { RefCell::new(Vec::new()) };
    static REMOVAL_IDS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

pub(super) struct RemovalGuard(HashSet<String>);
impl Drop for RemovalGuard {
    fn drop(&mut self) {
        REMOVAL_IDS.with(|ids| {
            *ids.borrow_mut() = self.0.clone();
        });
    }
}
pub(super) fn pending_removals(ids: HashSet<String>) -> RemovalGuard {
    RemovalGuard(REMOVAL_IDS.with(|current| std::mem::replace(&mut *current.borrow_mut(), ids)))
}

pub(super) fn is_native_alias(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("mod-garrison-native-") && name.ends_with(".dll"))
}

pub(super) fn shared_alias_is_retained(
    root: &Path,
    profile: &GameProfile,
    record: &InstalledModRecord,
    path: &Path,
) -> Result<bool, String> {
    if !enshrouded::is_profile(profile) || !is_native_alias(path) {
        return Ok(false);
    }
    let store =
        read_store::<InstalledModRecord>(&installed_mods_path(root)).map_err(error_to_string)?;
    let identity = normalize_filesystem_identity(path.to_string_lossy().as_ref());
    Ok(store.items.iter().any(|other| {
        other.id != record.id
            && other.profile_id == profile.id
            && other.enabled
            && !REMOVAL_IDS.with(|ids| ids.borrow().contains(&other.id))
            && other
                .files_written
                .iter()
                .any(|file| normalize_filesystem_identity(file) == identity)
    }))
}

pub(super) fn with_replacement_baseline<T>(
    record: &InstalledModRecord,
    operation: impl FnOnce() -> T,
) -> T {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            REPLACEMENT_BASELINES.with(|stack| {
                stack.borrow_mut().pop();
            });
        }
    }
    REPLACEMENT_BASELINES.with(|stack| stack.borrow_mut().push(record.clone()));
    let _guard = Guard;
    operation()
}

pub(super) fn carry_replacement_baseline(profile: &GameProfile, key: &str, plan: &mut InstallPlan) {
    if !enshrouded::is_profile(profile) {
        return;
    }
    REPLACEMENT_BASELINES.with(|stack| {
        if let Some(previous) = stack.borrow().iter().rev().find(|record| {
            record.profile_id == profile.id && record.package_id.as_deref() == Some(key)
        }) {
            enshrouded_cleanup::carry_forward_ownership(previous, plan);
            if !automatic_ids(std::slice::from_ref(previous)).is_empty() {
                mark_automatic(plan, &Uuid::new_v4().to_string());
            }
        }
    });
}

pub(super) fn prepare_plan(
    store_root: &Path,
    profile: &GameProfile,
    plan: &mut InstallPlan,
    package_key: &str,
    source: &Path,
) -> Result<(), String> {
    if !enshrouded::is_profile(profile) || plan.adapter_id != "enshrouded" {
        return Ok(());
    }
    if enshrouded_runtime::plan_supplies_eml(plan) {
        enshrouded_patch_state::capture(store_root, profile)?;
        return Ok(());
    }
    if enshrouded_loaders::plan_supplies_shroudtopia(plan)
        || enshrouded_loaders::runtime_supplies_proxy(plan)
    {
        return Ok(());
    }
    enshrouded_runtime::separate_bundled_eml(source, profile, plan)?;
    // An existing verified EML installation can satisfy the dependency without
    // deploying a runtime. Still capture the pre-mod game state in that case.
    if plan
        .dependencies
        .iter()
        .any(|dependency| runtime_from_dependency(dependency) == Some("enshrouded-eml"))
    {
        enshrouded_patch_state::capture(store_root, profile)?;
    }
    let rewritten =
        enshrouded_loaders::normalize_native_plan_from_source(plan, package_key, source)?;
    if rewritten.aliases.is_empty() {
        return Ok(());
    }
    let game_root = Path::new(&profile.game_path);
    let proxy_name = if game_root.join("enshrouded.exe").is_file() {
        "dinput8.dll"
    } else {
        "powrprof.dll"
    };
    let proxy_path = game_root.join(proxy_name);
    validate_deployment_containment(game_root, &proxy_path)?;
    if proxy_path.exists() && !enshrouded_loaders::installed_proxy(profile) {
        return Err(format!(
            "{proxy_name} is already installed outside Mod Garrison's verified chainloader. The existing loader was preserved; automatic replacement cannot prove compatibility."
        ));
    }
    let dependency = known_runtime_dependency(profile, "enshrouded-proxy");
    append_unique_dependencies(&mut plan.dependencies, vec![dependency]);
    // Validate a possible input-forwarding decision before downloading anything.
    // The complete active set is checked again when its shared config is staged.
    let _ = rewritten.input_forward_target;
    let _ = store_root;
    Ok(())
}

pub(super) fn mark_automatic(plan: &mut InstallPlan, install_id: &str) {
    if plan
        .mappings
        .iter()
        .any(|mapping| mapping.source_path == AUTOMATIC_SOURCE)
    {
        return;
    }
    let mut marker = generated_mapping(
        AUTOMATIC_SOURCE,
        "profile",
        &format!(".mod-garrison/automatic-dependencies/{install_id}.json"),
        "This dependency was provisioned for another mod and can be retired when unused.",
        "{\"automatic\":true}",
    );
    marker.source_path = AUTOMATIC_SOURCE.to_string();
    plan.mappings.push(marker);
}

pub(super) fn automatic_ids(records: &[InstalledModRecord]) -> HashSet<String> {
    records
        .iter()
        .filter(|record| {
            record.plan.as_ref().is_some_and(|plan| {
                plan.mappings.iter().any(|mapping| {
                    mapping.target_root == "profile"
                        && mapping.source_path == AUTOMATIC_SOURCE
                        && mapping.generated_content.as_deref() == Some("{\"automatic\":true}")
                })
            })
        })
        .map(|record| record.id.clone())
        .collect()
}

/// Stage the shared loader config and amend its canonical receipt. The caller
/// must batch the receipt and this deployment journal with its other mutations.
pub(super) fn prepare_shared_update(
    root: &Path,
    profile: &GameProfile,
    records: &mut [InstalledModRecord],
) -> Result<Option<DeploymentOutcome>, String> {
    if !enshrouded::is_profile(profile) {
        return Ok(None);
    }
    enshrouded_loaders::validate_active_alias_families(records, &profile.id)?;
    let aliases = enshrouded_loaders::active_native_aliases(records, &profile.id);
    let forward = enshrouded_loaders::active_input_forward_target(records, &profile.id)?;
    let runtime_index = records.iter().position(|record| {
        record.profile_id == profile.id
            && record.enabled
            && record.runtime_id.as_deref() == Some("enshrouded-proxy")
            && !record.externally_managed
    });
    let Some(index) = runtime_index else {
        if aliases.is_empty() {
            return Ok(None);
        }
        return Err("The verified Enshrouded native chainloader is missing.".to_string());
    };
    let record = &records[index];
    if !enshrouded_loaders::installed_proxy(profile) {
        return Err("The Enshrouded chainloader binary or provenance has changed; no shared configuration was written.".to_string());
    }
    let destination = Path::new(&profile.game_path).join("chainloader.ini");
    validate_deployment_containment(Path::new(&profile.game_path), &destination)?;
    let owner_path = normalize_filesystem_identity(destination.to_string_lossy().as_ref());
    if !record
        .files_written
        .iter()
        .any(|path| normalize_filesystem_identity(path) == owner_path)
    {
        return Err(
            "The chainloader configuration is not owned by its verified runtime receipt."
                .to_string(),
        );
    }
    let existing = read_utf8_bounded(&destination, MAX_CONFIG_READ_BYTES)?;
    let content = enshrouded_loaders::chainloader_config(&existing, &aliases, forward.as_deref())?;
    if content == existing {
        return Ok(None);
    }
    let config_mapping = generated_mapping(
        "enshrouded-native-chainloader-config",
        "game",
        "chainloader.ini",
        "Load only the enabled Enshrouded native mods recorded in this profile.",
        &content,
    );
    let plan = InstallPlan {
        adapter_id: "enshrouded".to_string(),
        adapter_name: "Enshrouded native loader".to_string(),
        confidence: 1.0,
        summary: "Synchronize enabled native mods.".to_string(),
        mappings: vec![config_mapping.clone()],
        dependencies: Vec::new(),
        warnings: Vec::new(),
        requires_confirmation: false,
    };
    let deployment = deploy_plan_transaction(
        root,
        profile,
        &record.id,
        Path::new(&record.archive_path),
        &plan,
        Some(&record.id),
    )?;
    let record = &mut records[index];
    for (path, hash) in &deployment.written_file_hashes {
        upsert_written_file_hash(record, Path::new(path), hash);
    }
    if let Some(plan) = record.plan.as_mut() {
        plan.mappings.retain(|mapping| {
            !(mapping.target_root == "game"
                && mapping
                    .target_relative_path
                    .eq_ignore_ascii_case("chainloader.ini"))
        });
        plan.mappings.push(config_mapping);
    }
    Ok(Some(deployment))
}

pub(super) fn commit_removals(
    root: &Path,
    profile: &GameProfile,
    ids: &HashSet<String>,
    restored: Option<&DeploymentOutcome>,
) -> Result<(), String> {
    let _commit = lock_mutations()?;
    let store_path = installed_mods_path(root);
    let mut store = read_store::<InstalledModRecord>(&store_path).map_err(error_to_string)?;
    store
        .items
        .retain(|record| record.profile_id != profile.id || !ids.contains(&record.id));
    let shared = prepare_shared_update(root, profile, &mut store.items)?;
    let result = (|| -> Result<(), String> {
        let mut writes = vec![StoreBatchWrite {
            target: store_path,
            content: serialize_json_line(&store)?,
        }];
        for record in store.items.iter().filter(|record| {
            record.profile_id == profile.id
                && record.runtime_id.as_deref() == Some("enshrouded-proxy")
        }) {
            writes.push(StoreBatchWrite {
                target: installed_mod_receipt_path(root, profile, &record.id),
                content: serialize_json_line(record)?,
            });
        }
        if let Some(shared) = shared.as_ref() {
            let mut journal = shared.journal.clone();
            journal.phase = DeploymentJournalPhase::StateCommitted;
            writes.push(StoreBatchWrite {
                target: deployment_journal_path(&shared.transaction_root),
                content: serialize_json_line(&journal)?,
            });
        }
        if let Some(restored) = restored {
            let mut journal = restored.journal.clone();
            journal.phase = DeploymentJournalPhase::StateCommitted;
            writes.push(StoreBatchWrite {
                target: deployment_journal_path(&restored.transaction_root),
                content: serialize_json_line(&journal)?,
            });
        }
        commit_store_batch(root, writes).map_err(error_to_string)
    })();
    match result {
        Ok(()) => {
            if let Some(shared) = shared {
                let _ = shared.commit();
            }
            Ok(())
        }
        Err(error) => {
            if let Some(shared) = shared {
                if let Err(rollback) = shared.rollback() {
                    return Err(format!("{error} {rollback}"));
                }
            }
            Err(error)
        }
    }
}

fn read_utf8_bounded(path: &Path, maximum: u64) -> Result<String, String> {
    let file = File::open(path).map_err(error_to_string)?;
    if file.metadata().map_err(error_to_string)?.len() > maximum {
        return Err("The chainloader configuration is too large.".to_string());
    }
    let mut content = String::new();
    file.take(maximum + 1)
        .read_to_string(&mut content)
        .map_err(error_to_string)?;
    if content.len() as u64 > maximum {
        return Err("The chainloader configuration is too large.".to_string());
    }
    Ok(content)
}

pub(super) fn remove_mod(
    root: &Path,
    profile: &GameProfile,
    selected: &InstalledModRecord,
    records: &[InstalledModRecord],
    force_forget_modified: bool,
) -> Result<ModActionResult, String> {
    let retained = records
        .iter()
        .filter(|record| record.id != selected.id)
        .cloned()
        .collect::<Vec<_>>();
    let automatic = automatic_ids(records);
    let unused = enshrouded_cleanup::unused_automatic_dependencies(profile, &retained, &automatic);
    let mut ids = unused.into_iter().collect::<HashSet<_>>();
    ids.insert(selected.id.clone());
    let retained = records
        .iter()
        .filter(|record| !ids.contains(&record.id))
        .cloned()
        .collect::<Vec<_>>();
    let mut removals = vec![selected.clone()];
    removals.extend(
        records
            .iter()
            .rev()
            .filter(|record| {
                record.id != selected.id
                    && record.profile_id == profile.id
                    && ids.contains(&record.id)
            })
            .cloned(),
    );
    let last_eml_removed = removals
        .iter()
        .any(|record| record.runtime_id.as_deref() == Some("enshrouded-eml"));
    let rebase_eml = last_eml_removed
        || selected
            .dependencies
            .iter()
            .any(|dependency| runtime_from_dependency(dependency) == Some("enshrouded-eml"));
    let restoration = if rebase_eml {
        enshrouded_patch_state::restore_plan(root, profile)?
    } else {
        None
    };
    let _guard = pending_removals(ids.clone());
    let mut outcomes = Vec::new();
    let mut restored = None;
    let operation = (|| -> Result<(), String> {
        for record in &removals {
            let mut expanded =
                enshrouded_cleanup::expanded_removal_record(profile, record, &retained)?;
            if rebase_eml && record.id == selected.id {
                let mut augmented =
                    enshrouded_patch_state::augment_removal_record(root, profile, &expanded)?;
                if !last_eml_removed {
                    augmented.files_written.retain(|path| {
                        expanded.files_written.contains(path)
                            || Path::new(path)
                                .extension()
                                .is_some_and(|extension| extension.eq_ignore_ascii_case("dat"))
                            || path.replace('\\', "/").contains("/.cache/")
                    });
                }
                expanded = augmented;
            }
            if !record.enabled {
                // Disabled mods may have restored original game files already.
                // Only their remaining config/generated files belong to removal.
                expanded.files_written.retain(|path| {
                    let path = Path::new(path);
                    if !path.is_file() {
                        return false;
                    }
                    let baseline = path.strip_prefix(&profile.game_path).ok().map(|relative| {
                        profile_backup_dir(root, &profile.id, &record.id).join(relative)
                    });
                    if baseline.as_ref().is_some_and(|backup| {
                        backup.is_file() && sha256_file(backup).ok() == sha256_file(path).ok()
                    }) {
                        return false;
                    }
                    is_user_editable_config_file(path)
                        || !record
                            .files_written
                            .iter()
                            .any(|file| Path::new(file) == path)
                });
                expanded.backups_written.clear();
            }
            let outcome = deactivate_mod_files_with_policy(
                root,
                profile,
                &expanded,
                if force_forget_modified {
                    ModifiedFilePolicy::Preserve
                } else {
                    ModifiedFilePolicy::AllowConfigChanges
                },
            )?;
            outcomes.push(outcome);
        }
        if let Some((source, mut plan)) = restoration.clone() {
            if !last_eml_removed {
                plan.mappings.retain(|mapping| {
                    mapping.target_relative_path.ends_with(".kfc")
                        || mapping.target_relative_path.ends_with(".kfc_resources")
                });
            }
            if !plan.mappings.is_empty() {
                restored = Some(deploy_plan_transaction(
                    root,
                    profile,
                    &selected.id,
                    &source,
                    &plan,
                    None,
                )?);
            }
        }
        commit_removals(root, profile, &ids, restored.as_ref())
    })();
    if let Err(error) = operation {
        let mut recovery_errors = Vec::new();
        if let Some(restored) = restored.as_ref() {
            if let Err(recovery) = restored.rollback() {
                recovery_errors.push(recovery);
            }
        }
        for outcome in outcomes.iter().rev() {
            if let Some(transaction) = outcome.transaction_root.as_deref() {
                if let Err(recovery) =
                    recover_deactivation_transaction_for_profile(root, profile, transaction)
                {
                    recovery_errors.push(recovery);
                }
            }
        }
        return Err(format!("{error} {}", recovery_errors.join(" "))
            .trim()
            .to_string());
    }
    if let Some(restored) = restored {
        let _ = restored.commit();
    }
    let mut files_changed = Vec::new();
    let mut warnings = Vec::new();
    for outcome in outcomes {
        if let Some(transaction) = outcome.transaction_root.as_deref() {
            if let Err(error) =
                recover_deactivation_transaction_for_profile(root, profile, transaction)
            {
                warnings.push(error);
            }
        }
        files_changed.extend(outcome.files_changed);
        warnings.extend(outcome.warnings);
    }
    for record in &removals {
        // Cleanup only after the store no longer claims the removed payload.
        if let Err(error) = enshrouded_cleanup::cleanup_emptydirs(profile, record) {
            warnings.push(error);
        }
        cleanup_install_data(root, &profile.id, &record.id);
    }
    if last_eml_removed {
        if let Err(error) = enshrouded_patch_state::cleanup(root, profile) {
            warnings.push(error);
        }
    }
    Ok(ModActionResult {
        profile_id: profile.id.clone(),
        installed_mod_id: selected.id.clone(),
        status: "removed".to_string(),
        files_changed,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        base: PathBuf,
        store: PathBuf,
        profile: GameProfile,
    }
    impl Fixture {
        fn new() -> Self {
            let base = std::env::temp_dir()
                .join(format!("mod-garrison-enshrouded-cycle-{}", Uuid::new_v4()));
            let game = base.join("game");
            fs::create_dir_all(&game).unwrap();
            fs::write(game.join("enshrouded.exe"), b"inert test executable").unwrap();
            fs::write(game.join("enshrouded.kfc"), b"unchanged vanilla test asset").unwrap();
            fs::write(
                game.join("enshrouded.kfc_resources"),
                b"unchanged vanilla test resources",
            )
            .unwrap();
            fs::write(
                game.join("enshrouded_000.dat"),
                b"unchanged vanilla test container",
            )
            .unwrap();
            Self {
                store: base.join("store"),
                profile: GameProfile {
                    id: "enshrouded-cycle".into(),
                    name: "Enshrouded".into(),
                    game_path: game.to_string_lossy().into_owned(),
                    game_id: Some("enshrouded".into()),
                    steam_app_id: Some("1203620".into()),
                    engine: "enshrouded".into(),
                    loader: "none".into(),
                    setup_status: "ready".into(),
                    setup_warnings: vec![],
                    setup_updated_at: None,
                    mods_enabled: true,
                    created_at: now_string(),
                    updated_at: now_string(),
                },
                base,
            }
        }
        fn records(&self) -> Vec<InstalledModRecord> {
            read_store::<InstalledModRecord>(&installed_mods_path(&self.store))
                .unwrap()
                .items
        }
        fn install(&self, source: &Path, id: &str) -> InstallResult {
            let scanned = scan_import_source(&self.store, source).unwrap();
            let identity = provider_source_identity(
                "nexus",
                format!("nexus:enshrouded/{id}"),
                None,
                Some("enshrouded".into()),
                "Enshrouded regression fixture",
            );
            let analysis =
                analyze_scanned_archive_with_identity(scanned, &self.profile, Some(identity));
            let plan = analysis
                .recommended_plan
                .expect("Enshrouded package must have a complete route");
            assert_eq!(plan.adapter_id, "enshrouded");
            install_archive_impl_with_metadata(
                &self.store,
                &self.profile,
                &analysis.archive_path,
                &plan,
                InstallOptions {
                    metadata: InstallMetadata {
                        package_id: Some(format!("nexus:enshrouded/{id}")),
                        package_provider: Some("nexus".into()),
                        package_identity: Some(analysis.package_identity),
                        ..InstallMetadata::default()
                    },
                    resolve_dependencies: true,
                    visited_dependencies: &mut HashSet::new(),
                    dependency_depth: 0,
                },
            )
            .unwrap()
        }
        fn remove(&self, id: &str) {
            let records = self.records();
            let record = records.iter().find(|record| record.id == id).unwrap();
            super::remove_mod(&self.store, &self.profile, record, &records, false).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    #[test]
    #[ignore = "Downloads official EML into a temporary test store; MOD_GARRISON_TEST_ARCHIVE is an already authorized local archive. Never runs game/mod code."]
    fn live_max_stack_archive_installs_all_companions_and_removes_every_owned_file() {
        let fixture = Fixture::new();
        let archive = PathBuf::from(
            std::env::var("MOD_GARRISON_TEST_ARCHIVE")
                .expect("Set the local Max Stack test archive path"),
        );
        let result = fixture.install(&archive, "111");
        let root = Path::new(&fixture.profile.game_path);
        for path in ["mod.json", "src/mod.lua", "config.txt", "items.txt"] {
            assert!(
                root.join("mods/max-stack").join(path).is_file(),
                "missing {path}"
            );
        }
        let records = fixture.records();
        let ordinary = records
            .iter()
            .find(|record| record.id == result.installed_mod_id)
            .unwrap();
        assert!(
            ordinary.runtime_id.is_none(),
            "a gameplay mod must remain removable"
        );
        assert_eq!(automatic_ids(&records).len(), 1);
        fs::write(
            root.join("mods/max-stack/generated.log"),
            b"generated after launch",
        )
        .unwrap();
        // Simulate EML's documented persistent outputs without executing it.
        fs::copy(root.join("enshrouded.kfc"), root.join("enshrouded.kfc.bak")).unwrap();
        fs::write(root.join("enshrouded.kfc"), b"modded test descriptor").unwrap();
        fs::write(
            root.join("enshrouded.kfc_resources"),
            b"modded test resources",
        )
        .unwrap();
        fs::write(
            root.join("enshrouded_001.dat"),
            b"new modded test container",
        )
        .unwrap();
        fs::create_dir_all(root.join(".cache")).unwrap();
        fs::write(root.join(".cache/files.json"), b"{}").unwrap();
        fixture.remove(&result.installed_mod_id);
        assert!(
            fixture.records().is_empty(),
            "unused automatic loader must also be removed"
        );
        assert!(!root.join("mods").exists());
        assert!(!root.join("dbghelp.dll").exists());
        assert!(!root.join(".mod-garrison").exists());
        assert_eq!(
            fs::read(root.join("enshrouded.kfc")).unwrap(),
            b"unchanged vanilla test asset"
        );
        assert_eq!(
            fs::read(root.join("enshrouded.kfc_resources")).unwrap(),
            b"unchanged vanilla test resources"
        );
        assert_eq!(
            fs::read(root.join("enshrouded_000.dat")).unwrap(),
            b"unchanged vanilla test container"
        );
        assert!(!root.join("enshrouded_001.dat").exists());
        assert!(!root.join("enshrouded.kfc.bak").exists());
        assert!(!root.join(".cache").exists());
    }

    #[test]
    #[ignore = "Downloads the official native chainloader into a temporary test store. Synthetic x64 test DLLs are copied but never executed."]
    fn live_native_shared_loader_install_remove_and_bulk_disable_cycle() {
        let fixture = Fixture::new();
        let mut ids = Vec::new();
        for (id, name) in [("165", "first"), ("214", "second")] {
            let source = fixture.base.join(name);
            fs::create_dir_all(source.join(format!("mods/{name}"))).unwrap();
            fs::write(
                source.join("dinput8.dll"),
                enshrouded_loaders::native_dll_fixture(&["DllMain"]),
            )
            .unwrap();
            fs::write(
                source.join(format!("mods/{name}/settings.ini")),
                b"enabled=true",
            )
            .unwrap();
            ids.push(fixture.install(&source, id).installed_mod_id);
        }
        let records = fixture.records();
        let aliases = enshrouded_loaders::active_native_aliases(&records, &fixture.profile.id);
        assert_eq!(aliases.len(), 1, "identical shared binary must load once");
        assert_eq!(automatic_ids(&records).len(), 1);
        // Exercise the same deactivation/commit core as Disable All / Enable All.
        let _guard = pending_removals(ids.iter().cloned().collect());
        let mut disabled = Vec::new();
        for id in &ids {
            let mut record = records
                .iter()
                .find(|record| &record.id == id)
                .unwrap()
                .clone();
            deactivate_mod_files_with_policy(
                &fixture.store,
                &fixture.profile,
                &record,
                ModifiedFilePolicy::PreserveConfigChanges,
            )
            .unwrap();
            record.enabled = false;
            disabled.push(record);
        }
        drop(_guard);
        commit_installed_mod_records(&fixture.store, &fixture.profile, &disabled, &[]).unwrap();
        assert!(!Path::new(&fixture.profile.game_path)
            .join(&aliases[0])
            .exists());
        for record in &mut disabled {
            let plan = record.plan.clone().unwrap();
            let deployment = deploy_plan_transaction(
                &fixture.store,
                &fixture.profile,
                &record.id,
                Path::new(&record.archive_path),
                &plan,
                Some(&record.id),
            )
            .unwrap();
            record.enabled = true;
            record.files_written = deployment.files_written.clone();
            record.backups_written = deployment.backups_written.clone();
            record.written_file_hashes = deployment.written_file_hashes.clone();
            commit_installed_mod_records(
                &fixture.store,
                &fixture.profile,
                std::slice::from_ref(record),
                &[&deployment],
            )
            .unwrap();
            deployment.commit().unwrap();
        }
        fixture.remove(&ids[0]);
        assert!(Path::new(&fixture.profile.game_path)
            .join(&aliases[0])
            .is_file());
        let config =
            fs::read_to_string(Path::new(&fixture.profile.game_path).join("chainloader.ini"))
                .unwrap();
        assert_eq!(config.matches(&aliases[0]).count(), 1);
        fixture.remove(&ids[1]);
        assert!(fixture.records().is_empty());
        for file in [&aliases[0], "dinput8.dll", "chainloader.ini"] {
            assert!(
                !Path::new(&fixture.profile.game_path).join(file).exists(),
                "leftover {file}"
            );
        }
        assert!(
            !Path::new(&fixture.profile.game_path).join("mods").exists(),
            "leftover dirs: {:?}; snapshots: {:?}",
            walk_game_folder(Path::new(&fixture.profile.game_path))
                .iter()
                .map(|entry| &entry.relative_path)
                .collect::<Vec<_>>(),
            disabled
                .iter()
                .map(|record| &record.plan)
                .collect::<Vec<_>>()
        );
    }
}
