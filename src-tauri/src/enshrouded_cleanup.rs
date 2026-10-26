//! Per-mod ownership of files generated after installation.
//!
//! The inventory is stored in the normal managed profile receipt. Removal adds
//! newly generated files to a copy of the receipt, so the existing deactivation
//! journal can remove them and restore them if any later removal step fails.

use super::*;

const OWNERSHIP_SOURCE: &str = ".mod-garrison/generated/enshrouded-owned-directories";
const OWNERSHIP_DIRECTORY: &str = ".mod-garrison/enshrouded-ownership";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OwnershipSnapshot {
    version: u32,
    profile_id: String,
    directories: Vec<OwnedDirectory>,
    #[serde(default)]
    created_parents: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OwnedDirectory {
    path: String,
    existed_before: bool,
    preexisting_paths: Vec<String>,
}

fn namespace_for_path(path: &str) -> Option<String> {
    let path = normalize_archive_path(path);
    if validate_archive_relative_path(&path).is_err() {
        return None;
    }
    let segments = path.split('/').collect::<Vec<_>>();
    if segments.len() >= 3
        && matches!(
            segments[0].to_ascii_lowercase().as_str(),
            "mods" | "greeneye"
        )
        && !segments[1].starts_with('.')
    {
        return Some(segments[..2].join("/"));
    }
    if segments.len() >= 2
        && matches!(
            segments[0].to_ascii_lowercase().as_str(),
            "aldrich-modmenu" | "enshrouded_vr" | "creative"
        )
    {
        return Some(segments[0].to_string());
    }
    None
}

fn allowed_namespace(path: &str) -> bool {
    namespace_for_path(&format!("{path}/__ownership_probe__"))
        .is_some_and(|namespace| namespace.eq_ignore_ascii_case(path))
}

fn ownership_mapping(mapping: &InstallMapping) -> bool {
    mapping.target_root == "profile"
        && mapping.source_path == OWNERSHIP_SOURCE
        && mapping
            .target_relative_path
            .starts_with(&format!("{OWNERSHIP_DIRECTORY}/"))
        && mapping.generated_content.is_some()
}

fn snapshot_from_plan(
    plan: &InstallPlan,
    profile_id: &str,
) -> Result<Option<OwnershipSnapshot>, String> {
    let mappings = plan
        .mappings
        .iter()
        .filter(|mapping| ownership_mapping(mapping))
        .collect::<Vec<_>>();
    if mappings.len() > 1 {
        return Err(
            "The Enshrouded mod has more than one generated-file ownership inventory.".to_string(),
        );
    }
    let Some(mapping) = mappings.first() else {
        return Ok(None);
    };
    let snapshot: OwnershipSnapshot = serde_json::from_str(
        mapping.generated_content.as_deref().unwrap_or_default(),
    )
    .map_err(|_| "The Enshrouded generated-file ownership inventory is invalid.".to_string())?;
    if snapshot.version != 1
        || snapshot.profile_id != profile_id
        || snapshot.directories.len() > MAX_ARCHIVE_ENTRIES
        || snapshot.created_parents.len() > MAX_ARCHIVE_ENTRIES
    {
        return Err(
            "The Enshrouded generated-file ownership inventory does not match this profile."
                .to_string(),
        );
    }
    if snapshot
        .created_parents
        .iter()
        .any(|parent| validate_archive_relative_path(parent).is_err())
    {
        return Err(
            "The Enshrouded ownership inventory contains an invalid created parent directory."
                .to_string(),
        );
    }
    for directory in &snapshot.directories {
        if !allowed_namespace(&directory.path)
            || directory.preexisting_paths.len() > MAX_ARCHIVE_ENTRIES
            || directory.preexisting_paths.iter().any(|path| {
                validate_archive_relative_path(path).is_err()
                    || !archive_path_is_within(path, &directory.path)
            })
        {
            return Err(
                "The Enshrouded generated-file inventory contains an invalid directory."
                    .to_string(),
            );
        }
    }
    Ok(Some(snapshot))
}

fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Walk only a verified per-mod namespace, never the game root or shared `mods`.
fn directory_tree(game_root: &Path, namespace: &str) -> Result<Vec<(String, bool)>, String> {
    if !allowed_namespace(namespace) {
        return Err("Refused an unscoped Enshrouded mod directory.".to_string());
    }
    let root = safe_join(game_root, namespace)?;
    validate_deployment_containment(game_root, &root)?;
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    let mut queue = VecDeque::from([root]);
    while let Some(path) = queue.pop_front() {
        validate_deployment_containment(game_root, &path)?;
        let metadata = fs::symlink_metadata(&path).map_err(error_to_string)?;
        if is_link(&metadata) {
            return Err("Refused to clean a linked Enshrouded mod directory or file.".to_string());
        }
        let relative = path.strip_prefix(game_root).map_err(error_to_string)?;
        let relative = to_portable_path(relative);
        validate_archive_relative_path(&relative)?;
        if !archive_path_is_within(&relative, namespace) {
            return Err("An Enshrouded mod directory escaped its ownership namespace.".to_string());
        }
        paths.push((relative.clone(), metadata.is_dir()));
        if paths.len() > MAX_ARCHIVE_ENTRIES || relative.split('/').count() > MAX_ARCHIVE_PATH_DEPTH
        {
            return Err(
                "The Enshrouded mod's generated-file inventory exceeds its size limit.".to_string(),
            );
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path).map_err(error_to_string)? {
                if paths.len() + queue.len() >= MAX_ARCHIVE_ENTRIES {
                    return Err(
                        "The Enshrouded mod's generated-file inventory exceeds its size limit."
                            .to_string(),
                    );
                }
                queue.push_back(entry.map_err(error_to_string)?.path());
            }
        } else if !metadata.is_file() {
            return Err("An Enshrouded mod generated a non-regular file.".to_string());
        }
    }
    paths.sort_by_key(|(path, _)| path.to_ascii_lowercase());
    Ok(paths)
}

/// Copy a previous install's baseline into its replacement before snapshot_plan.
/// This avoids treating an earlier version's generated logs/config as user data.
pub(super) fn carry_forward_ownership(record: &InstalledModRecord, plan: &mut InstallPlan) {
    if plan.mappings.iter().any(ownership_mapping) {
        return;
    }
    if let Some(mapping) = record.plan.as_ref().and_then(|plan| {
        plan.mappings
            .iter()
            .find(|mapping| ownership_mapping(mapping))
    }) {
        plan.mappings.push(mapping.clone());
    }
}

pub(super) fn snapshot_plan(
    profile: &GameProfile,
    plan: &mut InstallPlan,
    install_id: &str,
) -> Result<(), String> {
    if profile.game_id.as_deref() != Some("enshrouded") || plan.adapter_id != "enshrouded" {
        return Ok(());
    }
    let mut snapshot = snapshot_from_plan(plan, &profile.id)?.unwrap_or(OwnershipSnapshot {
        version: 1,
        profile_id: profile.id.clone(),
        directories: Vec::new(),
        created_parents: Vec::new(),
    });
    // Record parent directories before deployment creates them. This includes
    // shared roots such as mods/greeneye and loader provenance directories, but
    // cleanup only removes them when empty, after all dependent mods are gone.
    let game_root = Path::new(&profile.game_path);
    for mapping in plan
        .mappings
        .iter()
        .filter(|mapping| mapping.target_root == "game")
    {
        validate_archive_relative_path(&mapping.target_relative_path)?;
        let mut parent = archive_path_parent(&mapping.target_relative_path);
        while !parent.is_empty() {
            let path = safe_join(game_root, &parent)?;
            validate_deployment_containment(game_root, &path)?;
            if !path.exists()
                && !snapshot
                    .created_parents
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(&parent))
            {
                snapshot.created_parents.push(parent.clone());
            }
            parent = archive_path_parent(&parent);
        }
    }
    let mut namespaces = plan
        .mappings
        .iter()
        .filter(|mapping| mapping.target_root == "game")
        .filter_map(|mapping| namespace_for_path(&mapping.target_relative_path))
        .collect::<Vec<_>>();
    namespaces.sort_by_key(|path| path.to_ascii_lowercase());
    namespaces.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    for namespace in namespaces {
        if snapshot
            .directories
            .iter()
            .any(|directory| directory.path.eq_ignore_ascii_case(&namespace))
        {
            continue;
        }
        let tree = directory_tree(Path::new(&profile.game_path), &namespace)?;
        snapshot.directories.push(OwnedDirectory {
            path: namespace,
            existed_before: !tree.is_empty(),
            preexisting_paths: tree.into_iter().map(|(path, _)| path).collect(),
        });
    }
    if snapshot.directories.is_empty() && snapshot.created_parents.is_empty() {
        return Ok(());
    }
    plan.mappings.retain(|mapping| !ownership_mapping(mapping));
    let mut metadata = generated_mapping(
        "enshrouded-owned-directories",
        "profile",
        &format!(
            "{OWNERSHIP_DIRECTORY}/{}.json",
            sanitize_file_segment(install_id)
        ),
        "Pre-install inventory for transactional removal of mod-generated files.",
        &serde_json::to_string(&snapshot).map_err(error_to_string)?,
    );
    // A stable source key allows the baseline to survive a new install UUID.
    metadata.source_path = OWNERSHIP_SOURCE.to_string();
    plan.mappings.push(metadata);
    Ok(())
}

pub(super) fn expanded_removal_record(
    profile: &GameProfile,
    record: &InstalledModRecord,
    other_records: &[InstalledModRecord],
) -> Result<InstalledModRecord, String> {
    let mut expanded = record.clone();
    if profile.game_id.as_deref() != Some("enshrouded") || record.profile_id != profile.id {
        return Ok(expanded);
    }
    let Some(snapshot) = record
        .plan
        .as_ref()
        .map(|plan| snapshot_from_plan(plan, &profile.id))
        .transpose()?
        .flatten()
    else {
        return Ok(expanded);
    };
    let retained = other_records
        .iter()
        .filter(|other| {
            other.id != record.id
                && other.profile_id == profile.id
                && other.last_status != "removed"
        })
        .collect::<Vec<_>>();
    let protected_paths = retained
        .iter()
        .flat_map(|other| other.files_written.iter())
        .map(|path| normalize_filesystem_identity(path))
        .collect::<HashSet<_>>();
    let mut shared_namespaces = Vec::new();
    for other in &retained {
        if let Some(plan) = &other.plan {
            shared_namespaces.extend(
                plan.mappings
                    .iter()
                    .filter(|mapping| mapping.target_root == "game")
                    .filter_map(|mapping| namespace_for_path(&mapping.target_relative_path)),
            );
        }
    }
    let mut recorded = record
        .files_written
        .iter()
        .map(|path| normalize_filesystem_identity(path))
        .collect::<HashSet<_>>();
    for directory in snapshot.directories {
        if shared_namespaces.iter().any(|namespace| {
            archive_path_is_within(namespace, &directory.path)
                || archive_path_is_within(&directory.path, namespace)
        }) {
            continue;
        }
        let preexisting = directory
            .preexisting_paths
            .iter()
            .map(|path| path.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        for (relative, is_directory) in
            directory_tree(Path::new(&profile.game_path), &directory.path)?
        {
            if is_directory || preexisting.contains(&relative.to_ascii_lowercase()) {
                continue;
            }
            let path = safe_join(Path::new(&profile.game_path), &relative)?;
            let absolute = path.to_string_lossy().to_string();
            let identity = normalize_filesystem_identity(&absolute);
            if protected_paths.contains(&identity) || !recorded.insert(identity) {
                continue;
            }
            expanded
                .written_file_hashes
                .insert(absolute.clone(), sha256_file(&path)?);
            expanded.files_written.push(absolute);
        }
    }
    Ok(expanded)
}

/// Share proof that a common parent was originally absent with later users of
/// that parent, so the final owner can remove it even if its creator went first.
pub(super) fn inherit_shared_parents(
    profile: &GameProfile,
    plan: &mut InstallPlan,
    records: &[InstalledModRecord],
) -> Result<(), String> {
    let Some(mut snapshot) = snapshot_from_plan(plan, &profile.id)? else {
        return Ok(());
    };
    let game_targets = plan
        .mappings
        .iter()
        .filter(|mapping| mapping.target_root == "game")
        .map(|mapping| mapping.target_relative_path.clone())
        .collect::<Vec<_>>();
    for record in records
        .iter()
        .filter(|record| record.profile_id == profile.id && record.last_status != "removed")
    {
        let Some(previous) = record
            .plan
            .as_ref()
            .map(|plan| snapshot_from_plan(plan, &profile.id))
            .transpose()?
            .flatten()
        else {
            continue;
        };
        for parent in previous.created_parents {
            if game_targets
                .iter()
                .any(|target| target != &parent && archive_path_is_within(target, &parent))
                && !snapshot
                    .created_parents
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(&parent))
            {
                snapshot.created_parents.push(parent);
            }
        }
    }
    if let Some(mapping) = plan
        .mappings
        .iter_mut()
        .find(|mapping| ownership_mapping(mapping))
    {
        mapping.generated_content =
            Some(serde_json::to_string(&snapshot).map_err(error_to_string)?);
    }
    Ok(())
}

/// Only remove empty, newly-created directories after journaled file removal.
pub(super) fn cleanup_emptydirs(
    profile: &GameProfile,
    record: &InstalledModRecord,
) -> Result<(), String> {
    if profile.game_id.as_deref() != Some("enshrouded") {
        return Ok(());
    }
    let Some(snapshot) = record
        .plan
        .as_ref()
        .map(|plan| snapshot_from_plan(plan, &profile.id))
        .transpose()?
        .flatten()
    else {
        return Ok(());
    };
    for directory in snapshot.directories {
        let preexisting = directory
            .preexisting_paths
            .iter()
            .map(|path| path.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        let mut paths = directory_tree(Path::new(&profile.game_path), &directory.path)?;
        paths.sort_by_key(|(path, _)| std::cmp::Reverse(path.split('/').count()));
        for (relative, is_directory) in paths {
            if !is_directory
                || preexisting.contains(&relative.to_ascii_lowercase())
                || (directory.existed_before && relative.eq_ignore_ascii_case(&directory.path))
            {
                continue;
            }
            let path = safe_join(Path::new(&profile.game_path), &relative)?;
            validate_deployment_containment(Path::new(&profile.game_path), &path)?;
            match fs::remove_dir(path) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::DirectoryNotEmpty | io::ErrorKind::NotFound
                    ) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    let mut parents = snapshot.created_parents;
    parents.sort_by_key(|path| std::cmp::Reverse(path.split('/').count()));
    for relative in parents {
        let path = safe_join(Path::new(&profile.game_path), &relative)?;
        validate_deployment_containment(Path::new(&profile.game_path), &path)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        if is_link(&metadata) || !metadata.is_dir() {
            return Err(
                "Refused to remove a linked or non-directory Enshrouded parent.".to_string(),
            );
        }
        match fs::remove_dir(path) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::DirectoryNotEmpty | io::ErrorKind::NotFound
                ) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

/// Return only auto-provisioned dependencies unreachable from retained user mods.
/// Keep dependencies of disabled mods too, so re-enabling remains possible.
pub(super) fn unused_automatic_dependencies(
    profile: &GameProfile,
    records: &[InstalledModRecord],
    automatic_ids: &HashSet<String>,
) -> Vec<String> {
    let alive = records
        .iter()
        .filter(|record| record.profile_id == profile.id && record.last_status != "removed")
        .collect::<Vec<_>>();
    let mut needed = alive
        .iter()
        .filter(|record| !automatic_ids.contains(&record.id))
        .map(|record| record.id.clone())
        .collect::<HashSet<_>>();
    loop {
        let before = needed.len();
        for dependent in alive
            .iter()
            .filter(|record| needed.contains(&record.id))
            .copied()
            .collect::<Vec<_>>()
        {
            for dependency in dependent
                .dependencies
                .iter()
                .filter(|dependency| dependency.required)
            {
                for candidate in &alive {
                    if candidate.id != dependent.id
                        && installed_record_matches_dependency_package(
                            profile, candidate, dependency,
                        )
                        && record_payload_covers_dependent_target(candidate, dependent)
                    {
                        needed.insert(candidate.id.clone());
                    }
                }
            }
        }
        if before == needed.len() {
            break;
        }
    }
    alive
        .into_iter()
        .filter(|record| automatic_ids.contains(&record.id) && !needed.contains(&record.id))
        .map(|record| record.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        profile: GameProfile,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("mod-garrison-cleanup-test-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            let profile = serde_json::from_value(serde_json::json!({"id":"cleanup-test", "name":"Enshrouded", "gamePath":root,
                "gameId":"enshrouded", "engine":"enshrouded", "loader":"none", "createdAt":"", "updatedAt":""})).unwrap();
            Self { root, profile }
        }
        fn write(&self, relative: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"fixture").unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn plan(path: &str) -> InstallPlan {
        InstallPlan {
            adapter_id: "enshrouded".to_string(),
            adapter_name: "Enshrouded".to_string(),
            confidence: 1.0,
            summary: String::new(),
            mappings: vec![mapping(path, "game", path, "fixture")],
            dependencies: Vec::new(),
            warnings: Vec::new(),
            requires_confirmation: false,
        }
    }
    fn record(profile: &GameProfile, id: &str, plan: InstallPlan) -> InstalledModRecord {
        let mut record: InstalledModRecord = serde_json::from_value(serde_json::json!({
            "id":id,"profileId":profile.id,"archivePath":"","archiveName":"fixture.zip","adapterId":"enshrouded",
            "summary":"","installedAt":"","filesWritten":[],"backupsWritten":[],"dependencies":[],"enabled":true,"lastStatus":"installed"
        })).unwrap();
        record.plan = Some(plan);
        record
    }

    #[test]
    fn cleanup_inventories_are_scoped_to_individual_mod_namespaces() {
        assert_eq!(
            namespace_for_path("mods/creative/logs/a.log").as_deref(),
            Some("mods/creative")
        );
        assert_eq!(
            namespace_for_path("greeneye/find/settings.ini").as_deref(),
            Some("greeneye/find")
        );
        assert!(namespace_for_path("mods/../enshrouded.kfc").is_none());
        assert!(namespace_for_path("enshrouded.kfc").is_none());
        assert!(!allowed_namespace("mods"));
        assert!(!allowed_namespace("."));
    }

    #[test]
    fn generated_files_are_added_to_the_existing_removal_journal_receipt() {
        let fixture = Fixture::new();
        let mut plan = plan("mods/creative/creative.dll");
        snapshot_plan(&fixture.profile, &mut plan, "install").unwrap();
        fixture.write("mods/creative/creative.dll");
        fixture.write("mods/creative/logs/session.log");
        fixture.write("mods/creative/settings.json");
        let record = record(&fixture.profile, "install", plan);
        let expanded = expanded_removal_record(&fixture.profile, &record, &[]).unwrap();
        assert_eq!(expanded.files_written.len(), 3);
        assert_eq!(expanded.written_file_hashes.len(), 3);
        // Expansion is read-only; the caller's existing transaction removes files.
        assert!(fixture.root.join("mods/creative/settings.json").is_file());
    }

    #[test]
    fn preexisting_user_files_and_other_mod_namespaces_are_preserved() {
        let fixture = Fixture::new();
        fixture.write("mods/creative/user-notes.txt");
        let mut plan = plan("mods/creative/creative.dll");
        snapshot_plan(&fixture.profile, &mut plan, "install").unwrap();
        fixture.write("mods/creative/logs/generated.log");
        fixture.write("mods/other-mod/generated.log");
        let record = record(&fixture.profile, "install", plan);
        let expanded = expanded_removal_record(&fixture.profile, &record, &[]).unwrap();
        assert_eq!(expanded.files_written.len(), 1);
        assert!(expanded.files_written[0].ends_with("generated.log"));
        assert!(!expanded
            .files_written
            .iter()
            .any(|path| path.contains("other-mod")));
    }

    #[test]
    fn shared_namespace_generated_files_are_retained_for_the_other_mod() {
        let fixture = Fixture::new();
        let mut owned_plan = plan("mods/shared/first.dll");
        snapshot_plan(&fixture.profile, &mut owned_plan, "first").unwrap();
        fixture.write("mods/shared/settings.ini");
        let first = record(&fixture.profile, "first", owned_plan);
        let other = record(&fixture.profile, "second", plan("mods/shared/second.dll"));
        let expanded = expanded_removal_record(&fixture.profile, &first, &[other]).unwrap();
        assert!(expanded.files_written.is_empty());
    }

    #[test]
    fn replacement_keeps_original_preinstall_inventory() {
        let fixture = Fixture::new();
        let mut old_plan = plan("mods/creative/creative.dll");
        snapshot_plan(&fixture.profile, &mut old_plan, "old").unwrap();
        fixture.write("mods/creative/settings.json");
        let old = record(&fixture.profile, "old", old_plan);
        let mut new_plan = plan("mods/creative/creative.dll");
        carry_forward_ownership(&old, &mut new_plan);
        snapshot_plan(&fixture.profile, &mut new_plan, "new").unwrap();
        let new = record(&fixture.profile, "new", new_plan);
        assert_eq!(
            expanded_removal_record(&fixture.profile, &new, &[])
                .unwrap()
                .files_written
                .len(),
            1
        );
        assert!(new
            .plan
            .unwrap()
            .mappings
            .iter()
            .any(|mapping| mapping.target_relative_path.ends_with("/new.json")));
    }

    #[test]
    fn empty_directory_cleanup_preserves_preexisting_empty_directories() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root.join("mods/creative/existing")).unwrap();
        let mut owned_plan = plan("mods/creative/creative.dll");
        snapshot_plan(&fixture.profile, &mut owned_plan, "install").unwrap();
        fs::create_dir_all(fixture.root.join("mods/creative/logs/new")).unwrap();
        let record = record(&fixture.profile, "install", owned_plan);
        cleanup_emptydirs(&fixture.profile, &record).unwrap();
        assert!(fixture.root.join("mods/creative/existing").is_dir());
        assert!(!fixture.root.join("mods/creative/logs").exists());
    }

    #[test]
    fn shared_automatic_runtime_is_retained_until_the_last_dependent_is_removed() {
        let fixture = Fixture::new();
        let mut runtime = record(&fixture.profile, "eml", plan("dbghelp.dll"));
        runtime.runtime_id = Some("enshrouded-eml".to_string());
        let mut first = record(&fixture.profile, "first", plan("mods/first/src/mod.lua"));
        first.dependencies = vec![known_runtime_dependency(&fixture.profile, "enshrouded-eml")];
        let mut second = first.clone();
        second.id = "second".to_string();
        // A disabled retained mod still owns its requirements for later re-enable.
        second.enabled = false;
        first.last_status = "removed".to_string();
        let automatic = HashSet::from([runtime.id.clone()]);
        assert!(unused_automatic_dependencies(
            &fixture.profile,
            &[runtime.clone(), first.clone(), second.clone()],
            &automatic
        )
        .is_empty());
        second.last_status = "removed".to_string();
        assert_eq!(
            unused_automatic_dependencies(&fixture.profile, &[runtime, first, second], &automatic),
            vec!["eml".to_string()]
        );
    }

    #[test]
    fn explicitly_installed_runtimes_are_never_pruned_as_automatic_dependencies() {
        let fixture = Fixture::new();
        let mut runtime = record(&fixture.profile, "explicit-eml", plan("dbghelp.dll"));
        runtime.runtime_id = Some("enshrouded-eml".to_string());
        assert!(
            unused_automatic_dependencies(&fixture.profile, &[runtime], &HashSet::new()).is_empty()
        );
    }

    #[test]
    fn cleanup_removes_only_recorded_new_parents_after_their_last_file_is_removed() {
        let fixture = Fixture::new();
        let mut owned_plan = plan(".mod-garrison/enshrouded-eml.json");
        snapshot_plan(&fixture.profile, &mut owned_plan, "runtime").unwrap();
        fixture.write(".mod-garrison/enshrouded-eml.json");
        let record = record(&fixture.profile, "runtime", owned_plan);
        cleanup_emptydirs(&fixture.profile, &record).unwrap();
        assert!(fixture.root.join(".mod-garrison").is_dir());
        fs::remove_file(fixture.root.join(".mod-garrison/enshrouded-eml.json")).unwrap();
        cleanup_emptydirs(&fixture.profile, &record).unwrap();
        assert!(!fixture.root.join(".mod-garrison").exists());
    }

    #[test]
    fn cleanup_preserves_a_parent_directory_that_was_empty_before_installation() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root.join("mods")).unwrap();
        let mut owned_plan = plan("mods/creative/creative.dll");
        snapshot_plan(&fixture.profile, &mut owned_plan, "creative").unwrap();
        fixture.write("mods/creative/creative.dll");
        fs::remove_file(fixture.root.join("mods/creative/creative.dll")).unwrap();
        let record = record(&fixture.profile, "creative", owned_plan);
        cleanup_emptydirs(&fixture.profile, &record).unwrap();
        assert!(!fixture.root.join("mods/creative").exists());
        assert!(fixture.root.join("mods").is_dir());
    }

    #[test]
    fn inherited_parent_creation_proof_survives_removing_its_first_owner() {
        let fixture = Fixture::new();
        let mut first_plan = plan("mods/first/first.dll");
        snapshot_plan(&fixture.profile, &mut first_plan, "first").unwrap();
        fixture.write("mods/first/first.dll");
        let first = record(&fixture.profile, "first", first_plan);
        let mut second_plan = plan("mods/second/second.dll");
        snapshot_plan(&fixture.profile, &mut second_plan, "second").unwrap();
        inherit_shared_parents(
            &fixture.profile,
            &mut second_plan,
            std::slice::from_ref(&first),
        )
        .unwrap();
        fixture.write("mods/second/second.dll");
        let second = record(&fixture.profile, "second", second_plan);
        fs::remove_file(fixture.root.join("mods/first/first.dll")).unwrap();
        cleanup_emptydirs(&fixture.profile, &first).unwrap();
        assert!(fixture.root.join("mods").is_dir());
        fs::remove_file(fixture.root.join("mods/second/second.dll")).unwrap();
        cleanup_emptydirs(&fixture.profile, &second).unwrap();
        assert!(!fixture.root.join("mods").exists());
    }
}
