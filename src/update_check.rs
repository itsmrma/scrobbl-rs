//! Periodic and on-demand application update check against GitHub Releases,
//! plus daily SQLite/memory maintenance.
use crate::{AppState, auth, db, error::Result};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use ts_rs::TS;

const RELEASES_LATEST_URL: &str = "https://api.github.com/repos/itsmrma/scrobbl-rs/releases/latest";
const RELEASE_URL_PREFIX: &str = "https://github.com/itsmrma/scrobbl-rs/";
const CHECK_INTERVAL_MS: i64 = 86_400_000;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct UpdateStatus {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub release_url: Option<String>,
    pub checked_at_ms: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct SemVer<'a> {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<&'a str>,
}

fn parse_semver(raw: &str) -> Option<SemVer<'_>> {
    let trimmed = raw.trim().strip_prefix('v').unwrap_or(raw.trim());
    let (core, pre) = match trimmed.split_once('-') {
        Some((c, p)) if !p.is_empty() => (c, Some(p)),
        Some((_, _)) => return None,
        None => (trimmed, None),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(SemVer {
        major,
        minor,
        patch,
        pre,
    })
}

pub fn is_newer(current: &str, latest: &str) -> bool {
    let (Some(cur), Some(lat)) = (parse_semver(current), parse_semver(latest)) else {
        return false;
    };
    match (lat.major, lat.minor, lat.patch).cmp(&(cur.major, cur.minor, cur.patch)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match (cur.pre, lat.pre) {
            // Stable release is newer than pre-release of the same version.
            (Some(_), None) => true,
            (None, _) => false,
            (Some(c), Some(l)) => l > c,
        },
    }
}

pub async fn read_status(pool: &sqlx::SqlitePool, error: Option<String>) -> Result<UpdateStatus> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let latest_version = db::setting(pool, "update_latest_version")
        .await?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let release_url = db::setting(pool, "update_release_url")
        .await?
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with(RELEASE_URL_PREFIX));
    let checked_at_ms = db::setting(pool, "update_checked_at_ms")
        .await?
        .and_then(|s| s.parse::<i64>().ok());
    let update_available = latest_version
        .as_deref()
        .is_some_and(|latest| is_newer(&current_version, latest));
    Ok(UpdateStatus {
        current_version,
        latest_version,
        update_available,
        release_url,
        checked_at_ms,
        error,
    })
}

pub async fn check_now(state: &AppState) -> Result<UpdateStatus> {
    let now = auth::now_ms();
    match crate::artwork_http::github_json(RELEASES_LATEST_URL).await {
        Ok(Some(json)) => {
            let tag = json["tag_name"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("");
            let normalized_latest = tag.strip_prefix('v').unwrap_or(tag);
            let html_url = json["html_url"]
                .as_str()
                .map(str::trim)
                .filter(|u| u.starts_with(RELEASE_URL_PREFIX))
                .unwrap_or("");
            let mut tx = state.db.begin().await?;
            db::set_setting(&mut tx, "update_latest_version", normalized_latest).await?;
            db::set_setting(&mut tx, "update_release_url", html_url).await?;
            db::set_setting(&mut tx, "update_checked_at_ms", &now.to_string()).await?;
            tx.commit().await?;
            read_status(&state.read_db, None).await
        }
        Ok(None) => {
            let mut tx = state.db.begin().await?;
            db::set_setting(&mut tx, "update_latest_version", "").await?;
            db::set_setting(&mut tx, "update_release_url", "").await?;
            db::set_setting(&mut tx, "update_checked_at_ms", &now.to_string()).await?;
            tx.commit().await?;
            read_status(&state.read_db, None).await
        }
        Err(_) => {
            read_status(
                &state.read_db,
                Some("Could not reach GitHub to check for updates.".to_string()),
            )
            .await
        }
    }
}

pub async fn get(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<UpdateStatus>> {
    auth::session(&state, &headers).await?;
    Ok(Json(read_status(&state.read_db, None).await?))
}

pub async fn check(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<UpdateStatus>> {
    auth::require_write(&state, &headers).await?;
    Ok(Json(check_now(&state).await?))
}

pub async fn worker(state: AppState) {
    tokio::time::sleep(Duration::from_secs(30)).await;
    loop {
        state.storage.maintain_storage().await;
        let disabled = std::env::var("SCROBBL_DISABLE_UPDATE_CHECK")
            .ok()
            .as_deref()
            == Some("1");
        let wait_ms = if disabled {
            CHECK_INTERVAL_MS as u64
        } else {
            let checked = db::setting(&state.read_db, "update_checked_at_ms")
                .await
                .ok()
                .flatten()
                .and_then(|s| s.parse::<i64>().ok());
            let now = auth::now_ms();
            let due = checked
                .map(|ts| ts.saturating_add(CHECK_INTERVAL_MS))
                .unwrap_or(now);
            if now >= due {
                let _ = check_now(&state).await;
                CHECK_INTERVAL_MS as u64
            } else {
                (due - now).max(60_000) as u64
            }
        };
        tokio::time::sleep(Duration::from_millis(wait_ms)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn semver_comparison_handles_stable_and_prerelease_tags() {
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("1.0.0", "v1.1.0"));
        assert!(is_newer("1.2.0", "1.10.0"));
        assert!(is_newer("1.0.0-alpha.1", "1.0.0"));
        assert!(is_newer("1.0.0-alpha.1", "1.0.0-beta.1"));

        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "v1.0.0"));
        assert!(!is_newer("1.1.0", "1.0.9"));
        assert!(!is_newer("1.0.0", "1.0.0-rc.1"));
        assert!(!is_newer("1.0.0", "invalid"));
        assert!(!is_newer("1.0.0", "v1.0"));
    }
}
