use super::*;

const DEFAULT_MODIO_API_BASE: &str = "https://api.mod.io/v1";
const MODIO_MAX_PAGE_SIZE: usize = 100;
const MODIO_DOWNLOAD_EXPIRY_MARGIN_SECONDS: i64 = 30;
const MODIO_AUTHORISED_DOWNLOADS: u64 = 4;
const MODIO_PAID_DOWNLOADS: u64 = 8;
static MODIO_API_BASE_CACHE: OnceLock<Result<String, String>> = OnceLock::new();

#[derive(Debug, Clone, Default, Deserialize)]
struct ModioCollection<T> {
    #[serde(default)]
    data: Vec<T>,
    #[serde(default)]
    result_total: usize,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ModioGame {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    status: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    api_access_options: Option<u64>,
    #[serde(default)]
    other_urls: Vec<ModioLink>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ModioLink {
    #[serde(default)]
    url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ModioMod {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    game_id: u64,
    #[serde(default)]
    status: i64,
    #[serde(default)]
    visible: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description_plaintext: String,
    #[serde(default)]
    profile_url: String,
    #[serde(default)]
    homepage_url: String,
    #[serde(default)]
    date_added: i64,
    #[serde(default)]
    date_updated: i64,
    #[serde(default)]
    date_live: i64,
    #[serde(default)]
    submitted_by: ModioUser,
    #[serde(default)]
    logo: ModioLogo,
    #[serde(default)]
    modfile: Option<ModioFile>,
    #[serde(default)]
    dependencies: bool,
    #[serde(default)]
    revenue_type: Option<i64>,
    #[serde(default)]
    tags: Vec<ModioTag>,
    #[serde(default)]
    stats: ModioStats,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ModioUser {
    #[serde(default)]
    username: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ModioLogo {
    #[serde(default)]
    original: String,
    #[serde(default)]
    thumb_320x180: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ModioTag {
    #[serde(default)]
    name: String,
    #[serde(default)]
    name_localized: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ModioStats {
    #[serde(default)]
    downloads_total: u64,
    #[serde(default)]
    ratings_weighted_aggregate: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ModioFile {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    mod_id: u64,
    #[serde(default)]
    date_added: i64,
    #[serde(default)]
    date_updated: i64,
    #[serde(default)]
    virus_status: i64,
    #[serde(default)]
    virus_positive: i64,
    #[serde(default)]
    filesize: u64,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    changelog: String,
    #[serde(default)]
    download: ModioDownload,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ModioDownload {
    #[serde(default)]
    binary_url: String,
    #[serde(default)]
    date_expires: i64,
}

pub(super) fn validate_api_key(api_key: &str) -> Result<(), String> {
    validate_api_key_shape(api_key)?;
    let client = provider_client()?;
    let url = format!("{}/games", modio_api_base()?);
    let response = send_get(&client, &url, api_key, &[("_limit", "1".to_string())])?;
    parse_json::<ModioCollection<ModioGame>>(response, "game catalogue")?;
    Ok(())
}

pub(super) fn resolve_mapping(
    profile: &GameProfile,
    api_key: &str,
    force_refresh: bool,
) -> Result<Option<ProviderGameMapping>, String> {
    validate_api_key_shape(api_key)?;
    let canonical_game = verified_steam_game_for_profile(profile)?;
    let canonical_name = canonical_game.name;
    let steam_app_id = profile
        .steam_app_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "Automatic mod.io catalogue matching requires a verified Steam App ID.".to_string()
        })?
        .to_string();
    let candidates = provider_name_candidates(profile);
    let game_id = cached_modio_game_id(profile, force_refresh, || {
        let client = provider_client()?;
        let mut matches = HashMap::new();
        for candidate in &candidates {
            let games = fetch_games_by_name(&client, api_key, candidate)?;
            for game in games
                .into_iter()
                .filter(|game| modio_game_matches_profile(game, &candidates, &steam_app_id))
            {
                matches.entry(game.id).or_insert(game);
            }
        }
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.into_keys().next().map(|id| id.to_string())),
            _ => Err(format!(
                    "mod.io returned multiple exact catalogue matches for '{}', so Mod Garrison refused to guess.",
                    canonical_name
                )),
        }
    })?;

    let Some(game_id) = game_id else {
        return Ok(None);
    };
    let client = provider_client()?;
    let provider_game = fetch_game_by_id(&client, api_key, &game_id)?;
    if !modio_game_matches_profile(&provider_game, &candidates, &steam_app_id) {
        return Ok(None);
    }
    let resolution = if modio_game_steam_app_ids(&provider_game).is_empty() {
        "catalog-exact-name"
    } else {
        "catalog-exact-name+steam-link"
    };
    Ok(Some(ProviderGameMapping {
        provider: "modio".to_string(),
        steam_app_id,
        canonical_game_name: canonical_name,
        provider_game_id: game_id.clone(),
        provider_catalogue_id: Some(game_id),
        resolution: resolution.to_string(),
        verified_at: now_string(),
    }))
}

/// A successful catalogue miss is cacheable; a failed lookup is not a miss.
/// Keep errors out of the shared cache so a later request can retry immediately
/// and cannot mistake an authentication/network/ambiguity failure for absence.
fn cached_modio_game_id<F>(
    profile: &GameProfile,
    force_refresh: bool,
    resolve: F,
) -> Result<Option<String>, String>
where
    F: FnOnce() -> Result<Option<String>, String>,
{
    cached_provider_mapping_result("modio", profile, force_refresh, resolve)
}

pub(super) fn discover_mods(
    store_root: &Path,
    profile: &GameProfile,
    api_key: &str,
    provider_game_id: &str,
    fetch: NexusDiscoveryFetch<'_>,
) -> Result<DiscoveryProviderBatch, String> {
    validate_numeric_id(provider_game_id, "mod.io game")?;
    let client = provider_client()?;
    let provider_game = fetch_game_by_id(&client, api_key, provider_game_id)?;
    let verified_game = verified_steam_game_for_profile(profile)?;
    let candidates = provider_name_candidates(profile);
    if provider_game.id.to_string() != provider_game_id
        || !modio_game_matches_profile(&provider_game, &candidates, &verified_game.app_id)
    {
        return Err(
            "The linked mod.io game catalogue no longer matches the selected Steam game. Refresh the profile and try again."
                .to_string(),
        );
    }
    let direct_install_supported = modio_game_allows_api_key_downloads(&provider_game);
    let installed_keys = installed_package_identity_keys(store_root, &profile.id)?;
    let mut records = Vec::new();
    let mut offset = fetch.start_offset;
    let mut total = usize::MAX;
    let mut filtered_out = 0_usize;
    let mut complete = false;

    while offset < total
        && offset < DISCOVERY_PROVIDER_CACHE_MAX_RECORDS
        && records.len() < fetch.max_new_results
    {
        ensure_discovery_is_current(&profile.id, fetch.request_id)?;
        let remaining = fetch.max_new_results.saturating_sub(records.len());
        let uncached = DISCOVERY_PROVIDER_CACHE_MAX_RECORDS.saturating_sub(offset);
        let limit = remaining.min(uncached).clamp(1, MODIO_MAX_PAGE_SIZE);
        let mut query = vec![
            ("_offset", offset.to_string()),
            ("_limit", limit.to_string()),
            ("_sort", modio_sort(fetch.sort).to_string()),
            ("revenue_type", "0".to_string()),
        ];
        if !fetch.query.trim().is_empty() {
            query.push(("_q", fetch.query.trim().to_string()));
        }
        let url = format!(
            "{}/games/{}/mods",
            modio_api_base()?,
            sanitize_url_path_segment(provider_game_id)
        );
        let page = parse_json::<ModioCollection<ModioMod>>(
            send_get(&client, &url, api_key, &query)?,
            "mod catalogue",
        )?;
        ensure_discovery_is_current(&profile.id, fetch.request_id)?;
        total = page.result_total;
        let returned = page.data.len();
        if returned == 0 {
            complete = true;
            break;
        }
        for item in page.data {
            if let Some(record) = mod_to_online_record(
                profile,
                provider_game_id,
                item,
                &installed_keys,
                direct_install_supported,
            ) {
                if is_external_mod_manager_listing(&record) {
                    filtered_out = filtered_out.saturating_add(1);
                } else {
                    records.push(record);
                }
            } else {
                filtered_out = filtered_out.saturating_add(1);
            }
        }
        offset = offset.saturating_add(returned);
        complete = offset >= total || offset >= DISCOVERY_PROVIDER_CACHE_MAX_RECORDS;
    }

    Ok(DiscoveryProviderBatch {
        start_offset: fetch.start_offset,
        records,
        raw_total: total.min(DISCOVERY_PROVIDER_CACHE_MAX_RECORDS),
        next_offset: offset,
        filtered_out,
        complete,
    })
}

pub(super) fn available_mod_documents(
    profile: &GameProfile,
    api_key: &str,
    mapping: &ProviderGameMapping,
    requested_limit: usize,
) -> Result<Vec<ProviderRouteDocument>, String> {
    validate_api_key_shape(api_key)?;
    validate_numeric_id(&mapping.provider_game_id, "mod.io game")?;
    let steam_app_id = profile
        .steam_app_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "Automatic mod.io metadata learning requires a verified Steam App ID.".to_string()
        })?;
    if !mapping.provider.eq_ignore_ascii_case("modio") || mapping.steam_app_id != steam_app_id {
        return Err(
            "The saved mod.io game mapping does not match this verified Steam profile.".to_string(),
        );
    }

    let limit = requested_limit
        .min(PROFILE_RUNTIME_SAMPLE_SIZE.max(PROFILE_ROUTE_SAMPLE_SIZE))
        .min(MODIO_MAX_PAGE_SIZE);
    if limit == 0 {
        return Ok(Vec::new());
    }

    let client = provider_client()?;
    let provider_game = fetch_game_by_id(&client, api_key, &mapping.provider_game_id)?;
    let verified_game = verified_steam_game_for_profile(profile)?;
    let candidates = provider_name_candidates(profile);
    if provider_game.id.to_string() != mapping.provider_game_id
        || !modio_game_matches_profile(&provider_game, &candidates, &verified_game.app_id)
    {
        return Err(
            "The saved mod.io catalogue no longer matches the selected Steam game.".to_string(),
        );
    }

    let url = format!(
        "{}/games/{}/mods",
        modio_api_base()?,
        sanitize_url_path_segment(&mapping.provider_game_id)
    );
    let page = parse_json::<ModioCollection<ModioMod>>(
        send_get(
            &client,
            &url,
            api_key,
            &[
                ("_offset", "0".to_string()),
                ("_limit", limit.to_string()),
                ("_sort", modio_sort("downloads").to_string()),
                ("revenue_type", "0".to_string()),
            ],
        )?,
        "mod metadata",
    )?;
    Ok(page
        .data
        .into_iter()
        .filter(|item| mod_is_discoverable_metadata(item, &mapping.provider_game_id))
        .map(|item| {
            let primary_file = item.modfile.as_ref();
            provider_document(&mapping.provider_game_id, &item, primary_file)
        })
        .take(limit)
        .collect())
}

pub(super) fn list_files(
    store_root: &Path,
    profile: &GameProfile,
    api_key: &str,
    mod_id: &str,
    supplied_provider_game_id: Option<&str>,
) -> Result<Vec<OnlineModFileOption>, String> {
    let (game_id, numeric_mod_id) = parse_online_mod_id(mod_id)?;
    verified_mapping(
        store_root,
        profile,
        api_key,
        &game_id,
        supplied_provider_game_id,
    )?;
    ensure_api_key_downloads_supported(api_key, &game_id)?;
    let details = fetch_mod(api_key, &game_id, numeric_mod_id)?;
    ensure_public_free_mod(&details, &game_id)?;
    let files = include_primary_file(
        fetch_mod_files(api_key, &game_id, numeric_mod_id)?,
        details.modfile.as_ref(),
        numeric_mod_id,
    );
    let primary_file_id = details.modfile.as_ref().map(|file| file.id);
    Ok(files
        .into_iter()
        .filter(|file| file.mod_id == numeric_mod_id && file_is_installable(file))
        .map(|file| file_to_option(file, primary_file_id))
        .collect())
}

pub(super) fn preflight(
    store_root: &Path,
    profile: &GameProfile,
    api_key: &str,
    mod_id: &str,
    selected_file_id: Option<&str>,
) -> Result<(Vec<DependencySpec>, Vec<InstallTargetOption>), String> {
    let (game_id, numeric_mod_id) = parse_online_mod_id(mod_id)?;
    let provider_game_id =
        verified_mapping(store_root, profile, api_key, &game_id, None)?.provider_game_id;
    ensure_api_key_downloads_supported(api_key, &game_id)?;
    let details = fetch_mod(api_key, &game_id, numeric_mod_id)?;
    ensure_public_free_mod(&details, &provider_game_id)?;
    ensure_dependencies_supported(&details)?;
    let files = fetch_mod_files(api_key, &game_id, numeric_mod_id)?;
    let file = choose_file(
        &files,
        numeric_mod_id,
        selected_file_id,
        details.modfile.as_ref(),
    )?;
    let document = provider_document(&provider_game_id, &details, Some(&file));
    let provided_runtime = runtime_id_for_provider_package(profile, "modio", None, &details.name);
    let dependencies =
        description_runtime_dependencies(profile, &document.text, provided_runtime.as_deref());
    let install_targets = provider_install_target_options(store_root, profile, &document);
    Ok((dependencies, install_targets))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn install(
    store_root: &Path,
    profile: &GameProfile,
    api_key: &str,
    mod_id: &str,
    version: Option<String>,
    supplied_provider_game_id: Option<&str>,
    selected_file_id: Option<&str>,
    install_target_id: Option<&str>,
) -> Result<InstallResult, String> {
    let (game_id, numeric_mod_id) = parse_online_mod_id(mod_id)?;
    let provider_game_id = verified_mapping(
        store_root,
        profile,
        api_key,
        &game_id,
        supplied_provider_game_id,
    )?
    .provider_game_id;
    ensure_api_key_downloads_supported(api_key, &game_id)?;
    let details = fetch_mod(api_key, &game_id, numeric_mod_id)?;
    ensure_public_free_mod(&details, &provider_game_id)?;
    ensure_dependencies_supported(&details)?;
    let files = fetch_mod_files(api_key, &game_id, numeric_mod_id)?;
    let file = choose_file(
        &files,
        numeric_mod_id,
        selected_file_id,
        details.modfile.as_ref(),
    )?;
    let archive_path = download_file(store_root, &game_id, numeric_mod_id, &file)?;
    let package_id = format!("modio:{game_id}/{numeric_mod_id}");
    let provider_document = provider_document(&provider_game_id, &details, Some(&file));
    let source_identity = provider_source_identity(
        "modio",
        package_id.clone(),
        non_empty_string(file.version.trim()).or(version),
        Some(provider_game_id),
        "mod.io public game catalogue and verified mod file",
    );
    let scanned = scan_import_source(store_root, &archive_path)?;
    let analysis =
        analyze_scanned_archive_with_identity(scanned.clone(), profile, Some(source_identity));
    let install_source_path = analysis.archive_path.clone();
    let package_identity = analysis.package_identity.clone();
    let mut plan = analysis
        .recommended_plan
        .ok_or_else(|| analysis.compatibility.reason.clone())?;
    let route_outcome = prepare_package_install_routes(
        store_root,
        profile,
        &scanned,
        &provider_document,
        &mut plan,
    );
    append_unique_dependencies(
        &mut plan.dependencies,
        description_runtime_dependencies(profile, &provider_document.text, None),
    );
    plan.warnings.extend(route_outcome.warnings);
    let install_targets =
        provider_plan_install_target_options(store_root, profile, &provider_document, &plan);
    let selected_install_target_id = provider_selected_install_target_id(
        store_root,
        profile,
        &provider_document,
        &install_targets,
        install_target_id,
    );
    if plan.adapter_id == "loose-files" || plan.requires_confirmation {
        return Err(format!(
            "{} downloaded, but Mod Garrison could not identify a safe automatic install layout.",
            details.name
        ));
    }

    let mut visited_dependencies = HashSet::new();
    let details_icon_url = mod_logo_url(&details);
    let install_result = install_archive_impl_with_metadata(
        store_root,
        profile,
        &install_source_path,
        &plan,
        InstallOptions {
            metadata: InstallMetadata {
                archive_name: Some(file.filename.clone()),
                package_id: Some(package_id.clone()),
                dependency_string: Some(format!("{package_id}#{}", file.id)),
                package_provider: Some("modio".to_string()),
                package_version: non_empty_string(file.version.trim()),
                provider_file_id: Some(file.id.to_string()),
                provider_variant: non_empty_string(file.version.trim())
                    .or_else(|| Some(file.filename.clone())),
                display_name: Some(details.name),
                package_identity: Some(package_identity),
                runtime_id: None,
                icon_url: details_icon_url,
                install_targets,
                selected_install_target_id,
            },
            resolve_dependencies: true,
            visited_dependencies: &mut visited_dependencies,
            dependency_depth: 0,
        },
    );
    if install_result.is_ok() {
        cleanup_successful_provider_artifacts(
            store_root,
            &archive_path,
            Path::new(&install_source_path),
        );
    }
    install_result
}

pub(super) fn parse_online_mod_id(mod_id: &str) -> Result<(String, u64), String> {
    let raw = mod_id
        .strip_prefix("modio:")
        .ok_or_else(|| format!("Invalid mod.io mod id: {mod_id}"))?;
    let (game_id, mod_id) = raw
        .split_once('/')
        .ok_or_else(|| format!("Invalid mod.io mod id: {mod_id}"))?;
    validate_numeric_id(game_id, "mod.io game")?;
    let mod_id = mod_id
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| format!("Invalid mod.io mod id: {mod_id}"))?;
    Ok((game_id.to_string(), mod_id))
}

fn verified_mapping(
    store_root: &Path,
    profile: &GameProfile,
    api_key: &str,
    encoded_game_id: &str,
    supplied_provider_game_id: Option<&str>,
) -> Result<ProviderGameMapping, String> {
    validate_numeric_id(encoded_game_id, "mod.io game")?;
    if let Some(supplied) = supplied_provider_game_id {
        validate_numeric_id(supplied, "mod.io game")?;
        if supplied != encoded_game_id {
            return Err(
                "The selected mod.io result no longer matches its game catalogue. Refresh Discovery and try again."
                    .to_string(),
            );
        }
    }
    let mapping = resolve_mapping(profile, api_key, true)?.ok_or_else(|| {
        format!(
            "mod.io could not verify an exact game catalogue match for {}.",
            profile_game_label(profile)
        )
    })?;
    if mapping.provider_game_id != encoded_game_id {
        return Err(
            "This mod.io item belongs to a different game catalogue. No files were changed."
                .to_string(),
        );
    }
    remember_provider_game_mapping(store_root, mapping.clone())?;
    Ok(mapping)
}

fn fetch_games_by_name(
    client: &Client,
    api_key: &str,
    name: &str,
) -> Result<Vec<ModioGame>, String> {
    let response = parse_json::<ModioCollection<ModioGame>>(
        send_get(
            client,
            &format!("{}/games", modio_api_base()?),
            api_key,
            &[("_q", name.to_string()), ("_limit", "20".to_string())],
        )?,
        "game catalogue",
    )?;
    Ok(response.data)
}

fn fetch_game_by_id(client: &Client, api_key: &str, game_id: &str) -> Result<ModioGame, String> {
    validate_numeric_id(game_id, "mod.io game")?;
    parse_json(
        send_get(
            client,
            &format!(
                "{}/games/{}",
                modio_api_base()?,
                sanitize_url_path_segment(game_id)
            ),
            api_key,
            &[],
        )?,
        "game catalogue",
    )
}

fn fetch_mod(api_key: &str, game_id: &str, mod_id: u64) -> Result<ModioMod, String> {
    let client = provider_client()?;
    parse_json(
        send_get(
            &client,
            &format!(
                "{}/games/{}/mods/{mod_id}",
                modio_api_base()?,
                sanitize_url_path_segment(game_id)
            ),
            api_key,
            &[],
        )?,
        "mod details",
    )
}

fn fetch_mod_files(api_key: &str, game_id: &str, mod_id: u64) -> Result<Vec<ModioFile>, String> {
    let client = provider_client()?;
    let response = parse_json::<ModioCollection<ModioFile>>(
        send_get(
            &client,
            &format!(
                "{}/games/{}/mods/{mod_id}/files",
                modio_api_base()?,
                sanitize_url_path_segment(game_id)
            ),
            api_key,
            &[
                ("_sort", "-date_added".to_string()),
                ("_limit", MODIO_MAX_PAGE_SIZE.to_string()),
            ],
        )?,
        "mod files",
    )?;
    Ok(response.data)
}

fn mod_to_online_record(
    profile: &GameProfile,
    provider_game_id: &str,
    item: ModioMod,
    installed_keys: &HashSet<String>,
    direct_install_supported: bool,
) -> Option<OnlineModRecord> {
    if !mod_is_discoverable_metadata(&item, provider_game_id) {
        return None;
    }
    let file = item
        .modfile
        .as_ref()
        .filter(|file| file.mod_id == item.id && file_is_installable(file))?;
    let package_id = format!("modio:{provider_game_id}/{}", item.id);
    let installed = installed_keys.contains(&format!("package:{}", package_id.to_lowercase()));
    let description = non_empty_string(item.summary.trim())
        .or_else(|| non_empty_string(item.description_plaintext.trim()))
        .unwrap_or_default();
    let categories = item
        .tags
        .iter()
        .filter_map(|tag| {
            non_empty_string(tag.name_localized.trim())
                .or_else(|| non_empty_string(tag.name.trim()))
        })
        .collect::<Vec<_>>();
    let owner = non_empty_string(item.submitted_by.username.trim())
        .unwrap_or_else(|| "mod.io creator".to_string());
    let icon_url = mod_logo_url(&item);
    let package_url = non_empty_string(item.profile_url.trim())
        .or_else(|| non_empty_string(item.homepage_url.trim()));
    let website_url = non_empty_string(item.profile_url.trim());
    let install_supported = direct_install_supported && !item.dependencies;
    let install_note = if !direct_install_supported {
        Some(
            "This game requires each player to sign in to mod.io before downloading files. API-key-only installation is unavailable."
                .to_string(),
        )
    } else if item.dependencies {
        Some(
            "mod.io reports required mod dependencies but does not provide enough verified dependency information for safe automatic installation."
                .to_string(),
        )
    } else {
        None
    };
    Some(OnlineModRecord {
        id: package_id,
        provider: "modio".to_string(),
        provider_label: "mod.io".to_string(),
        game_id: profile.game_id.clone(),
        provider_game_id: Some(provider_game_id.to_string()),
        name: item.name,
        owner,
        version: file.version.clone(),
        description,
        categories,
        downloads: item.stats.downloads_total,
        rating_score: item.stats.ratings_weighted_aggregate.round() as i64,
        dependency_count: usize::from(item.dependencies),
        file_size: Some(file.filesize),
        icon_url,
        package_url,
        website_url,
        installed,
        created_at: unix_timestamp(item.date_live.max(item.date_added)),
        updated_at: unix_timestamp(item.date_updated),
        install_supported,
        install_note,
    })
}

fn mod_is_discoverable_metadata(item: &ModioMod, provider_game_id: &str) -> bool {
    item.id > 0
        && item.game_id.to_string() == provider_game_id
        && item.status == 1
        && item.visible == 1
        && item.revenue_type == Some(0)
        && !item.name.trim().is_empty()
        && !is_known_runtime_provider_package("modio", None, &item.name)
        && item
            .modfile
            .as_ref()
            .is_some_and(|file| file.mod_id == item.id && file_is_installable(file))
}

fn provider_document(
    provider_game_id: &str,
    item: &ModioMod,
    file: Option<&ModioFile>,
) -> ProviderRouteDocument {
    ProviderRouteDocument {
        provider: "mod.io".to_string(),
        mod_id: format!("modio:{provider_game_id}/{}", item.id),
        mod_name: item.name.clone(),
        text: [
            item.summary.as_str(),
            item.description_plaintext.as_str(),
            file.map(|file| file.changelog.as_str()).unwrap_or_default(),
        ]
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n"),
    }
}

fn ensure_public_free_mod(item: &ModioMod, provider_game_id: &str) -> Result<(), String> {
    if item.id == 0
        || item.game_id.to_string() != provider_game_id
        || item.status != 1
        || item.visible != 1
        || item.revenue_type != Some(0)
    {
        return Err(
            "The selected mod.io item is no longer a free public mod for this game. No files were changed."
                .to_string(),
        );
    }
    Ok(())
}

fn ensure_dependencies_supported(item: &ModioMod) -> Result<(), String> {
    if item.dependencies {
        return Err(format!(
            "mod.io reports required mod dependencies for '{}', but its catalogue response does not provide enough verified dependency information for Mod Garrison to install them safely. No files were downloaded or installed.",
            item.name
        ));
    }
    Ok(())
}

fn mod_logo_url(item: &ModioMod) -> Option<String> {
    [
        item.logo.thumb_320x180.as_str(),
        item.logo.original.as_str(),
    ]
    .into_iter()
    .find_map(|url| {
        let url = url.trim();
        (url.starts_with("https://")).then(|| url.to_string())
    })
}

fn file_to_option(file: ModioFile, primary_file_id: Option<u64>) -> OnlineModFileOption {
    OnlineModFileOption {
        id: file.id.to_string(),
        name: non_empty_string(file.version.trim())
            .map(|version| format!("Version {version}"))
            .unwrap_or_else(|| file.filename.clone()),
        version: non_empty_string(file.version.trim()),
        category: Some("Release".to_string()),
        description: non_empty_string(file.changelog.trim()),
        file_name: Some(file.filename),
        file_size: Some(file.filesize),
        uploaded_at: unix_timestamp(file.date_updated.max(file.date_added)),
        primary: primary_file_id == Some(file.id),
        action: "direct".to_string(),
        download_page_url: None,
    }
}

fn include_primary_file(
    mut files: Vec<ModioFile>,
    primary_file: Option<&ModioFile>,
    expected_mod_id: u64,
) -> Vec<ModioFile> {
    if let Some(primary) = primary_file.filter(|file| {
        file.mod_id == expected_mod_id
            && file_is_installable(file)
            && !files.iter().any(|candidate| candidate.id == file.id)
    }) {
        files.push(primary.clone());
    }
    files
}

fn file_is_installable(file: &ModioFile) -> bool {
    file.id > 0
        && file.mod_id > 0
        && file.virus_status == 1
        && file.virus_positive == 0
        && (1..=MAX_DOWNLOAD_BYTES).contains(&file.filesize)
        && is_supported_archive_name(&file.filename)
        && validate_modio_download_url(file.download.binary_url.trim()).is_ok()
        && (file.download.date_expires <= 0
            || file.download.date_expires
                > Utc::now().timestamp() + MODIO_DOWNLOAD_EXPIRY_MARGIN_SECONDS)
}

fn choose_file(
    files: &[ModioFile],
    expected_mod_id: u64,
    selected_file_id: Option<&str>,
    primary_file: Option<&ModioFile>,
) -> Result<ModioFile, String> {
    if let Some(selected) = selected_file_id {
        let selected = selected
            .parse::<u64>()
            .map_err(|_| "The selected mod.io file id is invalid.".to_string())?;
        if let Some(primary) = primary_file.filter(|file| {
            file.id == selected && file.mod_id == expected_mod_id && file_is_installable(file)
        }) {
            return Ok(primary.clone());
        }
        return files
            .iter()
            .find(|file| {
                file.id == selected && file.mod_id == expected_mod_id && file_is_installable(file)
            })
            .cloned()
            .ok_or_else(|| {
                "The selected mod.io file is no longer a clean supported archive.".to_string()
            });
    }
    if let Some(primary) = primary_file {
        if primary.mod_id == expected_mod_id && file_is_installable(primary) {
            return Ok(primary.clone());
        }
    }
    files
        .iter()
        .filter(|file| file.mod_id == expected_mod_id && file_is_installable(file))
        .max_by_key(|file| (file.date_updated.max(file.date_added), file.id))
        .cloned()
        .ok_or_else(|| {
            "mod.io returned no virus-scanned ZIP, 7Z, or RAR archive for this mod.".to_string()
        })
}

fn download_file(
    store_root: &Path,
    game_id: &str,
    mod_id: u64,
    file: &ModioFile,
) -> Result<PathBuf, String> {
    let safe_name = sanitize_file_segment(&file.filename);
    let download_dir = store_root
        .join("downloads")
        .join("modio")
        .join(sanitize_file_segment(game_id))
        .join(mod_id.to_string())
        .join(file.id.to_string());
    fs::create_dir_all(&download_dir).map_err(error_to_string)?;
    let archive_path = download_dir.join(format!("{}-{safe_name}", Uuid::new_v4()));
    let client = provider_client()?;
    let artifact_source = format!("modio:{game_id}/{mod_id}/{}", file.id);
    download_url_to_file_with_source_identity(
        &client,
        &file.download.binary_url,
        &archive_path,
        &artifact_source,
    )
    .map_err(|error| format!("mod.io download failed: {error}"))?;
    let downloaded_size = fs::metadata(&archive_path).map_err(error_to_string)?.len();
    if downloaded_size != file.filesize {
        remove_download_artifact(&archive_path);
        return Err(format!(
            "mod.io download size mismatch: expected {} bytes, received {}. The file was removed.",
            file.filesize, downloaded_size
        ));
    }
    Ok(archive_path)
}

fn send_get(
    client: &Client,
    url: &str,
    api_key: &str,
    query: &[(&str, String)],
) -> Result<Response, String> {
    validate_api_key_shape(api_key)?;
    let mut request = client
        .get(url)
        .header("Accept", "application/json")
        .header("X-Modio-Platform", "windows")
        .header("X-Modio-Portal", "steam")
        .query(&[("api_key", api_key.trim())]);
    if !query.is_empty() {
        request = request.query(query);
    }
    let response = request
        .send()
        .map_err(|_| "mod.io could not be reached. Try again shortly.".to_string())?;
    if response.status().is_success() {
        return Ok(response);
    }
    match response.status().as_u16() {
        401 | 403 => Err(
            "mod.io rejected the configured API key or this catalogue is not available."
                .to_string(),
        ),
        429 => Err("mod.io rate-limited this request. Try again shortly.".to_string()),
        status => Err(format!("mod.io returned HTTP {status}.")),
    }
}

fn parse_json<T: DeserializeOwned>(response: Response, label: &str) -> Result<T, String> {
    response
        .json::<T>()
        .map_err(|_| format!("mod.io returned invalid {label} data."))
}

pub(super) fn validate_api_key_shape(api_key: &str) -> Result<(), String> {
    let key = api_key.trim();
    if key.len() != 32
        || !key
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return Err("The mod.io API key must be the full 32-character value.".to_string());
    }
    Ok(())
}

fn modio_api_base() -> Result<&'static str, String> {
    match MODIO_API_BASE_CACHE.get_or_init(|| {
        validate_modio_api_base(option_env!("MOD_GARRISON_MODIO_API_BASE").unwrap_or_default())
    }) {
        Ok(base) => Ok(base.as_str()),
        Err(error) => Err(error.clone()),
    }
}

fn validate_modio_api_base(value: &str) -> Result<String, String> {
    let value = value.trim();
    let value = if value.is_empty() {
        DEFAULT_MODIO_API_BASE
    } else {
        value
    };
    let parsed = url::Url::parse(value)
        .map_err(|_| "The configured mod.io API base is not a valid URL.".to_string())?;
    let host = parsed
        .host_str()
        .map(|host| host.to_ascii_lowercase())
        .unwrap_or_default();
    let official_host = host == "api.mod.io" || host.ends_with(".modapi.io");
    let api_path = parsed.path().trim_end_matches('/');
    if parsed.scheme() != "https"
        || !official_host
        || parsed.port().is_some()
        || api_path != "/v1"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(
            "The configured mod.io API base must be an official HTTPS mod.io API path ending in /v1, without credentials, a port, query, or fragment."
                .to_string(),
        );
    }
    Ok(format!("https://{host}/v1"))
}

fn validate_numeric_id(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || !value.chars().all(|character| character.is_ascii_digit())
        || value == "0"
    {
        return Err(format!("Invalid {label} id."));
    }
    Ok(())
}

fn provider_game_name_matches(candidate: &str, returned: &str) -> bool {
    let candidate =
        editionless_provider_name_variant(candidate).unwrap_or_else(|| candidate.to_string());
    let returned =
        editionless_provider_name_variant(returned).unwrap_or_else(|| returned.to_string());
    compact_provider_slug(&candidate) == compact_provider_slug(&returned)
}

fn modio_game_matches_profile(game: &ModioGame, candidates: &[String], steam_app_id: &str) -> bool {
    if game.id == 0
        || game.status != 1
        || !candidates
            .iter()
            .any(|candidate| provider_game_name_matches(candidate, &game.name))
    {
        return false;
    }
    let linked_app_ids = modio_game_steam_app_ids(game);
    linked_app_ids.is_empty() || linked_app_ids.iter().any(|id| id == steam_app_id)
}

fn modio_game_steam_app_ids(game: &ModioGame) -> Vec<String> {
    let mut ids = game
        .other_urls
        .iter()
        .filter_map(|link| steam_app_id_from_url(&link.url))
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn steam_app_id_from_url(value: &str) -> Option<String> {
    let parsed = url::Url::parse(value.trim()).ok()?;
    let host = parsed
        .host_str()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host != "steampowered.com" && !host.ends_with(".steampowered.com") {
        return None;
    }
    let segments = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let candidate = segments
        .windows(2)
        .find(|pair| pair[0].eq_ignore_ascii_case("app"))
        .map(|pair| pair[1])
        .or_else(|| segments.last().copied())?;
    candidate
        .chars()
        .all(|character| character.is_ascii_digit())
        .then(|| candidate.to_string())
}

fn modio_game_allows_api_key_downloads(game: &ModioGame) -> bool {
    game.api_access_options
        .is_some_and(|options| options & (MODIO_AUTHORISED_DOWNLOADS | MODIO_PAID_DOWNLOADS) == 0)
}

fn ensure_api_key_downloads_supported(api_key: &str, game_id: &str) -> Result<(), String> {
    let client = provider_client()?;
    let game = fetch_game_by_id(&client, api_key, game_id)?;
    if game.id.to_string() != game_id || game.status != 1 {
        return Err("The linked mod.io game catalogue is no longer public.".to_string());
    }
    if !modio_game_allows_api_key_downloads(&game) {
        return Err(
            "This game requires each player to sign in to mod.io before downloading files. Mod Garrison currently supports API-key-only mod.io downloads."
                .to_string(),
        );
    }
    Ok(())
}

fn validate_modio_download_url(value: &str) -> Result<url::Url, String> {
    let parsed = validate_https_url(value)?;
    let host = parsed
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let trusted = host == "api.mod.io"
        || host == "mod.io"
        || host.ends_with(".modapi.io")
        || host.ends_with(".modcdn.io");
    if !trusted || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("mod.io returned an untrusted download URL.".to_string());
    }
    Ok(parsed)
}

fn modio_sort(sort: &str) -> &'static str {
    match sort {
        "newest" => "-date_updated",
        "oldest" => "date_updated",
        _ => "-downloads_total",
    }
}

fn is_supported_archive_name(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".7z") || lower.ends_with(".rar")
}

fn unix_timestamp(timestamp: i64) -> Option<String> {
    chrono::DateTime::<Utc>::from_timestamp(timestamp, 0).map(|value| value.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_test_profile() -> GameProfile {
        serde_json::from_value(serde_json::json!({
            "id": format!("modio-cache-{}", Uuid::new_v4()),
            "name": "Offline catalogue cache fixture",
            "gamePath": std::env::temp_dir().join(format!("modio-cache-game-{}", Uuid::new_v4())),
            "engine": "enshrouded", "loader": "none", "createdAt": "", "updatedAt": ""
        }))
        .unwrap()
    }

    #[test]
    fn mapping_cache_retains_successful_catalogue_absence() {
        let profile = cache_test_profile();
        assert_eq!(
            cached_modio_game_id(&profile, false, || Ok(None)).unwrap(),
            None
        );
        assert_eq!(
            cached_modio_game_id(&profile, false, || panic!(
                "a successful absence should be cached"
            ))
            .unwrap(),
            None
        );
        clear_cached_provider_mapping("modio", &profile);
    }

    #[test]
    fn mapping_errors_stay_visible_and_recover_without_waiting_for_negative_cache_expiry() {
        let profile = cache_test_profile();
        for error in [
            "mod.io authentication failed",
            "mod.io network unavailable",
            "mod.io returned multiple exact catalogue matches",
        ] {
            assert_eq!(
                cached_modio_game_id(&profile, false, || Err(error.to_string())).unwrap_err(),
                error
            );
        }
        assert_eq!(
            cached_modio_game_id(&profile, false, || Ok(Some("123".to_string()))).unwrap(),
            Some("123".to_string())
        );
        assert_eq!(
            cached_modio_game_id(&profile, false, || panic!(
                "the recovered verified ID should be cached"
            ))
            .unwrap(),
            Some("123".to_string())
        );
        assert_eq!(
            cached_modio_game_id(&profile, true, || Ok(Some("456".to_string()))).unwrap(),
            Some("456".to_string())
        );
        clear_cached_provider_mapping("modio", &profile);
    }

    #[test]
    fn failed_forced_mapping_refresh_cannot_leave_an_old_negative_result_masking_retries() {
        let profile = cache_test_profile();
        assert_eq!(
            cached_modio_game_id(&profile, false, || Ok(None)).unwrap(),
            None
        );
        assert_eq!(
            cached_modio_game_id(&profile, true, || Err("mod.io refresh failed".to_string()))
                .unwrap_err(),
            "mod.io refresh failed"
        );
        assert_eq!(
            cached_modio_game_id(&profile, false, || Ok(Some("789".to_string()))).unwrap(),
            Some("789".to_string())
        );
        clear_cached_provider_mapping("modio", &profile);
    }

    fn clean_modio_file(id: u64, mod_id: u64, timestamp: i64) -> ModioFile {
        ModioFile {
            id,
            mod_id,
            date_added: timestamp,
            date_updated: timestamp,
            virus_status: 1,
            virus_positive: 0,
            filesize: 10,
            filename: format!("file-{id}.zip"),
            download: ModioDownload {
                binary_url: format!("https://assets.modcdn.io/file-{id}.zip"),
                date_expires: 0,
            },
            ..ModioFile::default()
        }
    }

    #[test]
    fn key_shape_validation_never_accepts_partial_or_query_text() {
        assert!(validate_api_key_shape("0123456789abcdef0123456789abcdef").is_ok());
        assert!(validate_api_key_shape("short").is_err());
        assert!(validate_api_key_shape("0123456789abcdef0123456789abcde?").is_err());
    }

    #[test]
    fn api_base_validation_uses_the_official_default_and_rejects_unsafe_values() {
        assert_eq!(validate_modio_api_base("").unwrap(), DEFAULT_MODIO_API_BASE);
        assert_eq!(
            validate_modio_api_base("https://u-123.modapi.io/v1/").unwrap(),
            "https://u-123.modapi.io/v1"
        );
        assert!(validate_modio_api_base("https://example.invalid/modio/v1/").is_err());
        assert!(validate_modio_api_base("https://modapi.io/v1").is_err());
        assert!(validate_modio_api_base("https://api.mod.io/other").is_err());
        assert!(validate_modio_api_base("https://api.mod.io:444/v1").is_err());
        assert!(validate_modio_api_base("http://api.mod.io/v1").is_err());
        assert!(validate_modio_api_base("https://user:pass@api.mod.io/v1").is_err());
        assert!(validate_modio_api_base("https://api.mod.io/v1?key=value").is_err());
    }

    #[test]
    fn modio_ids_are_provider_and_game_qualified() {
        assert_eq!(
            parse_online_mod_id("modio:123/456").unwrap(),
            ("123".to_string(), 456)
        );
        assert!(parse_online_mod_id("modio:0/456").is_err());
        assert!(parse_online_mod_id("modio:123/nope").is_err());
        assert!(parse_online_mod_id("nexus:123/456").is_err());
    }

    #[test]
    fn only_clean_scanned_supported_archives_are_installable() {
        let file = ModioFile {
            id: 1,
            mod_id: 2,
            virus_status: 1,
            virus_positive: 0,
            filesize: 10,
            filename: "safe.zip".to_string(),
            download: ModioDownload {
                binary_url: "https://assets.modcdn.io/safe.zip".to_string(),
                date_expires: Utc::now().timestamp() + 60,
            },
            ..ModioFile::default()
        };
        assert!(file_is_installable(&file));
        assert!(!file_is_installable(&ModioFile {
            virus_positive: 2,
            ..file.clone()
        }));
        assert!(!file_is_installable(&ModioFile {
            filename: "installer.exe".to_string(),
            ..file
        }));
    }

    #[test]
    fn declared_primary_file_wins_over_a_newer_auxiliary_archive() {
        let older_primary = clean_modio_file(10, 77, 100);
        let newer_auxiliary = clean_modio_file(11, 77, 200);
        let files = vec![newer_auxiliary.clone()];

        let listed_files = include_primary_file(files.clone(), Some(&older_primary), 77);
        assert!(listed_files.iter().any(|file| file.id == older_primary.id));

        assert_eq!(
            choose_file(&files, 77, None, Some(&older_primary))
                .unwrap()
                .id,
            older_primary.id
        );
        assert_eq!(
            choose_file(&files, 77, Some("11"), Some(&older_primary))
                .unwrap()
                .id,
            newer_auxiliary.id
        );

        let rejected_primary = ModioFile {
            virus_positive: 1,
            ..older_primary
        };
        assert_eq!(
            choose_file(&files, 77, None, Some(&rejected_primary))
                .unwrap()
                .id,
            newer_auxiliary.id
        );
    }

    #[test]
    fn reported_mod_dependencies_fail_closed_before_download() {
        let item = ModioMod {
            name: "Dependent overhaul".to_string(),
            dependencies: true,
            ..ModioMod::default()
        };
        let error = ensure_dependencies_supported(&item).unwrap_err();
        assert!(error.contains("required mod dependencies"));
        assert!(error.contains("No files were downloaded or installed"));

        assert!(ensure_dependencies_supported(&ModioMod {
            dependencies: false,
            ..item
        })
        .is_ok());
    }

    #[test]
    fn steam_link_must_match_the_verified_profile_when_present() {
        let mut game = ModioGame {
            id: 77,
            status: 1,
            name: "Example Game".to_string(),
            other_urls: vec![ModioLink {
                url: "https://store.steampowered.com/app/12345/Example_Game/".to_string(),
            }],
            ..ModioGame::default()
        };
        let names = vec!["Example Game".to_string()];
        assert!(modio_game_matches_profile(&game, &names, "12345"));
        assert!(!modio_game_matches_profile(&game, &names, "98765"));

        game.other_urls.clear();
        assert!(modio_game_matches_profile(&game, &names, "98765"));
        game.status = 0;
        assert!(!modio_game_matches_profile(&game, &names, "98765"));
    }

    #[test]
    fn download_urls_are_limited_to_modio_hosts() {
        assert!(validate_modio_download_url(
            "https://g-123.modapi.io/v1/games/1/mods/2/files/3/download"
        )
        .is_ok());
        assert!(validate_modio_download_url("https://assets.modcdn.io/file.zip").is_ok());
        assert!(validate_modio_download_url("https://modcdn.io.example.com/file.zip").is_err());
        assert!(validate_modio_download_url("https://example.com/file.zip").is_err());
    }

    #[test]
    fn api_key_only_installs_reject_authenticated_download_games() {
        assert!(modio_game_allows_api_key_downloads(&ModioGame {
            api_access_options: Some(3),
            ..ModioGame::default()
        }));
        assert!(!modio_game_allows_api_key_downloads(&ModioGame {
            api_access_options: Some(MODIO_AUTHORISED_DOWNLOADS),
            ..ModioGame::default()
        }));
        assert!(!modio_game_allows_api_key_downloads(&ModioGame {
            api_access_options: Some(MODIO_PAID_DOWNLOADS),
            ..ModioGame::default()
        }));
        assert!(!modio_game_allows_api_key_downloads(&ModioGame::default()));
    }

    #[test]
    fn metadata_learning_excludes_runtime_and_non_game_catalogue_rows() {
        let mut item = ModioMod {
            id: 77,
            game_id: 12,
            status: 1,
            visible: 1,
            name: "Gameplay Overhaul".to_string(),
            revenue_type: Some(0),
            modfile: Some(clean_modio_file(10, 77, 100)),
            ..ModioMod::default()
        };
        assert!(mod_is_discoverable_metadata(&item, "12"));

        item.name = "UE4SS for Example Game".to_string();
        assert!(!mod_is_discoverable_metadata(&item, "12"));

        item.name = "Gameplay Overhaul".to_string();
        assert!(!mod_is_discoverable_metadata(&item, "13"));
    }
}
