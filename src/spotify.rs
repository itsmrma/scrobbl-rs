use crate::{
    AppState, auth,
    db::{self, Artist, Event, Track},
    error::{AppError, Result},
    imports::spotify_uri,
};
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
    response::Redirect,
};
use chrono::DateTime;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, Notify};

#[derive(Clone)]
pub struct Spotify {
    client: Client,
    pub gate: Arc<Mutex<()>>,
    collection_gate: Arc<Mutex<()>>,
    now_gate: Arc<Mutex<()>>,
    wake: Arc<Notify>,
    cache: Arc<Mutex<Option<(i64, Value)>>>,
    // Fixed production endpoints. Tests substitute a local HTTP server, never user-controlled URLs.
    pub(crate) api_base: String,
    pub(crate) token_url: String,
}
impl Default for Spotify {
    fn default() -> Self {
        Self::new()
    }
}
impl Spotify {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("HTTP client initialization failed"),
            gate: Default::default(),
            collection_gate: Default::default(),
            now_gate: Default::default(),
            wake: Default::default(),
            cache: Default::default(),
            api_base: "https://api.spotify.com/v1".into(),
            token_url: "https://accounts.spotify.com/api/token".into(),
        }
    }
}
async fn credentials(state: &AppState) -> Result<Option<(String, String)>> {
    if let (Some(id), Some(secret)) = (&state.config.env_client_id, &state.config.env_client_secret)
    {
        return Ok(Some((id.clone(), secret.clone())));
    }
    let id = db::setting(&state.private_db, "spotify_client_id").await?;
    let secret = db::setting(&state.private_db, "spotify_client_secret").await?;
    Ok(id.zip(secret))
}
async fn diagnostic(state: &AppState, key: &str, value: &str) -> Result<()> {
    let mut conn = state.db.acquire().await?;
    db::set_setting(&mut conn, key, value).await
}
pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<crate::dto::SpotifyStatus>> {
    auth::session(&state, &headers).await?;
    let configured = credentials(&state).await?.is_some();
    let connected: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM spotify_tokens)")
        .fetch_one(&state.private_db)
        .await?;
    let mut view = state.storage.read_view().await?;
    let conn = &mut *view.transaction;
    async fn setting(conn: &mut sqlx::SqliteConnection, key: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
                .bind(key)
                .fetch_optional(conn)
                .await?
                .filter(|v| !v.is_empty()),
        )
    }
    async fn timestamp(conn: &mut sqlx::SqliteConnection, key: &str) -> Result<Option<i64>> {
        Ok(setting(conn, key).await?.and_then(|v| v.parse().ok()))
    }
    let verified_at_ms = timestamp(conn, "spotify_verified_at_ms").await?;
    let retry_at_ms = timestamp(conn, "retry_at_ms").await?;
    let required = setting(conn, "spotify_action_required").await?;
    let last_error = setting(conn, "spotify_error").await?;
    let collection_error = setting(conn, "collection_error").await?;
    let connection = if !connected {
        "disconnected"
    } else if required.is_some() {
        "action_required"
    } else if verified_at_ms.is_some() {
        "verified"
    } else {
        "unverified"
    };
    let collection = if !connected {
        "paused"
    } else if retry_at_ms.is_some_and(|v| v > auth::now_ms()) {
        "waiting"
    } else if required.is_some() || collection_error.is_some() {
        "error"
    } else {
        "active"
    };
    let jobs = sqlx::query(
        "SELECT filename,created_at_ms,report_json FROM import_jobs ORDER BY id DESC LIMIT 10",
    )
    .fetch_all(&mut *conn)
    .await?;
    let imports = jobs
        .into_iter()
        .map(|r| {
            Ok(crate::dto::ImportJob {
                filename: r.get("filename"),
                created_at_ms: r.get("created_at_ms"),
                report: serde_json::from_str(&r.get::<String, _>("report_json"))
                    .map_err(|_| AppError::Internal)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Json(crate::dto::SpotifyStatus {
        configured,
        connected,
        credentials_from_env: state.config.env_client_id.is_some(),
        redirect_uri: state.config.redirect_uri(),
        sync_seconds: state.config.sync_seconds,
        now_playing_seconds: state.config.now_playing_seconds,
        connection: connection.into(),
        collection: collection.into(),
        last_attempt_ms: timestamp(conn, "last_attempt_ms").await?,
        last_sync_ms: timestamp(conn, "last_sync_ms").await?,
        last_added: timestamp(conn, "last_sync_added").await?,
        verified_at_ms,
        last_error: collection_error.or(last_error),
        now_playing_error: setting(conn, "now_playing_error").await?,
        now_playing_timing: setting(conn, "now_playing_timing")
            .await?
            .and_then(|v| serde_json::from_str(&v).ok()),
        coverage_warning: setting(conn, "coverage_warning").await?,
        retry_at_ms,
        imports,
        review_count: sqlx::query_scalar("SELECT COUNT(*) FROM listens WHERE needs_review=1")
            .fetch_one(&mut *conn)
            .await?,
    }))
}
#[derive(Deserialize)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
}
pub async fn save_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<Credentials>,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    if state.config.env_client_id.is_some() {
        return Err(AppError::Conflict(
            "Spotify credentials are controlled by environment variables.".into(),
        ));
    }
    let id = form.client_id.trim();
    let secret = form.client_secret.trim();
    if id.len() < 10
        || id.len() > 256
        || secret.len() < 10
        || secret.len() > 256
        || !id.bytes().all(|v| v.is_ascii_alphanumeric())
        || !secret.bytes().all(|v| v.is_ascii_alphanumeric())
    {
        return Err(AppError::BadRequest(
            "Enter the Client ID and Client Secret from your Spotify app settings.".into(),
        ));
    }
    let _collection = state.spotify.collection_gate.lock().await;
    let _gate = state.spotify.gate.lock().await;
    let mut tx = state.private_db.begin().await?;
    db::set_setting(&mut tx, "spotify_client_id", id).await?;
    db::set_setting(&mut tx, "spotify_client_secret", secret).await?;
    sqlx::query("DELETE FROM spotify_tokens")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM oauth_states")
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO access_operations(id,kind) VALUES(?,'spotify_access_changed')")
        .bind(db::new_id())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    crate::storage::recover_access(&state.db, &state.private_db)
        .await
        .map_err(|_| AppError::Internal)?;
    *state.spotify.cache.lock().await = None;
    Ok(Json(json!({"ok":true})))
}
pub async fn connect(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let session = auth::require_write(&state, &headers).await?;
    let (id, _) = credentials(&state)
        .await?
        .ok_or_else(|| AppError::BadRequest("Configure your Spotify app first.".into()))?;
    let secret = auth::random_secret();
    let mut tx = state.private_db.begin().await?;
    sqlx::query("DELETE FROM oauth_states WHERE session_digest = ? OR expires_at_ms <= ?")
        .bind(&session.digest)
        .bind(auth::now_ms())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO oauth_states(digest, session_digest, expires_at_ms) VALUES (?, ?, ?)")
        .bind(auth::digest(&secret))
        .bind(session.digest)
        .bind(auth::now_ms() + 600_000)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let mut url = url::Url::parse("https://accounts.spotify.com/authorize")
        .map_err(|_| AppError::Internal)?;
    url.query_pairs_mut()
        .append_pair("client_id", &id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &state.config.redirect_uri())
        .append_pair("state", &secret)
        .append_pair(
            "scope",
            "user-read-recently-played user-read-currently-playing",
        );
    Ok(Json(json!({"url":url.to_string()})))
}
#[derive(Deserialize)]
pub struct Callback {
    pub state: Option<String>,
    pub code: Option<String>,
    pub error: Option<String>,
}
pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Callback>,
) -> Result<Redirect> {
    let session = auth::session(&state, &headers).await?;
    let secret = query
        .state
        .filter(|s| s.len() == 64)
        .ok_or(AppError::Forbidden)?;
    let _collection = state.spotify.collection_gate.lock().await;
    let _gate = state.spotify.gate.lock().await;
    let used = sqlx::query(
        "DELETE FROM oauth_states WHERE digest = ? AND session_digest = ? AND expires_at_ms > ?",
    )
    .bind(auth::digest(&secret))
    .bind(session.digest)
    .bind(auth::now_ms())
    .execute(&state.private_db)
    .await?;
    if used.rows_affected() != 1 {
        return Err(AppError::Forbidden);
    }
    if query.error.is_some() {
        diagnostic(
            &state,
            "spotify_error",
            "Spotify authorization was declined. You can try connecting again.",
        )
        .await?;
        return Ok(Redirect::to("/#settings"));
    }
    let code = query
        .code
        .ok_or_else(|| AppError::BadRequest("Missing authorization code.".into()))?;
    let (id, secret) = credentials(&state)
        .await?
        .ok_or_else(|| AppError::BadRequest("Configure your Spotify app first.".into()))?;
    let response = state
        .spotify
        .client
        .post(&state.spotify.token_url)
        .basic_auth(id, Some(secret))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", state.config.redirect_uri().as_str()),
        ])
        .send()
        .await
        .map_err(|_| AppError::Spotify("Could not reach Spotify. Try connecting again.".into()))?;
    let value = checked_json(&state, response).await?;
    save_tokens(&state, &value, None).await?;
    diagnostic(&state, "spotify_error", "").await?;
    state.spotify.wake.notify_one();
    Ok(Redirect::to("/#settings"))
}
async fn save_tokens(
    state: &AppState,
    value: &Value,
    previous_refresh: Option<&str>,
) -> Result<()> {
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Spotify("Spotify returned an invalid token response.".into()))?;
    let refresh = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .or(previous_refresh)
        .ok_or_else(|| {
            AppError::Spotify("Spotify did not provide a refresh token. Please reconnect.".into())
        })?;
    let seconds = value
        .get("expires_in")
        .and_then(Value::as_i64)
        .filter(|v| *v > 0 && *v <= 86_400)
        .ok_or_else(|| AppError::Spotify("Spotify returned an invalid token lifetime.".into()))?;
    sqlx::query("INSERT INTO spotify_tokens(id, access_token, refresh_token, expires_at_ms) VALUES (1, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET access_token = excluded.access_token, refresh_token = excluded.refresh_token, expires_at_ms = excluded.expires_at_ms")
        .bind(access).bind(refresh).bind(auth::now_ms()+seconds*1000).execute(&state.private_db).await?;
    Ok(())
}
async fn checked_json(state: &AppState, response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS {
        let wait = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(300)
            .clamp(1, 604_800);
        diagnostic(
            state,
            "retry_at_ms",
            &(auth::now_ms() + wait * 1000).to_string(),
        )
        .await?;
        return Err(AppError::Spotify(format!(
            "Spotify request limit reached. Retrying in {wait} seconds."
        )));
    }
    if matches!(status.as_u16(), 400 | 401 | 403) {
        diagnostic(state, "spotify_action_required", "true").await?;
    }
    if !status.is_success() {
        // Never return upstream payloads: they can include credentials, tokens or URLs.
        let message = match status.as_u16() {
            400 | 401 => {
                "Spotify authorization expired or was rejected. Please reconnect your account."
            }
            403 => {
                "Spotify denied access. Check Premium, the app allowlist and granted permissions."
            }
            _ => "Spotify is temporarily unavailable. Collection will retry automatically.",
        };
        return Err(AppError::Spotify(message.into()));
    }
    if status == StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    response
        .json()
        .await
        .map_err(|_| AppError::Spotify("Spotify returned an unreadable response.".into()))
}
async fn access_token(state: &AppState, force_refresh: bool) -> Result<String> {
    if let Some(retry) = db::setting(&state.db, "retry_at_ms")
        .await?
        .and_then(|v| v.parse::<i64>().ok())
        && retry > auth::now_ms()
    {
        return Err(AppError::Spotify(
            "Waiting for Spotify's request limit to reset.".into(),
        ));
    }
    let row = sqlx::query(
        "SELECT access_token, refresh_token, expires_at_ms FROM spotify_tokens WHERE id = 1",
    )
    .fetch_optional(&state.private_db)
    .await?
    .ok_or_else(|| AppError::BadRequest("Connect your Spotify account first.".into()))?;
    if !force_refresh && row.get::<i64, _>("expires_at_ms") > auth::now_ms() + 60_000 {
        return Ok(row.get("access_token"));
    }
    let refresh: String = row.get("refresh_token");
    let (id, secret) = credentials(state)
        .await?
        .ok_or_else(|| AppError::BadRequest("Configure your Spotify app first.".into()))?;
    let response = state
        .spotify
        .client
        .post(&state.spotify.token_url)
        .basic_auth(id, Some(secret))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
        ])
        .send()
        .await
        .map_err(|_| {
            AppError::Spotify(
                "Could not refresh Spotify authorization. Collection will retry.".into(),
            )
        })?;
    let value = checked_json(state, response).await?;
    save_tokens(state, &value, Some(&refresh)).await?;
    Ok(value["access_token"]
        .as_str()
        .ok_or(AppError::Internal)?
        .into())
}
async fn api_get(state: &AppState, path: &str, params: &[(&str, String)]) -> Result<Value> {
    let _gate = state.spotify.gate.lock().await;
    api_get_locked(state, path, params)
        .await
        .map(|(value, _, _)| value)
}
async fn api_get_locked(
    state: &AppState,
    path: &str,
    params: &[(&str, String)],
) -> Result<(Value, i64, i64)> {
    let token = access_token(state, false).await?;
    let url = format!("{}{path}", state.spotify.api_base);
    let mut source_requested_at_ms = auth::now_ms();
    let mut response = state
        .spotify
        .client
        .get(&url)
        .bearer_auth(token)
        .query(params)
        .send()
        .await
        .map_err(|_| AppError::Spotify("Could not reach Spotify. Collection will retry.".into()))?;
    if response.status() == StatusCode::UNAUTHORIZED {
        let token = access_token(state, true).await?;
        source_requested_at_ms = auth::now_ms();
        response = state
            .spotify
            .client
            .get(&url)
            .bearer_auth(token)
            .query(params)
            .send()
            .await
            .map_err(|_| {
                AppError::Spotify("Could not reach Spotify. Collection will retry.".into())
            })?;
    }
    let value = checked_json(state, response).await?;
    let response_received_at_ms = auth::now_ms();
    diagnostic(state, "spotify_verified_at_ms", &auth::now_ms().to_string()).await?;
    diagnostic(state, "spotify_action_required", "").await?;
    Ok((value, source_requested_at_ms, response_received_at_ms))
}
pub fn parse_track(row: &Value) -> Result<Track> {
    let id = row
        .get("uri")
        .and_then(Value::as_str)
        .filter(|v| spotify_uri(v))
        .ok_or_else(|| {
            AppError::Spotify("Spotify returned a music track without a valid identifier.".into())
        })?;
    let title = row
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("Unknown track")
        .to_owned();
    let album = row.get("album");
    let album_id = album
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(|id| format!("spotify:album:{id}"));
    let album_name = album
        .and_then(|v| v.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let cover_url = album
        .and_then(|v| v.get("images"))
        .and_then(Value::as_array)
        .and_then(|a| a.last())
        .and_then(|v| v.get("url"))
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("https://i.scdn.co/"))
        .map(str::to_owned);
    let mut artists = Vec::new();
    if let Some(items) = row.get("artists").and_then(Value::as_array) {
        for item in items {
            if let Some(name) = item.get("name").and_then(Value::as_str) {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|id| format!("spotify:artist:{id}"))
                    .unwrap_or_else(|| db::metadata_id("import-artist", &[name]));
                if !artists.iter().any(|a: &Artist| a.id == id) {
                    artists.push(Artist {
                        id,
                        name: name.into(),
                    });
                }
            }
        }
    }
    Ok(Track {
        id: id.into(),
        title,
        album_id,
        album: album_name,
        cover_url,
        duration_ms: row
            .get("duration_ms")
            .and_then(Value::as_i64)
            .filter(|v| *v >= 0),
        artists,
        structured: true,
    })
}
pub async fn sync(state: &AppState) -> Result<Value> {
    let _collection = state.spotify.collection_gate.lock().await;
    let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='spotify_collection' AND state IN ('queued','waiting'))").fetch_one(&state.db).await?;
    if !pending {
        state
            .storage
            .enqueue(
                "spotify_collection",
                None,
                Some("Spotify background collection"),
                &json!({}),
            )
            .await?;
    }
    let job = state
        .storage
        .claim("spotify_collection")
        .await?
        .ok_or_else(|| AppError::Conflict("Collection is waiting for its next attempt.".into()))?;
    diagnostic(state, "last_attempt_ms", &auth::now_ms().to_string()).await?;
    let result = sync_locked(state, &job).await;
    if let Err(ref error) = result {
        diagnostic(state, "collection_error", &error.to_string()).await?;
        let retry = db::setting(&state.db, "retry_at_ms")
            .await?
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v > auth::now_ms())
            .unwrap_or(auth::now_ms() + state.config.sync_seconds as i64 * 1000);
        state.storage.pause(&job,Some(retry),"spotify_collection_failed: see Connections for authorization, request limit or network details").await?;
    }
    result
}
async fn sync_locked(state: &AppState, job: &crate::jobs::Job) -> Result<Value> {
    let previous = db::setting(&state.db, "recent_cursor_ms")
        .await?
        .and_then(|v| v.parse::<i64>().ok());
    let mut before: Option<i64> = None;
    let mut events = Vec::new();
    let mut excluded = Vec::new();
    let mut newest = previous;
    let mut warning = None;
    // Walk backwards using our own cursors. Never follow arbitrary upstream URLs with a bearer token.
    for page in 0..20 {
        let mut params = vec![("limit", "50".into())];
        if let Some(cursor) = before {
            params.push(("before", cursor.to_string()));
        }
        let value = api_get(state, "/me/player/recently-played", &params).await?;
        let rows = value
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                AppError::Spotify("Spotify returned invalid listening history.".into())
            })?;
        if rows.len() > 50 {
            return Err(AppError::Spotify(
                "Spotify returned more history rows than requested. The cursor has not advanced."
                    .into(),
            ));
        }
        let mut oldest: Option<i64> = None;
        for row in rows {
            let timestamp = row.get("played_at").and_then(Value::as_str).and_then(|s|DateTime::parse_from_rfc3339(s).ok()).map(|v|v.timestamp_millis()).ok_or_else(||AppError::Spotify("Spotify returned an invalid listening timestamp. The sync cursor has not advanced.".into()))?;
            oldest = Some(oldest.map_or(timestamp, |v| v.min(timestamp)));
            newest = Some(newest.map_or(timestamp, |v| v.max(timestamp)));
            if row["track"]["type"].as_str().is_some_and(|v| v != "track")
                || row["track"]["is_local"] == true
            {
                excluded.push(crate::jobs::SourceRow::Skipped {
                    raw: row.clone(),
                    reason: "Non-musical or local track excluded from API collection.".into(),
                });
                continue;
            }
            let track = parse_track(&row["track"])?;
            events.push(Event {
                source: "spotify_api",
                source_key: db::metadata_id("event", &[&track.id, &timestamp.to_string()]),
                source_timestamp_ms: timestamp,
                played_at_ms: timestamp,
                listened_ms: track.duration_ms,
                duration_kind: if track.duration_ms.is_some() {
                    db::DurationKind::Estimated
                } else {
                    db::DurationKind::Unknown
                },
                counted: true,
                track,
                raw: row.clone(),
            });
        }
        let reached_previous = previous.is_some_and(|p| oldest.is_some_and(|o| o <= p));
        if reached_previous || rows.len() < 50 || oldest.is_none() {
            if previous.is_some() && !reached_previous {
                warning = Some(
                    "Spotify did not return history reaching the previous sync. There may be a gap; an extended-history import can repair it.",
                );
            }
            break;
        }
        if page == 19 || before.is_some_and(|b| oldest.is_some_and(|o| o >= b)) {
            warning = Some(
                "Recent-history recovery stopped at the pagination limit. Some listens may be missing; import extended history to repair gaps.",
            );
            break;
        }
        before = oldest;
    }
    events.sort_by_key(|e| e.played_at_ms);
    let mut tx = state.db.begin().await?;
    let mut rows: Vec<_> = events
        .into_iter()
        .map(crate::jobs::SourceRow::from)
        .collect();
    rows.extend(excluded);
    let report = state
        .storage
        .commit_member_batch_in(&mut tx, job, "recently-played", 1, &rows, true)
        .await?;
    if let Some(cursor) = newest {
        db::set_setting(&mut tx, "recent_cursor_ms", &cursor.to_string()).await?;
    }
    sqlx::query("UPDATE jobs SET phase='finished',outcome='complete',retry_at_ms=NULL,error_code=NULL,inventory_complete=1 WHERE id=?").bind(&job.id).execute(&mut *tx).await?;
    db::set_setting(&mut tx, "last_sync_ms", &auth::now_ms().to_string()).await?;
    db::set_setting(&mut tx, "spotify_error", "").await?;
    db::set_setting(&mut tx, "collection_error", "").await?;
    db::set_setting(&mut tx, "last_sync_added", &report.added.to_string()).await?;
    if let Some(warning) = warning {
        db::set_setting(&mut tx, "coverage_warning", warning).await?;
    }
    tx.commit().await?;
    Ok(json!(report))
}
pub async fn sync_now(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    // Coalesce manual clicks; do not let them bypass the background cadence.
    if db::setting(&state.db, "last_sync_ms")
        .await?
        .and_then(|v| v.parse::<i64>().ok())
        .is_some_and(|at| auth::now_ms() - at < 15_000)
    {
        return Err(AppError::Conflict(
            "The archive was just synchronized. Please wait a few seconds.".into(),
        ));
    }
    let result = sync(&state).await;
    if let Err(ref e) = result {
        diagnostic(&state, "spotify_error", &e.to_string()).await?;
    }
    result.map(Json)
}
pub async fn now_playing(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::session(&state, &headers).await?;
    let requested_at_ms = auth::now_ms();
    let _now = state.spotify.now_gate.lock().await;
    if let Some((time, value)) = &*state.spotify.cache.lock().await
        && auth::now_ms() / (state.config.now_playing_seconds as i64 * 1000)
            == time / (state.config.now_playing_seconds as i64 * 1000)
    {
        let mut value = value.clone();
        if let Some(object) = value.as_object_mut() {
            object.insert("served_at_ms".into(), json!(auth::now_ms()));
        }
        return Ok(Json(value));
    }
    let connected: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM spotify_tokens)")
        .fetch_one(&state.private_db)
        .await?;
    if !connected {
        return Ok(Json(Value::Null));
    }
    let _gate = state.spotify.gate.lock().await;
    let result = api_get_locked(&state, "/me/player/currently-playing", &[]).await;
    let (row, source_requested_at_ms, response_received_at_ms) = match result {
        Ok(sample) => sample,
        Err(e) => {
            diagnostic(&state, "now_playing_error", &e.to_string()).await?;
            return Err(e);
        }
    };
    let value = if row["is_playing"] == true && row["currently_playing_type"] == "track" {
        match parse_track(&row["item"]) {
            Ok(track) => {
                json!({"title":track.title,"artists":track.artists,"album":track.album,"cover_url":track.cover_url,"spotify_url":format!("https://open.spotify.com/track/{}",track.id.trim_start_matches("spotify:track:")),"duration_ms":track.duration_ms,"progress_ms":row["progress_ms"],"observed_at_ms":response_received_at_ms,"source_timestamp_ms":row["timestamp"].as_i64(),"requested_at_ms":requested_at_ms,"source_requested_at_ms":source_requested_at_ms,"response_received_at_ms":response_received_at_ms,"served_at_ms":auth::now_ms()})
            }
            Err(_) => Value::Null,
        }
    } else {
        Value::Null
    };
    diagnostic(&state, "now_playing_timing", &json!({"requested_at_ms":requested_at_ms,"source_requested_at_ms":source_requested_at_ms,"response_received_at_ms":response_received_at_ms,"source_timestamp_ms":row["timestamp"].as_i64()}).to_string()).await?;
    diagnostic(&state, "now_playing_error", "").await?;
    *state.spotify.cache.lock().await = Some((source_requested_at_ms, value.clone()));
    Ok(Json(value))
}
pub async fn disconnect(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    let _collection = state.spotify.collection_gate.lock().await;
    let _gate = state.spotify.gate.lock().await;
    let mut tx = state.private_db.begin().await?;
    sqlx::query("DELETE FROM spotify_tokens")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM oauth_states")
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO access_operations(id,kind) VALUES(?,'spotify_access_changed')")
        .bind(db::new_id())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    crate::storage::recover_access(&state.db, &state.private_db)
        .await
        .map_err(|_| AppError::Internal)?;
    *state.spotify.cache.lock().await = None;
    Ok(Json(json!({"ok":true})))
}
pub async fn worker(state: AppState) {
    loop {
        let connected =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM spotify_tokens)")
                .fetch_one(&state.private_db)
                .await
                .unwrap_or(false);
        if connected && let Err(error) = sync(&state).await {
            let _ = diagnostic(&state, "spotify_error", &error.to_string()).await;
        }
        tokio::select! { _ = tokio::time::sleep(Duration::from_secs(state.config.sync_seconds)) => {}, _ = state.spotify.wake.notified() => {} }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use axum::{
        Router,
        extract::{OriginalUri, State},
        http::{HeaderMap, HeaderValue, Method},
        response::IntoResponse,
        routing::any,
    };
    use std::collections::VecDeque;

    struct Reply {
        path: &'static str,
        status: StatusCode,
        body: Value,
        retry: Option<&'static str>,
    }
    #[derive(Clone, Default)]
    struct Mock {
        replies: Arc<Mutex<VecDeque<Reply>>>,
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }
    async fn respond(
        State(mock): State<Mock>,
        OriginalUri(uri): OriginalUri,
        method: Method,
        headers: HeaderMap,
    ) -> axum::response::Response {
        mock.calls
            .lock()
            .await
            .push((method.to_string(), uri.to_string()));
        let reply = mock
            .replies
            .lock()
            .await
            .pop_front()
            .expect("unexpected Spotify request");
        assert_eq!(uri.path(), reply.path);
        assert!(headers.get("authorization").is_some());
        if let Some(ms) = reply.body["_test_delay_ms"].as_u64() {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        let mut response = (reply.status, Json(reply.body)).into_response();
        if let Some(wait) = reply.retry {
            response
                .headers_mut()
                .insert("retry-after", HeaderValue::from_static(wait));
        }
        response
    }
    async fn fixture(
        replies: Vec<Reply>,
        expired: bool,
    ) -> (
        tempfile::TempDir,
        AppState,
        Mock,
        tokio::task::JoinHandle<()>,
    ) {
        let mock = Mock::default();
        *mock.replies.lock().await = replies.into();
        let app = Router::new()
            .fallback(any(respond))
            .with_state(mock.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let db = db::open(dir.path()).await.unwrap();
        let config = Config {
            port: 3210,
            data_dir: dir.path().to_owned(),
            sync_seconds: 120,
            now_playing_seconds: 30,
            import_limits: Default::default(),
            env_client_id: Some("clientid".into()),
            env_client_secret: Some("secret-never-returned".into()),
        };
        let mut state = AppState::new(db, config);
        state.spotify.api_base = format!("http://{address}/v1");
        state.spotify.token_url = format!("http://{address}/api/token");
        sqlx::query("INSERT INTO spotify_tokens(id,access_token,refresh_token,expires_at_ms) VALUES (1,'initial-access','initial-refresh',?)").bind(if expired{0}else{auth::now_ms()+3_600_000}).execute(&state.private_db).await.unwrap();
        (dir, state, mock, handle)
    }
    fn reply(path: &'static str, body: Value) -> Reply {
        Reply {
            path,
            status: StatusCode::OK,
            body,
            retry: None,
        }
    }
    fn track() -> Value {
        json!({"uri":"spotify:track:0000000000000000000001","type":"track","name":"Test track","duration_ms":180000,"album":{"id":"album1","name":"An album","images":[]},"artists":[{"id":"artist1","name":"Artist"}]})
    }
    async fn owner_headers(state: &AppState) -> HeaderMap {
        let cookie = auth::random_secret();
        sqlx::query("INSERT INTO sessions(digest,csrf,expires_at_ms) VALUES (?,'csrf',?)")
            .bind(auth::digest(&cookie))
            .bind(auth::now_ms() + 60000)
            .execute(&state.private_db)
            .await
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            HeaderValue::from_str(&format!("scrobbl_session={cookie}")).unwrap(),
        );
        headers
    }
    #[tokio::test]
    async fn status_distinguishes_saved_authorization_collection_and_now_playing_errors() {
        let (_dir, state, _mock, handle) = fixture(
            vec![
                reply("/v1/me/player/recently-played", json!({"items":[]})),
                Reply {
                    path: "/v1/me/player/currently-playing",
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    body: json!({"token":"secret-never-returned"}),
                    retry: None,
                },
            ],
            false,
        )
        .await;
        let headers = owner_headers(&state).await;
        let before = status(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(before.connection, "unverified");
        assert!(before.last_sync_ms.is_none());
        sync(&state).await.unwrap();
        let collected = status(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(collected.connection, "verified");
        assert_eq!(collected.collection, "active");
        assert_eq!(collected.last_added, Some(0));
        assert!(collected.last_attempt_ms.is_some());
        assert!(collected.verified_at_ms.is_some());
        assert!(
            now_playing(State(state.clone()), headers.clone())
                .await
                .is_err()
        );
        let after = status(State(state.clone()), headers).await.unwrap().0;
        assert_eq!(after.collection, "active");
        assert_eq!(after.last_sync_ms, collected.last_sync_ms);
        assert!(after.last_error.is_none());
        assert!(after.now_playing_error.is_some());
        assert!(
            !serde_json::to_string(&after)
                .unwrap()
                .contains("secret-never-returned")
        );
        handle.abort();
        state.storage.close().await;
    }
    #[tokio::test]
    async fn status_requires_action_after_rejected_refresh_and_keeps_rate_limit_visible() {
        let (_dir, state, _mock, handle) = fixture(
            vec![Reply {
                path: "/api/token",
                status: StatusCode::BAD_REQUEST,
                body: json!({"token":"secret-never-returned"}),
                retry: None,
            }],
            true,
        )
        .await;
        let headers = owner_headers(&state).await;
        assert!(sync(&state).await.is_err());
        let rejected = status(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(rejected.connection, "action_required");
        assert_eq!(rejected.collection, "error");
        assert!(rejected.last_error.is_some());
        assert!(
            !serde_json::to_string(&rejected)
                .unwrap()
                .contains("secret-never-returned")
        );
        handle.abort();
        state.storage.close().await;
        let (_dir, state, _mock, handle) = fixture(
            vec![Reply {
                path: "/v1/me/player/recently-played",
                status: StatusCode::TOO_MANY_REQUESTS,
                body: json!({}),
                retry: Some("60"),
            }],
            false,
        )
        .await;
        let headers = owner_headers(&state).await;
        assert!(sync(&state).await.is_err());
        let limited = status(State(state.clone()), headers).await.unwrap().0;
        assert_eq!(limited.collection, "waiting");
        assert!(limited.retry_at_ms.unwrap() > auth::now_ms());
        assert!(limited.last_sync_ms.is_none());
        handle.abort();
        state.storage.close().await;
    }
    fn item(at: i64) -> Value {
        json!({"track":track(),"played_at":DateTime::from_timestamp_millis(at).unwrap().to_rfc3339()})
    }
    #[tokio::test]
    async fn expired_tokens_are_refreshed_and_collection_survives_a_restart() {
        let at = 1_749_000_000_000;
        let (dir, state, mock, handle) = fixture(
            vec![
                reply(
                    "/api/token",
                    json!({"access_token":"new-access","expires_in":3600}),
                ),
                reply("/v1/me/player/recently-played", json!({"items":[item(at)]})),
            ],
            true,
        )
        .await;
        assert_eq!(sync(&state).await.unwrap()["added"], 1);
        let refresh: String = sqlx::query_scalar("SELECT refresh_token FROM spotify_tokens")
            .fetch_one(&state.private_db)
            .await
            .unwrap();
        assert_eq!(refresh, "initial-refresh");
        let checkpoint = db::setting(&state.db, "recent_cursor_ms")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(checkpoint, at.to_string());
        state.storage.close().await;
        let pool = db::open(dir.path()).await.unwrap();
        let mut restarted = AppState::new(pool, state.config.clone());
        restarted.spotify = state.spotify.clone();
        mock.replies.lock().await.push_back(reply(
            "/v1/me/player/recently-played",
            json!({"items":[item(at)]}),
        ));
        assert_eq!(sync(&restarted).await.unwrap()["duplicates"], 1);
        assert_eq!(mock.calls.lock().await.len(), 3);
        handle.abort();
    }
    #[tokio::test]
    async fn refreshes_once_after_a_401() {
        let (_dir,state,mock,handle)=fixture(vec![Reply{path:"/v1/me/player/recently-played",status:StatusCode::UNAUTHORIZED,body:json!({"error":"do not return this token"}),retry:None},reply("/api/token",json!({"access_token":"replacement","refresh_token":"rotated-refresh","expires_in":3600})),reply("/v1/me/player/recently-played",json!({"items":[]}))],false).await;
        sync(&state).await.unwrap();
        let refresh: String = sqlx::query_scalar("SELECT refresh_token FROM spotify_tokens")
            .fetch_one(&state.private_db)
            .await
            .unwrap();
        assert_eq!(refresh, "rotated-refresh");
        assert!(mock.replies.lock().await.is_empty());
        handle.abort();
    }
    #[tokio::test]
    async fn rate_limits_are_shared_with_now_playing_and_persisted() {
        let (_dir, state, mock, handle) = fixture(
            vec![Reply {
                path: "/v1/me/player/recently-played",
                status: StatusCode::TOO_MANY_REQUESTS,
                body: json!({"error":"private upstream payload"}),
                retry: Some("120"),
            }],
            false,
        )
        .await;
        assert!(sync(&state).await.unwrap_err().to_string().contains("120"));
        assert!(sync(&state).await.is_err());
        assert!(
            api_get(&state, "/me/player/currently-playing", &[])
                .await
                .is_err()
        );
        assert_eq!(mock.calls.lock().await.len(), 1);
        assert!(
            db::setting(&state.db, "retry_at_ms")
                .await
                .unwrap()
                .unwrap()
                .parse::<i64>()
                .unwrap()
                > auth::now_ms()
        );
        assert!(
            db::setting(&state.db, "recent_cursor_ms")
                .await
                .unwrap()
                .is_none()
        );
        handle.abort();
    }
    #[tokio::test]
    async fn paginates_backwards_and_does_not_advance_after_partial_failure() {
        let at = 1_749_000_000_000;
        let batch: Vec<_> = (0..50).map(|i| item(at - i * 180_000)).collect();
        let (_dir, state, mock, handle) = fixture(
            vec![
                reply("/v1/me/player/recently-played", json!({"items":batch})),
                Reply {
                    path: "/v1/me/player/recently-played",
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    body: json!({}),
                    retry: None,
                },
            ],
            false,
        )
        .await;
        assert!(sync(&state).await.is_err());
        assert!(
            db::setting(&state.db, "recent_cursor_ms")
                .await
                .unwrap()
                .is_none()
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM listens")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert!(mock.calls.lock().await[1].1.contains("before="));
        mock.replies.lock().await.extend([
            reply("/v1/me/player/recently-played", json!({"items":batch})),
            reply(
                "/v1/me/player/recently-played",
                json!({"items":[item(at-50*180_000)]}),
            ),
        ]);
        // The failed attempt is durably waiting. Simulate its scheduled retry becoming due.
        sqlx::query(
            "UPDATE jobs SET retry_at_ms=0 WHERE kind='spotify_collection' AND state='waiting'",
        )
        .execute(&state.db)
        .await
        .unwrap();
        assert_eq!(sync(&state).await.unwrap()["added"], 51);
        let completed: crate::jobs::Job =
            sqlx::query_as("SELECT * FROM jobs WHERE kind='spotify_collection'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(completed.outcome.as_deref(), Some("complete"));
        assert!(completed.error_code.is_none());
        assert!(completed.retry_at_ms.is_none());
        handle.abort();
    }
    #[tokio::test]
    async fn oauth_callback_is_bound_to_the_owner_session_and_consumed_once() {
        let (_dir,state,mock,handle)=fixture(vec![reply("/api/token",json!({"access_token":"access-from-code","refresh_token":"refresh-from-code","expires_in":3600}))],false).await;
        let cookie = auth::random_secret();
        let secret = auth::random_secret();
        sqlx::query("INSERT INTO sessions(digest,csrf,expires_at_ms) VALUES (?,'csrf',?)")
            .bind(auth::digest(&cookie))
            .bind(auth::now_ms() + 60000)
            .execute(&state.private_db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO oauth_states(digest,session_digest,expires_at_ms) VALUES (?,?,?)")
            .bind(auth::digest(&secret))
            .bind(auth::digest(&cookie))
            .bind(auth::now_ms() + 60000)
            .execute(&state.private_db)
            .await
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            HeaderValue::from_str(&format!("scrobbl_session={cookie}")).unwrap(),
        );
        let result = callback(
            State(state.clone()),
            headers.clone(),
            Query(Callback {
                state: Some(secret.clone()),
                code: Some("authorization-code".into()),
                error: None,
            }),
        )
        .await;
        assert!(result.is_ok());
        let replay = callback(
            State(state),
            headers,
            Query(Callback {
                state: Some(secret),
                code: Some("authorization-code".into()),
                error: None,
            }),
        )
        .await;
        assert!(matches!(replay, Err(AppError::Forbidden)));
        assert_eq!(mock.calls.lock().await.len(), 1);
        handle.abort();
    }
    #[tokio::test]
    async fn now_playing_interleaves_with_paginated_collection_and_reports_separate_clocks() {
        let at = 1_749_000_000_000;
        let batch: Vec<_> = (0..50).map(|i| item(at - i * 180000)).collect();
        let (_dir,state,mock,handle)=fixture(vec![
            reply("/v1/me/player/recently-played",json!({"items":batch,"_test_delay_ms":100})),
            reply("/v1/me/player/currently-playing",json!({"is_playing":true,"currently_playing_type":"track","timestamp":at,"item":track(),"progress_ms":1000,"_test_delay_ms":20})),
            reply("/v1/me/player/recently-played",json!({"items":[item(at-50*180000)]})),
        ],false).await;
        let headers = owner_headers(&state).await;
        let collector = state.clone();
        let collecting = tokio::spawn(async move { sync(&collector).await });
        while mock.calls.lock().await.is_empty() {
            tokio::task::yield_now().await;
        }
        let sample = now_playing(State(state.clone()), headers).await.unwrap().0;
        assert_eq!(
            mock.calls.lock().await[1].1,
            "/v1/me/player/currently-playing"
        );
        assert_eq!(sample["source_timestamp_ms"], at);
        let requested = sample["requested_at_ms"].as_i64().unwrap();
        let sent = sample["source_requested_at_ms"].as_i64().unwrap();
        let received = sample["response_received_at_ms"].as_i64().unwrap();
        assert!(requested <= sent && sent + 15 <= received);
        assert_eq!(collecting.await.unwrap().unwrap()["added"], 51);
        let activity: crate::jobs::Job =
            sqlx::query_as("SELECT * FROM jobs WHERE kind='spotify_collection'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(activity.state, "completed");
        assert_eq!(activity.processed, 51);
        let originals: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM source_occurrences WHERE job_id=?")
                .bind(activity.id)
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(originals, 51);
        println!(
            "simulated now-playing queue={} ms, provider={} ms, source={} ms",
            sent - requested,
            received - sent,
            at
        );
        handle.abort();
        state.storage.close().await;
    }
    #[tokio::test]
    async fn a_new_poll_window_refreshes_even_when_previous_sample_is_younger_than_the_interval() {
        let (_dir,state,mock,handle)=fixture(vec![reply("/v1/me/player/currently-playing",json!({"is_playing":true,"currently_playing_type":"track","item":track(),"timestamp":1234}))],false).await;
        let headers = owner_headers(&state).await;
        let bucket = auth::now_ms() / 30000 * 30000;
        *state.spotify.cache.lock().await = Some((bucket - 1, json!({"title":"stale sample"})));
        let sample = now_playing(State(state.clone()), headers).await.unwrap().0;
        assert_eq!(sample["title"], "Test track");
        assert_eq!(mock.calls.lock().await.len(), 1);
        handle.abort();
        state.storage.close().await;
    }
    #[tokio::test]
    async fn caches_now_playing_and_never_inserts_a_listen_from_it() {
        let (_dir,state,mock,handle)=fixture(vec![reply("/v1/me/player/currently-playing",json!({"is_playing":true,"currently_playing_type":"track","item":track(),"progress_ms":1000}))],false).await;
        let cookie = auth::random_secret();
        sqlx::query("INSERT INTO sessions(digest,csrf,expires_at_ms) VALUES (?,'csrf',?)")
            .bind(auth::digest(&cookie))
            .bind(auth::now_ms() + 60000)
            .execute(&state.private_db)
            .await
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            HeaderValue::from_str(&format!("scrobbl_session={cookie}")).unwrap(),
        );
        let first = now_playing(State(state.clone()), headers.clone())
            .await
            .unwrap();
        assert_eq!(first.0["title"], "Test track");
        let _ = now_playing(State(state.clone()), headers).await.unwrap();
        assert_eq!(mock.calls.lock().await.len(), 1);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM listens")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(count, 0);
        handle.abort();
    }
}
