use super::*;

const API_BASE: &str = "https://api.gamebanana.com";
const SITE_BASE: &str = "https://gamebanana.com";
const CATALOGUE_INDEX_PATH: &str = "/apiv11/Mod/Index";
const CATALOGUE_PAGE_SIZE: usize = 20;
const DETAIL_BATCH_SIZE: usize = 8;
const DETAIL_FIELDS: &str = "name,description,downloads,likes,date,mdate,Owner().name,Game().name,RootCategory().name,Nsfw().bIsNsfw(),Trash().bIsTrashed(),Withhold().bIsWithheld(),is_obsolete,Files().aFiles(),Preview().sSubFeedImageUrl(),Url().sProfileUrl(),text,install_instructions";

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogueIndexRecord {
    id: u64,
    name: String,
    game_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogueIndexPage {
    total: usize,
    records: Vec<CatalogueIndexRecord>,
    raw_record_count: usize,
    complete: bool,
}

#[derive(Debug, Clone)]
struct GameBananaMod {
    id: u64,
    name: String,
    description: String,
    text: String,
    install_instructions: String,
    owner: String,
    game_name: String,
    category: String,
    downloads: u64,
    likes: i64,
    date_added: Option<i64>,
    date_modified: Option<i64>,
    icon_url: Option<String>,
    profile_url: String,
    nsfw: bool,
    trashed: bool,
    withheld: bool,
    obsolete: bool,
    files: Vec<GameBananaFile>,
}

#[derive(Debug, Clone)]
struct GameBananaFile {
    id: u64,
    name: String,
    description: Option<String>,
    size: Option<u64>,
    date_added: Option<i64>,
    downloads: u64,
    download_url: String,
    analysis_state: Option<String>,
    analysis_result: Option<String>,
    antivirus_state: Option<String>,
    antivirus_result: Option<String>,
    archived: bool,
    has_contents: Option<bool>,
}

pub(super) fn resolve_mapping(
    store_root: &Path,
    profile: &GameProfile,
    force_refresh: bool,
) -> ProviderGameResolution {
    let saved = match persisted_provider_game_mapping(store_root, "gamebanana", profile) {
        Ok(mapping) => mapping,
        Err(error) => {
            return ProviderGameResolution {
                mapping: None,
                warning: Some(format!(
                    "GameBanana catalogue matching is available, but Mod Garrison could not read its saved provider mapping: {error}"
                )),
            };
        }
    };

    if !force_refresh {
        if let Some(mapping) = saved {
            return ProviderGameResolution {
                mapping: Some(mapping),
                warning: None,
            };
        }
    }

    match cached_exact_game_mapping(profile, force_refresh) {
        Ok(Some(mapping)) => {
            let warning = remember_provider_game_mapping(store_root, mapping.clone())
                .err()
                .map(|error| {
                    format!(
                        "GameBanana was linked automatically, but Mod Garrison could not remember the verified mapping: {error}"
                    )
                });
            ProviderGameResolution {
                mapping: Some(mapping),
                warning,
            }
        }
        Ok(None) => {
            let warning = forget_provider_game_mapping(store_root, "gamebanana", profile)
                .err()
                .map(|error| {
                    format!(
                        "GameBanana has no exact game-name match, and an obsolete saved mapping could not be removed: {error}"
                    )
                });
            ProviderGameResolution {
                mapping: None,
                warning,
            }
        }
        Err(error) => {
            if let Some(mapping) = saved {
                ProviderGameResolution {
                    mapping: Some(mapping),
                    warning: Some(format!(
                        "GameBanana could not refresh its catalogue mapping ({error}). Mod Garrison kept the last verified mapping."
                    )),
                }
            } else {
                ProviderGameResolution {
                    mapping: None,
                    warning: Some(format!(
                        "GameBanana could not be reached while checking for an exact {} catalogue match: {error}",
                        profile_game_label(profile)
                    )),
                }
            }
        }
    }
}

pub(super) fn discover_mods(
    store_root: &Path,
    profile: &GameProfile,
    mapping: &ProviderGameMapping,
    fetch: NexusDiscoveryFetch<'_>,
) -> Result<DiscoveryProviderBatch, String> {
    validate_mapping_for_profile(profile, mapping)?;
    let game_id = parse_positive_id(&mapping.provider_game_id, "GameBanana game")?;
    ensure_discovery_is_current(&profile.id, fetch.request_id)?;
    let raw_budget = rounded_catalogue_row_budget(fetch.max_new_results);
    if raw_budget == 0 {
        return Ok(DiscoveryProviderBatch {
            start_offset: fetch.start_offset,
            records: Vec::new(),
            raw_total: fetch.start_offset,
            next_offset: fetch.start_offset,
            filtered_out: 0,
            complete: false,
        });
    }
    if !fetch.start_offset.is_multiple_of(CATALOGUE_PAGE_SIZE) {
        return Err(
            "GameBanana discovery received an invalid catalogue cursor. Refresh Discovery and try again."
                .to_string(),
        );
    }

    let installed_keys = installed_package_identity_keys(store_root, &profile.id)?;
    let client = provider_client()?;
    let normalized_query = fetch.query.trim().to_lowercase();
    let mut records = Vec::new();
    let mut raw_total = usize::MAX;
    let mut next_offset = fetch.start_offset;
    let mut raw_rows_scanned = 0usize;
    let mut filtered_out = 0usize;
    let mut complete = false;
    let mut seen_ids = HashSet::new();

    while raw_rows_scanned < raw_budget && next_offset < raw_total && !complete {
        let page_number = next_offset / CATALOGUE_PAGE_SIZE + 1;
        let page = fetch_catalogue_index_page(
            &client,
            profile,
            game_id,
            page_number,
            fetch.sort,
            fetch.request_id,
        )?;
        raw_total = page.total;

        if next_offset >= raw_total || page.raw_record_count == 0 {
            complete = true;
            break;
        }
        if page.raw_record_count > CATALOGUE_PAGE_SIZE {
            return Err(format!(
                "GameBanana returned more than {CATALOGUE_PAGE_SIZE} catalogue rows in one page."
            ));
        }

        let rows_remaining = raw_budget.saturating_sub(raw_rows_scanned);
        if page.raw_record_count > rows_remaining {
            return Err(
                "GameBanana returned a catalogue page beyond the bounded discovery budget."
                    .to_string(),
            );
        }

        let candidate_ids = page
            .records
            .into_iter()
            .filter(|record| record.game_id.is_none_or(|id| id == game_id))
            .filter(|record| catalogue_name_matches_query(&record.name, &normalized_query))
            .filter_map(|record| seen_ids.insert(record.id).then_some(record.id))
            .collect::<Vec<_>>();
        let records_before_page = records.len();
        for ids in candidate_ids.chunks(DETAIL_BATCH_SIZE) {
            ensure_discovery_is_current(&profile.id, fetch.request_id)?;
            let items = fetch_mod_batch(&client, ids)?;
            ensure_discovery_is_current(&profile.id, fetch.request_id)?;
            for item in items {
                let installed = installed_keys.contains(&format!(
                    "package:{}",
                    package_id(game_id, item.id).to_ascii_lowercase()
                ));
                if let Some(record) = item_to_online_mod(profile, mapping, &item, installed) {
                    if normalized_query.is_empty()
                        || online_mod_matches_query(&record, &normalized_query)
                    {
                        records.push(record);
                    }
                }
            }
        }

        filtered_out = filtered_out.saturating_add(
            page.raw_record_count
                .saturating_sub(records.len().saturating_sub(records_before_page)),
        );
        raw_rows_scanned = raw_rows_scanned.saturating_add(page.raw_record_count);
        next_offset = next_offset.saturating_add(page.raw_record_count);
        complete = page.complete
            || next_offset >= raw_total
            || page.raw_record_count < CATALOGUE_PAGE_SIZE;
    }

    ensure_discovery_is_current(&profile.id, fetch.request_id)?;
    Ok(DiscoveryProviderBatch {
        start_offset: fetch.start_offset,
        records,
        raw_total: raw_total.min(usize::MAX - 1),
        next_offset,
        filtered_out,
        complete,
    })
}

pub(super) fn list_files(
    store_root: &Path,
    profile: &GameProfile,
    mod_id: &str,
    supplied_provider_game_id: Option<&str>,
) -> Result<Vec<OnlineModFileOption>, String> {
    let (encoded_game_id, item_id) = parse_online_mod_id(mod_id)?;
    let mapping = verified_mapping(
        store_root,
        profile,
        encoded_game_id,
        supplied_provider_game_id,
    )?;
    let item = fetch_single_mod(item_id)?;
    verify_item_game(&mapping, &item)?;
    if !item_is_discoverable(&item) {
        return Err(
            "This GameBanana mod is unavailable, unsafe, obsolete, or no longer has an installable archive."
                .to_string(),
        );
    }

    Ok(file_options(&item))
}

pub(super) fn preflight(
    store_root: &Path,
    profile: &GameProfile,
    mod_id: &str,
) -> Result<(Vec<DependencySpec>, Vec<InstallTargetOption>), String> {
    let (encoded_game_id, item_id) = parse_online_mod_id(mod_id)?;
    let mapping = verified_mapping(store_root, profile, encoded_game_id, None)?;
    let item = fetch_single_mod(item_id)?;
    verify_item_game(&mapping, &item)?;
    if !item_is_discoverable(&item) {
        return Err(
            "This GameBanana mod is unavailable, unsafe, obsolete, or no longer has an installable archive."
                .to_string(),
        );
    }
    let document = provider_document(encoded_game_id, &item);
    let dependencies = description_runtime_dependencies(profile, &document.text, None);
    let install_targets = provider_install_target_options(store_root, profile, &document);
    Ok((dependencies, install_targets))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn install(
    store_root: &Path,
    profile: &GameProfile,
    mod_id: &str,
    _version: Option<String>,
    supplied_provider_game_id: Option<&str>,
    selected_file_id: Option<&str>,
    install_target_id: Option<&str>,
) -> Result<InstallResult, String> {
    let (encoded_game_id, item_id) = parse_online_mod_id(mod_id)?;
    let mapping = verified_mapping(
        store_root,
        profile,
        encoded_game_id,
        supplied_provider_game_id,
    )?;
    let expected_game_id = parse_positive_id(&mapping.provider_game_id, "GameBanana game")?;
    let item = fetch_single_mod(item_id)?;
    verify_item_game(&mapping, &item)?;
    if !item_is_discoverable(&item) {
        return Err(
            "This GameBanana mod is unavailable, unsafe, obsolete, or no longer has an installable archive."
                .to_string(),
        );
    }
    let file = choose_file(&item, selected_file_id)?;
    let provider_document = provider_document(expected_game_id, &item);
    let dependencies = description_runtime_dependencies(profile, &provider_document.text, None);
    let archive_path = download_file(store_root, expected_game_id, item_id, &file)?;
    let resolved_version = Some(provider_file_version(file.id));
    let package_id = package_id(expected_game_id, item_id);
    let source_identity = provider_source_identity(
        "gamebanana",
        package_id.clone(),
        resolved_version.clone(),
        Some(expected_game_id.to_string()),
        "GameBanana exact-name game catalogue and Core/Item/Data file record",
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
    append_unique_dependencies(&mut plan.dependencies, dependencies);
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
            item.name
        ));
    }

    let mut visited_dependencies = HashSet::new();
    install_archive_impl_with_metadata(
        store_root,
        profile,
        &install_source_path,
        &plan,
        InstallOptions {
            metadata: InstallMetadata {
                archive_name: Some(file.name.clone()),
                package_id: Some(package_id.clone()),
                dependency_string: Some(format!("{package_id}#{}", file.id)),
                package_provider: Some("gamebanana".to_string()),
                package_version: resolved_version,
                provider_file_id: Some(file.id.to_string()),
                provider_variant: Some(file.name.clone()),
                display_name: Some(item.name),
                package_identity: Some(package_identity),
                runtime_id: None,
                icon_url: item.icon_url,
                install_targets,
                selected_install_target_id,
            },
            resolve_dependencies: true,
            visited_dependencies: &mut visited_dependencies,
            dependency_depth: 0,
        },
    )
}

fn cached_exact_game_mapping(
    profile: &GameProfile,
    force_refresh: bool,
) -> Result<Option<ProviderGameMapping>, String> {
    let mut fetched_mapping = None;
    let mut lookup_error = None;
    let provider_game_id = cached_provider_mapping("gamebanana", profile, force_refresh, || {
        match fetch_exact_game_mapping(profile) {
            Ok(Some(mapping)) => {
                let provider_game_id = mapping.provider_game_id.clone();
                fetched_mapping = Some(mapping);
                Some(provider_game_id)
            }
            Ok(None) => None,
            Err(error) => {
                lookup_error = Some(error);
                None
            }
        }
    });

    let Some(provider_game_id) = provider_game_id else {
        return lookup_error.map_or(Ok(None), Err);
    };
    if let Some(mapping) = fetched_mapping {
        return Ok(Some(mapping));
    }
    exact_game_mapping_from_cached_id(profile, &provider_game_id).map(Some)
}

fn exact_game_mapping_from_cached_id(
    profile: &GameProfile,
    provider_game_id: &str,
) -> Result<ProviderGameMapping, String> {
    let canonical_game = verified_steam_game_for_profile(profile)?;
    let canonical_name = canonical_game.name.trim().to_string();
    if canonical_name.len() < 2 {
        return Err(
            "Automatic GameBanana catalogue matching requires a verified game name.".to_string(),
        );
    }
    let steam_app_id = profile
        .steam_app_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "Automatic GameBanana catalogue matching requires a verified Steam App ID.".to_string()
        })?
        .to_string();
    let game_id = parse_positive_id(provider_game_id, "GameBanana game")?;
    Ok(ProviderGameMapping {
        provider: "gamebanana".to_string(),
        steam_app_id,
        canonical_game_name: canonical_name,
        provider_game_id: game_id.to_string(),
        provider_catalogue_id: None,
        resolution: "catalog-exact-name".to_string(),
        verified_at: now_string(),
    })
}

fn fetch_exact_game_mapping(profile: &GameProfile) -> Result<Option<ProviderGameMapping>, String> {
    let canonical_game = verified_steam_game_for_profile(profile)?;
    let canonical_name = canonical_game.name.trim().to_string();
    if canonical_name.len() < 2 {
        return Ok(None);
    }
    let steam_app_id = profile
        .steam_app_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "Automatic GameBanana catalogue matching requires a verified Steam App ID.".to_string()
        })?
        .to_string();
    let client = provider_client()?;
    let response = client
        .get(format!("{API_BASE}/Core/List/Like"))
        .query(&[
            ("itemtype", "Game"),
            ("field", "name"),
            ("match", canonical_name.as_str()),
            ("format", "json_min"),
        ])
        .send()
        .map_err(|error| format!("GameBanana game lookup failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("GameBanana game lookup failed: {error}"))?;
    let value = response
        .json::<serde_json::Value>()
        .map_err(|error| format!("GameBanana returned invalid game lookup data: {error}"))?;
    let matches = value
        .as_array()
        .ok_or_else(|| gamebanana_api_error(&value, "game lookup"))?
        .iter()
        .filter_map(|entry| {
            let id = value_u64(entry.get("id")?)?;
            let name = entry.get("name")?.as_str()?.trim().to_string();
            exact_game_name_match(&name, &canonical_name).then_some((id, name))
        })
        .collect::<Vec<_>>();
    let mut unique = matches;
    unique.sort_by_key(|(id, _)| *id);
    unique.dedup_by_key(|(id, _)| *id);
    match unique.as_slice() {
        [] => Ok(None),
        [(game_id, provider_name)] => Ok(Some(ProviderGameMapping {
            provider: "gamebanana".to_string(),
            steam_app_id,
            canonical_game_name: provider_name.clone(),
            provider_game_id: game_id.to_string(),
            provider_catalogue_id: None,
            resolution: "catalog-exact-name".to_string(),
            verified_at: now_string(),
        })),
        _ => Err(format!(
            "GameBanana returned multiple exact matches for '{canonical_name}', so Mod Garrison refused to guess."
        )),
    }
}

fn rounded_catalogue_row_budget(max_new_results: usize) -> usize {
    if max_new_results == 0 {
        return 0;
    }
    max_new_results
        .saturating_add(CATALOGUE_PAGE_SIZE - 1)
        .checked_div(CATALOGUE_PAGE_SIZE)
        .unwrap_or(usize::MAX / CATALOGUE_PAGE_SIZE)
        .saturating_mul(CATALOGUE_PAGE_SIZE)
}

fn gamebanana_sort_alias(sort: &str) -> &'static str {
    match sort {
        "newest" => "Generic_Newest",
        "oldest" => "Generic_Oldest",
        _ => "Generic_MostDownloaded",
    }
}

fn catalogue_name_matches_query(name: &str, normalized_query: &str) -> bool {
    normalized_query.is_empty() || name.to_lowercase().contains(normalized_query)
}

fn fetch_catalogue_index_page(
    client: &Client,
    profile: &GameProfile,
    game_id: u64,
    page: usize,
    sort: &str,
    request_id: u64,
) -> Result<CatalogueIndexPage, String> {
    ensure_discovery_is_current(&profile.id, request_id)?;
    let response = client
        .get(format!("{SITE_BASE}{CATALOGUE_INDEX_PATH}"))
        .query(&[
            ("_aFilters[Generic_Game]", game_id.to_string()),
            ("_sSort", gamebanana_sort_alias(sort).to_string()),
            ("_nPage", page.max(1).to_string()),
            ("_nPerpage", CATALOGUE_PAGE_SIZE.to_string()),
        ])
        .send()
        .map_err(|error| format!("GameBanana catalogue request failed: {error}"))?;
    ensure_discovery_is_current(&profile.id, request_id)?;
    let response = response
        .error_for_status()
        .map_err(|error| format!("GameBanana catalogue request failed: {error}"))?;
    let value = response
        .json::<serde_json::Value>()
        .map_err(|error| format!("GameBanana returned invalid catalogue data: {error}"))?;
    ensure_discovery_is_current(&profile.id, request_id)?;
    parse_catalogue_index_page(&value)
}

fn parse_catalogue_index_page(value: &serde_json::Value) -> Result<CatalogueIndexPage, String> {
    let object = value
        .as_object()
        .ok_or_else(|| gamebanana_api_error(value, "catalogue"))?;
    let metadata = object
        .get("_aMetadata")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| gamebanana_api_error(value, "catalogue metadata"))?;
    let total = metadata
        .get("_nRecordCount")
        .and_then(value_u64)
        .and_then(|total| usize::try_from(total).ok())
        .ok_or_else(|| {
            "GameBanana catalogue metadata did not include a valid total record count.".to_string()
        })?;
    let raw_records = object
        .get("_aRecords")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| gamebanana_api_error(value, "catalogue records"))?;
    let raw_record_count = raw_records.len();
    let records = raw_records
        .iter()
        .filter_map(|record| {
            let record = record.as_object()?;
            let id = record
                .get("_idRow")
                .and_then(value_u64)
                .filter(|id| *id > 0)?;
            let name = value_string(record.get("_sName")).unwrap_or_default();
            let game_id = record
                .get("_aGame")
                .and_then(serde_json::Value::as_object)
                .and_then(|game| game.get("_idRow"))
                .and_then(value_u64);
            Some(CatalogueIndexRecord { id, name, game_id })
        })
        .collect::<Vec<_>>();
    let complete = metadata
        .get("_bIsComplete")
        .and_then(value_bool)
        .unwrap_or(raw_record_count < CATALOGUE_PAGE_SIZE);
    Ok(CatalogueIndexPage {
        total,
        records,
        raw_record_count,
        complete,
    })
}

fn fetch_mod_batch(client: &Client, ids: &[u64]) -> Result<Vec<GameBananaMod>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = Vec::<(String, String)>::new();
    for id in ids {
        query.push(("itemtype[]".to_string(), "Mod".to_string()));
        query.push(("itemid[]".to_string(), id.to_string()));
        query.push(("fields[]".to_string(), DETAIL_FIELDS.to_string()));
    }
    query.push(("return_keys".to_string(), "1".to_string()));
    query.push(("format".to_string(), "json_min".to_string()));
    let response = client
        .get(format!("{API_BASE}/Core/Item/Data"))
        .query(&query)
        .send()
        .map_err(|error| format!("GameBanana mod details request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("GameBanana mod details request failed: {error}"))?;
    let value = response
        .json::<serde_json::Value>()
        .map_err(|error| format!("GameBanana returned invalid mod details: {error}"))?;
    let rows = value
        .as_array()
        .ok_or_else(|| gamebanana_api_error(&value, "mod details"))?;
    Ok(ids
        .iter()
        .zip(rows)
        .filter_map(|(id, row)| parse_mod(*id, row).ok())
        .collect())
}

fn fetch_single_mod(item_id: u64) -> Result<GameBananaMod, String> {
    let mut mods = fetch_mod_batch(&provider_client()?, &[item_id])?;
    mods.pop()
        .ok_or_else(|| format!("GameBanana no longer returns usable details for mod {item_id}."))
}

fn parse_mod(id: u64, value: &serde_json::Value) -> Result<GameBananaMod, String> {
    let object = value
        .as_object()
        .ok_or_else(|| gamebanana_api_error(value, &format!("details for mod {id}")))?;
    let name = value_string(object.get("name"))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("GameBanana mod {id} has no name."))?;
    let profile_url = value_string(object.get("Url().sProfileUrl()"))
        .filter(|value| official_gamebanana_url(value))
        .unwrap_or_else(|| format!("{SITE_BASE}/mods/{id}"));
    Ok(GameBananaMod {
        id,
        name,
        description: value_string(object.get("description")).unwrap_or_default(),
        text: value_string(object.get("text")).unwrap_or_default(),
        install_instructions: value_string(object.get("install_instructions")).unwrap_or_default(),
        owner: value_string(object.get("Owner().name")).unwrap_or_else(|| "GameBanana".to_string()),
        game_name: value_string(object.get("Game().name")).unwrap_or_default(),
        category: value_string(object.get("RootCategory().name")).unwrap_or_default(),
        downloads: object
            .get("downloads")
            .and_then(value_u64)
            .unwrap_or_default(),
        likes: object.get("likes").and_then(value_i64).unwrap_or_default(),
        date_added: object.get("date").and_then(value_i64),
        date_modified: object.get("mdate").and_then(value_i64),
        icon_url: value_string(object.get("Preview().sSubFeedImageUrl()"))
            .filter(|value| official_gamebanana_url(value)),
        profile_url,
        nsfw: object
            .get("Nsfw().bIsNsfw()")
            .and_then(value_bool)
            .unwrap_or(false),
        trashed: object
            .get("Trash().bIsTrashed()")
            .and_then(value_bool)
            .unwrap_or(false),
        withheld: object
            .get("Withhold().bIsWithheld()")
            .and_then(value_bool)
            .unwrap_or(false),
        obsolete: object
            .get("is_obsolete")
            .and_then(value_bool)
            .unwrap_or(false),
        files: parse_files(object.get("Files().aFiles()")),
    })
}

fn parse_files(value: Option<&serde_json::Value>) -> Vec<GameBananaFile> {
    let mut values = Vec::new();
    match value {
        Some(serde_json::Value::Object(object)) => {
            for (key, value) in object {
                values.push((key.as_str(), value));
            }
        }
        Some(serde_json::Value::Array(array)) => {
            for value in array {
                values.push(("", value));
            }
        }
        _ => {}
    }
    values
        .into_iter()
        .filter_map(|(key, value)| parse_file(key, value))
        .collect()
}

fn parse_file(key: &str, value: &serde_json::Value) -> Option<GameBananaFile> {
    let object = value.as_object()?;
    let id = object
        .get("_idRow")
        .and_then(value_u64)
        .or_else(|| key.parse::<u64>().ok())?;
    let name = value_string(object.get("_sFile"))?;
    let download_url = value_string(object.get("_sDownloadUrl"))?;
    Some(GameBananaFile {
        id,
        name,
        description: value_string(object.get("_sDescription"))
            .and_then(|value| non_empty_string(value.trim())),
        size: object.get("_nFilesize").and_then(value_u64),
        date_added: object.get("_tsDateAdded").and_then(value_i64),
        downloads: object
            .get("_nDownloadCount")
            .and_then(value_u64)
            .unwrap_or_default(),
        download_url,
        analysis_state: value_string(object.get("_sAnalysisState")),
        analysis_result: value_string(object.get("_sAnalysisResult")),
        antivirus_state: value_string(object.get("_sAvState")),
        antivirus_result: value_string(object.get("_sAvResult")),
        archived: object
            .get("_bIsArchived")
            .and_then(value_bool)
            .unwrap_or(false),
        has_contents: object.get("_bHasContents").and_then(value_bool),
    })
}

fn item_to_online_mod(
    profile: &GameProfile,
    mapping: &ProviderGameMapping,
    item: &GameBananaMod,
    installed: bool,
) -> Option<OnlineModRecord> {
    if !exact_game_name_match(&item.game_name, &mapping.canonical_game_name)
        || !item_is_discoverable(item)
    {
        return None;
    }
    let game_id = parse_positive_id(&mapping.provider_game_id, "GameBanana game").ok()?;
    let files = supported_files(item);
    let primary_file = files.first();
    let description = clean_provider_text(&item.description);
    let provider_document = provider_document(game_id, item);
    let dependency_count =
        description_runtime_dependencies(profile, &provider_document.text, None).len();
    let record = OnlineModRecord {
        id: package_id(game_id, item.id),
        provider: "gamebanana".to_string(),
        provider_label: "GameBanana".to_string(),
        game_id: profile.game_id.clone(),
        provider_game_id: Some(game_id.to_string()),
        name: item.name.clone(),
        owner: item.owner.clone(),
        version: primary_file
            .map(|file| provider_file_version(file.id))
            .unwrap_or_else(|| "latest".to_string()),
        description,
        categories: non_empty_string(item.category.trim()).into_iter().collect(),
        downloads: item.downloads,
        rating_score: item.likes,
        dependency_count,
        file_size: primary_file.and_then(|file| file.size),
        icon_url: item.icon_url.clone(),
        package_url: Some(item.profile_url.clone()),
        website_url: Some(item.profile_url.clone()),
        installed,
        created_at: timestamp_string(item.date_added),
        updated_at: timestamp_string(item.date_modified),
        install_supported: true,
        install_note: Some("Choose a GameBanana archive to install directly.".to_string()),
    };
    (!is_external_mod_manager_listing(&record)).then_some(record)
}

fn item_is_discoverable(item: &GameBananaMod) -> bool {
    !item.nsfw
        && !item.trashed
        && !item.withheld
        && !item.obsolete
        && !item.name.trim().is_empty()
        && !item_is_runtime_or_tool(item)
        && !supported_files(item).is_empty()
}

fn item_is_runtime_or_tool(item: &GameBananaMod) -> bool {
    let category = item.category.to_ascii_lowercase();
    category == "tools"
        || category == "modding tools"
        || is_known_runtime_provider_package("gamebanana", None, &item.name)
        || is_known_runtime_provider_package("nexus", None, &item.name)
        || is_known_runtime_provider_package("thunderstore", Some(&item.owner), &item.name)
}

fn supported_files(item: &GameBananaMod) -> Vec<GameBananaFile> {
    let mut files = item
        .files
        .iter()
        .filter(|file| file_is_supported(file))
        .cloned()
        .collect::<Vec<_>>();
    files.sort_by(|first, second| {
        second
            .date_added
            .unwrap_or_default()
            .cmp(&first.date_added.unwrap_or_default())
            .then_with(|| second.downloads.cmp(&first.downloads))
            .then_with(|| second.id.cmp(&first.id))
    });
    files
}

fn file_is_supported(file: &GameBananaFile) -> bool {
    let lower = file.name.to_ascii_lowercase();
    let archive = lower.ends_with(".zip") || lower.ends_with(".7z") || lower.ends_with(".rar");
    let analysis_ok = file
        .analysis_result
        .as_deref()
        .is_none_or(|value| value.eq_ignore_ascii_case("ok"));
    let antivirus_ok = file
        .antivirus_result
        .as_deref()
        .is_none_or(|value| value.eq_ignore_ascii_case("clean"));
    let analysis_finished = file
        .analysis_state
        .as_deref()
        .is_none_or(|value| value.eq_ignore_ascii_case("done"));
    let antivirus_finished = file
        .antivirus_state
        .as_deref()
        .is_none_or(|value| value.eq_ignore_ascii_case("done"));
    archive
        && !file.archived
        && file.has_contents.unwrap_or(true)
        && analysis_ok
        && antivirus_ok
        && analysis_finished
        && antivirus_finished
        && official_download_url(&file.download_url, file.id)
}

fn file_options(item: &GameBananaMod) -> Vec<OnlineModFileOption> {
    supported_files(item)
        .into_iter()
        .enumerate()
        .map(|(index, file)| OnlineModFileOption {
            id: file.id.to_string(),
            name: file.name.clone(),
            version: Some(provider_file_version(file.id)),
            category: non_empty_string(item.category.trim()),
            description: file
                .description
                .as_deref()
                .map(clean_provider_text)
                .and_then(|value| non_empty_string(value.trim())),
            file_name: Some(file.name),
            file_size: file.size,
            uploaded_at: timestamp_string(file.date_added),
            primary: index == 0,
            action: "direct".to_string(),
            download_page_url: Some(item.profile_url.clone()),
        })
        .collect()
}

fn choose_file(
    item: &GameBananaMod,
    selected_file_id: Option<&str>,
) -> Result<GameBananaFile, String> {
    let files = supported_files(item);
    if let Some(selected) = selected_file_id {
        let file_id = parse_positive_id(selected, "GameBanana file")?;
        return files
            .into_iter()
            .find(|file| file.id == file_id)
            .ok_or_else(|| {
                "The selected GameBanana file is unavailable or did not pass provider safety checks."
                    .to_string()
            });
    }
    files.into_iter().next().ok_or_else(|| {
        "GameBanana returned no ZIP, 7Z, or RAR archive that passed its file safety checks."
            .to_string()
    })
}

fn download_file(
    store_root: &Path,
    game_id: u64,
    item_id: u64,
    file: &GameBananaFile,
) -> Result<PathBuf, String> {
    if !file_is_supported(file) {
        return Err(
            "The selected GameBanana download is not a supported, verified archive.".to_string(),
        );
    }
    let download_dir = store_root
        .join("downloads")
        .join("gamebanana")
        .join(format!("{game_id}-{item_id}-{}", file.id));
    fs::create_dir_all(&download_dir).map_err(error_to_string)?;
    let archive_path = download_dir.join(format!(
        "{}-{}",
        Uuid::new_v4(),
        sanitize_file_segment(&file.name)
    ));
    download_url_to_file(&provider_client()?, &file.download_url, &archive_path)?;
    if let Some(expected) = file.size.filter(|size| *size > 0) {
        let actual = fs::metadata(&archive_path).map_err(error_to_string)?.len();
        if actual != expected {
            let _ = fs::remove_file(&archive_path);
            let _ = fs::remove_file(download_artifact_metadata_path(&archive_path));
            return Err(format!(
                "GameBanana file {} failed its size check (expected {} bytes, received {} bytes).",
                file.id, expected, actual
            ));
        }
    }
    Ok(archive_path)
}

fn verified_mapping(
    store_root: &Path,
    profile: &GameProfile,
    encoded_game_id: u64,
    supplied_provider_game_id: Option<&str>,
) -> Result<ProviderGameMapping, String> {
    let resolution = resolve_mapping(store_root, profile, false);
    let mapping = resolution.mapping.ok_or_else(|| {
        format!(
            "Mod Garrison could not verify an exact GameBanana game catalogue for {}.",
            profile_game_label(profile)
        )
    })?;
    validate_mapping_for_profile(profile, &mapping)?;
    let expected = parse_positive_id(&mapping.provider_game_id, "GameBanana game")?;
    if encoded_game_id != expected {
        return Err(
            "This GameBanana result belongs to a different game catalogue. Refresh Discovery and try again."
                .to_string(),
        );
    }
    if let Some(supplied) = supplied_provider_game_id {
        if parse_positive_id(supplied, "GameBanana game")? != expected {
            return Err(
                "The selected GameBanana result no longer matches its verified game catalogue."
                    .to_string(),
            );
        }
    }
    Ok(mapping)
}

fn validate_mapping_for_profile(
    profile: &GameProfile,
    mapping: &ProviderGameMapping,
) -> Result<(), String> {
    let steam_app_id = profile
        .steam_app_id
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if !mapping.provider.eq_ignore_ascii_case("gamebanana")
        || steam_app_id.is_empty()
        || mapping.steam_app_id != steam_app_id
        || parse_positive_id(&mapping.provider_game_id, "GameBanana game").is_err()
    {
        return Err(
            "The saved GameBanana game mapping does not match this verified Steam profile."
                .to_string(),
        );
    }
    Ok(())
}

fn verify_item_game(mapping: &ProviderGameMapping, item: &GameBananaMod) -> Result<(), String> {
    if !exact_game_name_match(&item.game_name, &mapping.canonical_game_name) {
        return Err(format!(
            "GameBanana reports that '{}' belongs to '{}', not the verified '{}' catalogue. No files were installed.",
            item.name, item.game_name, mapping.canonical_game_name
        ));
    }
    Ok(())
}

fn provider_document(game_id: u64, item: &GameBananaMod) -> ProviderRouteDocument {
    let file_descriptions = item
        .files
        .iter()
        .filter_map(|file| file.description.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    ProviderRouteDocument {
        provider: "GameBanana".to_string(),
        mod_id: package_id(game_id, item.id),
        mod_name: item.name.clone(),
        text: [
            item.description.as_str(),
            item.text.as_str(),
            item.install_instructions.as_str(),
            file_descriptions.as_str(),
        ]
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .map(clean_provider_text)
        .collect::<Vec<_>>()
        .join("\n"),
    }
}

fn parse_online_mod_id(value: &str) -> Result<(u64, u64), String> {
    let raw = value
        .strip_prefix("gamebanana:")
        .ok_or_else(|| format!("Invalid GameBanana mod id: {value}"))?;
    let (game, item) = raw
        .split_once('/')
        .ok_or_else(|| format!("Invalid GameBanana mod id: {value}"))?;
    if item.contains('/') {
        return Err(format!("Invalid GameBanana mod id: {value}"));
    }
    Ok((
        parse_positive_id(game, "GameBanana game")?,
        parse_positive_id(item, "GameBanana mod")?,
    ))
}

fn package_id(game_id: u64, item_id: u64) -> String {
    format!("gamebanana:{game_id}/{item_id}")
}

fn provider_file_version(file_id: u64) -> String {
    format!("file-{file_id}")
}

fn parse_positive_id(value: &str, label: &str) -> Result<u64, String> {
    let id = value
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("Invalid {label} id."))?;
    if id == 0 {
        return Err(format!("Invalid {label} id."));
    }
    Ok(id)
}

fn exact_game_name_match(first: &str, second: &str) -> bool {
    normalized_exact_name(first) == normalized_exact_name(second)
}

fn normalized_exact_name(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn clean_provider_text(value: &str) -> String {
    clean_nexus_summary(Some(value.to_string()))
}

fn timestamp_string(value: Option<i64>) -> Option<String> {
    value.and_then(|timestamp| {
        chrono::DateTime::<Utc>::from_timestamp(timestamp, 0).map(|value| value.to_rfc3339())
    })
}

fn official_download_url(value: &str, file_id: u64) -> bool {
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
    {
        return false;
    }
    let Some(host) = parsed.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    let official_host = host == "gamebanana.com" || host.ends_with(".gamebanana.com");
    if !official_host {
        return false;
    }
    if host == "gamebanana.com" || host == "www.gamebanana.com" {
        return parsed.path() == format!("/dl/{file_id}");
    }
    true
}

fn official_gamebanana_url(value: &str) -> bool {
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("gamebanana.com")
                || host.to_ascii_lowercase().ends_with(".gamebanana.com")
        })
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port().is_none()
}

fn value_string(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn value_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
}

fn value_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
}

fn value_bool(value: &serde_json::Value) -> Option<bool> {
    value.as_bool().or_else(|| {
        let value = value.as_str()?.trim();
        if value.eq_ignore_ascii_case("true") || value == "1" {
            Some(true)
        } else if value.eq_ignore_ascii_case("false") || value == "0" {
            Some(false)
        } else {
            None
        }
    })
}

fn gamebanana_api_error(value: &serde_json::Value, context: &str) -> String {
    let message = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("message").and_then(serde_json::Value::as_str))
        .unwrap_or("unexpected response");
    format!("GameBanana returned an invalid {context} response: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_game_mapping_is_case_insensitive_but_not_fuzzy() {
        assert!(exact_game_name_match(
            "RuneScape: Dragonwilds",
            "runescape:  dragonwilds"
        ));
        assert!(!exact_game_name_match(
            "RuneScape: Dragonwilds",
            "RuneScape Dragonwilds"
        ));
        assert!(!exact_game_name_match("Valheim", "Valheim Plus"));
    }

    #[test]
    fn catalogue_sort_modes_use_gamebanana_index_aliases() {
        assert_eq!(gamebanana_sort_alias("downloads"), "Generic_MostDownloaded");
        assert_eq!(gamebanana_sort_alias("newest"), "Generic_Newest");
        assert_eq!(gamebanana_sort_alias("oldest"), "Generic_Oldest");
        assert_eq!(
            gamebanana_sort_alias("unsupported"),
            "Generic_MostDownloaded"
        );
    }

    #[test]
    fn catalogue_row_budget_rounds_up_to_complete_provider_pages() {
        assert_eq!(rounded_catalogue_row_budget(0), 0);
        assert_eq!(rounded_catalogue_row_budget(1), CATALOGUE_PAGE_SIZE);
        assert_eq!(rounded_catalogue_row_budget(20), CATALOGUE_PAGE_SIZE);
        assert_eq!(rounded_catalogue_row_budget(21), 2 * CATALOGUE_PAGE_SIZE);
        assert_eq!(rounded_catalogue_row_budget(300), 300);
    }

    #[test]
    fn catalogue_index_parser_keeps_exact_metadata_and_valid_rows() {
        let page = parse_catalogue_index_page(&serde_json::json!({
            "_aMetadata": {
                "_nRecordCount": "2637",
                "_nPerpage": 20,
                "_bIsComplete": "false"
            },
            "_aRecords": [
                {
                    "_idRow": 41,
                    "_sName": "Popular Overhaul",
                    "_aGame": { "_idRow": 8898 }
                },
                {
                    "_idRow": "42",
                    "_sName": "Small Fix",
                    "_aGame": { "_idRow": "8898" }
                },
                {
                    "_idRow": 0,
                    "_sName": "Invalid row"
                }
            ]
        }))
        .unwrap();

        assert_eq!(page.total, 2637);
        assert_eq!(page.raw_record_count, 3);
        assert!(!page.complete);
        assert_eq!(
            page.records,
            vec![
                CatalogueIndexRecord {
                    id: 41,
                    name: "Popular Overhaul".to_string(),
                    game_id: Some(8898),
                },
                CatalogueIndexRecord {
                    id: 42,
                    name: "Small Fix".to_string(),
                    game_id: Some(8898),
                },
            ]
        );
        assert!(catalogue_name_matches_query(
            &page.records[0].name,
            "overhaul"
        ));
        assert!(!catalogue_name_matches_query(
            &page.records[1].name,
            "overhaul"
        ));
    }

    #[test]
    fn online_ids_are_strict_and_positive() {
        assert_eq!(
            parse_online_mod_id("gamebanana:6506/574014").unwrap(),
            (6506, 574014)
        );
        assert!(parse_online_mod_id("gamebanana:6506/574014/extra").is_err());
        assert!(parse_online_mod_id("gamebanana:0/574014").is_err());
        assert!(parse_online_mod_id("nexus:6506/574014").is_err());
    }

    #[test]
    fn provider_files_require_an_official_safe_archive() {
        let file = GameBananaFile {
            id: 1377860,
            name: "mod.zip".to_string(),
            description: None,
            size: Some(42),
            date_added: None,
            downloads: 0,
            download_url: "https://gamebanana.com/dl/1377860".to_string(),
            analysis_state: Some("done".to_string()),
            analysis_result: Some("ok".to_string()),
            antivirus_state: Some("done".to_string()),
            antivirus_result: Some("clean".to_string()),
            archived: false,
            has_contents: Some(true),
        };
        assert!(file_is_supported(&file));
        let mut unsafe_host = file.clone();
        unsafe_host.download_url = "https://example.com/dl/1377860".to_string();
        assert!(!file_is_supported(&unsafe_host));
        let mut executable = file.clone();
        executable.name = "installer.exe".to_string();
        assert!(!file_is_supported(&executable));
        let mut infected = file;
        infected.antivirus_result = Some("infected".to_string());
        assert!(!file_is_supported(&infected));
    }

    #[test]
    fn details_parser_accepts_gamebanana_string_ids_and_flags() {
        let value = serde_json::json!({
            "name": "Example Mod",
            "description": "Description",
            "downloads": 12,
            "likes": 3,
            "date": 1700000000,
            "mdate": 1700000100,
            "Owner().name": "Author",
            "Game().name": "Example Game",
            "RootCategory().name": "Game files",
            "Nsfw().bIsNsfw()": false,
            "Trash().bIsTrashed()": "false",
            "Withhold().bIsWithheld()": false,
            "is_obsolete": "false",
            "Files().aFiles()": {
                "99": {
                    "_idRow": "99",
                    "_sFile": "example.zip",
                    "_nFilesize": 123,
                    "_sDownloadUrl": "https://gamebanana.com/dl/99",
                    "_sAnalysisState": "done",
                    "_sAnalysisResult": "ok",
                    "_sAvState": "done",
                    "_sAvResult": "clean",
                    "_bIsArchived": false,
                    "_bHasContents": true
                }
            }
        });
        let parsed = parse_mod(7, &value).unwrap();
        assert_eq!(parsed.name, "Example Mod");
        assert_eq!(parsed.files[0].id, 99);
        assert!(file_is_supported(&parsed.files[0]));
    }

    #[test]
    fn runtimes_and_tools_are_not_regular_gamebanana_mods() {
        let mut item = parse_mod(
            7,
            &serde_json::json!({
                "name": "BepInEx for Example Game",
                "Game().name": "Example Game",
                "RootCategory().name": "Game files",
                "Files().aFiles()": {}
            }),
        )
        .unwrap();
        assert!(item_is_runtime_or_tool(&item));
        item.name = "Example Gameplay Overhaul".to_string();
        item.category = "Tools".to_string();
        assert!(item_is_runtime_or_tool(&item));
        item.category = "Game files".to_string();
        assert!(!item_is_runtime_or_tool(&item));
    }
}
