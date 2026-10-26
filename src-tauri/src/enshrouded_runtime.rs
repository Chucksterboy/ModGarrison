//! Audited provisioning of the upstream Enshrouded Mod Loader DLL.
//!
//! EML is distributed as a raw DLL, not a ZIP. Keep that exception restricted to
//! its official repository and asset, then adapt it to the ordinary transactional
//! archive installer. No downloaded code is executed by this module.

use super::*;

pub(super) const EML_ID: &str = "enshrouded-eml";
const EML_SOURCE: &str = "github-release:Brabb3l/kfc-parser#dbghelp.dll";
const EML_MARKER: &str = ".mod-garrison/enshrouded-eml.json";
const EML_MAX_BYTES: u64 = 64 * 1024 * 1024;
const EML_V020_SHA256: &str = "038b7a1787026007a68c82a6adeb8e79c2e644f936d4cd00f8b0db206a2349fa";
const EML_V020_DINPUT8_SHA256: &str =
    "1ff5334c46ab635ab7602701e2d63f32447bc4b258e942d85b2c0273b8457e2e";

static VERIFIED_EML_RELEASES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmlProof {
    format: u32,
    source: String,
    version: String,
    sha256: String,
    size: u64,
}

pub(super) fn is_eml_dependency(dependency: &DependencySpec) -> bool {
    dependency.id.strip_prefix("runtime:") == Some(EML_ID)
        && dependency.provider == "github-release"
        && dependency.source.as_deref() == Some(EML_SOURCE)
}

fn supported_version(version: &str) -> bool {
    Version::parse(version.trim_start_matches('v'))
        .is_ok_and(|version| version >= Version::new(0, 2, 0) && version.pre.is_empty())
}

fn valid_proof(proof: &EmlProof) -> bool {
    proof.format == 1
        && proof.source == EML_SOURCE
        && supported_version(&proof.version)
        && (512..=EML_MAX_BYTES).contains(&proof.size)
        && proof.sha256.len() == 64
        && proof.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn proof_has_official_digest(proof: &EmlProof) -> bool {
    if !valid_proof(proof) {
        return false;
    }
    if proof.version.trim_start_matches('v') == "0.2.0"
        && proof.sha256.eq_ignore_ascii_case(EML_V020_SHA256)
    {
        return true;
    }
    VERIFIED_EML_RELEASES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|releases| releases.get(&proof.version).cloned())
        .is_some_and(|digest| digest.eq_ignore_ascii_case(&proof.sha256))
}

fn verify_unknown_release_proof(proof: &EmlProof) -> bool {
    if !valid_proof(proof) || proof.version.trim_start_matches('v') == "0.2.0" {
        return false;
    }
    // Managed packages survive restarts and may contain a future release. Bind
    // that version back to upstream metadata rather than trusting the package's
    // self-declared digest or accepting a DLL merely because of its filename.
    let result = (|| -> Result<bool, String> {
        let client = provider_client()?;
        let release = client
            .get(format!(
                "{GITHUB_API_BASE}/Brabb3l/kfc-parser/releases/tags/{}",
                proof.version
            ))
            .send()
            .map_err(error_to_string)?
            .error_for_status()
            .map_err(error_to_string)?
            .json::<GithubReleaseResponse>()
            .map_err(error_to_string)?;
        if release.tag_name != proof.version {
            return Ok(false);
        }
        let Some(asset) = release
            .assets
            .iter()
            .find(|asset| asset.name == "dbghelp.dll")
        else {
            return Ok(false);
        };
        let Some(digest) = github_asset_declared_sha256(asset)? else {
            return Ok(false);
        };
        let expected_url = format!(
            "https://github.com/Brabb3l/kfc-parser/releases/download/{}/dbghelp.dll",
            proof.version
        );
        if asset.browser_download_url != expected_url
            || asset.size != Some(proof.size)
            || !digest.eq_ignore_ascii_case(&proof.sha256)
        {
            return Ok(false);
        }
        VERIFIED_EML_RELEASES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .map_err(|_| "Could not verify the EML release identity.".to_string())?
            .insert(proof.version.clone(), digest);
        Ok(true)
    })();
    result.unwrap_or(false)
}

fn validate_release_source(release: &ReleaseDependencyRef) -> Result<(), String> {
    if release.source_key != EML_SOURCE || !supported_version(&release.version) {
        return Err(
            "EML must come from a supported official Brabb3l/kfc-parser release (v0.2.0 or newer)."
                .to_string(),
        );
    }
    let url = url::Url::parse(&release.download_url).map_err(error_to_string)?;
    let expected_path = format!(
        "/Brabb3l/kfc-parser/releases/download/{}/dbghelp.dll",
        release.version
    );
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != expected_path
    {
        return Err(
            "The EML dependency did not resolve to the official dbghelp.dll release asset."
                .to_string(),
        );
    }
    let Some(digest) = release.expected_sha256.as_deref() else {
        return Err("The EML release does not publish a SHA-256 digest, so its DLL cannot be installed automatically.".to_string());
    };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("The EML release has an invalid SHA-256 digest.".to_string());
    }
    if !release
        .expected_size
        .is_some_and(|size| (512..=EML_MAX_BYTES).contains(&size))
    {
        return Err("The EML release has an invalid DLL size.".to_string());
    }
    Ok(())
}

/// Validate the Windows x64 PE/DLL header before treating a raw release as code.
pub(super) fn validate_x64_dll(bytes: &[u8]) -> Result<(), String> {
    let invalid = || "The EML download is not a valid Windows x64 DLL.".to_string();
    if bytes.len() < 512 || bytes.get(..2) != Some(b"MZ") {
        return Err(invalid());
    }
    let offset = u32::from_le_bytes(bytes[0x3c..0x40].try_into().map_err(|_| invalid())?) as usize;
    let end = offset.checked_add(26).ok_or_else(invalid)?;
    if end > bytes.len() || bytes.get(offset..offset + 4) != Some(b"PE\0\0") {
        return Err(invalid());
    }
    let machine = u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]);
    let sections = u16::from_le_bytes([bytes[offset + 6], bytes[offset + 7]]);
    let optional_size = u16::from_le_bytes([bytes[offset + 20], bytes[offset + 21]]) as usize;
    let characteristics = u16::from_le_bytes([bytes[offset + 22], bytes[offset + 23]]);
    let optional_magic = u16::from_le_bytes([bytes[offset + 24], bytes[offset + 25]]);
    let section_end = offset
        .checked_add(24)
        .and_then(|end| end.checked_add(optional_size))
        .and_then(|end| end.checked_add(usize::from(sections) * 40));
    if machine != 0x8664
        || sections == 0
        || sections > 96
        || optional_size < 112
        || optional_magic != 0x20b
        || characteristics & 0x2000 == 0
        || !section_end.is_some_and(|end| end <= bytes.len())
    {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn download_eml_archive(
    store_root: &Path,
    dependency: &DependencySpec,
    release: &ReleaseDependencyRef,
) -> Result<PathBuf, String> {
    if !is_eml_dependency(dependency) {
        return Err(
            "Raw DLL provisioning is limited to the registered Enshrouded Mod Loader.".to_string(),
        );
    }
    validate_release_source(release)?;
    let cache_dir = store_root
        .join("cache")
        .join("github-release")
        .join(sanitize_file_segment(EML_SOURCE))
        .join(sanitize_file_segment(&release.version));
    fs::create_dir_all(&cache_dir).map_err(error_to_string)?;
    let dll_path = cache_dir.join("dbghelp.dll");
    if !cached_download_is_verified(&dll_path, &release.download_url)? {
        let client = provider_client()?;
        download_url_to_file(&client, &release.download_url, &dll_path)?;
    }
    if let Err(error) = verify_declared_download_integrity(
        &dll_path,
        release.expected_size,
        release.expected_sha256.as_deref(),
        &release.display_name,
    ) {
        remove_download_artifact(&dll_path);
        return Err(error);
    }
    let bytes = read_bounded_file(&dll_path, EML_MAX_BYTES)?;
    validate_x64_dll(&bytes)?;
    let proof = EmlProof {
        format: 1,
        source: EML_SOURCE.to_string(),
        version: release.version.clone(),
        sha256: hex::encode(Sha256::digest(&bytes)),
        size: bytes.len() as u64,
    };
    // Only provider-verified bytes reach this allowlist. An archive's own marker
    // cannot add arbitrary digests to it.
    VERIFIED_EML_RELEASES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "Could not verify the EML release identity.".to_string())?
        .insert(proof.version.clone(), proof.sha256.clone());
    let archive_path = cache_dir.join("eml-runtime.zip");
    write_runtime_archive(&archive_path, &bytes, &proof)?;
    Ok(archive_path)
}

fn write_runtime_archive(path: &Path, bytes: &[u8], proof: &EmlProof) -> Result<(), String> {
    let temporary = path.with_file_name(format!(".eml-{}.zip", Uuid::new_v4()));
    let result = (|| {
        let file = File::create(&temporary).map_err(error_to_string)?;
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("dbghelp.dll", options)
            .map_err(error_to_string)?;
        zip.write_all(bytes).map_err(error_to_string)?;
        zip.start_file(EML_MARKER, options)
            .map_err(error_to_string)?;
        zip.write_all(&serde_json::to_vec(proof).map_err(error_to_string)?)
            .map_err(error_to_string)?;
        zip.finish()
            .map_err(error_to_string)?
            .sync_all()
            .map_err(error_to_string)?;
        replace_file_from_path(&temporary, path)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn read_bounded_file(path: &Path, maximum: u64) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(error_to_string)?;
    if file.metadata().map_err(error_to_string)?.len() > maximum {
        return Err("The EML runtime file exceeds its allowed size.".to_string());
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(error_to_string)?;
    if bytes.len() as u64 > maximum {
        return Err("The EML runtime file exceeds its allowed size.".to_string());
    }
    Ok(bytes)
}

fn read_scanned_member(
    scanned: &ScannedArchive,
    path: &str,
    maximum: u64,
) -> Result<Vec<u8>, String> {
    let source = Path::new(&scanned.archive_path);
    if source.is_dir() {
        let file = safe_join(source, path)?;
        validate_deployment_containment(source, &file)?;
        return read_bounded_file(&file, maximum);
    }
    let mut archive =
        ZipArchive::new(File::open(source).map_err(error_to_string)?).map_err(error_to_string)?;
    let index = enshrouded_loaders::zip_member_index(&mut archive, path)?;
    let file = archive.by_index(index).map_err(error_to_string)?;
    if file.size() > maximum {
        return Err("The EML runtime archive entry is too large.".to_string());
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(error_to_string)?;
    if bytes.len() as u64 > maximum {
        return Err("The EML runtime archive entry is too large.".to_string());
    }
    Ok(bytes)
}

fn read_bundled_member(source: &Path, member: &str) -> Result<Vec<u8>, String> {
    validate_archive_relative_path(member)?;
    if source.is_dir() {
        let path = safe_join(source, member)?;
        validate_deployment_containment(source, &path)?;
        return read_bounded_file(&path, EML_MAX_BYTES);
    }
    if source
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
    {
        if !basename(member).eq_ignore_ascii_case(
            source
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default(),
        ) {
            return Err("The raw EML filename does not match its package member.".to_string());
        }
        return read_bounded_file(source, EML_MAX_BYTES);
    }
    let mut archive =
        ZipArchive::new(File::open(source).map_err(error_to_string)?).map_err(error_to_string)?;
    let mut found = None;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(error_to_string)?;
        if normalize_archive_path(entry.name())
            .eq_ignore_ascii_case(&normalize_archive_path(member))
        {
            if found.is_some() {
                return Err("The bundled EML package contains ambiguous DLL aliases.".to_string());
            }
            found = Some(index);
        }
    }
    let file = archive
        .by_index(found.ok_or_else(|| "The bundled EML DLL is missing.".to_string())?)
        .map_err(error_to_string)?;
    if file.size() > EML_MAX_BYTES {
        return Err("The bundled EML DLL exceeds its allowed size.".to_string());
    }
    let mut bytes = Vec::new();
    file.take(EML_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(error_to_string)?;
    if bytes.len() as u64 > EML_MAX_BYTES {
        return Err("The bundled EML DLL exceeds its allowed size.".to_string());
    }
    Ok(bytes)
}

fn official_bundled_digest(source: &Path, member: &str) -> Option<&'static str> {
    let expected = match basename(member).to_ascii_lowercase().as_str() {
        "dbghelp.dll" => EML_V020_SHA256,
        "dinput8.dll" => EML_V020_DINPUT8_SHA256,
        _ => return None,
    };
    let bytes = read_bundled_member(source, member).ok()?;
    (validate_x64_dll(&bytes).is_ok()
        && hex::encode(Sha256::digest(&bytes)).eq_ignore_ascii_case(expected))
    .then_some(expected)
}

#[cfg(test)]
pub(super) fn bundled_eml_is_official(source: &Path, member: &str) -> bool {
    official_bundled_digest(source, member).is_some()
}

/// A mod may bundle the same official EML DLL used by another mod. Keep that
/// archive member in profile metadata and provision one shared managed runtime,
/// instead of giving each mod ownership of the game-root proxy filename.
pub(super) fn separate_bundled_eml(
    source: &Path,
    profile: &GameProfile,
    plan: &mut InstallPlan,
) -> Result<(), String> {
    if profile.game_id.as_deref() != Some("enshrouded")
        || plan.adapter_id != "enshrouded"
        || plan_supplies_eml(plan)
    {
        return Ok(());
    }
    let mut separated = false;
    for mapping in &mut plan.mappings {
        if mapping.target_root != "game"
            || mapping.generated_content.is_some()
            || !matches!(
                mapping.target_relative_path.to_ascii_lowercase().as_str(),
                "dbghelp.dll" | "dinput8.dll"
            )
        {
            continue;
        }
        let Some(digest) = official_bundled_digest(source, &mapping.source_path) else {
            continue;
        };
        let member_identity = hex::encode(Sha256::digest(mapping.source_path.as_bytes()));
        mapping.target_root = "profile".to_string();
        mapping.target_relative_path = format!(
            "runtime-metadata/enshrouded-eml/{}-{}-{}",
            &digest[..16],
            &member_identity[..8],
            basename(&mapping.source_path)
        );
        mapping.reason = "Bundled official EML is retained with the package while one shared loader is provisioned.".to_string();
        separated = true;
    }
    if separated
        && !plan
            .dependencies
            .iter()
            .any(|dependency| runtime_from_dependency(dependency) == Some(EML_ID))
    {
        plan.dependencies
            .push(known_runtime_dependency(profile, EML_ID));
    }
    Ok(())
}

pub(super) fn verified_runtime_plan(
    scanned: &ScannedArchive,
    profile: &GameProfile,
) -> Option<InstallPlan> {
    if profile.game_id.as_deref() != Some("enshrouded") {
        return None;
    }
    // Another tool may own this proxy name; do not overwrite an unidentified
    // library merely because it is not in our managed records.
    let game_root = Path::new(&profile.game_path);
    let existing_proxy = game_root.join("dbghelp.dll");
    if existing_proxy.exists()
        && !sha256_file(&existing_proxy)
            .is_ok_and(|digest| digest.eq_ignore_ascii_case(EML_V020_SHA256))
        && !installed_eml_proof_at(game_root)
    {
        return None;
    }
    let files = scanned
        .entries
        .iter()
        .filter(|entry| !entry.is_directory)
        .collect::<Vec<_>>();
    if files.len() != 2 {
        return None;
    }
    let marker = files.iter().find(|entry| entry.path == EML_MARKER)?;
    let dll = files.iter().find(|entry| entry.path == "dbghelp.dll")?;
    let proof: EmlProof =
        serde_json::from_slice(&read_scanned_member(scanned, &marker.path, 8192).ok()?).ok()?;
    if (!proof_has_official_digest(&proof) && !verify_unknown_release_proof(&proof))
        || proof.size != dll.size
    {
        return None;
    }
    let bytes = read_scanned_member(scanned, &dll.path, EML_MAX_BYTES).ok()?;
    if validate_x64_dll(&bytes).is_err()
        || !hex::encode(Sha256::digest(&bytes)).eq_ignore_ascii_case(&proof.sha256)
    {
        return None;
    }
    Some(InstallPlan {
        adapter_id: "enshrouded".to_string(),
        adapter_name: "Enshrouded Mod Loader".to_string(),
        confidence: 1.0,
        summary: format!(
            "Install verified Enshrouded Mod Loader {} beside the game executable.",
            proof.version
        ),
        mappings: vec![
            mapping(
                &dll.path,
                "game",
                "dbghelp.dll",
                "Official Enshrouded Mod Loader x64 runtime.",
            ),
            generated_mapping(
                "enshrouded-eml-provenance",
                "game",
                EML_MARKER,
                "Verified loader source, version and SHA-256 for runtime detection.",
                &serde_json::to_string(&proof).ok()?,
            ),
        ],
        dependencies: Vec::new(),
        warnings: Vec::new(),
        requires_confirmation: false,
    })
}

pub(super) fn plan_supplies_eml(plan: &InstallPlan) -> bool {
    plan.adapter_id == "enshrouded"
        && plan.mappings.iter().any(|mapping| {
            mapping.target_root == "game"
                && mapping.target_relative_path == "dbghelp.dll"
                && mapping.generated_content.is_none()
        })
        && plan.mappings.iter().any(|mapping| {
            mapping.target_root == "game"
                && mapping.target_relative_path == EML_MARKER
                && mapping
                    .generated_content
                    .as_deref()
                    .and_then(|content| serde_json::from_str::<EmlProof>(content).ok())
                    .is_some_and(|proof| proof_has_official_digest(&proof))
        })
}

pub(super) fn installed_eml(profile: &GameProfile) -> bool {
    if profile.game_id.as_deref() != Some("enshrouded") {
        return false;
    }
    installed_eml_at(Path::new(&profile.game_path))
}

pub(super) fn installed_eml_at(root: &Path) -> bool {
    if !root.join("enshrouded.exe").is_file() && !root.join("enshrouded_server.exe").is_file() {
        return false;
    }
    // Manually installed current upstream builds can be identified by the
    // official release digest, including EML's alternate dinput8 proxy.
    for (name, expected_size, digest) in [
        ("dbghelp.dll", 20614895, EML_V020_SHA256),
        ("dinput8.dll", 20576572, EML_V020_DINPUT8_SHA256),
    ] {
        let path = root.join(name);
        if path
            .metadata()
            .ok()
            .is_some_and(|metadata| metadata.len() == expected_size)
            && validate_deployment_containment(root, &path).is_ok()
            && sha256_file(&path).is_ok_and(|actual| actual.eq_ignore_ascii_case(digest))
        {
            return true;
        }
    }
    installed_eml_proof_at(root)
}

fn installed_eml_proof_at(root: &Path) -> bool {
    let dll = root.join("dbghelp.dll");
    let marker = root.join(EML_MARKER);
    if validate_deployment_containment(root, &dll).is_err()
        || validate_deployment_containment(root, &marker).is_err()
    {
        return false;
    }
    let Some(proof) = read_bounded_file(&marker, 8192)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<EmlProof>(&bytes).ok())
    else {
        // A raw filename alone never establishes which proxy library is present.
        return false;
    };
    let Ok(bytes) = read_bounded_file(&dll, EML_MAX_BYTES) else {
        return false;
    };
    (proof_has_official_digest(&proof) || verify_unknown_release_proof(&proof))
        && proof.size == bytes.len() as u64
        && validate_x64_dll(&bytes).is_ok()
        && hex::encode(Sha256::digest(&bytes)).eq_ignore_ascii_case(&proof.sha256)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dll_fixture() -> Vec<u8> {
        let mut bytes = vec![0; 512];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&128_u32.to_le_bytes());
        bytes[128..132].copy_from_slice(b"PE\0\0");
        bytes[132..134].copy_from_slice(&0x8664_u16.to_le_bytes());
        bytes[134..136].copy_from_slice(&1_u16.to_le_bytes());
        bytes[148..150].copy_from_slice(&112_u16.to_le_bytes());
        bytes[150..152].copy_from_slice(&0x2000_u16.to_le_bytes());
        bytes[152..154].copy_from_slice(&0x20b_u16.to_le_bytes());
        bytes
    }

    fn release_fixture() -> ReleaseDependencyRef {
        ReleaseDependencyRef {
            source_key: EML_SOURCE.to_string(),
            display_name: "Enshrouded Mod Loader".to_string(),
            download_url:
                "https://github.com/Brabb3l/kfc-parser/releases/download/v0.2.0/dbghelp.dll"
                    .to_string(),
            version: "v0.2.0".to_string(),
            expected_size: Some(20614895),
            expected_sha256: Some(EML_V020_SHA256.to_string()),
        }
    }

    fn proof_fixture(bytes: &[u8]) -> EmlProof {
        EmlProof {
            format: 1,
            source: EML_SOURCE.to_string(),
            version: "v99.0.0-test".to_string(),
            sha256: hex::encode(Sha256::digest(bytes)),
            size: bytes.len() as u64,
        }
    }

    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("mod-garrison-eml-test-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn profile(root: &Path) -> GameProfile {
        serde_json::from_value(serde_json::json!({"id":"eml-test", "name":"Enshrouded", "gamePath":root,
            "gameId":"enshrouded", "engine":"enshrouded", "loader":"none", "createdAt":"", "updatedAt":""})).unwrap()
    }

    #[test]
    fn eml_raw_asset_requires_official_repository_and_current_release() {
        let valid = release_fixture();
        assert!(validate_release_source(&valid).is_ok());
        let mut invalid = release_fixture();
        invalid.download_url = invalid.download_url.replace("Brabb3l", "another-author");
        assert!(validate_release_source(&invalid).is_err());
        invalid = release_fixture();
        invalid.version = "v0.1.2".to_string();
        assert!(validate_release_source(&invalid).is_err());
        invalid = release_fixture();
        invalid.expected_sha256 = None;
        assert!(validate_release_source(&invalid).is_err());
    }

    #[test]
    fn eml_raw_asset_rejects_url_credentials_fragments_and_alternate_asset() {
        for url in [
            "https://github.com/Brabb3l/kfc-parser/releases/download/v0.2.0/dinput8.dll",
            "https://github.com/Brabb3l/kfc-parser/releases/download/v0.2.0/dbghelp.dll?download=1",
            "https://user@github.com/Brabb3l/kfc-parser/releases/download/v0.2.0/dbghelp.dll",
            "https://github.com.example.com/Brabb3l/kfc-parser/releases/download/v0.2.0/dbghelp.dll",
        ] {
            let mut release = release_fixture();
            release.download_url = url.to_string();
            assert!(validate_release_source(&release).is_err(), "{url}");
        }
    }

    #[test]
    fn eml_binary_validation_rejects_webpages_exes_x86_and_truncated_headers() {
        assert!(validate_x64_dll(&dll_fixture()).is_ok());
        assert!(validate_x64_dll(b"<html>not a DLL</html>").is_err());
        let mut exe = dll_fixture();
        exe[150..152].copy_from_slice(&0x0002_u16.to_le_bytes());
        assert!(validate_x64_dll(&exe).is_err());
        let mut x86 = dll_fixture();
        x86[132..134].copy_from_slice(&0x14c_u16.to_le_bytes());
        assert!(validate_x64_dll(&x86).is_err());
        let mut bad_offset = dll_fixture();
        bad_offset[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(validate_x64_dll(&bad_offset).is_err());
    }

    #[test]
    fn eml_imported_proof_cannot_self_authorize_an_arbitrary_dll() {
        let bytes = dll_fixture();
        let proof = proof_fixture(&bytes);
        assert!(!proof_has_official_digest(&proof));
        let root = TempRoot::new();
        let archive = root.0.join("eml.zip");
        write_runtime_archive(&archive, &bytes, &proof).unwrap();
        assert!(
            verified_runtime_plan(&scan_zip_archive(&archive).unwrap(), &profile(&root.0))
                .is_none()
        );
    }

    #[test]
    fn eml_verified_release_wraps_into_a_complete_transactional_plan() {
        let mut bytes = dll_fixture();
        bytes[500] = 17;
        let mut proof = proof_fixture(&bytes);
        proof.version = "v99.1.0".to_string();
        VERIFIED_EML_RELEASES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap()
            .insert(proof.version.clone(), proof.sha256.clone());
        let root = TempRoot::new();
        let archive = root.0.join("runtime.zip");
        write_runtime_archive(&archive, &bytes, &proof).unwrap();
        let plan =
            verified_runtime_plan(&scan_zip_archive(&archive).unwrap(), &profile(&root.0)).unwrap();
        assert!(plan_supplies_eml(&plan));
        assert_eq!(plan.mappings.len(), 2);
        assert!(plan.dependencies.is_empty());
        assert!(!plan.requires_confirmation);

        bytes[500] = 18;
        write_runtime_archive(&archive, &bytes, &proof).unwrap();
        assert!(
            verified_runtime_plan(&scan_zip_archive(&archive).unwrap(), &profile(&root.0))
                .is_none()
        );
    }

    #[test]
    fn eml_installed_runtime_requires_proof_and_detects_tampering() {
        let root = TempRoot::new();
        fs::write(root.0.join("enshrouded.exe"), b"fixture").unwrap();
        let bytes = dll_fixture();
        fs::write(root.0.join("dbghelp.dll"), &bytes).unwrap();
        let profile = profile(&root.0);
        assert!(!installed_eml(&profile));
        let mut proof = proof_fixture(&bytes);
        proof.version = "v99.2.0".to_string();
        VERIFIED_EML_RELEASES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap()
            .insert(proof.version.clone(), proof.sha256.clone());
        fs::create_dir_all(root.0.join(".mod-garrison")).unwrap();
        fs::write(root.0.join(EML_MARKER), serde_json::to_vec(&proof).unwrap()).unwrap();
        assert!(installed_eml(&profile));
        fs::write(root.0.join("dbghelp.dll"), b"changed").unwrap();
        assert!(!installed_eml(&profile));
    }

    #[test]
    fn eml_installed_runtime_rejects_self_declared_official_provenance() {
        let root = TempRoot::new();
        fs::write(root.0.join("enshrouded.exe"), b"fixture").unwrap();
        let bytes = dll_fixture();
        fs::write(root.0.join("dbghelp.dll"), &bytes).unwrap();
        let mut proof = proof_fixture(&bytes);
        proof.version = "v0.2.0".to_string();
        fs::create_dir_all(root.0.join(".mod-garrison")).unwrap();
        fs::write(root.0.join(EML_MARKER), serde_json::to_vec(&proof).unwrap()).unwrap();
        assert!(valid_proof(&proof), "the forged marker has a valid schema");
        assert!(
            !installed_eml(&profile(&root.0)),
            "a matching self-declared hash must not establish official runtime identity"
        );
    }

    #[test]
    fn eml_registry_is_not_a_generic_raw_executable_installer() {
        let dependency = DependencySpec {
            id: "runtime:enshrouded-eml".to_string(),
            name: "EML".to_string(),
            provider: "github-release".to_string(),
            source: Some(EML_SOURCE.to_string()),
            version: None,
            required: true,
            status: "missing".to_string(),
            notes: None,
        };
        assert!(is_eml_dependency(&dependency));
        let mut other = dependency;
        other.source = Some("github-release:unknown/repo#setup.exe".to_string());
        assert!(!is_eml_dependency(&other));
    }

    #[test]
    fn bundled_proxy_filename_is_not_enough_to_identify_or_rewrite_eml() {
        let root = TempRoot::new();
        let file = root.0.join("dbghelp.dll");
        fs::write(&file, dll_fixture()).unwrap();
        assert!(!bundled_eml_is_official(&file, "dbghelp.dll"));
        let mut plan = InstallPlan {
            adapter_id: "enshrouded".to_string(),
            adapter_name: "Enshrouded".to_string(),
            confidence: 1.0,
            summary: String::new(),
            mappings: vec![mapping("dbghelp.dll", "game", "dbghelp.dll", "fixture")],
            dependencies: Vec::new(),
            warnings: Vec::new(),
            requires_confirmation: false,
        };
        separate_bundled_eml(&file, &profile(&root.0), &mut plan).unwrap();
        assert_eq!(plan.mappings[0].target_root, "game");
        assert_eq!(plan.mappings[0].target_relative_path, "dbghelp.dll");
        assert!(plan.dependencies.is_empty());
    }

    #[test]
    #[ignore = "Downloads the official EML DLL; never executes it or changes a real game folder."]
    fn live_eml_release_download_verifies_and_produces_a_supported_archive() {
        let root = TempRoot::new();
        fs::write(root.0.join("enshrouded.exe"), b"fixture game executable").unwrap();
        let profile = profile(&root.0);
        let dependency = known_runtime_dependency(&profile, EML_ID);
        let release = resolve_github_release_dependency(&dependency).unwrap();
        let archive = download_eml_archive(&root.0, &dependency, &release).unwrap();
        let scanned = scan_zip_archive(&archive).unwrap();
        let plan = verified_runtime_plan(&scanned, &profile).unwrap();
        assert!(plan_supplies_eml(&plan));
        assert!(bundled_eml_is_official(&archive, "dbghelp.dll"));
        let mut bundled = plan.clone();
        bundled
            .mappings
            .retain(|mapping| mapping.generated_content.is_none());
        separate_bundled_eml(&archive, &profile, &mut bundled).unwrap();
        assert_eq!(bundled.mappings[0].target_root, "profile");
        assert!(bundled.dependencies.iter().any(is_eml_dependency));
        assert_eq!(
            runtime_supplied_by_plan(&profile, &plan).as_deref(),
            Some(EML_ID)
        );
        let analysis = analyze_scanned_archive(scanned, &profile);
        assert!(
            analysis.recommended_plan.is_some(),
            "{}",
            analysis.compatibility.reason
        );
    }
}
