//! Owner annotations and bounded artwork retrieval, separate from source metadata.
use crate::{
    AppState, auth, db,
    error::{AppError, Result},
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
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
const PROVIDER_BUDGET: i64 = 128 * 1024 * 1024;
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
        .is_some_and(|v| !v.is_finite() || !(1.0..=5.0).contains(&v) || (v * 2.0).fract() != 0.0)
    {
        return Err(AppError::BadRequest(
            "Choose 1 to 5 stars, in half-star steps, or clear the rating.".into(),
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
fn image_type(bytes: &[u8]) -> Result<&'static str> {
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
async fn download(url: &str) -> Result<Vec<u8>> {
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
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, 443))
        .await
        .map_err(|_| AppError::BadRequest("Could not resolve the image host.".into()))?
        .collect();
    if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
        return Err(AppError::BadRequest(
            "Image links must point to the public internet, not a local or private address.".into(),
        ));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| AppError::Internal)?;
    let mut response = client.get(url).send().await.map_err(|_| {
        AppError::BadRequest(
            "Could not download the image. Try a direct image link or upload a file.".into(),
        )
    })?;
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
    let expires = if source == "spotify" {
        Some(now + if data.is_some() { 7 * DAY } else { DAY })
    } else {
        None
    };
    let mut tx = state.db.begin().await?;
    if source == "spotify" {
        sqlx::query("DELETE FROM artwork WHERE source='spotify' AND expires_at_ms<=?")
            .bind(now)
            .execute(&mut *tx)
            .await?;
        let mut total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(length(bytes)),0) FROM artwork WHERE source='spotify'",
        )
        .fetch_one(&mut *tx)
        .await?;
        while total + data.map_or(0, |v| v.len() as i64) > PROVIDER_BUDGET {
            let deleted:Option<i64>=sqlx::query_scalar("DELETE FROM artwork WHERE rowid=(SELECT rowid FROM artwork WHERE source='spotify' ORDER BY checked_at_ms LIMIT 1) RETURNING COALESCE(length(bytes),0)").fetch_optional(&mut *tx).await?;
            if let Some(size) = deleted {
                total -= size
            } else {
                break;
            }
        }
    }
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
        let bytes = download(&url).await?;
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
async fn spotify_image(state: &AppState, kind: &str, id: &str) -> Result<Option<String>> {
    if kind == "albums" {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT a.cover_url FROM main.albums a JOIN name_group_members m ON m.entity_id=a.id AND m.kind='albums' WHERE m.group_id=? AND a.cover_url IS NOT NULL LIMIT 1")
                .bind(id)
                .fetch_optional(&state.read_db)
                .await?
                .flatten();
        if existing
            .as_ref()
            .is_some_and(|v| v.starts_with("https://i.scdn.co/image/"))
        {
            return Ok(existing);
        }
    }
    let singular = if kind == "artists" { "artist" } else { "album" };
    let external:Option<String>=sqlx::query_scalar("SELECT external_id FROM entity_links WHERE entity_id IN (SELECT entity_id FROM name_group_members WHERE group_id=?) AND kind=? AND namespace='spotify' ORDER BY external_id LIMIT 1").bind(id).bind(singular).fetch_optional(&state.read_db).await?;
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
            "SELECT e.external_id FROM entity_links e JOIN tracks t ON t.id=e.entity_id WHERE e.kind='track' AND e.namespace='spotify' AND t.album_id IN (SELECT entity_id FROM name_group_members WHERE group_id=?) ORDER BY e.external_id LIMIT 1"
        } else {
            "SELECT e.external_id FROM entity_links e JOIN track_artists ta ON ta.track_id=e.entity_id WHERE e.kind='track' AND e.namespace='spotify' AND ta.artist_id IN (SELECT entity_id FROM name_group_members WHERE group_id=?) ORDER BY e.external_id LIMIT 1"
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
    Ok(image["images"]
        .as_array()
        .and_then(|images| {
            images
                .iter()
                .rev()
                .find(|image| image["width"].as_i64().unwrap_or(320) >= 300)
                .or_else(|| images.first())
        })
        .and_then(|image| image["url"].as_str())
        .filter(|s| s.starts_with("https://i.scdn.co/image/"))
        .map(str::to_owned))
}
pub async fn artwork(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Response> {
    auth::session(&state, &headers).await?;
    let id =
        crate::name_groups::canonical(&mut *state.read_db.acquire().await?, &kind, &id).await?;
    if kind != "artists" && kind != "albums" {
        return Err(AppError::NotFound);
    }
    exists(&state, &kind, &id).await?;
    // Coalesce per identity. A cold thumbnail cannot hold up cached artwork.
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
    let _gate = entity_gate.lock().await;
    let cached=sqlx::query("SELECT bytes,mime FROM artwork WHERE kind=? AND entity_id=? AND (source='custom' OR expires_at_ms>?) ORDER BY CASE source WHEN 'custom' THEN 0 ELSE 1 END LIMIT 1").bind(&kind).bind(&id).bind(auth::now_ms()).fetch_optional(&state.read_db).await?;
    if let Some(row) = cached {
        let bytes: Option<Vec<u8>> = row.get("bytes");
        let bytes = bytes.ok_or(AppError::NotFound)?;
        // A restored database is untrusted input too: derive the response type
        // from the image bytes, never from an editable MIME string in SQLite.
        return Ok(([(header::CONTENT_TYPE, image_type(&bytes)?)], bytes).into_response());
    }
    writable(&state).await?;
    let connected: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM spotify_tokens)")
        .fetch_one(&state.private_db)
        .await?;
    if !connected {
        return Err(AppError::NotFound);
    }
    let _capacity = state
        .artwork_fetches
        .acquire()
        .await
        .map_err(|_| AppError::Internal)?;
    // A provider failure is not evidence that this entity has no image.
    // Respect the shared request limit without poisoning the negative cache.
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
    let bytes = match spotify_image(&state, &kind, &id).await {
        Ok(Some(url)) => match download(&url).await {
            Ok(bytes) => Some(bytes),
            Err(_) => return Ok(unavailable()),
        },
        Ok(None) | Err(AppError::NotFound) => None,
        Err(_) => return Ok(unavailable()),
    };
    save_image(&state, &kind, &id, "spotify", bytes.as_deref()).await?;
    let bytes = bytes.ok_or(AppError::NotFound)?;
    Ok(([(header::CONTENT_TYPE, image_type(&bytes)?)], bytes).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
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
