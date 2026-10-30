use super::*;

pub(super) fn fetch_package_version(
    package_ref: &ThunderstorePackageRef,
) -> Result<ThunderstoreVersion, String> {
    // Discovery already downloaded this official catalogue, including exact
    // version metadata. Reuse it instead of requiring a second API to be up.
    // Do not fetch the large community feed just to install one package.
    if let Some(version) = cached_package_version(package_ref) {
        return Ok(version);
    }

    let client = thunderstore_client()?;
    let url = package_metadata_url(package_ref, package_ref.version.as_deref());
    fetch_package_version_with_client(&client, package_ref, &url)
}

pub(super) fn package_metadata_url(
    package_ref: &ThunderstorePackageRef,
    version: Option<&str>,
) -> String {
    let mut url = url::Url::parse(&format!("{THUNDERSTORE_API_BASE}/"))
        .expect("the built-in Thunderstore API base is valid");
    {
        let mut segments = url.path_segments_mut().expect("the API URL has a path");
        segments
            .pop_if_empty()
            .push(&package_ref.namespace)
            .push(&package_ref.name);
        if let Some(version) = version {
            segments.push(version);
        }
        segments.push("");
    }
    url.into()
}

fn cached_package_version(package_ref: &ThunderstorePackageRef) -> Option<ThunderstoreVersion> {
    // An unpinned runtime update must still ask the provider for the latest.
    package_ref.version.as_ref()?;
    let cache = THUNDERSTORE_CACHE.get()?.lock().ok()?;
    cached_package_version_from_entries(&cache, package_ref)
}

fn cached_package_version_from_entries(
    cache: &HashMap<String, ThunderstoreCacheEntry>,
    package_ref: &ThunderstorePackageRef,
) -> Option<ThunderstoreVersion> {
    package_ref.version.as_ref()?;
    cache
        .values()
        .filter(|entry| entry.fetched_at.elapsed() < Duration::from_secs(10 * 60))
        .find_map(|entry| catalogue_package_version(&entry.packages, package_ref).cloned())
}

fn catalogue_package_version<'a>(
    packages: &'a [ThunderstoreCommunityPackage],
    package_ref: &ThunderstorePackageRef,
) -> Option<&'a ThunderstoreVersion> {
    let requested_version = package_ref.version.as_deref()?;
    packages
        .iter()
        .filter(|package| {
            package.owner.eq_ignore_ascii_case(&package_ref.namespace)
                && package.name.eq_ignore_ascii_case(&package_ref.name)
        })
        .flat_map(|package| &package.versions)
        .find(|version| {
            version.is_active
                && version.version_number == requested_version
                && version_matches_package(version, package_ref)
        })
}

fn version_matches_package(
    version: &ThunderstoreVersion,
    package_ref: &ThunderstorePackageRef,
) -> bool {
    package_ref
        .version
        .as_deref()
        .is_none_or(|requested| version.version_number == requested)
        && version.full_name.eq_ignore_ascii_case(&format!(
            "{}-{}-{}",
            package_ref.namespace, package_ref.name, version.version_number
        ))
}

fn fetch_package_version_with_client(
    client: &Client,
    package_ref: &ThunderstorePackageRef,
    url: &str,
) -> Result<ThunderstoreVersion, String> {
    let label = match package_ref.version.as_deref() {
        Some(version) => format!(
            "{}/{} version {version}",
            package_ref.namespace, package_ref.name
        ),
        None => format!("{}/{}", package_ref.namespace, package_ref.name),
    };
    let response = request_metadata_with_retry(client, url)
        .map_err(|error| format!("Thunderstore could not load {label}: {error}"))?;
    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        return Err(format!("Thunderstore no longer lists {label} (HTTP 404)."));
    }
    if !status.is_success() {
        return Err(format!(
            "Thunderstore could not load {label} (HTTP {}). {}",
            status.as_u16(),
            if is_transient_download_status(status) {
                "The provider is temporarily unavailable; try installing again shortly."
            } else {
                "The provider rejected the metadata request."
            }
        ));
    }
    let version = if package_ref.version.is_some() {
        response.json::<ThunderstoreVersion>()
    } else {
        response
            .json::<ThunderstorePackageResponse>()
            .map(|package| package.latest)
    }
    .map_err(|error| {
        format!(
            "Thunderstore returned invalid metadata for {label}: {}",
            error.without_url()
        )
    })?;
    if !version_matches_package(&version, package_ref) {
        return Err(format!(
            "Thunderstore returned a different package or version for {label}; installation was stopped."
        ));
    }
    if !version.is_active {
        return Err(format!("Thunderstore has deactivated {label}."));
    }
    Ok(version)
}

fn request_metadata_with_retry(client: &Client, url: &str) -> Result<Response, String> {
    for attempt in 0..MAX_DOWNLOAD_ATTEMPTS {
        match client.get(url).send() {
            Ok(response) => {
                if is_transient_download_status(response.status())
                    && attempt + 1 < MAX_DOWNLOAD_ATTEMPTS
                {
                    // Leave a long provider cooldown to the next user retry
                    // instead of shortening Retry-After to our bounded backoff.
                    if response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .is_some_and(|value| {
                            value
                                .trim()
                                .parse::<u64>()
                                .map_or(true, |seconds| seconds > DOWNLOAD_RETRY_MAX_DELAY_SECS)
                        })
                    {
                        return Ok(response);
                    }
                    let delay = download_retry_delay(&response, attempt);
                    drop(response);
                    std::thread::sleep(delay);
                    continue;
                }
                return Ok(response);
            }
            Err(error) => {
                if (error.is_connect() || error.is_timeout()) && attempt + 1 < MAX_DOWNLOAD_ATTEMPTS
                {
                    std::thread::sleep(default_download_retry_delay(attempt));
                    continue;
                }
                return Err(format!("Metadata request failed: {}", error.without_url()));
            }
        }
    }
    Err("The metadata server did not respond after several attempts.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn package_ref(version: &str) -> ThunderstorePackageRef {
        ThunderstorePackageRef {
            namespace: "notnotnotswipez".to_string(),
            name: "MoreCompany".to_string(),
            version: Some(version.to_string()),
        }
    }

    fn version_json(version: &str, active: bool) -> String {
        serde_json::json!({
            "version_number": version,
            "full_name": format!("notnotnotswipez-MoreCompany-{version}"),
            "download_url": format!("https://thunderstore.io/package/download/notnotnotswipez/MoreCompany/{version}/"),
            "dependencies": ["BepInEx-BepInExPack-5.4.2100"],
            "is_active": active,
            "file_size": 11234567
        }).to_string()
    }

    fn metadata_server(responses: Vec<(u16, String)>) -> (String, std::thread::JoinHandle<()>) {
        metadata_server_with_retry_after(responses, "0")
    }

    fn metadata_server_with_retry_after(
        responses: Vec<(u16, String)>,
        retry_after: &str,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let retry_after = retry_after.to_string();
        let server = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = [0; 2048];
                let _ = stream.read(&mut request).unwrap();
                write!(stream, "HTTP/1.1 {status} test\r\nContent-Type: application/json\r\nRetry-After: {retry_after}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                stream.flush().unwrap();
            }
        });
        (format!("http://{address}/metadata/"), server)
    }

    #[test]
    fn metadata_urls_preserve_dotted_versions_and_encode_segment_boundaries() {
        let package = package_ref("1.14.0");
        assert_eq!(
            package_metadata_url(&package, Some("1.14.0")),
            "https://thunderstore.io/api/experimental/package/notnotnotswipez/MoreCompany/1.14.0/"
        );
        assert_eq!(
            package_metadata_url(&package, None),
            "https://thunderstore.io/api/experimental/package/notnotnotswipez/MoreCompany/"
        );
        let malicious_hint = "1.14.0/../../other?download=https://example.invalid#fragment";
        let url = url::Url::parse(&package_metadata_url(&package, Some(malicious_hint))).unwrap();
        assert_eq!(url.host_str(), Some("thunderstore.io"));
        assert!(url.query().is_none());
        assert!(url.fragment().is_none());
        let segments = url.path_segments().unwrap().collect::<Vec<_>>();
        assert_eq!(
            &segments[..5],
            &[
                "api",
                "experimental",
                "package",
                "notnotnotswipez",
                "MoreCompany"
            ]
        );
        assert_eq!(segments.len(), 7);
        assert!(segments[5].contains("%2F"));
        assert!(segments[5].contains("%3F"));
        assert!(segments[5].contains("%23"));
    }

    #[test]
    fn catalogue_reuses_the_exact_active_version_with_dependencies_and_size() {
        let mut package: ThunderstoreCommunityPackage = serde_json::from_value(serde_json::json!({
            "name": "MoreCompany", "owner": "notnotnotswipez", "full_name": "notnotnotswipez-MoreCompany",
            "versions": [serde_json::from_str::<serde_json::Value>(&version_json("1.14.0", true)).unwrap(),
                         serde_json::from_str::<serde_json::Value>(&version_json("1.13.3", true)).unwrap()]
        })).unwrap();
        let packages = vec![package.clone()];
        let version = catalogue_package_version(&packages, &package_ref("1.13.3")).unwrap();
        assert_eq!(version.version_number, "1.13.3");
        assert_eq!(version.dependencies, vec!["BepInEx-BepInExPack-5.4.2100"]);
        assert_eq!(version.file_size, Some(11234567));
        assert!(catalogue_package_version(&packages, &package_ref("99.0.0")).is_none());
        package.versions[0].is_active = false;
        assert!(catalogue_package_version(&[package.clone()], &package_ref("1.14.0")).is_none());
        package.owner = "another-author".to_string();
        assert!(catalogue_package_version(&[package], &package_ref("1.13.3")).is_none());
    }

    #[test]
    fn metadata_cache_expires_and_never_answers_an_unpinned_update() {
        let package: ThunderstoreCommunityPackage = serde_json::from_value(serde_json::json!({
            "name": "MoreCompany", "owner": "notnotnotswipez", "full_name": "notnotnotswipez-MoreCompany",
            "versions": [serde_json::from_str::<serde_json::Value>(&version_json("1.14.0", true)).unwrap()]
        })).unwrap();
        let mut cache = HashMap::from([(
            "lethal-company".to_string(),
            ThunderstoreCacheEntry {
                fetched_at: Instant::now(),
                packages: vec![package],
            },
        )]);
        let mut requested = package_ref("1.14.0");
        assert!(cached_package_version_from_entries(&cache, &requested).is_some());
        requested.version = None;
        assert!(cached_package_version_from_entries(&cache, &requested).is_none());
        requested.version = Some("1.14.0".to_string());
        cache.get_mut("lethal-company").unwrap().fetched_at =
            Instant::now() - Duration::from_secs(10 * 60 + 1);
        assert!(cached_package_version_from_entries(&cache, &requested).is_none());
    }

    #[test]
    fn transient_metadata_error_recovers_without_changing_the_requested_version() {
        let (url, server) = metadata_server(vec![
            (502, String::new()),
            (200, version_json("1.14.0", true)),
        ]);
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let version =
            fetch_package_version_with_client(&client, &package_ref("1.14.0"), &url).unwrap();
        assert_eq!(version.version_number, "1.14.0");
        server.join().unwrap();
    }

    #[test]
    fn persistent_provider_outage_is_not_reported_as_a_missing_version() {
        let (url, server) = metadata_server(vec![(502, String::new()); MAX_DOWNLOAD_ATTEMPTS]);
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let error =
            fetch_package_version_with_client(&client, &package_ref("1.14.0"), &url).unwrap_err();
        assert!(error.contains("HTTP 502"), "{error}");
        assert!(error.contains("temporarily unavailable"), "{error}");
        assert!(!error.contains("does not have version"), "{error}");
        assert!(!error.contains("no longer lists"), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn missing_version_is_reported_without_retrying_permanent_errors() {
        let (url, server) = metadata_server(vec![(404, String::new())]);
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let error =
            fetch_package_version_with_client(&client, &package_ref("1.14.0"), &url).unwrap_err();
        assert!(error.contains("no longer lists"), "{error}");
        assert!(error.contains("HTTP 404"), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn long_provider_cooldown_is_not_shortened_for_an_automatic_retry() {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        for retry_after in ["60", "Wed, 21 Oct 2037 07:28:00 GMT"] {
            let (url, server) =
                metadata_server_with_retry_after(vec![(429, String::new())], retry_after);
            let error = fetch_package_version_with_client(&client, &package_ref("1.14.0"), &url)
                .unwrap_err();
            assert!(error.contains("HTTP 429"), "{error}");
            server.join().unwrap();
        }
    }

    #[test]
    fn different_or_deactivated_version_cannot_be_installed() {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let wrong_namespace = version_json("1.14.0", true).replace(
            "notnotnotswipez-MoreCompany-1.14.0",
            "another-author-MoreCompany-1.14.0",
        );
        for (json, expected) in [
            (version_json("1.13.3", true), "different package or version"),
            (wrong_namespace, "different package or version"),
            (version_json("1.14.0", false), "deactivated"),
        ] {
            let (url, server) = metadata_server(vec![(200, json)]);
            let error = fetch_package_version_with_client(&client, &package_ref("1.14.0"), &url)
                .unwrap_err();
            assert!(error.contains(expected), "{error}");
            server.join().unwrap();
        }
    }
}
