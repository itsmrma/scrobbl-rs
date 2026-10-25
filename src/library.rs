//! Owner annotations and persistent local artwork, separate from source metadata.
use crate::{
    AppState, auth, db,
    error::{AppError, Result},
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::{
    collections::BTreeSet,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

// CTEs overlay corrections without altering source metadata or executable schema.
// Readers of a frozen archive use the same overlays as the live consultation.
pub const METADATA: &str = include_str!("library-metadata.sql");
const IMAGE_LIMIT: usize = 5 * 1024 * 1024;
const DAY: i64 = 86_400_000;

fn entity(kind: &str) -> Result<(&'static str, &'static str, &'static str)> {
    match kind {
        "artists" => Ok(("artists", "artist", "name")),
        "albums" => Ok(("albums", "album", "name")),
        "tracks" => Ok(("tracks", "track", "title")),
        _ => Err(AppError::BadRequest(
            "Choose artists, albums or tracks.".into(),
        )),
    }
}
async fn exists(state: &AppState, kind: &str, id: &str) -> Result<()> {
    let (table, _, _) = entity(kind)?;
    let yes: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "{METADATA} SELECT EXISTS(SELECT 1 FROM {table} WHERE id=?)"
    )))
    .bind(id)
    .fetch_one(&state.read_db)
    .await?;
    if yes { Ok(()) } else { Err(AppError::NotFound) }
}
pub(crate) async fn writable(state: &AppState) -> Result<()> {
    if state.storage.directory.join("recovery.pending").exists()
        || crate::recovery::paused(&state.storage).await
        || sqlx::query_scalar::<_, bool>("SELECT maintenance FROM archive_state WHERE singleton=1")
            .fetch_one(&state.read_db)
            .await?
    {
        return Err(AppError::Conflict(
            "Archive recovery is in progress. Try again after recovery.".into(),
        ));
    }
    Ok(())
}
#[derive(Serialize, ts_rs::TS)]
pub struct Details {
    pub name: String,
    pub original_name: String,
    pub rating: Option<f64>,
    pub custom_image: bool,
    pub spotify_url: Option<String>,
    pub image_credit: Option<crate::artwork_sources::ImageCredit>,
}
#[derive(Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub name: Option<String>,
    pub rating: Option<f64>,
}
pub async fn details(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Json<Details>> {
    auth::session(&state, &headers).await?;
    let id =
        crate::name_groups::canonical(&mut *state.read_db.acquire().await?, &kind, &id).await?;
    let (_, singular, _) = entity(&kind)?;
    let mut tx = state.read_db.begin().await?;
    let original_sql = if kind == "tracks" {
        "SELECT title FROM main.tracks WHERE id=?"
    } else if kind == "artists" {
        "SELECT a.name FROM name_groups g JOIN main.artists a ON a.id=g.representative_id WHERE g.id=?"
    } else {
        "SELECT a.name FROM name_groups g JOIN main.albums a ON a.id=g.representative_id WHERE g.id=?"
    };
    let original_name: String = sqlx::query_scalar(original_sql)
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    let name: Option<String> = sqlx::query_scalar("SELECT json_extract(value_json,'$') FROM local_corrections WHERE entity_kind=? AND entity_id=? AND field='name'").bind(singular).bind(&id).fetch_optional(&mut *tx).await?;
    let rating: Option<f64> = sqlx::query_scalar("SELECT json_extract(value_json,'$') FROM local_corrections WHERE entity_kind=? AND entity_id=? AND field='rating'").bind(singular).bind(&id).fetch_optional(&mut *tx).await?;
    let custom_image: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM artwork WHERE kind=? AND entity_id=? AND source='custom')",
    )
    .bind(&kind)
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    let external: Option<String> = sqlx::query_scalar("SELECT external_id FROM entity_links WHERE kind=? AND (entity_id=? OR entity_id IN (SELECT entity_id FROM name_group_members WHERE group_id=?)) AND namespace='spotify' ORDER BY external_id LIMIT 1").bind(singular).bind(&id).bind(&id).fetch_optional(&mut *tx).await?;
    let spotify_url = external.and_then(|s| {
        s.strip_prefix(&format!("spotify:{singular}:"))
            .filter(|id| valid_spotify_id(id))
            .map(|id| format!("https://open.spotify.com/{singular}/{id}"))
    });
    Ok(Json(Details {
        name: name.unwrap_or_else(|| original_name.clone()),
        original_name,
        rating,
        custom_image,
        spotify_url,
        image_credit: crate::artwork_sources::credit(&state, &kind, &id).await?,
    }))
}
pub async fn edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
    Json(form): Json<Edit>,
) -> Result<Json<serde_json::Value>> {
    auth::require_write(&state, &headers).await?;
    let id =
        crate::name_groups::canonical(&mut *state.read_db.acquire().await?, &kind, &id).await?;
    writable(&state).await?;
    exists(&state, &kind, &id).await?;
    let (_, singular, _) = entity(&kind)?;
    if form
        .name
        .as_ref()
        .is_some_and(|n| n.trim().is_empty() || n.chars().count() > 300)
    {
        return Err(AppError::BadRequest(
            "Use a name between 1 and 300 characters.".into(),
        ));
    }
    if form
        .rating
        .is_some_and(|v| !v.is_finite() || !(0.5..=5.0).contains(&v) || (v * 2.0).fract() != 0.0)
    {
        return Err(AppError::BadRequest(
            "Choose 0.5 to 5 stars, in half-star steps, or clear the rating.".into(),
        ));
    }
    let mut tx = state.db.begin().await?;
    for (field, value) in [
        (
            "name",
            form.name.map(|s| serde_json::json!(s.trim()).to_string()),
        ),
        (
            "rating",
            form.rating.map(|r| serde_json::json!(r).to_string()),
        ),
    ] {
        if let Some(value) = value {
            sqlx::query("INSERT INTO local_corrections VALUES(?,?,?,?) ON CONFLICT(entity_kind,entity_id,field) DO UPDATE SET value_json=excluded.value_json").bind(singular).bind(&id).bind(field).bind(value).execute(&mut *tx).await?;
        } else {
            sqlx::query(
                "DELETE FROM local_corrections WHERE entity_kind=? AND entity_id=? AND field=?",
            )
            .bind(singular)
            .bind(&id)
            .bind(field)
            .execute(&mut *tx)
            .await?;
        }
    }
    let affected_sql = match kind.as_str() {
        "artists" => {
            "SELECT id FROM listens WHERE track_id IN (SELECT track_id FROM track_artists WHERE artist_id IN (SELECT entity_id FROM name_group_members WHERE group_id=?))"
        }
        "albums" => {
            "SELECT id FROM listens WHERE track_id IN (SELECT id FROM tracks WHERE album_id IN (SELECT entity_id FROM name_group_members WHERE group_id=?))"
        }
        _ => "SELECT id FROM listens WHERE track_id=?",
    };
    let ids: BTreeSet<String> = sqlx::query_scalar(affected_sql)
        .bind(&id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
    db::publish(&mut tx, &ids).await?;
    tx.commit().await?;
    state.dashboard_cache.lock().await.clear();
    Ok(Json(serde_json::json!({"saved":true})))
}

fn valid_spotify_id(id: &str) -> bool {
    id.len() == 22 && id.bytes().all(|c| c.is_ascii_alphanumeric())
}
pub(crate) fn image_type(bytes: &[u8]) -> Result<&'static str> {
    if bytes.len() > IMAGE_LIMIT {
        return Err(AppError::BadRequest(
            "Images must be 5 MB or smaller.".into(),
        ));
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Ok("image/jpeg")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Ok("image/webp")
    } else {
        Err(AppError::BadRequest(
            "Choose a PNG, JPEG or WebP image. SVG and other file types are not supported.".into(),
        ))
    }
}
fn public_ip(ip: IpAddr) -> bool {
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
                // Only global unicast; exclude documentation, special protocols and 6to4.
                let s = v.segments();
                (s[0] & 0xe000) == 0x2000
                    && s[0] != 0x2002
                    && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
            }),
    }
}
fn image_retry_at(value: Option<&str>, now: i64) -> i64 {
    value
        .and_then(|value| {
            value
                .parse::<i64>()
                .ok()
                .filter(|seconds| *seconds >= 0)
                .map(|seconds| now.saturating_add(seconds.saturating_mul(1000)))
                .or_else(|| {
                    chrono::DateTime::parse_from_rfc2822(value)
                        .ok()
                        .map(|date| date.timestamp_millis())
                })
        })
        .unwrap_or_else(|| now.saturating_add(60_000))
        .max(now.saturating_add(1000))
}
pub(crate) fn archive_https(mut target: url::Url) -> url::Url {
    // CAA still returns legacy HTTP Archive links. Request HTTPS directly;
    // never transmit an HTTP hop or relax public-address validation.
    if target.scheme() == "http"
        && target.host_str().is_some_and(|host| {
            host == "coverartarchive.org" || host == "archive.org" || host.ends_with(".archive.org")
        })
    {
        if target.port() == Some(80) {
            let _ = target.set_port(None);
        }
        let _ = target.set_scheme("https");
    }
    target
}
pub(crate) async fn public_response(
    url: &str,
    state: Option<&AppState>,
) -> Result<reqwest::Response> {
    let mut target = url.to_owned();
    for _ in 0..5 {
        let url = target.as_str();
        let url = url::Url::parse(url)
            .map_err(|_| AppError::BadRequest("Enter a valid public HTTPS image URL.".into()))?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port_or_known_default() != Some(443)
        {
            return Err(AppError::BadRequest(
                "Use a public HTTPS image URL on port 443, without credentials.".into(),
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| AppError::BadRequest("The image URL needs a public hostname.".into()))?;
        if let Some(state) = state {
            crate::artwork_sources::request_slot(state, host).await?;
        }
        let addresses: Vec<SocketAddr> = match tokio::net::lookup_host((host, 443)).await {
            Ok(addresses) => addresses.collect(),
            Err(_) => {
                if let Some(state) = state {
                    return Err(crate::artwork_sources::request_failed(state, host, None).await?);
                }
                return Err(AppError::BadRequest(
                    "Could not resolve the image host.".into(),
                ));
            }
        };
        if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
            return Err(AppError::BadRequest(
                "Image links must point to the public internet, not a local or private address."
                    .into(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|_| AppError::Internal)?;
        let response = client
            .get(url.clone())
            .header(
                header::USER_AGENT,
                "scrobbl.rs/0.1 (https://github.com/itsmrma/scrobbl-rs)",
            )
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(_) => {
                if let Some(state) = state {
                    return Err(crate::artwork_sources::request_failed(state, host, None).await?);
                }
                return Err(AppError::BadRequest(
                    "Could not download the image. Try a direct image link or upload a file."
                        .into(),
                ));
            }
        };
        if (response.status().is_server_error()
            || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS)
            && let Some(state) = state
        {
            let retry = response
                .headers()
                .get(header::RETRY_AFTER)
                .map(|v| image_retry_at(v.to_str().ok(), auth::now_ms()));
            return Err(crate::artwork_sources::request_failed(state, host, retry).await?);
        }
        if let Some(state) = state {
            crate::artwork_sources::request_succeeded(state, host).await?;
        }
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or(AppError::Internal)?;
            target = archive_https(url.join(location).map_err(|_| AppError::Internal)?).to_string();
            continue;
        }
        return Ok(response);
    }
    Err(AppError::BadRequest(
        "Too many image provider redirects.".into(),
    ))
}
pub(crate) async fn download(url: &str, state: Option<&AppState>) -> Result<Vec<u8>> {
    let mut response = public_response(url, state).await?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|v| v > IMAGE_LIMIT as u64)
    {
        return Err(AppError::BadRequest("The image host did not return an image within the 5 MB limit. Use a direct link or upload a file.".into()));
    }
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AppError::BadRequest("Image download was interrupted. Please retry.".into()))?
    {
        if data.len() + chunk.len() > IMAGE_LIMIT {
            return Err(AppError::BadRequest(
                "Images must be 5 MB or smaller.".into(),
            ));
        }
        data.extend_from_slice(&chunk);
    }
    image_type(&data)?;
    Ok(data)
}
async fn save_image(
    state: &AppState,
    kind: &str,
    id: &str,
    source: &str,
    data: Option<&[u8]>,
) -> Result<()> {
    writable(state).await?;
    let mime = data.map(image_type).transpose()?;
    let now = auth::now_ms();
    // Successful downloads are part of the local library. Only a confirmed
    // absence expires, so the provider can be checked again later.
    let expires = (source == "spotify" && data.is_none()).then_some(now + DAY);
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision").fetch_optional(&mut *tx).await?.ok_or_else(||AppError::Conflict("Archive is in maintenance.".into()))?;
    sqlx::query(
        "DELETE FROM artwork WHERE source='spotify' AND bytes IS NULL AND expires_at_ms<=?",
    )
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms) VALUES(?,?,?,?,?,?,?) ON CONFLICT(kind,entity_id,source) DO UPDATE SET bytes=excluded.bytes,mime=excluded.mime,checked_at_ms=excluded.checked_at_ms,expires_at_ms=excluded.expires_at_ms").bind(kind).bind(id).bind(source).bind(data).bind(mime).bind(now).bind(expires).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn upload_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
    data: Bytes,
) -> Result<Json<serde_json::Value>> {
    auth::require_write(&state, &headers).await?;
    let id =
        crate::name_groups::canonical(&mut *state.read_db.acquire().await?, &kind, &id).await?;
    if kind != "artists" && kind != "albums" {
        return Err(AppError::BadRequest(
            "Custom images are available for artists and albums.".into(),
        ));
    }
    exists(&state, &kind, &id).await?;
    save_image(&state, &kind, &id, "custom", Some(&data)).await?;
    Ok(Json(serde_json::json!({"saved":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageUrl {
    pub url: Option<String>,
}
pub async fn image_url(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
    Json(form): Json<ImageUrl>,
) -> Result<Json<serde_json::Value>> {
    auth::require_write(&state, &headers).await?;
    let id =
        crate::name_groups::canonical(&mut *state.read_db.acquire().await?, &kind, &id).await?;
    writable(&state).await?;
    if kind != "artists" && kind != "albums" {
        return Err(AppError::BadRequest(
            "Custom images are available for artists and albums.".into(),
        ));
    }
    exists(&state, &kind, &id).await?;
    if let Some(url) = form.url {
        let bytes = download(&url, None).await?;
        save_image(&state, &kind, &id, "custom", Some(&bytes)).await?;
    } else {
        sqlx::query("DELETE FROM artwork WHERE kind=? AND entity_id=? AND source='custom'")
            .bind(&kind)
            .bind(&id)
            .execute(&state.db)
            .await?;
    }
    Ok(Json(serde_json::json!({"saved":true})))
}
pub(crate) async fn spotify_image(
    state: &AppState,
    kind: &str,
    id: &str,
) -> Result<Option<(String, String)>> {
    let singular = if kind == "artists" { "artist" } else { "album" };
    let external:Option<String>=sqlx::query_scalar("SELECT external_id FROM entity_links INDEXED BY entity_links_entity WHERE entity_id IN (SELECT entity_id FROM name_group_members WHERE group_id=?) AND kind=? AND namespace='spotify' ORDER BY external_id LIMIT 1").bind(id).bind(singular).fetch_optional(&state.read_db).await?;
    let external = external.and_then(|s| {
        s.strip_prefix(&format!("spotify:{singular}:"))
            .filter(|v| valid_spotify_id(v))
            .map(str::to_owned)
    });
    let image = if let Some(external) = external {
        crate::spotify::api_get(state, &format!("/{kind}/{external}"), &[]).await?
    } else {
        // Extended exports have track IDs, but do not contain album/artist IDs.
        let query = if kind == "albums" {
            "SELECT e.external_id FROM entity_links e INDEXED BY entity_links_entity JOIN tracks t ON t.id=e.entity_id WHERE e.kind='track' AND e.namespace='spotify' AND t.album_id IN (SELECT entity_id FROM name_group_members WHERE group_id=?) ORDER BY e.external_id LIMIT 1"
        } else {
            "SELECT e.external_id FROM entity_links e INDEXED BY entity_links_entity JOIN track_artists ta ON ta.track_id=e.entity_id WHERE e.kind='track' AND e.namespace='spotify' AND ta.artist_id IN (SELECT entity_id FROM name_group_members WHERE group_id=?) ORDER BY e.external_id LIMIT 1"
        };
        let track: Option<String> = sqlx::query_scalar(query)
            .bind(id)
            .fetch_optional(&state.read_db)
            .await?;
        let Some(track) = track.and_then(|s| {
            s.strip_prefix("spotify:track:")
                .filter(|v| valid_spotify_id(v))
                .map(str::to_owned)
        }) else {
            return Ok(None);
        };
        let track = crate::spotify::api_get(state, &format!("/tracks/{track}"), &[]).await?;
        if kind == "albums" {
            track["album"].clone()
        } else {
            let name: String = sqlx::query_scalar("SELECT a.name FROM main.artists a JOIN name_groups g ON g.representative_id=a.id WHERE g.id=?")
                .bind(id)
                .fetch_one(&state.read_db)
                .await?;
            let candidates = track["artists"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter(|a| a["name"].as_str() == Some(&name))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if candidates.len() != 1 {
                return Ok(None);
            }
            let Some(artist) = candidates[0]["id"].as_str().filter(|v| valid_spotify_id(v)) else {
                return Ok(None);
            };
            crate::spotify::api_get(state, &format!("/artists/{artist}"), &[]).await?
        }
    };
    let Some(external) = image["id"].as_str().filter(|v| valid_spotify_id(v)) else {
        return Ok(None);
    };
    Ok(crate::spotify::best_image(&image["images"]).map(|url| {
        (
            url,
            format!("https://open.spotify.com/{singular}/{external}"),
        )
    }))
}
#[derive(Default, Deserialize)]
pub struct ArtworkQuery {
    #[serde(default)]
    pub cached: bool,
}
pub async fn artwork_credit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Json<Option<crate::artwork_sources::ImageCredit>>> {
    auth::session(&state, &headers).await?;
    if kind != "artists" && kind != "albums" {
        return Err(AppError::NotFound);
    }
    let id =
        crate::name_groups::canonical(&mut *state.read_db.acquire().await?, &kind, &id).await?;
    Ok(Json(
        crate::artwork_sources::credit(&state, &kind, &id).await?,
    ))
}
pub async fn artwork(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
    Query(query): Query<ArtworkQuery>,
) -> Result<Response> {
    auth::session(&state, &headers).await?;
    let id =
        crate::name_groups::canonical(&mut *state.read_db.acquire().await?, &kind, &id).await?;
    if kind != "artists" && kind != "albums" {
        return Err(AppError::NotFound);
    }
    exists(&state, &kind, &id).await?;
    let unavailable = || {
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            [
                (header::CACHE_CONTROL, "no-store"),
                (header::RETRY_AFTER, "60"),
            ],
        )
            .into_response()
    };
    let cached=sqlx::query("SELECT bytes,mime FROM artwork WHERE kind=? AND entity_id=? AND (source='custom' OR expires_at_ms IS NULL OR expires_at_ms>?) ORDER BY CASE source WHEN 'custom' THEN 0 ELSE 1 END LIMIT 1").bind(&kind).bind(&id).bind(auth::now_ms()).fetch_optional(&state.read_db).await?;
    if let Some(row) = cached {
        let bytes: Option<Vec<u8>> = row.get("bytes");
        let bytes = bytes.ok_or(AppError::NotFound)?;
        // A restored database is untrusted input too: derive the response type
        // from the image bytes, never from an editable MIME string in SQLite.
        return Ok(([(header::CONTENT_TYPE, image_type(&bytes)?)], bytes).into_response());
    }
    // A fast cache lookup must never wait for Spotify or a cold download.
    if query.cached {
        return Ok((
            axum::http::StatusCode::NOT_FOUND,
            [(header::CACHE_CONTROL, "no-store")],
        )
            .into_response());
    }
    match ensure_artwork(&state, &kind, &id).await {
        Ok(ArtworkFetch::Busy) => Ok(unavailable()),
        Ok(ArtworkFetch::Missing) | Err(AppError::Unauthorized) => Err(AppError::NotFound),
        Ok(ArtworkFetch::Saved | ArtworkFetch::Cached) => {
            let bytes: Vec<u8> = sqlx::query_scalar("SELECT bytes FROM artwork WHERE kind=? AND entity_id=? AND bytes IS NOT NULL ORDER BY CASE source WHEN 'custom' THEN 0 ELSE 1 END LIMIT 1").bind(&kind).bind(&id).fetch_one(&state.read_db).await?;
            Ok(([(header::CONTENT_TYPE, image_type(&bytes)?)], bytes).into_response())
        }
        Err(_) => Ok(unavailable()),
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum ArtworkFetch {
    Cached,
    Saved,
    Missing,
    Busy,
}

pub(crate) async fn ensure_artwork(state: &AppState, kind: &str, id: &str) -> Result<ArtworkFetch> {
    ensure_artwork_with(
        state,
        kind,
        id,
        &crate::artwork_sources::PublicSources(state),
    )
    .await
}

pub(crate) async fn ensure_artwork_with(
    state: &AppState,
    kind: &str,
    id: &str,
    sources: &impl crate::artwork_sources::ImageLookup,
) -> Result<ArtworkFetch> {
    ensure_artwork_with_activity(state, kind, id, sources, None).await
}

pub(crate) async fn ensure_artwork_for_activity(
    state: &AppState,
    kind: &str,
    id: &str,
    sources: &impl crate::artwork_sources::ImageLookup,
    activity_id: &str,
    member: &str,
) -> Result<ArtworkFetch> {
    ensure_artwork_with_activity(state, kind, id, sources, Some((activity_id, member))).await
}

async fn ensure_artwork_with_activity(
    state: &AppState,
    kind: &str,
    id: &str,
    sources: &impl crate::artwork_sources::ImageLookup,
    activity: Option<(&str, &str)>,
) -> Result<ArtworkFetch> {
    let cached: Option<bool> = sqlx::query_scalar("SELECT bytes IS NOT NULL FROM artwork WHERE kind=? AND entity_id=? AND (source='custom' OR expires_at_ms IS NULL OR expires_at_ms>?) ORDER BY CASE source WHEN 'custom' THEN 0 ELSE 1 END LIMIT 1").bind(kind).bind(id).bind(auth::now_ms()).fetch_optional(&state.read_db).await?;
    if let Some(saved) = cached {
        return Ok(if saved {
            ArtworkFetch::Cached
        } else {
            ArtworkFetch::Missing
        });
    }
    let entity_gate = {
        let mut locks = state.artwork_gate.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        let key = format!("{kind}:{id}");
        if let Some(lock) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
            lock
        } else {
            let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(key, std::sync::Arc::downgrade(&lock));
            lock
        }
    };
    let Ok(_gate) = entity_gate.try_lock() else {
        return Ok(ArtworkFetch::Busy);
    };
    // A browser/worker may have completed while this request acquired its lock.
    let cached: Option<bool> = sqlx::query_scalar("SELECT bytes IS NOT NULL FROM artwork WHERE kind=? AND entity_id=? AND (source='custom' OR expires_at_ms IS NULL OR expires_at_ms>?) ORDER BY CASE source WHEN 'custom' THEN 0 ELSE 1 END LIMIT 1").bind(kind).bind(id).bind(auth::now_ms()).fetch_optional(&state.read_db).await?;
    if let Some(saved) = cached {
        return Ok(if saved {
            ArtworkFetch::Cached
        } else {
            ArtworkFetch::Missing
        });
    }
    writable(state).await?;
    let settings = crate::artwork_settings::read(&mut *state.read_db.acquire().await?).await?;
    let missing: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artwork_misses WHERE kind=? AND entity_id=? AND settings_version=? AND expires_at_ms>?)").bind(kind).bind(id).bind(settings.version).bind(auth::now_ms()).fetch_one(&state.read_db).await?;
    if missing {
        return Ok(ArtworkFetch::Missing);
    }
    let Ok(_capacity) = state.artwork_fetches.try_acquire() else {
        return Ok(ArtworkFetch::Busy);
    };
    crate::artwork_sources::fetch_with_activity(state, &settings, kind, id, sources, activity).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cold_artwork_does_not_queue_http_requests_behind_provider_downloads() {
        let directory = tempfile::tempdir().unwrap();
        let storage = crate::storage::open(directory.path()).await.unwrap();
        let config = crate::config::Config {
            port: 3210,
            bind_ip: "127.0.0.1".parse().unwrap(),
            external_origin: None,
            data_dir: directory.path().into(),
            sync_seconds: 120,
            now_playing_seconds: 30,
            import_limits: Default::default(),
            api_snapshot_limits: Default::default(),
            env_client_id: None,
            env_client_secret: None,
        };
        let state = AppState::new(storage, config);
        let token = auth::random_secret();
        sqlx::query("INSERT INTO sessions VALUES(?,'csrf',9999999999999)")
            .bind(auth::digest(&token))
            .execute(&state.private_db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO spotify_tokens VALUES(1,'unused','unused',9999999999999)")
            .execute(&state.private_db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO artists(id,name,metadata_rank) VALUES('source','Artist',1)")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO name_groups VALUES('group','artists','artist','source')")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO name_group_members VALUES('artists','source','group','group')")
            .execute(&state.db)
            .await
            .unwrap();
        let id: String =
            sqlx::query_scalar("SELECT group_id FROM name_group_members WHERE entity_id='source'")
                .fetch_one(&state.read_db)
                .await
                .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            format!("scrobbl_session={token}").parse().unwrap(),
        );
        // Simulate a slow download occupying the single optional catalogue slot.
        let _download = state.artwork_fetches.acquire().await.unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            artwork(
                State(state.clone()),
                headers.clone(),
                Path(("artists".into(), id.clone())),
                Query(ArtworkQuery::default()),
            ),
        )
        .await;
        assert!(
            result.is_ok(),
            "queued artwork holds browser HTTP connections while waiting for the provider"
        );
        assert_eq!(
            result.unwrap().unwrap().status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        let cached: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artwork")
            .fetch_one(&state.read_db)
            .await
            .unwrap();
        assert_eq!(cached, 0, "a busy provider must not cache a missing image");
        save_image(&state, "artists", &id, "custom", Some(b"\x89PNG\r\n\x1a\n"))
            .await
            .unwrap();
        let cached = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            artwork(
                State(state.clone()),
                headers,
                Path(("artists".into(), id)),
                Query(ArtworkQuery::default()),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(cached.status(), axum::http::StatusCode::OK);
    }
    #[test]
    fn image_retry_after_accepts_seconds_dates_and_a_conservative_fallback() {
        let now = 1_000_000;
        assert_eq!(image_retry_at(Some("1200"), now), now + 1_200_000);
        assert_eq!(
            image_retry_at(Some("Thu, 01 Jan 1970 01:00:00 GMT"), now),
            3_600_000
        );
        assert_eq!(image_retry_at(Some("invalid"), now), now + 60_000);
        assert_eq!(image_retry_at(Some("-1"), now), now + 60_000);
    }
    #[test]
    fn image_url_blocks_private_special_and_mapped_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.0.1",
            "169.254.169.254",
            "100.64.1.2",
            "198.18.0.1",
            "224.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "2002:7f00:1::",
            "2001:db8::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }
}
