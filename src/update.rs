// Adapted from Sofka's src/update.rs and the GET helper in src/plugin_catalog.rs.
// Copyright 2026 Nikola Milojević. Licensed under Apache-2.0.
// Translated and modified for xpdelve in 2026. See NOTICE.
//! Release update checks: is a newer xpdelve published?
//!
//! xpdelve never downloads or replaces itself; the header only reports the
//! latest release. The check runs at most once per [`CHECK_INTERVAL`] and its
//! result is cached under the state directory, so sessions in between read the
//! cache without touching the network.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use serde::{Deserialize, Serialize};

const LATEST_URL: &str = "https://api.github.com/repos/allanyung/xpdelve/releases/latest";
const RELEASES_URL: &str = "https://github.com/allanyung/xpdelve/releases";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_LIMIT: usize = 1024 * 1024;

/// The latest published release, as GitHub reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// Version without the leading `v`, e.g. `0.9.0`.
    pub version: String,
    /// The release notes page.
    pub url: String,
}

impl Release {
    /// Whether this release is newer than the running build.
    pub fn is_newer(&self) -> bool {
        is_newer(env!("CARGO_PKG_VERSION"), &self.version)
    }
}

/// The last network attempt and the last release any attempt found. A failed
/// attempt still updates `checked_at`, so an offline machine or a rate limit
/// does not turn every startup into another request.
#[derive(Serialize, Deserialize)]
struct Cache {
    checked_at: u64,
    release: Option<Release>,
}

/// Where the last check result lives: `<state-dir>/update-check.toml`.
pub fn cache_path() -> PathBuf {
    crate::config::state_dir().join("update-check.toml")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn read_cache(path: &Path) -> Option<Cache> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
}

/// The last release any check found, however old.
pub fn cached(path: &Path) -> Option<Release> {
    read_cache(path).and_then(|cache| cache.release)
}

/// A timestamp in the future (a clock that moved back) counts as stale.
fn is_fresh(cache: &Cache, now: u64) -> bool {
    now.checked_sub(cache.checked_at)
        .is_some_and(|age| age < CHECK_INTERVAL.as_secs())
}

fn store(path: &Path, now: u64, release: Option<&Release>) -> Result<(), String> {
    let cache = Cache {
        checked_at: now,
        release: release.cloned(),
    };
    let text = toml::to_string(&cache).map_err(|e| e.to_string())?;
    crate::atomicfile::write(path, &text).map_err(|e| e.to_string())
}

/// `latest` > `current` as semantic versions. Anything that does not parse is
/// never newer, so a malformed tag cannot produce a notification.
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (
        semver::Version::parse(current),
        semver::Version::parse(latest),
    ) {
        (Ok(current), Ok(latest)) => latest > current,
        _ => false,
    }
}

/// Read `tag_name` and `html_url` from a GitHub release response.
pub fn parse_release(bytes: &[u8]) -> Result<Release, String> {
    #[derive(Deserialize)]
    struct Response {
        tag_name: String,
        html_url: String,
    }
    let response: Response =
        serde_json::from_slice(bytes).map_err(|e| format!("reading release response: {e}"))?;
    let version = response
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&response.tag_name)
        .to_string();
    semver::Version::parse(&version)
        .map_err(|e| format!("release tag {:?} is not a version: {e}", response.tag_name))?;
    let url = if response.html_url.starts_with("https://github.com/") {
        response.html_url
    } else {
        format!("{RELEASES_URL}/tag/v{version}")
    };
    Ok(Release { version, url })
}

/// The latest release: from the cache while it is fresh, otherwise from
/// GitHub, refreshing the cache.
pub async fn check() -> Result<Release, String> {
    check_at(&cache_path(), now(), || get(LATEST_URL, RESPONSE_LIMIT)).await
}

async fn check_at<F, Fut>(path: &Path, now: u64, fetch: F) -> Result<Release, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u8>, String>>,
{
    let previous = read_cache(path);
    if let Some(cache) = previous.as_ref().filter(|cache| is_fresh(cache, now)) {
        return cache
            .release
            .clone()
            .ok_or_else(|| "the last update check failed; retrying within a day".to_string());
    }
    let result = fetch().await.and_then(|bytes| parse_release(&bytes));
    // Read again after a failure: another session may have saved a release
    // while this request was in flight.
    let known = match &result {
        Ok(release) => Some(release.clone()),
        Err(_) => read_cache(path)
            .and_then(|cache| cache.release)
            .or_else(|| previous.and_then(|cache| cache.release)),
    };
    // A cache that cannot be written only costs a repeat request next launch.
    let _ = store(path, now, known.as_ref());
    result
}

type HttpClient = hyper_util::client::legacy::Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Full<Bytes>,
>;

fn client() -> Result<HttpClient, String> {
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .map_err(|e| format!("loading system TLS roots: {e}"))?
        .https_only()
        .enable_http1()
        .build();
    Ok(
        hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build(https),
    )
}

/// One HTTPS GET. Redirects are not followed, the body is capped at `limit`
/// bytes, and the whole request is bounded by [`REQUEST_TIMEOUT`].
async fn get(url: &'static str, limit: usize) -> Result<Vec<u8>, String> {
    tokio::time::timeout(REQUEST_TIMEOUT, get_inner(url, limit))
        .await
        .map_err(|_| format!("request timed out after {}s", REQUEST_TIMEOUT.as_secs()))?
}

async fn get_inner(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let request = http::Request::get(url)
        .header(
            http::header::USER_AGENT,
            concat!("xpdelve/", env!("CARGO_PKG_VERSION")),
        )
        .header(http::header::ACCEPT, "application/vnd.github+json")
        .body(Full::new(Bytes::new()))
        .map_err(|e| format!("building request: {e}"))?;
    let response = client()?
        .request(request)
        .await
        .map_err(|e| format!("requesting latest release: {e}"))?;
    let status = response.status();
    let rate_limited = status == http::StatusCode::FORBIDDEN
        && response.headers().contains_key("x-ratelimit-remaining");
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| format!("reading response: {e}"))?;
        if let Ok(data) = frame.into_data() {
            if bytes.len().saturating_add(data.len()) > limit {
                return Err(format!("response exceeds {} MiB", limit / 1024 / 1024));
            }
            bytes.extend_from_slice(&data);
        }
    }
    if rate_limited {
        return Err("GitHub API rate limit reached; retry later".into());
    }
    if !status.is_success() {
        return Err(format!("HTTP {status} from GitHub"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn release(version: &str) -> Release {
        Release {
            version: version.into(),
            url: format!("https://github.com/allanyung/xpdelve/releases/tag/v{version}"),
        }
    }

    fn body(version: &str) -> Vec<u8> {
        format!(
            r#"{{"tag_name":"v{version}","html_url":"https://github.com/allanyung/xpdelve/releases/tag/v{version}"}}"#
        )
        .into_bytes()
    }

    #[test]
    fn newer_compares_semantic_versions() {
        assert!(is_newer("0.8.0", "0.9.0"));
        assert!(is_newer("0.9.0", "0.10.0"));
        assert!(is_newer("1.0.0-rc.1", "1.0.0"));
        assert!(!is_newer("0.8.0", "0.8.0"));
        assert!(!is_newer("0.9.0", "0.8.0"));
        assert!(!is_newer("0.8.0", "nightly"));
        assert!(!is_newer("garbage", "0.9.0"));
    }

    #[test]
    fn release_is_newer_uses_the_running_version() {
        assert!(!release(env!("CARGO_PKG_VERSION")).is_newer());
        assert!(release("999.0.0").is_newer());
    }

    #[test]
    fn parse_release_strips_the_tag_prefix() {
        assert_eq!(parse_release(&body("0.9.0")).unwrap(), release("0.9.0"));
    }

    #[test]
    fn parse_release_rejects_a_tag_that_is_not_a_version() {
        let body = br#"{"tag_name":"nightly","html_url":"https://github.com/x"}"#;
        assert!(parse_release(body).is_err());
        assert!(parse_release(b"not json").is_err());
    }

    #[test]
    fn parse_release_ignores_a_notes_link_off_github() {
        let body = br#"{"tag_name":"v1.0.0","html_url":"https://example.com/evil"}"#;
        assert_eq!(
            parse_release(body).unwrap().url,
            "https://github.com/allanyung/xpdelve/releases/tag/v1.0.0"
        );
    }

    #[tokio::test]
    async fn checks_reach_the_network_once_a_day() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state/update-check.toml");
        let calls = Cell::new(0);
        let fetch = |version: &'static str| {
            let calls = &calls;
            move || async move {
                calls.set(calls.get() + 1);
                Ok(body(version))
            }
        };
        let first = check_at(&path, 1_000, fetch("0.9.0")).await.unwrap();
        assert_eq!(first, release("0.9.0"));
        assert_eq!(cached(&path), Some(release("0.9.0")));
        let again = check_at(&path, 1_000 + 3_600, fetch("0.10.0"))
            .await
            .unwrap();
        assert_eq!(again, release("0.9.0"));
        assert_eq!(calls.get(), 1);
        let later = check_at(&path, 1_000 + 86_400, fetch("0.10.0"))
            .await
            .unwrap();
        assert_eq!(later, release("0.10.0"));
        assert_eq!(calls.get(), 2);
    }

    #[tokio::test]
    async fn a_failed_check_waits_a_day_and_keeps_the_known_release() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("update-check.toml");
        check_at(&path, 0, || async { Ok(body("0.9.0")) })
            .await
            .unwrap();
        let calls = Cell::new(0);
        let failing = || async {
            calls.set(calls.get() + 1);
            Err::<Vec<u8>, _>("offline".to_string())
        };
        let error = check_at(&path, 86_400, failing).await.unwrap_err();
        assert_eq!(error, "offline");
        assert_eq!(cached(&path), Some(release("0.9.0")));
        let skipped = check_at(&path, 86_400 + 60, failing).await.unwrap();
        assert_eq!(skipped, release("0.9.0"));
        assert_eq!(calls.get(), 1);
    }

    #[tokio::test]
    async fn a_failed_first_check_is_not_retried_within_a_day() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("update-check.toml");
        let calls = Cell::new(0);
        let failing = || async {
            calls.set(calls.get() + 1);
            Err::<Vec<u8>, _>("offline".to_string())
        };
        assert!(check_at(&path, 10, failing).await.is_err());
        assert!(check_at(&path, 20, failing).await.is_err());
        assert_eq!(calls.get(), 1);
        assert_eq!(cached(&path), None);
    }

    #[test]
    fn unreadable_cache_is_no_cache() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("update-check.toml");
        assert_eq!(cached(&path), None);
        std::fs::write(&path, "garbage = [").unwrap();
        assert_eq!(cached(&path), None);
        assert!(read_cache(&path).is_none());
    }

    #[test]
    fn a_cache_from_the_future_is_stale() {
        let cache = Cache {
            checked_at: 5_000,
            release: None,
        };
        assert!(!is_fresh(&cache, 1_000));
        assert!(is_fresh(&cache, 5_001));
    }
}
