use super::*;

const STEAM_WORKSHOP_DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const STEAM_WORKSHOP_DETAILS_BATCH_SIZE: usize = 100;
const STEAM_WORKSHOP_CACHE_MINUTES: u64 = 10;
const STEAM_WORKSHOP_INSTALL_NOTE: &str = "This Workshop content was detected in the selected Steam library and remains managed by Steam. Mod Garrison will not subscribe, download, update, move, or remove it.";

static STEAM_WORKSHOP_CACHE: OnceLock<Mutex<HashMap<String, SteamWorkshopCacheEntry>>> =
    OnceLock::new();

#[derive(Debug, Clone)]
struct SteamWorkshopCacheEntry {
    fetched_at: Instant,
    records: Vec<OnlineModRecord>,
}

#[derive(Debug, Default, Deserialize)]
struct SteamWorkshopDetailsEnvelope {
    #[serde(default)]
    response: SteamWorkshopDetailsResponse,
}

#[derive(Debug, Default, Deserialize)]
struct SteamWorkshopDetailsResponse {
    #[serde(default)]
    publishedfiledetails: Vec<SteamWorkshopItemDetail>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct SteamWorkshopItemDetail {
    #[serde(default)]
    publishedfileid: String,
    #[serde(default)]
    result: i64,
    #[serde(default)]
    creator: String,
    #[serde(default)]
    consumer_app_id: u64,
    #[serde(default, deserialize_with = "deserialize_steam_u64")]
    file_size: u64,
    #[serde(default)]
    preview_url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    time_created: i64,
    #[serde(default)]
    time_updated: i64,
    #[serde(default)]
    subscriptions: u64,
    #[serde(default)]
    lifetime_subscriptions: u64,
    #[serde(default)]
    banned: i64,
    #[serde(default)]
    tags: Vec<SteamWorkshopTag>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct SteamWorkshopTag {
    #[serde(default)]
    tag: String,
}

pub(super) fn discover_mods(
    profile: &GameProfile,
    force_refresh: bool,
) -> Result<Vec<OnlineModRecord>, String> {
    let game = verified_steam_game_for_profile(profile)?;
    let app_id = normalized_steam_app_id(&game.app_id)?;
    let workshop_root = Path::new(&game.library_path)
        .join("steamapps")
        .join("workshop")
        .join("content")
        .join(&app_id);
    let item_ids = workshop_item_ids(&workshop_root);
    if item_ids.is_empty() {
        if force_refresh {
            clear_cache_for_app(&app_id);
        }
        return Ok(Vec::new());
    }

    // Records carry the owning Mod Garrison game id, so profiles must not
    // share cache entries even when they point at the same Steam app.
    let cache_key = format!("{app_id}:{}:{}", profile.id, item_ids.join(","));
    if !force_refresh {
        if let Some(records) = cached_records(&cache_key) {
            return Ok(records);
        }
    } else {
        clear_cache_for_app(&app_id);
    }

    // The folders are the source of truth for this deliberately local-only
    // integration. Metadata enrichment must not make downloaded Workshop
    // content disappear when Steam's public endpoint is offline.
    let details = fetch_details(&item_ids).unwrap_or_default();
    let details_by_id = details
        .into_iter()
        .map(|detail| (detail.publishedfileid.clone(), detail))
        .collect::<HashMap<_, _>>();
    let mut records = Vec::new();

    for item_id in item_ids {
        let detail = details_by_id.get(&item_id);
        if detail.is_some_and(|detail| {
            detail.result == 1
                && (detail.banned != 0 || detail.consumer_app_id.to_string() != app_id)
        }) {
            continue;
        }
        let detail = detail.filter(|detail| detail.result == 1);

        let title = detail
            .map(|detail| detail.title.trim())
            .filter(|title| !title.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Steam Workshop item {item_id}"));
        if is_known_runtime_provider_package("steam-workshop", None, &title) {
            continue;
        }

        let description = detail
            .map(|detail| clean_provider_text(&detail.description))
            .unwrap_or_default();
        let categories = detail
            .map(|detail| {
                detail
                    .tags
                    .iter()
                    .map(|tag| tag.tag.trim())
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let creator = detail
            .map(|detail| detail.creator.trim())
            .filter(|creator| !creator.is_empty())
            .map(|creator| format!("Steam user {creator}"))
            .unwrap_or_else(|| "Steam Workshop author".to_string());
        let package_id = format!("steam-workshop:{app_id}/{item_id}");
        let page_url = format!("steam://url/CommunityFilePage/{item_id}");
        let website_url =
            format!("https://steamcommunity.com/sharedfiles/filedetails/?id={item_id}");

        records.push(OnlineModRecord {
            id: package_id,
            provider: "steam-workshop".to_string(),
            provider_label: "Steam Workshop".to_string(),
            game_id: profile.game_id.clone(),
            provider_game_id: Some(app_id.clone()),
            name: title,
            owner: creator,
            version: String::new(),
            description,
            categories,
            downloads: detail.map_or(0, |detail| {
                detail.lifetime_subscriptions.max(detail.subscriptions)
            }),
            rating_score: 0,
            dependency_count: 0,
            file_size: detail
                .map(|detail| detail.file_size)
                .filter(|size| *size > 0),
            icon_url: detail
                .map(|detail| detail.preview_url.trim())
                .filter(|url| url.starts_with("https://"))
                .map(str::to_string),
            package_url: Some(page_url),
            website_url: Some(website_url),
            installed: true,
            created_at: detail.and_then(|detail| unix_timestamp_string(detail.time_created)),
            updated_at: detail.and_then(|detail| unix_timestamp_string(detail.time_updated)),
            install_supported: false,
            install_note: Some(STEAM_WORKSHOP_INSTALL_NOTE.to_string()),
        });
    }

    records.retain(|record| !is_external_mod_manager_listing(record));
    sort_online_mods(&mut records, "downloads");
    remember_records(cache_key, records.clone());
    Ok(records)
}

pub(super) fn list_files(
    store_root: &Path,
    profile: &GameProfile,
    mod_id: &str,
    supplied_provider_game_id: Option<&str>,
) -> Result<Vec<OnlineModFileOption>, String> {
    let (app_id, item_id) = parse_online_mod_id(mod_id)?;
    let verified_app_id = verified_discovery_provider_game(
        store_root,
        profile,
        "steam-workshop",
        supplied_provider_game_id,
        Some(&app_id),
    )?;
    if verified_app_id != app_id {
        return Err(
            "This Steam Workshop item belongs to a different game. No files were changed."
                .to_string(),
        );
    }
    let game = verified_steam_game_for_profile(profile)?;
    let workshop_root = Path::new(&game.library_path)
        .join("steamapps")
        .join("workshop")
        .join("content")
        .join(&app_id);
    let Some(item_path) = safe_workshop_item_path(&workshop_root, &item_id) else {
        return Err(
            "Steam no longer reports this Workshop item as downloaded for the selected game."
                .to_string(),
        );
    };

    Ok(vec![OnlineModFileOption {
        id: item_id.clone(),
        name: "Locally downloaded Steam Workshop content".to_string(),
        version: None,
        category: Some("Steam-managed local content".to_string()),
        description: Some(STEAM_WORKSHOP_INSTALL_NOTE.to_string()),
        file_name: None,
        file_size: directory_size(&item_path),
        uploaded_at: None,
        primary: true,
        action: "unsupported".to_string(),
        download_page_url: Some(format!("steam://url/CommunityFilePage/{item_id}")),
    }])
}

pub(super) fn verified_app_id(profile: &GameProfile) -> Result<String, String> {
    let game = verified_steam_game_for_profile(profile)?;
    normalized_steam_app_id(&game.app_id)
}

fn normalized_steam_app_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    value
        .parse::<u32>()
        .ok()
        .filter(|app_id| *app_id > 0 && app_id.to_string() == value)
        .map(|app_id| app_id.to_string())
        .ok_or_else(|| "The selected profile does not have a valid Steam App ID.".to_string())
}

fn workshop_item_ids(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut ids = entries
        .flatten()
        .take(MAX_SCAN_ENTRIES)
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().to_string();
            normalized_workshop_item_id(&id)
                .ok()
                .filter(|normalized| normalized == &id)
                .and_then(|_| safe_workshop_item_path(root, &id).map(|_| id))
        })
        .collect::<Vec<_>>();
    ids.sort_by(|first, second| {
        first
            .parse::<u64>()
            .unwrap_or_default()
            .cmp(&second.parse::<u64>().unwrap_or_default())
    });
    ids.dedup();
    ids
}

fn normalized_workshop_item_id(value: &str) -> Result<String, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|item_id| *item_id > 0 && item_id.to_string() == value)
        .map(|item_id| item_id.to_string())
        .ok_or_else(|| format!("Invalid Steam Workshop item id: {value}"))
}

fn safe_workshop_item_path(root: &Path, item_id: &str) -> Option<PathBuf> {
    normalized_workshop_item_id(item_id).ok()?;
    let canonical_root = fs::canonicalize(root).ok()?;
    let item_path = root.join(item_id);
    let metadata = fs::symlink_metadata(&item_path).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    let canonical_item = fs::canonicalize(item_path).ok()?;
    if !canonical_item.starts_with(&canonical_root) || !directory_has_content(&canonical_item) {
        return None;
    }
    Some(canonical_item)
}

fn directory_has_content(path: &Path) -> bool {
    fs::read_dir(path)
        .ok()
        .and_then(|mut entries| entries.next())
        .is_some()
}

fn directory_size(path: &Path) -> Option<u64> {
    let mut total = 0_u64;
    let mut pending = vec![path.to_path_buf()];
    let mut visited = 0_usize;
    while let Some(current) = pending.pop() {
        if visited >= MAX_SCAN_ENTRIES {
            return None;
        }
        let entries = fs::read_dir(current).ok()?;
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_SCAN_ENTRIES {
                return None;
            }
            let metadata = fs::symlink_metadata(entry.path()).ok()?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Some(total)
}

fn fetch_details(item_ids: &[String]) -> Result<Vec<SteamWorkshopItemDetail>, String> {
    let client = provider_client()?;
    let mut details = Vec::new();
    for batch in item_ids.chunks(STEAM_WORKSHOP_DETAILS_BATCH_SIZE) {
        let mut form = Vec::with_capacity(batch.len() + 1);
        form.push(("itemcount".to_string(), batch.len().to_string()));
        for (index, item_id) in batch.iter().enumerate() {
            form.push((format!("publishedfileids[{index}]"), item_id.clone()));
        }
        let response = client
            .post(STEAM_WORKSHOP_DETAILS_URL)
            .form(&form)
            .send()
            .map_err(|error| format!("Steam Workshop details could not be loaded: {error}"))?
            .error_for_status()
            .map_err(|error| format!("Steam Workshop details could not be loaded: {error}"))?
            .json::<SteamWorkshopDetailsEnvelope>()
            .map_err(|error| format!("Steam returned invalid Workshop details: {error}"))?;
        details.extend(response.response.publishedfiledetails);
    }
    Ok(details)
}

fn parse_online_mod_id(mod_id: &str) -> Result<(String, String), String> {
    let raw = mod_id
        .strip_prefix("steam-workshop:")
        .ok_or_else(|| format!("Invalid Steam Workshop item id: {mod_id}"))?;
    let (app_id, item_id) = raw
        .split_once('/')
        .ok_or_else(|| format!("Invalid Steam Workshop item id: {mod_id}"))?;
    let app_id = normalized_steam_app_id(app_id)
        .map_err(|_| format!("Invalid Steam Workshop item id: {mod_id}"))?;
    let item_id = normalized_workshop_item_id(item_id)
        .map_err(|_| format!("Invalid Steam Workshop item id: {mod_id}"))?;
    Ok((app_id, item_id))
}

fn unix_timestamp_string(timestamp: i64) -> Option<String> {
    chrono::DateTime::<Utc>::from_timestamp(timestamp, 0).map(|value| value.to_rfc3339())
}

fn deserialize_steam_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum SteamUnsignedInteger {
        Number(u64),
        Text(String),
    }

    let value = Option::<SteamUnsignedInteger>::deserialize(deserializer)?;
    Ok(match value {
        Some(SteamUnsignedInteger::Number(value)) => value,
        Some(SteamUnsignedInteger::Text(value)) => value.parse::<u64>().unwrap_or_default(),
        None => 0,
    })
}

fn clean_provider_text(value: &str) -> String {
    let decoded = decode_provider_html_entities(value);
    let mut output = String::new();
    let mut inside_tag = false;
    for character in decoded.chars() {
        match character {
            '<' => inside_tag = true,
            '>' => {
                inside_tag = false;
                output.push(' ');
            }
            _ if !inside_tag => output.push(character),
            _ => {}
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cached_records(key: &str) -> Option<Vec<OnlineModRecord>> {
    let cache = STEAM_WORKSHOP_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().ok()?;
    let entry = cache.get(key)?;
    if entry.fetched_at.elapsed() > Duration::from_secs(STEAM_WORKSHOP_CACHE_MINUTES * 60) {
        cache.remove(key);
        return None;
    }
    Some(entry.records.clone())
}

fn remember_records(key: String, records: Vec<OnlineModRecord>) {
    let cache = STEAM_WORKSHOP_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut cache) = cache.lock() {
        let now = Instant::now();
        cache.retain(|_, entry| {
            now.duration_since(entry.fetched_at)
                <= Duration::from_secs(STEAM_WORKSHOP_CACHE_MINUTES * 60)
        });
        if cache.len() >= 32 && !cache.contains_key(&key) {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.fetched_at)
                .map(|(key, _)| key.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(
            key,
            SteamWorkshopCacheEntry {
                fetched_at: now,
                records,
            },
        );
    }
}

fn clear_cache_for_app(app_id: &str) {
    let prefix = format!("{app_id}:");
    if let Ok(mut cache) = STEAM_WORKSHOP_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        cache.retain(|key, _| !key.starts_with(&prefix));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workshop_item_scan_only_accepts_numeric_nonempty_directories() {
        let root =
            std::env::temp_dir().join(format!("mod-garrison-steam-workshop-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("123")).unwrap();
        fs::write(root.join("123").join("mod.dat"), b"mod").unwrap();
        fs::create_dir_all(root.join("456")).unwrap();
        fs::create_dir_all(root.join("not-an-id")).unwrap();
        fs::write(root.join("789"), b"not a directory").unwrap();

        assert_eq!(workshop_item_ids(&root), vec!["123".to_string()]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workshop_online_id_parser_rejects_cross_provider_and_malformed_ids() {
        assert_eq!(
            parse_online_mod_id("steam-workshop:3041230/123456").unwrap(),
            ("3041230".to_string(), "123456".to_string())
        );
        assert!(parse_online_mod_id("nexus:windrose/1").is_err());
        assert!(parse_online_mod_id("steam-workshop:3041230/not-a-number").is_err());
        assert!(parse_online_mod_id("steam-workshop:/123").is_err());
        assert!(parse_online_mod_id("steam-workshop:03041230/123").is_err());
        assert!(parse_online_mod_id("steam-workshop:3041230/0").is_err());
        assert!(parse_online_mod_id("steam-workshop:4294967296/123").is_err());
    }

    #[test]
    fn workshop_detail_deserialization_accepts_steam_string_file_sizes() {
        let envelope = serde_json::from_str::<SteamWorkshopDetailsEnvelope>(
            r#"{
                "response": {
                    "publishedfiledetails": [{
                        "publishedfileid": "123456",
                        "result": 1,
                        "consumer_app_id": 3041230,
                        "file_size": "98765",
                        "title": "Example",
                        "tags": [{"tag": "Mod"}]
                    }]
                }
            }"#,
        )
        .unwrap();

        let detail = &envelope.response.publishedfiledetails[0];
        assert_eq!(detail.file_size, 98_765);
        assert_eq!(detail.consumer_app_id, 3_041_230);
        assert_eq!(detail.tags[0].tag, "Mod");
    }

    #[test]
    fn workshop_detail_deserialization_accepts_numeric_and_missing_file_sizes() {
        let numeric = serde_json::from_str::<SteamWorkshopItemDetail>(
            r#"{"publishedfileid":"1","file_size":42}"#,
        )
        .unwrap();
        let missing =
            serde_json::from_str::<SteamWorkshopItemDetail>(r#"{"publishedfileid":"2"}"#).unwrap();

        assert_eq!(numeric.file_size, 42);
        assert_eq!(missing.file_size, 0);
    }

    #[test]
    fn workshop_description_cleanup_removes_markup_and_decodes_entities() {
        assert_eq!(
            clean_provider_text("<p>Balance &amp; fixes</p><br>Single-player"),
            "Balance & fixes Single-player"
        );
    }
}
