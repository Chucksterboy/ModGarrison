//! Restore the game-data files that EML changes after the game is launched.
//! Capture is performed before the first managed EML deployment. Restoration is
//! returned as an ordinary deployment plan; callers keep its rollback journal
//! until removal of the final loader/dependent records has committed.

use super::*;

const BASELINE_DIRECTORY: &str = "enshrouded-baseline";
const BASELINE_MANIFEST: &str = "baseline.json";
const MAX_BASELINE_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const AUXILIARY_FILES: &[&str] = &[
    "eml.json",
    ".cache/files.json",
    ".cache/types.json",
    ".cache/lua/base.lua",
    ".cache/lua/types.lua",
];

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Baseline {
    version: u32,
    complete: bool,
    profile_id: String,
    game_path: String,
    executables: HashMap<String, String>,
    originals: Vec<OriginalFile>,
    dat_files: HashMap<String, u64>,
    #[serde(default)]
    generated_dat_files: HashMap<String, String>,
    auxiliary_paths: Vec<String>,
    created_directories: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OriginalFile {
    path: String,
    source: String,
    size: u64,
    sha256: String,
}

fn baseline_dir(root: &Path, profile: &GameProfile) -> PathBuf {
    profile_dir(root, &profile.id).join(BASELINE_DIRECTORY)
}

fn relevant_bases(game: &Path) -> Vec<&'static str> {
    ["enshrouded", "enshrouded_server"]
        .into_iter()
        .filter(|base| game.join(format!("{base}.exe")).is_file())
        .collect()
}

fn dat_index(name: &str, base: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    let number = lower
        .strip_prefix(&format!("{base}_"))?
        .strip_suffix(".dat")?;
    (number.len() >= 3 && number.len() <= 6 && number.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| number.parse().ok())
        .flatten()
}

fn eml_log(name: &str) -> bool {
    if name.eq_ignore_ascii_case("eml.log") {
        return true;
    }
    let Some(date) = name.strip_suffix(".eml.log") else {
        return false;
    };
    date.len() == 10
        && date.as_bytes()[4] == b'-'
        && date.as_bytes()[7] == b'-'
        && date
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn checked_game_path(game: &Path, relative: &str) -> Result<PathBuf, String> {
    validate_archive_relative_path(relative)?;
    let path = safe_join(game, relative)?;
    validate_deployment_containment(game, &path)?;
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("An EML patch-state path is a junction or symbolic link.".to_string());
            }
        }
        if metadata.file_type().is_symlink() {
            return Err("An EML patch-state path is a symbolic link.".to_string());
        }
    }
    Ok(path)
}

fn auxiliary_paths(game: &Path, bases: &[&str]) -> Result<Vec<String>, String> {
    let mut paths = AUXILIARY_FILES
        .iter()
        .map(|path| path.to_string())
        .collect::<Vec<_>>();
    paths.extend(bases.iter().map(|base| format!("{base}.kfc.bak")));
    let logs = checked_game_path(game, "logs")?;
    if logs.is_dir() {
        for entry in fs::read_dir(logs).map_err(error_to_string)? {
            let entry = entry.map_err(error_to_string)?;
            let name = entry.file_name().to_string_lossy().to_string();
            if eml_log(&name) {
                paths.push(format!("logs/{name}"));
            }
            if paths.len() > MAX_ARCHIVE_ENTRIES {
                return Err("There are too many EML patch-state files to track safely.".to_string());
            }
        }
    }
    Ok(paths)
}

fn read_baseline(root: &Path, profile: &GameProfile) -> Result<Option<Baseline>, String> {
    let directory = baseline_dir(root, profile);
    if !directory.exists() {
        return Ok(None);
    }
    let profile_root = profile_dir(root, &profile.id);
    validate_deployment_containment(&profile_root, &directory)?;
    let manifest_path = directory.join(BASELINE_MANIFEST);
    validate_deployment_containment(&directory, &manifest_path)?;
    let file = File::open(&manifest_path)
        .map_err(|error| format!("The EML restoration baseline is incomplete: {error}"))?;
    if file.metadata().map_err(error_to_string)?.len() > MAX_HEALTH_DIAGNOSTICS_BYTES {
        return Err("The EML restoration baseline manifest is too large.".to_string());
    }
    let baseline: Baseline = serde_json::from_reader(file).map_err(error_to_string)?;
    if baseline.version != 1
        || !baseline.complete
        || baseline.profile_id != profile.id
        || normalize_filesystem_identity(&baseline.game_path)
            != normalize_filesystem_identity(&profile.game_path)
        || baseline.originals.len() > MAX_ARCHIVE_ENTRIES
        || baseline.dat_files.len() > MAX_ARCHIVE_ENTRIES
        || baseline.generated_dat_files.len() > MAX_ARCHIVE_ENTRIES
        || baseline.auxiliary_paths.len() > MAX_ARCHIVE_ENTRIES
        || baseline.executables.is_empty()
    {
        return Err("The EML restoration baseline does not match this game profile.".to_string());
    }
    let game = Path::new(&profile.game_path);
    let bases = baseline
        .executables
        .keys()
        .filter_map(|name| name.strip_suffix(".exe"))
        .collect::<Vec<_>>();
    for (name, expected) in &baseline.executables {
        if !matches!(name.as_str(), "enshrouded.exe" | "enshrouded_server.exe")
            || sha256_file(&checked_game_path(game, name)?)? != *expected
        {
            return Err("Enshrouded changed after its EML baseline was captured. The old game data cannot be restored over a different game build.".to_string());
        }
    }
    for original in &baseline.originals {
        validate_archive_relative_path(&original.path)?;
        validate_archive_relative_path(&original.source)?;
        let known_data = bases.iter().any(|base| {
            [
                format!("{base}.kfc"),
                format!("{base}.kfc_resources"),
                format!("{base}.kfc.bak"),
            ]
            .contains(&original.path)
        });
        let known_auxiliary = AUXILIARY_FILES.contains(&original.path.as_str())
            || original.path.strip_prefix("logs/").is_some_and(eml_log);
        if original.source != format!("original/{}", original.path)
            || (!known_data && !known_auxiliary)
        {
            return Err(
                "The EML restoration baseline contains an unaudited file target.".to_string(),
            );
        }
        let source = safe_join(&directory, &original.source)?;
        validate_deployment_containment(&directory, &source)?;
        if original.size > MAX_BASELINE_FILE_BYTES
            || source.metadata().map_err(error_to_string)?.len() != original.size
            || sha256_file(&source)? != original.sha256
        {
            return Err(format!("The EML restoration copy of {} is missing or changed; no game files were restored.", original.path));
        }
    }
    for (name, size) in &baseline.dat_files {
        validate_archive_relative_path(name)?;
        if !bases.iter().any(|base| dat_index(name, base).is_some()) {
            return Err("The EML baseline contains an unaudited DAT target.".to_string());
        }
        let path = checked_game_path(game, name)?;
        if path.metadata().map_err(error_to_string)?.len() != *size {
            return Err("An original Enshrouded DAT file changed after the EML baseline. Restoration was stopped to protect game updates.".to_string());
        }
    }
    for (name, digest) in &baseline.generated_dat_files {
        validate_archive_relative_path(name)?;
        if !bases.iter().any(|base| {
            dat_index(name, base).is_some_and(|index| {
                baseline
                    .dat_files
                    .keys()
                    .filter_map(|original| dat_index(original, base))
                    .max()
                    .is_none_or(|highest| index > highest)
            })
        }) || baseline
            .dat_files
            .keys()
            .any(|original| original.eq_ignore_ascii_case(name))
            || digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(
                "The EML baseline contains an invalid generated DAT fingerprint.".to_string(),
            );
        }
    }
    for path in baseline
        .auxiliary_paths
        .iter()
        .chain(&baseline.created_directories)
    {
        validate_archive_relative_path(path)?;
    }
    if baseline
        .created_directories
        .iter()
        .any(|path| !matches!(path.as_str(), ".cache" | ".cache/lua" | "logs"))
    {
        return Err("The EML baseline contains an unaudited generated directory.".to_string());
    }
    Ok(Some(baseline))
}

pub(super) fn capture(root: &Path, profile: &GameProfile) -> Result<(), String> {
    if profile.game_id.as_deref() != Some("enshrouded") {
        return Err(
            "EML patch-state capture is only supported for verified Enshrouded profiles."
                .to_string(),
        );
    }
    if read_baseline(root, profile)?.is_some() {
        return Ok(());
    }
    let game = Path::new(&profile.game_path);
    let bases = relevant_bases(game);
    if bases.is_empty() {
        return Err("The Enshrouded executable is missing; an EML restoration baseline could not be captured.".to_string());
    }
    let parent = profile_dir(root, &profile.id);
    fs::create_dir_all(&parent).map_err(error_to_string)?;
    let staging = parent.join(format!(".enshrouded-baseline-{}", Uuid::new_v4()));
    fs::create_dir(&staging).map_err(error_to_string)?;
    let operation = (|| {
        let mut baseline = Baseline {
            version: 1,
            complete: false,
            profile_id: profile.id.clone(),
            game_path: profile.game_path.clone(),
            executables: HashMap::new(),
            originals: Vec::new(),
            dat_files: HashMap::new(),
            generated_dat_files: HashMap::new(),
            auxiliary_paths: Vec::new(),
            created_directories: Vec::new(),
        };
        let mut originals = Vec::new();
        for base in &bases {
            baseline.executables.insert(
                format!("{base}.exe"),
                sha256_file(&checked_game_path(game, &format!("{base}.exe"))?)?,
            );
            for extension in ["kfc", "kfc_resources"] {
                let name = format!("{base}.{extension}");
                if !checked_game_path(game, &name)?.is_file() {
                    return Err(format!(
                        "Cannot capture EML restoration data: {name} is missing."
                    ));
                }
                originals.push(name);
            }
        }
        for entry in fs::read_dir(game).map_err(error_to_string)? {
            let entry = entry.map_err(error_to_string)?;
            let name = entry.file_name().to_string_lossy().to_string();
            if bases.iter().any(|base| dat_index(&name, base).is_some()) {
                let path = checked_game_path(game, &name)?;
                if !path.is_file() {
                    return Err(
                        "An original Enshrouded DAT path is not a regular file.".to_string()
                    );
                }
                baseline
                    .dat_files
                    .insert(name, path.metadata().map_err(error_to_string)?.len());
            }
            if baseline.dat_files.len() > MAX_ARCHIVE_ENTRIES {
                return Err(
                    "There are too many Enshrouded DAT files to capture safely.".to_string()
                );
            }
        }
        for path in auxiliary_paths(game, &bases)? {
            if checked_game_path(game, &path)?.is_file() {
                baseline.auxiliary_paths.push(path.clone());
                originals.push(path);
            }
        }
        for directory in [".cache", ".cache/lua", "logs"] {
            if !checked_game_path(game, directory)?.exists() {
                baseline.created_directories.push(directory.to_string());
            }
        }
        let mut total = 0_u64;
        for relative in originals {
            let source = checked_game_path(game, &relative)?;
            let size = source.metadata().map_err(error_to_string)?.len();
            total = total
                .checked_add(size)
                .ok_or_else(|| "EML baseline size overflowed.".to_string())?;
            if size > MAX_BASELINE_FILE_BYTES || total > MAX_ARCHIVE_EXPANDED_BYTES {
                return Err("The Enshrouded EML baseline exceeds its allowed size.".to_string());
            }
            let stored = format!("original/{relative}");
            let target = safe_join(&staging, &stored)?;
            let before = sha256_file(&source)?;
            copy_file_synced(&source, &target)?;
            if sha256_file(&target)? != before || sha256_file(&source)? != before {
                return Err(
                    "The game data changed while the EML baseline was being captured.".to_string(),
                );
            }
            baseline.originals.push(OriginalFile {
                path: relative,
                source: stored,
                size,
                sha256: before,
            });
        }
        for (name, expected) in &baseline.executables {
            if sha256_file(&checked_game_path(game, name)?)? != *expected {
                return Err(
                    "The Enshrouded executable changed while its EML baseline was captured."
                        .to_string(),
                );
            }
        }
        baseline.complete = true;
        atomic_write(
            &staging.join(BASELINE_MANIFEST),
            &serde_json::to_vec_pretty(&baseline).map_err(error_to_string)?,
        )
        .map_err(error_to_string)?;
        fs::rename(&staging, baseline_dir(root, profile)).map_err(error_to_string)
    })();
    if operation.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    operation
}

pub(super) fn restore_plan(
    root: &Path,
    profile: &GameProfile,
) -> Result<Option<(PathBuf, InstallPlan)>, String> {
    let Some(baseline) = read_baseline(root, profile)? else {
        return Ok(None);
    };
    let mut mappings = Vec::new();
    for original in baseline.originals {
        let current = checked_game_path(Path::new(&profile.game_path), &original.path)?;
        if current.is_file()
            && current.metadata().map_err(error_to_string)?.len() == original.size
            && sha256_file(&current)? == original.sha256
        {
            continue;
        }
        mappings.push(mapping(
            &original.source,
            "game",
            &original.path,
            "Restore the verified Enshrouded state captured before managed EML installation.",
        ));
    }
    if mappings.is_empty() {
        return Ok(None);
    }
    Ok(Some((
        baseline_dir(root, profile),
        InstallPlan {
            adapter_id: "enshrouded".to_string(),
            adapter_name: "Enshrouded original game data".to_string(),
            confidence: 1.0,
            summary: "Restore game data changed by the removed EML mods.".to_string(),
            mappings,
            dependencies: Vec::new(),
            warnings: Vec::new(),
            requires_confirmation: false,
        },
    )))
}

pub(super) fn record_uses_eml(record: &InstalledModRecord) -> bool {
    record.runtime_id.as_deref() == Some("enshrouded-eml")
        || record
            .dependencies
            .iter()
            .any(|dependency| runtime_from_dependency(dependency) == Some("enshrouded-eml"))
}

/// Temporarily return persistent patches to their pre-mod data without retiring
/// the loader baseline. The ordinary deployment journal rolls this back if the
/// associated disable/clean-launch state cannot be committed.
pub(super) fn rebase_mod_data(
    root: &Path,
    profile: &GameProfile,
    operation_id: &str,
) -> Result<Option<DeploymentOutcome>, String> {
    if profile.game_id.as_deref() != Some("enshrouded") {
        return Ok(None);
    }
    let Some((source, mut plan)) = restore_plan(root, profile)? else {
        return Ok(None);
    };
    plan.mappings.retain(|mapping| {
        mapping.target_relative_path.ends_with(".kfc")
            || mapping.target_relative_path.ends_with(".kfc_resources")
    });
    if plan.mappings.is_empty() {
        return Ok(None);
    }
    remember_generated_dat(root, profile)?;
    // EML v0.2 compares its cached build ID and mod timestamps. Restoring only
    // game data would leave unchanged re-enabled mods incorrectly cached as
    // already patched. This valid cache forces a rebuild at the next EML start.
    plan.mappings.push(generated_mapping(
        "enshrouded-eml-dirty-file-cache",
        "game",
        ".cache/files.json",
        "Rebuild enabled EML patches after restoring original game data.",
        "{\"build_id\":\"\",\"files\":{}}",
    ));
    deploy_plan_transaction(root, profile, operation_id, &source, &plan, None).map(Some)
}

pub(super) fn augment_removal_record(
    root: &Path,
    profile: &GameProfile,
    record: &InstalledModRecord,
) -> Result<InstalledModRecord, String> {
    let mut expanded = record.clone();
    let Some(baseline) = read_baseline(root, profile)? else {
        return Ok(expanded);
    };
    let game = Path::new(&profile.game_path);
    let bases = baseline
        .executables
        .keys()
        .filter_map(|name| name.strip_suffix(".exe"))
        .collect::<Vec<_>>();
    let mut generated = proven_new_dat(game, &baseline)?;
    for (name, expected) in &baseline.generated_dat_files {
        let path = checked_game_path(game, name)?;
        if path.is_file() && sha256_file(&path)? == *expected {
            generated.push(name.clone());
        }
    }
    for path in auxiliary_paths(game, &bases)? {
        if !baseline
            .auxiliary_paths
            .iter()
            .any(|original| original.eq_ignore_ascii_case(&path))
        {
            if path.ends_with(".kfc.bak") {
                let Some(original) = baseline
                    .originals
                    .iter()
                    .find(|original| format!("{}.bak", original.path) == path)
                else {
                    continue;
                };
                let backup = checked_game_path(game, &path)?;
                if backup.is_file() && sha256_file(&backup)? != original.sha256 {
                    return Err("An EML backup no longer matches its captured original game data. Removal was stopped to preserve it.".to_string());
                }
            }
            generated.push(path);
        }
    }
    let mut seen = record
        .files_written
        .iter()
        .map(|path| normalize_filesystem_identity(path))
        .collect::<HashSet<_>>();
    for relative in generated {
        let path = checked_game_path(game, &relative)?;
        if !path.exists() {
            continue;
        }
        if !path.is_file() {
            return Err("An EML generated-file path is not a regular file.".to_string());
        }
        let absolute = path.to_string_lossy().to_string();
        if seen.insert(normalize_filesystem_identity(&absolute)) {
            expanded
                .written_file_hashes
                .insert(absolute.clone(), sha256_file(&path)?);
            expanded.files_written.push(absolute);
        }
    }
    Ok(expanded)
}

fn proven_new_dat(game: &Path, baseline: &Baseline) -> Result<Vec<String>, String> {
    let mut generated = Vec::new();
    let bases = baseline
        .executables
        .keys()
        .filter_map(|name| name.strip_suffix(".exe"));
    for base in bases {
        let original = baseline
            .originals
            .iter()
            .find(|original| original.path == format!("{base}.kfc"))
            .ok_or_else(|| "The EML baseline is missing its original KFC entry.".to_string())?;
        let backup_name = format!("{base}.kfc.bak");
        let backup = checked_game_path(game, &backup_name)?;
        let original_backup = backup.is_file() && sha256_file(&backup)? == original.sha256;
        let patched =
            sha256_file(&checked_game_path(game, &format!("{base}.kfc"))?)? != original.sha256;
        let highest_original_index = baseline
            .dat_files
            .keys()
            .filter_map(|name| dat_index(name, base))
            .max();
        if patched && original_backup {
            for entry in fs::read_dir(game).map_err(error_to_string)? {
                let entry = entry.map_err(error_to_string)?;
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(index) = dat_index(&name, base) {
                    if !baseline
                        .dat_files
                        .keys()
                        .any(|existing| existing.eq_ignore_ascii_case(&name))
                        && highest_original_index.is_none_or(|highest| index > highest)
                    {
                        generated.push(name);
                    }
                }
            }
        }
    }
    Ok(generated)
}

fn remember_generated_dat(root: &Path, profile: &GameProfile) -> Result<(), String> {
    let Some(mut baseline) = read_baseline(root, profile)? else {
        return Ok(());
    };
    let game = Path::new(&profile.game_path);
    let proven = proven_new_dat(game, &baseline)?;
    if proven.is_empty() {
        return Ok(());
    }
    for name in proven {
        let digest = sha256_file(&checked_game_path(game, &name)?)?;
        baseline.generated_dat_files.insert(name, digest);
    }
    if baseline.generated_dat_files.len() > MAX_ARCHIVE_ENTRIES {
        return Err("There are too many generated EML data files to track safely.".to_string());
    }
    let directory = baseline_dir(root, profile);
    atomic_write(
        &directory.join(BASELINE_MANIFEST),
        &serialize_json_line(&baseline)?,
    )
    .map_err(error_to_string)
}

/// Called only after the final EML removal and restoration have committed.
pub(super) fn cleanup(root: &Path, profile: &GameProfile) -> Result<(), String> {
    let Some(baseline) = read_baseline(root, profile)? else {
        return Ok(());
    };
    let game = Path::new(&profile.game_path);
    let mut directories = baseline.created_directories;
    directories.sort_by_key(|path| std::cmp::Reverse(path.split('/').count()));
    for relative in directories {
        let path = checked_game_path(game, &relative)?;
        match fs::remove_dir(path) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    let directory = baseline_dir(root, profile);
    validate_deployment_containment(&profile_dir(root, &profile.id), &directory)?;
    fs::remove_dir_all(directory).map_err(error_to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        base: PathBuf,
        root: PathBuf,
        profile: GameProfile,
    }
    impl Fixture {
        fn new() -> Self {
            let base =
                std::env::temp_dir().join(format!("mod-garrison-eml-baseline-{}", Uuid::new_v4()));
            let game = base.join("game");
            fs::create_dir_all(&game).unwrap();
            for (name, bytes) in [
                ("enshrouded.exe", b"game build".as_slice()),
                ("enshrouded.kfc", b"vanilla kfc"),
                ("enshrouded.kfc_resources", b"vanilla resources"),
                ("enshrouded_000.dat", b"vanilla dat"),
            ] {
                fs::write(game.join(name), bytes).unwrap();
            }
            let profile = serde_json::from_value(serde_json::json!({"id":"patch-state", "name":"Enshrouded", "gamePath":game,
                "gameId":"enshrouded", "engine":"enshrouded", "loader":"none", "createdAt":"", "updatedAt":""})).unwrap();
            Self {
                root: base.join("store"),
                base,
                profile,
            }
        }
        fn game(&self) -> PathBuf {
            PathBuf::from(&self.profile.game_path)
        }
        fn record(&self) -> InstalledModRecord {
            serde_json::from_value(serde_json::json!({"id":"eml", "profileId":self.profile.id, "archivePath":"", "archiveName":"eml.zip", "adapterId":"enshrouded", "summary":"", "installedAt":"", "filesWritten":[], "backupsWritten":[], "dependencies":[], "runtimeId":"enshrouded-eml"})).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    #[test]
    fn baseline_restores_real_game_data_through_the_existing_transaction_and_can_roll_back() {
        let fixture = Fixture::new();
        capture(&fixture.root, &fixture.profile).unwrap();
        fs::write(fixture.game().join("enshrouded.kfc"), b"patched kfc").unwrap();
        fs::write(
            fixture.game().join("enshrouded.kfc_resources"),
            b"patched resources with trailing mod bytes",
        )
        .unwrap();
        let (source, plan) = restore_plan(&fixture.root, &fixture.profile)
            .unwrap()
            .unwrap();
        let deployment = deploy_plan_transaction(
            &fixture.root,
            &fixture.profile,
            "restore",
            &source,
            &plan,
            None,
        )
        .unwrap();
        assert_eq!(
            fs::read(fixture.game().join("enshrouded.kfc")).unwrap(),
            b"vanilla kfc"
        );
        assert_eq!(
            fs::read(fixture.game().join("enshrouded.kfc_resources")).unwrap(),
            b"vanilla resources"
        );
        deployment.rollback().unwrap();
        assert_eq!(
            fs::read(fixture.game().join("enshrouded.kfc")).unwrap(),
            b"patched kfc"
        );
        assert_eq!(
            fs::read(fixture.game().join("enshrouded.kfc_resources")).unwrap(),
            b"patched resources with trailing mod bytes"
        );
    }

    #[test]
    fn disabled_patch_rebase_is_rollbackable_and_keeps_generated_dat_ownership() {
        let fixture = Fixture::new();
        capture(&fixture.root, &fixture.profile).unwrap();
        fs::copy(
            fixture.game().join("enshrouded.kfc"),
            fixture.game().join("enshrouded.kfc.bak"),
        )
        .unwrap();
        fs::write(fixture.game().join("enshrouded.kfc"), b"patched kfc").unwrap();
        fs::write(fixture.game().join("enshrouded_001.dat"), b"generated data").unwrap();
        fs::create_dir_all(fixture.game().join(".cache")).unwrap();
        let cache = fixture.game().join(".cache/files.json");
        fs::write(&cache, b"original cache").unwrap();
        let deployment = rebase_mod_data(&fixture.root, &fixture.profile, "disable-test")
            .unwrap()
            .unwrap();
        assert_eq!(
            fs::read(fixture.game().join("enshrouded.kfc")).unwrap(),
            b"vanilla kfc"
        );
        assert_eq!(
            fs::read(&cache).unwrap(),
            b"{\"build_id\":\"\",\"files\":{}}"
        );
        let generated = fixture
            .game()
            .join("enshrouded_001.dat")
            .to_string_lossy()
            .to_string();
        let expanded =
            augment_removal_record(&fixture.root, &fixture.profile, &fixture.record()).unwrap();
        assert!(
            expanded.files_written.contains(&generated),
            "a clean launch must not lose later uninstall ownership"
        );
        deployment.rollback().unwrap();
        assert_eq!(
            fs::read(fixture.game().join("enshrouded.kfc")).unwrap(),
            b"patched kfc"
        );
        assert_eq!(fs::read(&cache).unwrap(), b"original cache");
        let deployment = rebase_mod_data(&fixture.root, &fixture.profile, "disable-test")
            .unwrap()
            .unwrap();
        deployment.commit().unwrap();
        fs::write(&generated, b"subsequent user change").unwrap();
        let expanded =
            augment_removal_record(&fixture.root, &fixture.profile, &fixture.record()).unwrap();
        assert!(
            !expanded.files_written.contains(&generated),
            "changed data must not be deleted using stale ownership"
        );
    }

    #[test]
    fn only_new_eml_generated_files_are_added_to_the_final_removal_receipt() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.game().join("logs")).unwrap();
        fs::write(
            fixture.game().join("logs/2026-10-01.eml.log"),
            b"old user log",
        )
        .unwrap();
        capture(&fixture.root, &fixture.profile).unwrap();
        fs::write(fixture.game().join("enshrouded.kfc"), b"patched").unwrap();
        fs::write(fixture.game().join("enshrouded.kfc.bak"), b"vanilla kfc").unwrap();
        fs::write(
            fixture.game().join("enshrouded_001.dat"),
            b"new mod content",
        )
        .unwrap();
        fs::write(
            fixture.game().join("logs/2026-10-26.eml.log"),
            b"new EML log",
        )
        .unwrap();
        fs::write(fixture.game().join("logs/some-other-app.log"), b"unrelated").unwrap();
        let record =
            augment_removal_record(&fixture.root, &fixture.profile, &fixture.record()).unwrap();
        assert_eq!(record.files_written.len(), 3);
        assert!(!record
            .files_written
            .iter()
            .any(|path| path.ends_with("enshrouded_000.dat")
                || path.ends_with("2026-10-01.eml.log")
                || path.ends_with("some-other-app.log")));
        assert!(fixture.game().join("enshrouded_001.dat").is_file());
    }

    #[test]
    fn steam_game_update_prevents_old_baseline_restoration() {
        let fixture = Fixture::new();
        capture(&fixture.root, &fixture.profile).unwrap();
        fs::write(fixture.game().join("enshrouded.exe"), b"new Steam build").unwrap();
        assert!(restore_plan(&fixture.root, &fixture.profile)
            .unwrap_err()
            .contains("different game build"));
        assert!(
            augment_removal_record(&fixture.root, &fixture.profile, &fixture.record()).is_err()
        );
    }

    #[test]
    fn tampered_original_copy_is_rejected_before_any_game_write() {
        let fixture = Fixture::new();
        capture(&fixture.root, &fixture.profile).unwrap();
        fs::write(
            baseline_dir(&fixture.root, &fixture.profile).join("original/enshrouded.kfc"),
            b"tampered",
        )
        .unwrap();
        assert!(restore_plan(&fixture.root, &fixture.profile).is_err());
        assert_eq!(
            fs::read(fixture.game().join("enshrouded.kfc")).unwrap(),
            b"vanilla kfc"
        );
    }

    #[test]
    fn preexisting_eml_state_is_captured_and_restored_instead_of_deleted() {
        let fixture = Fixture::new();
        fs::write(fixture.game().join("eml.json"), b"user settings").unwrap();
        fs::write(
            fixture.game().join("enshrouded.kfc.bak"),
            b"user preexisting backup",
        )
        .unwrap();
        capture(&fixture.root, &fixture.profile).unwrap();
        fs::write(fixture.game().join("eml.json"), b"changed settings").unwrap();
        let (_, plan) = restore_plan(&fixture.root, &fixture.profile)
            .unwrap()
            .unwrap();
        assert!(plan
            .mappings
            .iter()
            .any(|mapping| mapping.target_relative_path == "eml.json"));
        let record =
            augment_removal_record(&fixture.root, &fixture.profile, &fixture.record()).unwrap();
        assert!(record.files_written.is_empty());
        capture(&fixture.root, &fixture.profile).unwrap();
    }

    #[test]
    fn no_game_launch_requires_no_redundant_game_data_restoration() {
        let fixture = Fixture::new();
        capture(&fixture.root, &fixture.profile).unwrap();
        assert!(restore_plan(&fixture.root, &fixture.profile)
            .unwrap()
            .is_none());
        cleanup(&fixture.root, &fixture.profile).unwrap();
        assert!(!baseline_dir(&fixture.root, &fixture.profile).exists());
        assert_eq!(
            fs::read(fixture.game().join("enshrouded.kfc_resources")).unwrap(),
            b"vanilla resources"
        );
    }
}
