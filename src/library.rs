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
use std::collections::BTreeSet;

// CTEs overlay corrections without altering source metadata or executable schema.
// Readers of a frozen archive use the same overlays as the live consultation.
pub const METADATA: &str = include_str!("library-metadata.sql");
const IMAGE_LIMIT: usize = 5 * 1024 * 1024;

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
    let (_, singular, _) = entity(&kind)?;
    let mut view = state.storage.read_view().await?;
    let tx = &mut *view.transaction;
    let id = crate::name_groups::canonical(&mut *tx, &kind, &id).await?;
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
    let mut name: Option<String> = None;
    let mut rating: Option<f64> = None;
    let corrections: Vec<(String, String)> = sqlx::query_as(
        "SELECT field, value_json FROM local_corrections WHERE entity_kind=? AND entity_id=? AND field IN ('name', 'rating')",
    )
    .bind(singular)
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    for (field, value_json) in corrections {
        match field.as_str() {
            "name" => name = serde_json::from_str(&value_json).ok(),
            "rating" => rating = serde_json::from_str(&value_json).ok(),
            _ => {}
        }
    }
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
    let image_credit = if kind == "tracks" {
        let album: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "{METADATA} SELECT album_id FROM tracks WHERE id=?"
        )))
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
        match album {
            Some(album) => crate::artwork_sources::credit_in(&mut *tx, "albums", &album).await?,
            None => None,
        }
    } else {
        crate::artwork_sources::credit_in(&mut *tx, &kind, &id).await?
    };
    Ok(Json(Details {
        name: name.unwrap_or_else(|| original_name.clone()),
        original_name,
        rating,
        custom_image,
        spotify_url,
        image_credit,
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
/// Album artists: everyone credited first on at least one of the album's
/// tracks, plus artists credited on every track of a multi-track album. A
/// guest on one song (Halsey on "Boy With Luv") is not an album artist.
pub(crate) const ALBUM_ARTISTS_SQL: &str = "SELECT a.id,a.name FROM track_artists ta JOIN tracks t ON t.id=ta.track_id JOIN artists a ON a.id=ta.artist_id WHERE t.album_id=?1 GROUP BY a.id HAVING MIN(ta.ordinal)=0 OR (COUNT(DISTINCT ta.track_id)=(SELECT COUNT(*) FROM tracks WHERE album_id=?1) AND (SELECT COUNT(*) FROM tracks WHERE album_id=?1)>1) ORDER BY MIN(ta.ordinal),COUNT(DISTINCT ta.track_id) DESC,a.name COLLATE NOCASE,a.id";

/// Album artists of an album group, after local corrections and grouping.
pub(crate) async fn album_artists(
    conn: &mut sqlx::SqliteConnection,
    album_id: &str,
) -> Result<Vec<db::Artist>> {
    Ok(sqlx::query(sqlx::AssertSqlSafe(format!(
        "{METADATA} {ALBUM_ARTISTS_SQL}"
    )))
    .bind(album_id)
    .fetch_all(conn)
    .await?
    .into_iter()
    .map(|row| db::Artist {
        id: row.get("id"),
        name: row.get("name"),
    })
    .collect())
}

/// Download a user-supplied public image link.
pub(crate) async fn download(url: &str) -> Result<Vec<u8>> {
    match crate::artwork_http::image(url).await {
        Ok(Some(bytes)) => Ok(bytes),
        Ok(None) => Err(AppError::BadRequest(
            "The image host has no file at that link. Use a direct image link or upload a file."
                .into(),
        )),
        Err(AppError::ArtworkWait { .. }) => Err(AppError::BadRequest(
            "The image host is not answering right now. Try again in a minute or upload a file."
                .into(),
        )),
        Err(error) => Err(error),
    }
}
async fn save_image(
    state: &AppState,
    kind: &str,
    id: &str,
    source: &str,
    data: &[u8],
    provenance_json: Option<&str>,
) -> Result<()> {
    writable(state).await?;
    let mime = image_type(data)?;
    let now = auth::now_ms();
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision").fetch_optional(&mut *tx).await?.ok_or_else(||AppError::Conflict("Archive is in maintenance.".into()))?;
    sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms,provenance_json) VALUES(?,?,?,?,?,?,NULL,?) ON CONFLICT(kind,entity_id,source) DO UPDATE SET bytes=excluded.bytes,mime=excluded.mime,checked_at_ms=excluded.checked_at_ms,expires_at_ms=NULL,provenance_json=excluded.provenance_json")
        .bind(kind)
        .bind(id)
        .bind(source)
        .bind(data)
        .bind(mime)
        .bind(now)
        .bind(provenance_json)
        .execute(&mut *tx)
        .await?;
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
    save_image(&state, &kind, &id, "custom", &data, None).await?;
    Ok(Json(serde_json::json!({"saved":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageUrl {
    pub url: Option<String>,
    #[serde(default)]
    pub credit: Option<crate::artwork_sources::ImageCredit>,
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
        let provenance = match form.credit {
            Some(mut credit) => {
                credit.source_url = crate::artwork_sources::secure_url(&credit.source_url)
                    .ok_or_else(|| AppError::BadRequest("Invalid credit source URL.".into()))?;
                credit.image_url = crate::artwork_sources::secure_url(&credit.image_url)
                    .ok_or_else(|| AppError::BadRequest("Invalid credit image URL.".into()))?;
                credit.license_url = credit
                    .license_url
                    .and_then(|u| crate::artwork_sources::secure_url(&u));
                Some(serde_json::to_string(&credit).map_err(|_| AppError::Internal)?)
            }
            None => None,
        };
        save_image(&state, &kind, &id, "custom", &bytes, provenance.as_deref()).await?;
    } else {
        sqlx::query("DELETE FROM artwork WHERE kind=? AND entity_id=? AND source='custom'")
            .bind(&kind)
            .bind(&id)
            .execute(&state.db)
            .await?;
    }
    Ok(Json(serde_json::json!({"saved":true})))
}

#[derive(Deserialize)]
pub struct PreviewQuery {
    pub url: String,
}

pub async fn artwork_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PreviewQuery>,
) -> Result<Response> {
    auth::session(&state, &headers).await?;
    let bytes = download(&query.url).await?;
    let mime = image_type(&bytes)?;
    Ok(([(header::CONTENT_TYPE, mime)], bytes).into_response())
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
    let unavailable = |retry_seconds: i64| {
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            [
                (header::CACHE_CONTROL, "no-store"),
                (header::RETRY_AFTER, &retry_seconds.to_string()),
            ],
        )
            .into_response()
    };
    let cached = sqlx::query(
        "SELECT bytes FROM artwork WHERE kind=? AND entity_id=? AND bytes IS NOT NULL ORDER BY CASE source WHEN 'custom' THEN 0 ELSE 1 END LIMIT 1",
    )
    .bind(&kind)
    .bind(&id)
    .fetch_optional(&state.read_db)
    .await?;
    if let Some(row) = cached {
        let bytes: Option<Vec<u8>> = row.get("bytes");
        let bytes = bytes.ok_or(AppError::NotFound)?;
        // A restored database is untrusted input too: derive the response type
        // from the image bytes, never from an editable MIME string in SQLite.
        return Ok((
            [
                (header::CONTENT_TYPE, image_type(&bytes)?),
                (header::CACHE_CONTROL, "private, no-cache"),
            ],
            bytes,
        )
            .into_response());
    }
    // A fast cache lookup must never wait for a cold provider download.
    if query.cached {
        return Ok((
            axum::http::StatusCode::NOT_FOUND,
            [(header::CACHE_CONTROL, "no-store")],
        )
            .into_response());
    }
    use crate::artwork::ArtworkFetch;
    match crate::artwork::resolve_for_browser(&state, &kind, &id).await {
        Ok(ArtworkFetch::Missing) | Err(AppError::Unauthorized) => Err(AppError::NotFound),
        Ok(ArtworkFetch::Saved | ArtworkFetch::Cached) => {
            let bytes: Vec<u8> = sqlx::query_scalar(
                "SELECT bytes FROM artwork WHERE kind=? AND entity_id=? AND bytes IS NOT NULL ORDER BY CASE source WHEN 'custom' THEN 0 ELSE 1 END LIMIT 1",
            )
            .bind(&kind)
            .bind(&id)
            .fetch_one(&state.read_db)
            .await?;
            Ok((
                [
                    (header::CONTENT_TYPE, image_type(&bytes)?),
                    (header::CACHE_CONTROL, "private, no-cache"),
                ],
                bytes,
            )
                .into_response())
        }
        // The browser retries; the download keeps running server-side.
        Err(AppError::ArtworkWait { retry_at_ms }) => Ok(unavailable(
            ((retry_at_ms.saturating_sub(auth::now_ms()) + 999) / 1000).clamp(1, 60),
        )),
        Err(_) => Ok(unavailable(30)),
    }
}
