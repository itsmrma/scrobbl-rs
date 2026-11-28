//! One pooled HTTPS client for every public image request.
//!
//! The previous design built a new client (and a new TLS handshake) for every
//! request, serialized all lookups behind one global slot and wrote pacing
//! state to SQLite before each call. Here:
//!
//! - connections are pooled and reused across providers;
//! - a custom DNS resolver refuses private, loopback and special addresses on
//!   every connection, including redirects, so user-supplied URLs stay safe;
//! - politeness is per host and in memory: MusicBrainz keeps its documented
//!   one request per second, fast APIs such as Deezer are barely paced, and a
//!   Retry-After from one host never blocks another.
use crate::error::{AppError, Result};
use reqwest::{
    Client, StatusCode,
    dns::{Addrs, Name, Resolve, Resolving},
    header,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

pub(crate) const IMAGE_LIMIT: usize = 5 * 1024 * 1024;
const JSON_LIMIT: usize = 2 * 1024 * 1024;
const USER_AGENT: &str = concat!(
    "scrobbl.rs/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/itsmrma/scrobbl-rs)"
);

/// Only global unicast addresses: no private, loopback, link-local, CGNAT,
/// documentation or benchmarking ranges, for IPv4 and IPv6.
pub(crate) fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let [a, b, c, _] = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_broadcast()
                || v.is_documentation()
                || a == 0
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(v) => v
            .to_ipv4_mapped()
            .map(|v| public_ip(IpAddr::V4(v)))
            .unwrap_or_else(|| {
                let s = v.segments();
                (s[0] & 0xe000) == 0x2000
                    && s[0] != 0x2002
                    && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
            }),
    }
}

#[derive(Debug)]
struct NonPublicAddress;

impl std::fmt::Display for NonPublicAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("image hosts must resolve to public addresses only")
    }
}

impl std::error::Error for NonPublicAddress {}

fn has_non_public_source(mut err: &(dyn std::error::Error + 'static)) -> bool {
    loop {
        if err.is::<NonPublicAddress>() {
            return true;
        }
        let Some(next) = err.source() else {
            return false;
        };
        err = next;
    }
}

struct PublicResolver;

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 443))
                .await?
                .collect();
            if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
                let err: Box<dyn std::error::Error + Send + Sync> = Box::new(NonPublicAddress);
                return Err(err);
            }
            let addresses: Addrs = Box::new(addresses.into_iter());
            Ok(addresses)
        })
    }
}

static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .user_agent(USER_AGENT)
        .https_only(true)
        .no_proxy()
        .dns_resolver(Arc::new(PublicResolver))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .pool_idle_timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many redirects")
            } else if attempt.url().scheme() != "https" {
                // Cover Art Archive still links some files over HTTP.
                let mut url = attempt.url().clone();
                if url.set_scheme("https").is_ok()
                    && url
                        .host_str()
                        .is_some_and(|h| h == "archive.org" || h.ends_with(".archive.org"))
                {
                    attempt.error(UpgradeTo(url))
                } else {
                    attempt.stop()
                }
            } else {
                attempt.follow()
            }
        }))
        .build()
        .expect("HTTP client initialization failed")
});

/// Redirect target that must be retried over HTTPS.
#[derive(Debug)]
struct UpgradeTo(url::Url);
impl std::fmt::Display for UpgradeTo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upgrade to {}", self.0)
    }
}
impl std::error::Error for UpgradeTo {}

/// Minimum spacing between requests to one host. Unlisted hosts (image CDNs)
/// are not paced: they serve static files.
fn pace(host: &str) -> Duration {
    match host {
        // https://musicbrainz.org/doc/MusicBrainz_API/Rate_Limiting
        "musicbrainz.org" => Duration::from_millis(1_100),
        // Deezer allows 50 requests per 5 seconds.
        "api.deezer.com" => Duration::from_millis(120),
        // Apple documents roughly 20 searches per minute.
        "itunes.apple.com" => Duration::from_millis(3_000),
        "coverartarchive.org" | "www.wikidata.org" | "commons.wikimedia.org" => {
            Duration::from_millis(200)
        }
        _ => Duration::ZERO,
    }
}

#[derive(Default)]
struct HostState {
    next: Option<Instant>,
    cooldown_until: Option<Instant>,
    failures: u32,
}

static HOSTS: LazyLock<Mutex<HashMap<String, HostState>>> = LazyLock::new(Default::default);

fn wall_ms(at: Instant) -> i64 {
    crate::auth::now_ms()
        .saturating_add(at.saturating_duration_since(Instant::now()).as_millis() as i64)
}

/// Wait for this host's turn, or fail fast with the host's cooldown.
async fn slot(host: &str) -> Result<()> {
    let wait = {
        let mut hosts = HOSTS.lock().await;
        let state = hosts.entry(host.to_owned()).or_default();
        let now = Instant::now();
        if let Some(until) = state.cooldown_until.filter(|until| *until > now) {
            return Err(AppError::ArtworkWait {
                retry_at_ms: wall_ms(until),
            });
        }
        let at = state.next.filter(|next| *next > now).unwrap_or(now);
        state.next = Some(at + pace(host));
        at.saturating_duration_since(now)
    };
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
    Ok(())
}

async fn failed(host: &str, retry_after: Option<Duration>) -> AppError {
    let mut hosts = HOSTS.lock().await;
    let state = hosts.entry(host.to_owned()).or_default();
    state.failures = state.failures.saturating_add(1);
    // Short, growing pauses: 5 s, 10 s, 20 s … capped at 10 minutes. A real
    // Retry-After wins, capped at one hour so one bad header cannot stall a
    // provider for a day.
    let backoff = Duration::from_secs(5 * (1 << state.failures.min(7).saturating_sub(1)))
        .min(Duration::from_secs(600));
    let pause = retry_after
        .map(|wait| wait.min(Duration::from_secs(3_600)))
        .unwrap_or(backoff);
    let until = Instant::now() + pause;
    state.cooldown_until = Some(state.cooldown_until.map_or(until, |prev| prev.max(until)));
    AppError::ArtworkWait {
        retry_at_ms: wall_ms(until),
    }
}

async fn succeeded(host: &str) {
    if let Some(state) = HOSTS.lock().await.get_mut(host) {
        state.failures = 0;
        state.cooldown_until = None;
    }
}

#[cfg(test)]
pub(crate) async fn reset_hosts() {
    HOSTS.lock().await.clear();
}

fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    let value = response.headers().get(header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds.max(1)));
    }
    let at = chrono::DateTime::parse_from_rfc2822(value.trim()).ok()?;
    let ms = at.timestamp_millis().saturating_sub(crate::auth::now_ms());
    Some(Duration::from_millis(ms.max(1_000) as u64))
}

/// Outcome of one public GET.
pub(crate) enum Fetched {
    Body(Vec<u8>),
    /// 404/410: the provider has nothing here. Not an error.
    Missing,
}

fn host_of(url: &url::Url) -> Result<String> {
    url.host_str()
        .map(str::to_owned)
        .ok_or_else(|| AppError::BadRequest("The image URL needs a public hostname.".into()))
}

/// GET a public HTTPS resource with the host's pace. Server errors, 429 and
/// network failures become a host cooldown (`ArtworkWait`); 404 is `Missing`.
pub(crate) async fn get(target: &str, limit: usize) -> Result<Fetched> {
    let mut url = url::Url::parse(target)
        .map_err(|_| AppError::BadRequest("Enter a valid public HTTPS image URL.".into()))?;
    for _ in 0..2 {
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port_or_known_default() != Some(443)
        {
            return Err(AppError::BadRequest(
                "Use a public HTTPS image URL on port 443, without credentials.".into(),
            ));
        }
        match url.host() {
            Some(url::Host::Ipv4(ip)) if !public_ip(IpAddr::V4(ip)) => {
                return Err(AppError::BadRequest(
                    "Image links must point to the public internet, not a local or private address."
                        .into(),
                ));
            }
            Some(url::Host::Ipv6(ip)) if !public_ip(IpAddr::V6(ip)) => {
                return Err(AppError::BadRequest(
                    "Image links must point to the public internet, not a local or private address."
                        .into(),
                ));
            }
            _ => {}
        }
        let host = host_of(&url)?;
        slot(&host).await?;
        let response = match CLIENT.get(url.clone()).send().await {
            Ok(response) => response,
            Err(error) => {
                if has_non_public_source(&error) {
                    return Err(AppError::BadRequest(
                        "Image links must point to the public internet, not a local or private address."
                            .into(),
                    ));
                }
                if let Some(upgrade) = std::error::Error::source(&error)
                    .and_then(|source| source.downcast_ref::<UpgradeTo>())
                {
                    url = upgrade.0.clone();
                    continue;
                }
                if error.is_builder() || error.is_redirect() {
                    return Err(AppError::BadRequest(
                        "The image link redirects somewhere that cannot be fetched safely.".into(),
                    ));
                }
                return Err(failed(&host, None).await);
            }
        };
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            let wait = retry_after(&response);
            return Err(failed(&host, wait).await);
        }
        succeeded(&host).await;
        if matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) {
            return Ok(Fetched::Missing);
        }
        if !status.is_success() {
            return Err(AppError::BadRequest(format!(
                "The image host answered {}.",
                status.as_u16()
            )));
        }
        if response.content_length().is_some_and(|v| v > limit as u64) {
            return Err(AppError::BadRequest(
                "The response is larger than the allowed size.".into(),
            ));
        }
        let mut response = response;
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            AppError::BadRequest("The download was interrupted. Please retry.".into())
        })? {
            if data.len() + chunk.len() > limit {
                return Err(AppError::BadRequest(
                    "The response is larger than the allowed size.".into(),
                ));
            }
            data.extend_from_slice(&chunk);
        }
        return Ok(Fetched::Body(data));
    }
    Err(AppError::BadRequest("Too many image redirects.".into()))
}

/// GET JSON. `None` when the resource does not exist.
pub(crate) async fn json(target: &str) -> Result<Option<Value>> {
    match get(target, JSON_LIMIT).await? {
        Fetched::Missing => Ok(None),
        Fetched::Body(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|_| {
            AppError::BadRequest("Image provider returned an invalid response.".into())
        }),
    }
}

/// GET JSON from GitHub API using the SSRF-safe public HTTPS client.
pub(crate) async fn github_json(target: &str) -> Result<Option<Value>> {
    let url = url::Url::parse(target)
        .map_err(|_| AppError::BadRequest("Invalid update check URL.".into()))?;
    if url.scheme() != "https" || url.host_str() != Some("api.github.com") {
        return Err(AppError::BadRequest("Invalid update check host.".into()));
    }
    let response = CLIENT
        .get(url)
        .header(header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| AppError::BadRequest("Could not reach GitHub.".into()))?;
    let status = response.status();
    if matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(AppError::BadRequest(format!(
            "GitHub returned HTTP {}.",
            status.as_u16()
        )));
    }
    if response
        .content_length()
        .is_some_and(|v| v > JSON_LIMIT as u64)
    {
        return Err(AppError::BadRequest("GitHub response too large.".into()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| AppError::BadRequest("Failed reading GitHub response.".into()))?;
    if bytes.len() > JSON_LIMIT {
        return Err(AppError::BadRequest("GitHub response too large.".into()));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| AppError::BadRequest("GitHub returned invalid JSON.".into()))
}

/// Download a PNG, JPEG or WebP image of at most 5 MB.
pub(crate) async fn image(target: &str) -> Result<Option<Vec<u8>>> {
    match get(target, IMAGE_LIMIT).await? {
        Fetched::Missing => Ok(None),
        Fetched::Body(bytes) => {
            crate::library::image_type(&bytes)?;
            Ok(Some(bytes))
        }
    }
}

/// Mark a provider host as cooling down after an application-level error,
/// for example Deezer's `{"error":{"code":4}}` quota answer.
pub(crate) async fn cool_down(host: &str, wait: Duration) -> AppError {
    failed(host, Some(wait)).await
}

pub(crate) fn build_url(base: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = url::Url::parse(base).expect("constant provider URL");
    url.query_pairs_mut().extend_pairs(pairs.iter().copied());
    url.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_special_addresses_are_refused() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fc00::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "2606:4700:4700::1111"] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn a_host_cooldown_does_not_block_other_hosts_and_success_clears_it() {
        reset_hosts().await;
        let error = failed("example-a.test", Some(Duration::from_secs(120))).await;
        assert!(matches!(error, AppError::ArtworkWait { .. }));
        assert!(matches!(
            slot("example-a.test").await,
            Err(AppError::ArtworkWait { .. })
        ));
        assert!(slot("example-b.test").await.is_ok());
        succeeded("example-a.test").await;
        assert!(slot("example-a.test").await.is_ok());
    }

    #[tokio::test]
    async fn non_https_and_credentials_are_rejected_before_any_request() {
        assert!(matches!(
            get("https://127.0.0.1/private.png", 10).await,
            Err(AppError::BadRequest(message)) if message.contains("public internet")
        ));
        for url in [
            "http://example.com/a.png",
            "https://user:pass@example.com/a.png",
            "https://example.com:8443/a.png",
        ] {
            assert!(
                matches!(get(url, 10).await, Err(AppError::BadRequest(_))),
                "{url}"
            );
        }
    }
}
