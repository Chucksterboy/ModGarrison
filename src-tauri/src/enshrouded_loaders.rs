//! Enshrouded loader recognition and native proxy configuration.
//!
//! Planning is read-only. Shared chainloader configuration is returned to the
//! caller so it can be deployed through the existing journaled transaction.

use super::*;

const SHROUDTOPIA_HASH: &str = "7a8fa4b339dfc20e6d800fbb01b9d298d57ee7f623e14c606367db206fd385c5";
const SHROUDTOPIA_PROXY_HASH: &str =
    "75159793a140fa37d506d3096e863ee209bac6738d195a22153a444ce52da9c9";
const SHROUDTOPIA_VERSION: &str = "0.2.1-shroudweave.1";
const SHROUDTOPIA_PROOF: &str = ".mod-garrison/enshrouded-shroudtopia.json";
const ALIAS_PREFIX: &str = "mod-garrison-native-";
const MANAGED_BEGIN: &str = "; BEGIN MOD GARRISON NATIVE MODS";
const MANAGED_END: &str = "; END MOD GARRISON NATIVE MODS";
const FORWARD_BEGIN: &str = "; BEGIN MOD GARRISON FORWARD TARGET";
const FORWARD_END: &str = "; END MOD GARRISON FORWARD TARGET";
const MAX_NATIVE_DLL: u64 = 128 * 1024 * 1024;
const PROXY_HASH: &str = "dca7bcdc33368419a5bdab7eb740f62008be003f84e50704bd71c6dbad440c75";
const PROXY_PROOF: &str = ".mod-garrison/enshrouded-proxy.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ManifestLoader {
    Eml,
    Shroudtopia,
    Flame,
    UnknownNative,
}

fn read_member(scanned: &ScannedArchive, path: &str, maximum: u64) -> Result<Vec<u8>, String> {
    read_source_member(Path::new(&scanned.archive_path), path, maximum)
}

/// Scanner and deployment paths use Windows separators and case identities.
/// ZIP's by_name is byte-exact, so resolve the unique scanned identity before
/// reading packages whose authors stored backslash member names.
pub(super) fn zip_member_index<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    path: &str,
) -> Result<usize, String> {
    let identity = normalized_archive_identity(path)?;
    let mut found = None;
    for index in 0..archive.len() {
        let member = archive.by_index(index).map_err(error_to_string)?;
        if normalized_archive_identity(member.name())? == identity && found.replace(index).is_some()
        {
            return Err("The Enshrouded archive has ambiguous Windows member paths.".to_string());
        }
    }
    found.ok_or_else(|| format!("Enshrouded archive entry is missing: {path}"))
}

fn read_source_member(source: &Path, path: &str, maximum: u64) -> Result<Vec<u8>, String> {
    validate_archive_relative_path(path)?;
    let mut bytes = Vec::new();
    if source.is_dir() {
        let member = safe_join(source, path)?;
        validate_deployment_containment(source, &member)?;
        let file = File::open(member).map_err(error_to_string)?;
        if file.metadata().map_err(error_to_string)?.len() > maximum {
            return Err("Enshrouded package metadata or DLL is too large.".to_string());
        }
        file.take(maximum + 1)
            .read_to_end(&mut bytes)
            .map_err(error_to_string)?;
    } else {
        let mut zip = ZipArchive::new(File::open(source).map_err(error_to_string)?)
            .map_err(error_to_string)?;
        let index = zip_member_index(&mut zip, path)?;
        let file = zip.by_index(index).map_err(error_to_string)?;
        if file.size() > maximum {
            return Err("Enshrouded package metadata or DLL is too large.".to_string());
        }
        file.take(maximum + 1)
            .read_to_end(&mut bytes)
            .map_err(error_to_string)?;
    }
    if bytes.len() as u64 > maximum {
        return Err("Enshrouded package metadata or DLL is too large.".to_string());
    }
    Ok(bytes)
}

pub(super) fn native_member_is_x64_dll(scanned: &ScannedArchive, path: &str) -> bool {
    read_member(scanned, path, MAX_NATIVE_DLL)
        .is_ok_and(|bytes| enshrouded_runtime::validate_x64_dll(&bytes).is_ok())
}

/// Read an export from the PE export directory rather than matching an
/// arbitrary string embedded in a DLL's data or debug information.
fn dll_exports(bytes: &[u8], expected: &str) -> bool {
    if enshrouded_runtime::validate_x64_dll(bytes).is_err() {
        return false;
    }
    fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
        Some(u16::from_le_bytes(
            bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
        ))
    }
    fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
        Some(u32::from_le_bytes(
            bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
        ))
    }
    let parse = || -> Option<bool> {
        let pe = u32_at(bytes, 0x3c)? as usize;
        let optional_size = usize::from(u16_at(bytes, pe + 20)?);
        if optional_size < 120 {
            return Some(false);
        }
        let sections = usize::from(u16_at(bytes, pe + 6)?);
        let section_table = pe.checked_add(24)?.checked_add(optional_size)?;
        let rva_to_offset = |rva: u32| -> Option<usize> {
            for index in 0..sections {
                let start = section_table.checked_add(index.checked_mul(40)?)?;
                let virtual_size = u32_at(bytes, start + 8)?;
                let address = u32_at(bytes, start + 12)?;
                let raw_size = u32_at(bytes, start + 16)?;
                let raw = u32_at(bytes, start + 20)?;
                if rva >= address && rva - address < virtual_size.max(raw_size) {
                    let relative = rva - address;
                    if relative >= raw_size {
                        return None;
                    }
                    let offset = (raw as usize).checked_add(relative as usize)?;
                    return (offset < bytes.len()).then_some(offset);
                }
            }
            None
        };
        let export_rva = u32_at(bytes, pe + 24 + 112)?;
        if export_rva == 0 {
            return Some(false);
        }
        let export = rva_to_offset(export_rva)?;
        let count = usize::try_from(u32_at(bytes, export + 24)?).ok()?;
        if count > 4096 {
            return Some(false);
        }
        let names = rva_to_offset(u32_at(bytes, export + 32)?)?;
        for index in 0..count {
            let name_rva = u32_at(bytes, names.checked_add(index.checked_mul(4)?)?)?;
            let name = rva_to_offset(name_rva)?;
            let tail = bytes.get(name..)?;
            if tail.starts_with(expected.as_bytes()) && tail.get(expected.len()).copied() == Some(0)
            {
                return Some(true);
            }
        }
        Some(false)
    };
    parse().unwrap_or(false)
}

pub(super) fn classify_manifest(
    scanned: &ScannedArchive,
    manifest_path: &str,
) -> Result<ManifestLoader, String> {
    let value: serde_json::Value =
        serde_json::from_slice(&read_member(scanned, manifest_path, 512 * 1024)?)
            .map_err(|_| "The Enshrouded mod.json is not valid JSON.".to_string())?;
    let object = value
        .as_object()
        .ok_or_else(|| "The Enshrouded mod.json must be a JSON object.".to_string())?;
    if let Some(id) = object.get("id") {
        if !id.as_str().is_some_and(|id| {
            !id.trim().is_empty()
                && !id.contains(['/', '\\', ':'])
                && !id.chars().any(char::is_control)
        }) {
            return Err("The Enshrouded mod.json contains an invalid mod id.".to_string());
        }
    }
    let parent = archive_path_parent(manifest_path);
    let flame = object.contains_key("flame")
        || scanned.entries.iter().any(|entry| {
            !entry.is_directory
                && archive_path_parent(&entry.path).eq_ignore_ascii_case(&parent)
                && basename(&entry.path).eq_ignore_ascii_case("flame.json")
        });
    if flame {
        return Ok(ManifestLoader::Flame);
    }
    let dlls = scanned
        .entries
        .iter()
        .filter(|entry| {
            !entry.is_directory
                && archive_path_is_within(&entry.path, &parent)
                && entry.path.to_ascii_lowercase().ends_with(".dll")
        })
        .collect::<Vec<_>>();
    if !dlls.is_empty() {
        // A Shroudtopia plugin exports this factory. XHL/Flame and ordinary
        // native DLLs use different interfaces despite also living in mods/.
        for dll in dlls {
            if dll_exports(
                &read_member(scanned, &dll.path, MAX_NATIVE_DLL)?,
                "CreateModInstance",
            ) {
                return Ok(ManifestLoader::Shroudtopia);
            }
        }
        return Ok(ManifestLoader::UnknownNative);
    }
    if object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|id| !id.trim().is_empty())
    {
        Ok(ManifestLoader::Eml)
    } else {
        Err("Enshrouded mod.json is missing its mod id; a loader cannot be determined.".to_string())
    }
}

pub(super) fn native_mod_dependency(
    scanned: &ScannedArchive,
    manifest_path: &str,
) -> Option<DependencySpec> {
    (classify_manifest(scanned, manifest_path).ok()? == ManifestLoader::Shroudtopia).then(|| DependencySpec {
        id: "runtime:enshrouded-shroudtopia".to_string(),
        name: "Shroudtopia (Shroudweave)".to_string(),
        version: None,
        provider: "github-release".to_string(),
        required: true,
        status: "missing".to_string(),
        source: Some("github-release:Aerox912/shroudtopia@v0.2.1-shroudweave.1#Shroudweave-Shroudtopia-*.zip".to_string()),
        notes: Some("Maintained Enshrouded native plugin loader; configuration is preserved on upgrades.".to_string()),
    })
}

fn shroudtopia_pair_is_verified(loader: &[u8], proxy: &[u8]) -> bool {
    enshrouded_runtime::validate_x64_dll(loader).is_ok()
        && enshrouded_runtime::validate_x64_dll(proxy).is_ok()
        && hex::encode(Sha256::digest(loader)) == SHROUDTOPIA_HASH
        && hex::encode(Sha256::digest(proxy)) == SHROUDTOPIA_PROXY_HASH
}

pub(super) fn shroudtopia_runtime_plan(
    scanned: &ScannedArchive,
    profile: &GameProfile,
) -> Option<InstallPlan> {
    if !enshrouded::is_profile(profile) {
        return None;
    }
    let loader = scanned
        .entries
        .iter()
        .find(|entry| !entry.is_directory && entry.path.eq_ignore_ascii_case("shroudtopia.dll"))?;
    let proxy = scanned
        .entries
        .iter()
        .find(|entry| !entry.is_directory && entry.path.eq_ignore_ascii_case("winmm.dll"))?;
    let loader_bytes = read_member(scanned, &loader.path, MAX_NATIVE_DLL).ok()?;
    let proxy_bytes = read_member(scanned, &proxy.path, MAX_NATIVE_DLL).ok()?;
    if !shroudtopia_pair_is_verified(&loader_bytes, &proxy_bytes) {
        return None;
    }
    let game_root = Path::new(&profile.game_path);
    for (name, expected) in [
        ("shroudtopia.dll", SHROUDTOPIA_HASH),
        ("winmm.dll", SHROUDTOPIA_PROXY_HASH),
    ] {
        let destination = game_root.join(name);
        if destination.exists() {
            validate_deployment_containment(game_root, &destination).ok()?;
            if fs::metadata(&destination).ok()?.len() > MAX_NATIVE_DLL
                || sha256_file(&destination).ok()? != expected
            {
                // An unknown proxy may be OptiScaler or another loader. A
                // backup alone does not make replacing its behavior safe.
                return None;
            }
        }
    }
    let mut mappings = Vec::new();
    for file in installable_files(&scanned.entries) {
        let lower = normalize_archive_path(&file.path).to_ascii_lowercase();
        if lower == "shroudtopia.dll" || lower == "winmm.dll" {
            mappings.push(mapping(
                &file.path,
                "game",
                &lower,
                "Verified maintained Shroudtopia runtime DLL.",
            ));
        } else {
            // Keep licenses, example config and release receipts with managed
            // package metadata; they are not extra game-root payloads.
            mappings.push(mapping(
                &file.path,
                "profile",
                &format!("runtime-metadata/enshrouded-shroudtopia/{lower}"),
                "Maintained loader license, example or release metadata.",
            ));
        }
    }
    if !Path::new(&profile.game_path)
        .join("shroudtopia.json")
        .exists()
    {
        mappings.push(generated_mapping("shroudtopia-default-config", "game", "shroudtopia.json", "Create loader configuration only when none exists.", "{\"active\":true,\"bootDelay\":3000,\"updateDelay\":500,\"enableLogging\":true,\"logLevel\":\"INFO\",\"mods\":{}}\n"));
    }
    mappings.push(generated_mapping("shroudtopia-verified-release", "game", SHROUDTOPIA_PROOF, "Verified maintained Shroudtopia DLL identities.", &format!("{{\"version\":\"{SHROUDTOPIA_VERSION}\",\"shroudtopiaSha256\":\"{SHROUDTOPIA_HASH}\",\"winmmSha256\":\"{SHROUDTOPIA_PROXY_HASH}\"}}\n")));
    Some(InstallPlan {
        adapter_id: "enshrouded".to_string(),
        adapter_name: "Enshrouded Shroudtopia Loader".to_string(),
        confidence: 1.0,
        summary:
            "Install the verified maintained Shroudtopia loader and preserve existing settings."
                .to_string(),
        mappings,
        dependencies: Vec::new(),
        warnings: Vec::new(),
        requires_confirmation: false,
    })
}

pub(super) fn plan_supplies_shroudtopia(plan: &InstallPlan) -> bool {
    plan.adapter_id == "enshrouded"
        && plan.mappings.iter().any(|mapping| {
            mapping.target_root == "game"
                && mapping.target_relative_path == SHROUDTOPIA_PROOF
                && mapping.generated_content.as_deref().is_some_and(|text| {
                    text.contains(SHROUDTOPIA_HASH) && text.contains(SHROUDTOPIA_PROXY_HASH)
                })
        })
        && ["shroudtopia.dll", "winmm.dll"].iter().all(|path| {
            plan.mappings.iter().any(|mapping| {
                mapping.target_root == "game"
                    && mapping.target_relative_path == *path
                    && mapping.generated_content.is_none()
            })
        })
}

pub(super) fn installed_shroudtopia(profile: &GameProfile) -> bool {
    if !enshrouded::is_profile(profile) {
        return false;
    }
    let root = Path::new(&profile.game_path);
    let read = |name: &str| -> Option<Vec<u8>> {
        let path = root.join(name);
        validate_deployment_containment(root, &path).ok()?;
        let file = File::open(path).ok()?;
        if file.metadata().ok()?.len() > MAX_NATIVE_DLL {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(MAX_NATIVE_DLL + 1).read_to_end(&mut bytes).ok()?;
        Some(bytes)
    };
    read("shroudtopia.dll")
        .zip(read("winmm.dll"))
        .is_some_and(|(loader, proxy)| shroudtopia_pair_is_verified(&loader, &proxy))
}

#[derive(Debug, Clone)]
pub(super) struct NativeProxyRewrite {
    pub(super) aliases: Vec<String>,
    pub(super) input_forward_target: Option<String>,
}

/// Preserve the root location and all companions; only the proxy basename is
/// changed. The parent must install/own ModLoader Proxy and journal its config.
#[cfg(test)]
pub(super) fn normalize_native_plan(
    plan: &mut InstallPlan,
    package_key: &str,
) -> Result<NativeProxyRewrite, String> {
    if plan.adapter_id != "enshrouded" || package_key.trim().is_empty() {
        return Err("A native Enshrouded package needs a verified package identity.".to_string());
    }
    if plan_supplies_shroudtopia(plan)
        || runtime_supplies_proxy(plan)
        || enshrouded_runtime::plan_supplies_eml(plan)
    {
        return Ok(NativeProxyRewrite {
            aliases: Vec::new(),
            input_forward_target: None,
        });
    }
    let key = hex::encode(Sha256::digest(package_key.as_bytes()));
    let mut candidate = plan.clone();
    let mut aliases = Vec::new();
    let mut input_forward_target = None;
    for mapping in &mut candidate.mappings {
        let path = normalize_archive_path(&mapping.target_relative_path).to_ascii_lowercase();
        if mapping.target_root != "game"
            || mapping.generated_content.is_some()
            || path.contains('/')
        {
            continue;
        }
        if !matches!(
            path.as_str(),
            "dinput8.dll" | "powrprof.dll" | "winmm.dll" | "dbgcore.dll" | "version.dll"
        ) {
            continue;
        }
        let alias = format!("{ALIAS_PREFIX}{}-{path}", &key[..16]);
        if path == "dinput8.dll" {
            input_forward_target = Some(alias.clone());
        }
        mapping.target_relative_path = alias.clone();
        aliases.push(alias);
    }
    let mut destinations = HashSet::new();
    for mapping in &candidate.mappings {
        if !destinations.insert((
            mapping.target_root.to_ascii_lowercase(),
            normalize_archive_path(&mapping.target_relative_path).to_ascii_lowercase(),
        )) {
            return Err("Native DLL aliases collide with another package payload.".to_string());
        }
    }
    aliases.sort();
    aliases.dedup();
    *plan = candidate;
    Ok(NativeProxyRewrite {
        aliases,
        input_forward_target,
    })
}

/// Use the validated DLL's content identity for shared native bootstraps.
/// Different packages bundling the same XHL or Greeneye loader then refer to
/// one root alias rather than starting identical loader copies twice. The
/// deployment layer must still verify shared owners' complete hashes and keep
/// the shared file until its last owner is removed.
pub(super) fn normalize_native_plan_from_source(
    plan: &mut InstallPlan,
    package_key: &str,
    source: &Path,
) -> Result<NativeProxyRewrite, String> {
    if plan.adapter_id != "enshrouded" || package_key.trim().is_empty() {
        return Err("A native Enshrouded package needs a verified package identity.".to_string());
    }
    if plan_supplies_shroudtopia(plan)
        || runtime_supplies_proxy(plan)
        || enshrouded_runtime::plan_supplies_eml(plan)
    {
        return Ok(NativeProxyRewrite {
            aliases: Vec::new(),
            input_forward_target: None,
        });
    }
    let mut candidate = plan.clone();
    let mut aliases = Vec::new();
    let mut input_forward_target = None;
    for mapping in &mut candidate.mappings {
        let path = normalize_archive_path(&mapping.target_relative_path).to_ascii_lowercase();
        if mapping.target_root != "game"
            || mapping.generated_content.is_some()
            || path.contains('/')
        {
            continue;
        }
        if !matches!(
            path.as_str(),
            "dinput8.dll" | "powrprof.dll" | "winmm.dll" | "dbgcore.dll" | "version.dll"
        ) {
            continue;
        }
        let bytes = read_source_member(source, &mapping.source_path, MAX_NATIVE_DLL)?;
        enshrouded_runtime::validate_x64_dll(&bytes)?;
        let digest = hex::encode(Sha256::digest(&bytes));
        let alias = format!("{ALIAS_PREFIX}{}-{path}", &digest[..16]);
        if path == "dinput8.dll" {
            input_forward_target = Some(alias.clone());
        }
        mapping.target_relative_path = alias.clone();
        aliases.push(alias);
    }
    let mut destinations = HashSet::new();
    for mapping in &candidate.mappings {
        if !destinations.insert((
            mapping.target_root.to_ascii_lowercase(),
            normalize_archive_path(&mapping.target_relative_path).to_ascii_lowercase(),
        )) {
            return Err("Native DLL aliases collide with another package payload.".to_string());
        }
    }
    aliases.sort();
    aliases.dedup();
    *plan = candidate;
    Ok(NativeProxyRewrite {
        aliases,
        input_forward_target,
    })
}

fn proxy_target_name(profile: &GameProfile) -> &'static str {
    if Path::new(&profile.game_path)
        .join("enshrouded.exe")
        .is_file()
    {
        "dinput8.dll"
    } else {
        "powrprof.dll"
    }
}

pub(super) fn proxy_runtime_plan(
    scanned: &ScannedArchive,
    profile: &GameProfile,
) -> Option<InstallPlan> {
    if !enshrouded::is_profile(profile) {
        return None;
    }
    let candidates = scanned
        .entries
        .iter()
        .filter(|entry| {
            !entry.is_directory && basename(&entry.path).eq_ignore_ascii_case("modloader-proxy.dll")
        })
        .collect::<Vec<_>>();
    if candidates.len() != 1 {
        return None;
    }
    let dll = candidates[0];
    let bytes = read_member(scanned, &dll.path, MAX_NATIVE_DLL).ok()?;
    if enshrouded_runtime::validate_x64_dll(&bytes).is_err()
        || hex::encode(Sha256::digest(&bytes)) != PROXY_HASH
    {
        return None;
    }
    let root = archive_path_parent(&dll.path);
    let ini = scanned.entries.iter().find(|entry| {
        !entry.is_directory
            && archive_path_parent(&entry.path).eq_ignore_ascii_case(&root)
            && basename(&entry.path).eq_ignore_ascii_case("chainloader.ini")
    })?;
    let target = proxy_target_name(profile);
    let destination = Path::new(&profile.game_path).join(target);
    if destination.exists() {
        validate_deployment_containment(Path::new(&profile.game_path), &destination).ok()?;
        if fs::metadata(&destination).ok()?.len() > MAX_NATIVE_DLL
            || sha256_file(&destination).ok()? != PROXY_HASH
        {
            // Never replace an unrelated or externally installed native proxy.
            return None;
        }
    }
    let ini_path = Path::new(&profile.game_path).join("chainloader.ini");
    let template = if ini_path.exists() {
        validate_deployment_containment(Path::new(&profile.game_path), &ini_path).ok()?;
        if fs::metadata(&ini_path).ok()?.len() > 512 * 1024 {
            return None;
        }
        fs::read_to_string(&ini_path).ok()?
    } else {
        String::from_utf8(read_member(scanned, &ini.path, 512 * 1024).ok()?).ok()?
    };
    let config = chainloader_config(&template, &[], None).ok()?;
    let mut mappings = Vec::new();
    for file in installable_files(&scanned.entries) {
        if file.path == dll.path {
            mappings.push(mapping(
                &file.path,
                "game",
                target,
                "Verified Windows x64 ModLoader Proxy runtime.",
            ));
        } else if file.path == ini.path {
            mappings.push(generated_mapping(
                "enshrouded-proxy-initial-config",
                "game",
                "chainloader.ini",
                "Preserve existing chainloader settings and reserve managed mod entries.",
                &config,
            ));
            // The original template remains in managed package metadata, so
            // companion validation accounts for every source file.
            mappings.push(mapping(
                &file.path,
                "profile",
                "runtime-metadata/enshrouded-proxy/chainloader.template.ini",
                "Original official chainloader template.",
            ));
        } else {
            let relative = archive_path_relative_to(&file.path, &root)?;
            mappings.push(mapping(
                &file.path,
                "profile",
                &format!("runtime-metadata/enshrouded-proxy/{relative}"),
                "Official chainloader license and documentation.",
            ));
        }
    }
    mappings.push(generated_mapping(
        "enshrouded-proxy-provenance",
        "game",
        PROXY_PROOF,
        "Verified ModLoader Proxy identity.",
        &format!("{{\"version\":\"1.1.0\",\"sha256\":\"{PROXY_HASH}\",\"proxy\":\"{target}\"}}\n"),
    ));
    Some(InstallPlan {
        adapter_id: "enshrouded".to_string(),
        adapter_name: "Enshrouded Native Mod Loader".to_string(),
        confidence: 1.0,
        summary: "Install the verified native DLL chainloader while preserving its configuration."
            .to_string(),
        mappings,
        dependencies: Vec::new(),
        warnings: Vec::new(),
        requires_confirmation: false,
    })
}

pub(super) fn runtime_supplies_proxy(plan: &InstallPlan) -> bool {
    plan.adapter_id == "enshrouded"
        && plan.mappings.iter().any(|mapping| {
            mapping.target_root == "game"
                && mapping.target_relative_path == PROXY_PROOF
                && mapping
                    .generated_content
                    .as_deref()
                    .is_some_and(|content| content.contains(PROXY_HASH))
        })
        && plan.mappings.iter().any(|mapping| {
            mapping.target_root == "game"
                && matches!(
                    mapping.target_relative_path.as_str(),
                    "dinput8.dll" | "powrprof.dll"
                )
                && basename(&mapping.source_path).eq_ignore_ascii_case("modloader-proxy.dll")
                && mapping.generated_content.is_none()
        })
}

pub(super) fn installed_proxy(profile: &GameProfile) -> bool {
    if !enshrouded::is_profile(profile) {
        return false;
    }
    let root = Path::new(&profile.game_path);
    let dll = root.join(proxy_target_name(profile));
    validate_deployment_containment(root, &dll).is_ok()
        && fs::metadata(&dll).is_ok_and(|metadata| metadata.len() <= MAX_NATIVE_DLL)
        && sha256_file(&dll).is_ok_and(|hash| hash == PROXY_HASH)
}

/// Only packages whose author documents input-export forwarding are selected.
/// There can only be one such target in the proxy; ambiguity must be surfaced
/// by the caller instead of silently choosing another mod.
pub(super) fn active_input_forward_target(
    records: &[InstalledModRecord],
    profile_id: &str,
) -> Result<Option<String>, String> {
    let mut targets = Vec::new();
    for record in records
        .iter()
        .filter(|record| record.profile_id == profile_id && record.enabled)
    {
        let identity = record
            .package_id
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_xhl = ["nexus:enshrouded/236", "nexus:enshrouded/182"].contains(&identity.as_str());
        if !is_xhl {
            continue;
        }
        if let Some(plan) = record.plan.as_ref() {
            for mapping in &plan.mappings {
                let path = normalize_archive_path(&mapping.target_relative_path);
                if mapping.target_root == "game"
                    && is_native_alias(&path)
                    && path.ends_with("-dinput8.dll")
                {
                    targets.push(path);
                }
            }
        }
    }
    targets.sort();
    targets.dedup();
    if targets.len() > 1 {
        return Err("More than one enabled native package requires the input forwarding slot; install the shared XHL Core instead of conflicting bundled copies.".to_string());
    }
    Ok(targets.pop())
}

pub(super) fn active_native_aliases(
    records: &[InstalledModRecord],
    profile_id: &str,
) -> Vec<String> {
    let mut aliases = records
        .iter()
        .filter(|record| record.profile_id == profile_id && record.enabled)
        .filter_map(|record| record.plan.as_ref())
        .filter(|plan| plan.adapter_id == "enshrouded")
        .flat_map(|plan| plan.mappings.iter())
        .filter(|mapping| mapping.target_root == "game" && mapping.generated_content.is_none())
        .filter_map(|mapping| {
            let path = normalize_archive_path(&mapping.target_relative_path);
            is_native_alias(&path).then_some(path)
        })
        .collect::<Vec<_>>();
    aliases.sort_by_key(|path| path.to_ascii_lowercase());
    aliases.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    aliases
}

/// Greeneye's dbgcore bootstrap is shared by its modules. Distinct versions
/// cannot safely be started together, even when unique filenames avoid an
/// on-disk collision. Identical content aliases have already been deduplicated.
pub(super) fn validate_active_alias_families(
    records: &[InstalledModRecord],
    profile_id: &str,
) -> Result<(), String> {
    let greeneye = active_native_aliases(records, profile_id)
        .into_iter()
        .filter(|alias| alias.ends_with("-dbgcore.dll"))
        .collect::<Vec<_>>();
    if greeneye.len() > 1 {
        return Err("The enabled Enshrouded mods bundle different Greeneye loader versions. Update the affected mods to the same loader version before enabling them together.".to_string());
    }
    Ok(())
}

fn is_native_alias(path: &str) -> bool {
    path.starts_with(ALIAS_PREFIX)
        && path.ends_with(".dll")
        && !path.contains(['/', '\\', ':', '\n', '\r'])
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn remove_managed_block(lines: &[String], begin: &str, end: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut inside = false;
    let mut found = false;
    for line in lines {
        if line.trim() == begin {
            if inside || found {
                return Err("The chainloader config has duplicate managed blocks.".to_string());
            }
            inside = true;
            found = true;
        } else if line.trim() == end {
            if !inside {
                return Err("The chainloader config has an unmatched managed block.".to_string());
            }
            inside = false;
        } else if !inside {
            out.push(line.clone());
        }
    }
    if inside {
        return Err("The chainloader config has an incomplete managed block.".to_string());
    }
    Ok(out)
}

/// Generate only managed portions of chainloader.ini. The caller verifies and
/// updates the canonical runtime receipt in the same installation transaction.
pub(super) fn chainloader_config(
    existing: &str,
    aliases: &[String],
    forward: Option<&str>,
) -> Result<String, String> {
    if existing.len() > 512 * 1024 || existing.contains('\0') {
        return Err("The chainloader configuration is invalid or too large.".to_string());
    }
    if aliases.iter().any(|path| !is_native_alias(path))
        || forward.is_some_and(|path| !is_native_alias(path))
    {
        return Err("A native chainloader entry is not a verified managed DLL alias.".to_string());
    }
    if forward.is_some_and(|target| {
        !aliases
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(target))
    }) {
        return Err("The native input forwarding target is not an active mod.".to_string());
    }
    let original = if existing.trim().is_empty() {
        "[Settings]\nDeferred=true\nLog=true\nLoadFromFolder=false\n\n[Load]\n"
    } else {
        existing
    };
    let lines = original.lines().map(str::to_string).collect::<Vec<_>>();
    let lines = remove_managed_block(&lines, MANAGED_BEGIN, MANAGED_END)?;
    let mut lines = remove_managed_block(&lines, FORWARD_BEGIN, FORWARD_END)?;
    let mut section = String::new();
    let mut settings = None;
    let mut load = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed.to_ascii_lowercase();
            if section == "[settings]" && settings.replace(index).is_some() {
                return Err("The chainloader config has duplicate Settings sections.".to_string());
            }
            if section == "[load]" && load.replace(index).is_some() {
                return Err("The chainloader config has duplicate Load sections.".to_string());
            }
        } else if section == "[settings]" {
            if let Some((key, value)) = trimmed.split_once('=') {
                if key.trim().eq_ignore_ascii_case("ForwardTo")
                    && !value.trim().is_empty()
                    && forward.is_some_and(|target| !target.eq_ignore_ascii_case(value.trim()))
                {
                    return Err("The chainloader already forwards input to another DLL; replacing that choice is not safe.".to_string());
                }
            }
        }
    }
    if settings.is_none() {
        lines.splice(0..0, ["[Settings]".to_string(), String::new()]);
    }
    if let Some(target) = forward {
        let settings = lines
            .iter()
            .position(|line| line.trim().eq_ignore_ascii_case("[Settings]"))
            .unwrap();
        // Do not duplicate an identical user-owned forwarding directive.
        let already_present = lines
            .iter()
            .skip(settings + 1)
            .take_while(|line| !line.trim().starts_with('['))
            .any(|line| {
                line.split_once('=').is_some_and(|(key, value)| {
                    key.trim().eq_ignore_ascii_case("ForwardTo")
                        && value.trim().eq_ignore_ascii_case(target)
                })
            });
        if !already_present {
            lines.splice(
                settings + 1..settings + 1,
                [
                    FORWARD_BEGIN.to_string(),
                    format!("ForwardTo={target}"),
                    FORWARD_END.to_string(),
                ],
            );
        }
    }
    let load = if let Some(index) = lines
        .iter()
        .position(|line| line.trim().eq_ignore_ascii_case("[Load]"))
    {
        index
    } else {
        lines.push(String::new());
        lines.push("[Load]".to_string());
        lines.len() - 1
    };
    let mut aliases = aliases.to_vec();
    aliases.sort_by_key(|path| path.to_ascii_lowercase());
    aliases.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    let mut block = vec![MANAGED_BEGIN.to_string()];
    block.extend(aliases);
    block.push(MANAGED_END.to_string());
    lines.splice(load + 1..load + 1, block);
    Ok(format!("{}\n", lines.join("\n")))
}

#[cfg(test)]
pub(super) fn native_dll_fixture(exports: &[&str]) -> Vec<u8> {
    let mut bytes = vec![0_u8; 2048];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&128_u32.to_le_bytes());
    bytes[128..132].copy_from_slice(b"PE\0\0");
    bytes[132..134].copy_from_slice(&0x8664_u16.to_le_bytes());
    bytes[134..136].copy_from_slice(&1_u16.to_le_bytes());
    bytes[148..150].copy_from_slice(&240_u16.to_le_bytes());
    bytes[150..152].copy_from_slice(&0x2000_u16.to_le_bytes());
    bytes[152..154].copy_from_slice(&0x20b_u16.to_le_bytes());
    bytes[264..268].copy_from_slice(&0x1000_u32.to_le_bytes());
    bytes[268..272].copy_from_slice(&256_u32.to_le_bytes());
    bytes[400..404].copy_from_slice(&1536_u32.to_le_bytes());
    bytes[404..408].copy_from_slice(&0x1000_u32.to_le_bytes());
    bytes[408..412].copy_from_slice(&1536_u32.to_le_bytes());
    bytes[412..416].copy_from_slice(&512_u32.to_le_bytes());
    bytes[536..540].copy_from_slice(&(exports.len() as u32).to_le_bytes());
    bytes[544..548].copy_from_slice(&0x1040_u32.to_le_bytes());
    let mut name_offset = 640;
    for (index, export) in exports.iter().enumerate() {
        let rva = 0x1000 + (name_offset - 512) as u32;
        bytes[576 + index * 4..580 + index * 4].copy_from_slice(&rva.to_le_bytes());
        bytes[name_offset..name_offset + export.len()].copy_from_slice(export.as_bytes());
        name_offset += export.len() + 1;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("mod-garrison-native-test-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, path: &str, bytes: &[u8]) {
            let destination = self.0.join(path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(destination, bytes).unwrap();
        }
        fn scan(&self) -> ScannedArchive {
            scan_folder_source(&self.0, "native-fixture".to_string()).unwrap()
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn plan(paths: &[&str]) -> InstallPlan {
        InstallPlan {
            adapter_id: "enshrouded".to_string(),
            adapter_name: "Enshrouded".to_string(),
            confidence: 1.0,
            summary: "fixture".to_string(),
            mappings: paths
                .iter()
                .map(|path| mapping(path, "game", path, "fixture"))
                .collect(),
            dependencies: Vec::new(),
            warnings: Vec::new(),
            requires_confirmation: false,
        }
    }

    fn record(package: &str, plan: InstallPlan) -> InstalledModRecord {
        serde_json::from_value(serde_json::json!({
            "id":package, "profileId":"native-test", "archivePath":"fixture", "archiveName":"fixture.zip",
            "packageId":package, "adapterId":"enshrouded", "summary":"fixture", "installedAt":"",
            "filesWritten":[], "backupsWritten":[], "dependencies":[], "enabled":true, "plan":plan
        })).unwrap()
    }

    #[test]
    fn enshrouded_factory_detection_reads_export_table_not_embedded_strings() {
        assert!(dll_exports(
            &native_dll_fixture(&["CreateModInstance"]),
            "CreateModInstance"
        ));
        let mut impostor = native_dll_fixture(&["DllMain"]);
        impostor[1500..1517].copy_from_slice(b"CreateModInstance");
        assert!(!dll_exports(&impostor, "CreateModInstance"));
        assert!(!dll_exports(b"CreateModInstance\0", "CreateModInstance"));
    }

    #[test]
    fn enshrouded_native_manifest_does_not_guess_loader_from_dll_extension() {
        let root = TempRoot::new();
        root.write("mods/example/mod.json", br#"{"id":"example"}"#);
        root.write(
            "mods/example/example.dll",
            &native_dll_fixture(&["OtherExport"]),
        );
        let scanned = root.scan();
        assert_eq!(
            classify_manifest(&scanned, "mods/example/mod.json").unwrap(),
            ManifestLoader::UnknownNative
        );
        assert!(native_mod_dependency(&scanned, "mods/example/mod.json").is_none());
    }

    #[test]
    fn enshrouded_native_manifest_resolves_maintained_shroudtopia_dependency() {
        let root = TempRoot::new();
        root.write(
            "mods/minimap_mod/mod.json",
            br#"{"id":"minimap_mod","shroudtopia":{"default":{"active":true}}}"#,
        );
        root.write(
            "mods/minimap_mod/minimap_mod.dll",
            &native_dll_fixture(&["CreateModInstance"]),
        );
        let scanned = root.scan();
        assert_eq!(
            classify_manifest(&scanned, "mods/minimap_mod/mod.json").unwrap(),
            ManifestLoader::Shroudtopia
        );
        let dependency = native_mod_dependency(&scanned, "mods/minimap_mod/mod.json").unwrap();
        assert_eq!(dependency.id, "runtime:enshrouded-shroudtopia");
        assert!(dependency
            .source
            .unwrap()
            .contains("@v0.2.1-shroudweave.1#"));
    }

    #[test]
    fn enshrouded_flame_manifest_is_not_loaded_by_shroudtopia_or_eml() {
        let root = TempRoot::new();
        root.write("mods/creative/mod.json", br#"{"id":"creative"}"#);
        root.write("mods/creative/flame.json", br#"{}"#);
        root.write(
            "mods/creative/creative.dll",
            &native_dll_fixture(&["CreateModInstance"]),
        );
        assert_eq!(
            classify_manifest(&root.scan(), "mods/creative/mod.json").unwrap(),
            ManifestLoader::Flame
        );
    }

    #[test]
    fn enshrouded_script_manifest_requires_valid_metadata() {
        let root = TempRoot::new();
        root.write("mod.json", br#"{"id":"data_mod"}"#);
        root.write("src/mod.lua", b"-- fixture");
        assert_eq!(
            classify_manifest(&root.scan(), "mod.json").unwrap(),
            ManifestLoader::Eml
        );
        root.write("mod.json", b"not json");
        assert!(classify_manifest(&root.scan(), "mod.json").is_err());
    }

    #[test]
    fn enshrouded_shroudtopia_runtime_rejects_unverified_dll_pair() {
        assert!(!shroudtopia_pair_is_verified(
            &native_dll_fixture(&["CreateModInstance"]),
            &native_dll_fixture(&["timeGetTime"])
        ));
    }

    #[test]
    fn enshrouded_proxy_rewrite_preserves_root_companions_and_stable_identity() {
        let mut first = plan(&["dinput8.dll", "climb-anywhere.ini", "climb-assets/data.bin"]);
        let rewrite = normalize_native_plan(&mut first, "nexus:enshrouded/214").unwrap();
        assert_eq!(rewrite.aliases.len(), 1);
        assert!(!rewrite.aliases[0].contains('/'));
        assert_eq!(first.mappings[1].target_relative_path, "climb-anywhere.ini");
        assert_eq!(
            first.mappings[2].target_relative_path,
            "climb-assets/data.bin"
        );
        assert_eq!(
            rewrite.input_forward_target.as_ref(),
            rewrite.aliases.first()
        );
        let mut update = plan(&["dinput8.dll"]);
        assert_eq!(
            normalize_native_plan(&mut update, "nexus:enshrouded/214")
                .unwrap()
                .aliases,
            rewrite.aliases
        );
        let mut another = plan(&["dinput8.dll"]);
        assert_ne!(
            normalize_native_plan(&mut another, "nexus:enshrouded/165")
                .unwrap()
                .aliases,
            rewrite.aliases
        );
    }

    #[test]
    fn enshrouded_proxy_config_is_idempotent_preserves_manual_settings_and_loads() {
        let alias = "mod-garrison-native-1234567890abcdef-dinput8.dll".to_string();
        let original = "[Settings]\nDeferred=false\nCustom=preserve\n[Load]\nmanual.dll\n[Other]\nsetting=keep\n";
        let generated =
            chainloader_config(original, std::slice::from_ref(&alias), Some(&alias)).unwrap();
        assert!(generated.contains("Deferred=false"));
        assert!(generated.contains("manual.dll\n"));
        assert!(generated.contains("[Other]\nsetting=keep"));
        assert_eq!(
            chainloader_config(&generated, std::slice::from_ref(&alias), Some(&alias)).unwrap(),
            generated
        );
        let disabled = chainloader_config(&generated, &[], None).unwrap();
        assert!(!disabled.contains(&alias));
        assert!(disabled.contains("manual.dll"));
    }

    #[test]
    fn enshrouded_proxy_config_rejects_unsafe_entries_and_conflicting_forward_target() {
        let alias = "mod-garrison-native-1234567890abcdef-dinput8.dll".to_string();
        assert!(chainloader_config("", &["../evil.dll".to_string()], None).is_err());
        assert!(chainloader_config("", &["normal.dll\n[Settings]".to_string()], None).is_err());
        assert!(chainloader_config(
            "[Settings]\nForwardTo=user.dll\n[Load]\n",
            std::slice::from_ref(&alias),
            Some(&alias)
        )
        .is_err());
        assert!(chainloader_config("", &[], Some(&alias)).is_err());
        assert!(chainloader_config(MANAGED_BEGIN, &[], None).is_err());
    }

    #[test]
    fn enshrouded_proxy_rewrite_does_not_rename_eml_or_nested_dlls() {
        let mut native = plan(&["dinput8.dll", "dbghelp.dll", "mods/creative/creative.dll"]);
        normalize_native_plan(&mut native, "fixture").unwrap();
        assert_eq!(native.mappings[1].target_relative_path, "dbghelp.dll");
        assert_eq!(
            native.mappings[2].target_relative_path,
            "mods/creative/creative.dll"
        );
    }

    #[test]
    fn enshrouded_source_proxy_alias_deduplicates_identical_bundled_loaders() {
        let root = TempRoot::new();
        root.write("dinput8.dll", &native_dll_fixture(&["DllMain"]));
        let mut first = plan(&["dinput8.dll", "first-mod.ini"]);
        let mut second = plan(&["dinput8.dll", "second-mod.ini"]);
        let first_alias =
            normalize_native_plan_from_source(&mut first, "nexus:enshrouded/236", &root.0).unwrap();
        let second_alias =
            normalize_native_plan_from_source(&mut second, "nexus:enshrouded/182", &root.0)
                .unwrap();
        assert_eq!(first_alias.aliases, second_alias.aliases);
        assert!(!first_alias.aliases[0].contains('/'));
        assert_eq!(first.mappings[1].target_relative_path, "first-mod.ini");
        assert_eq!(second.mappings[1].target_relative_path, "second-mod.ini");
        let records = vec![
            record("nexus:enshrouded/236", first),
            record("nexus:enshrouded/182", second),
        ];
        assert_eq!(
            active_native_aliases(&records, "native-test"),
            first_alias.aliases
        );
        assert_eq!(
            active_input_forward_target(&records, "native-test").unwrap(),
            first_alias.input_forward_target
        );
    }

    #[test]
    fn enshrouded_source_proxy_alias_distinguishes_versions_and_blocks_mixed_greeneye() {
        let root = TempRoot::new();
        let mut bytes = native_dll_fixture(&["DllMain"]);
        root.write("dbgcore.dll", &bytes);
        let mut first = plan(&["dbgcore.dll"]);
        let first_alias =
            normalize_native_plan_from_source(&mut first, "nexus:enshrouded/220", &root.0).unwrap();
        bytes[1900] ^= 1;
        root.write("dbgcore.dll", &bytes);
        let mut second = plan(&["dbgcore.dll"]);
        let second_alias =
            normalize_native_plan_from_source(&mut second, "nexus:enshrouded/195", &root.0)
                .unwrap();
        assert_ne!(first_alias.aliases, second_alias.aliases);
        let records = vec![
            record("nexus:enshrouded/220", first),
            record("nexus:enshrouded/195", second),
        ];
        assert!(validate_active_alias_families(&records, "native-test").is_err());
        assert!(validate_active_alias_families(&records[..1], "native-test").is_ok());
        assert!(validate_active_alias_families(&records, "another-profile").is_ok());
    }

    #[test]
    fn enshrouded_source_proxy_alias_reads_actual_zip_member_bytes() {
        let root = TempRoot::new();
        let bytes = native_dll_fixture(&["DllMain"]);
        root.write("dinput8.dll", &bytes);
        let archive_path = root.0.join("package.zip");
        let mut zip = ZipWriter::new(File::create(&archive_path).unwrap());
        zip.start_file("wrapper/dinput8.dll", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&bytes).unwrap();
        zip.finish().unwrap();
        let mut folder_plan = plan(&["dinput8.dll"]);
        let mut zip_plan = plan(&["dinput8.dll"]);
        zip_plan.mappings[0].source_path = "wrapper/dinput8.dll".to_string();
        assert_eq!(
            normalize_native_plan_from_source(&mut folder_plan, "folder", &root.0)
                .unwrap()
                .aliases,
            normalize_native_plan_from_source(&mut zip_plan, "zip", &archive_path)
                .unwrap()
                .aliases
        );
    }

    #[test]
    fn enshrouded_source_proxy_alias_rejects_invalid_binary_without_changing_plan() {
        let root = TempRoot::new();
        root.write("dinput8.dll", &native_dll_fixture(&["DllMain"]));
        root.write("winmm.dll", b"not a Windows DLL");
        let mut native = plan(&["dinput8.dll", "winmm.dll"]);
        assert!(normalize_native_plan_from_source(&mut native, "fixture", &root.0).is_err());
        assert_eq!(native.mappings[0].target_relative_path, "dinput8.dll");
        assert_eq!(native.mappings[1].target_relative_path, "winmm.dll");
        native.mappings[0].source_path = "../outside.dll".to_string();
        assert!(normalize_native_plan_from_source(&mut native, "fixture", &root.0).is_err());
    }
}
