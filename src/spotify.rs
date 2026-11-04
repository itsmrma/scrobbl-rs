use crate::{
    AppState, auth,
    db::{self, Artist, Event, Track},
    error::{AppError, Result},
    imports::spotify_uri,
};
use axum::{
    Json,
    extract::{Path, Query, State},
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
    pub(crate) collection_gate: Arc<Mutex<()>>,
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
    pub(crate) fn wake_collection(&self) {
        self.wake.notify_one();
    }
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
pub(crate) const RESET_STATUS_SQL: &str = "DELETE FROM settings WHERE key IN ('spotify_error','retry_at_ms','now_playing_retry_at_ms','collection_error','now_playing_error','now_playing_timing','spotify_action_required','spotify_verified_at_ms','last_attempt_ms','last_sync_ms','last_collection_finished_ms','last_sync_added','spotify_rate_limit_reason')";

async fn credentials(state: &AppState) -> Result<Option<(String, String)>> {
    if let (Some(id), Some(secret)) = (&state.config.env_client_id, &state.config.env_client_secret)
    {
        return Ok(Some((id.clone(), secret.clone())));
    }
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM settings WHERE key IN ('spotify_client_id', 'spotify_client_secret')",
    )
    .fetch_all(&state.private_db)
    .await?;
    let mut id = None;
    let mut secret = None;
    for (key, value) in rows {
        match key.as_str() {
            "spotify_client_id" => id = Some(value),
            "spotify_client_secret" => secret = Some(value),
            _ => {}
        }
    }
    Ok(id.zip(secret))
}
async fn diagnostic(state: &AppState, key: &str, value: &str) -> Result<()> {
    let mut conn = state.db.acquire().await?;
    db::set_setting(&mut conn, key, value).await
}
async fn diagnostics(state: &AppState, entries: &[(&str, &str)]) -> Result<()> {
    let mut tx = state.db.begin().await?;
    for (key, value) in entries {
        db::set_setting(&mut tx, key, value).await?;
    }
    tx.commit().await?;
    Ok(())
}
async fn intervals(state: &AppState) -> Result<crate::dto::SpotifyIntervals> {
    match db::setting(&state.read_db, "spotify_intervals").await? {
        Some(value) => serde_json::from_str(&value).map_err(|_| AppError::Internal),
        None => Ok(crate::dto::SpotifyIntervals {
            sync_seconds: state.config.sync_seconds,
            now_playing_seconds: state.config.now_playing_seconds,
        }),
    }
}

pub async fn save_intervals(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<crate::dto::SpotifyIntervals>,
) -> Result<Json<crate::dto::SpotifyIntervals>> {
    auth::require_write(&state, &headers).await?;
    if !(60..=86400).contains(&form.sync_seconds)
        || !(form.now_playing_seconds == 0 || (15..=86400).contains(&form.now_playing_seconds))
    {
        return Err(AppError::BadRequest(
            "Use 60–86400 seconds for collection and 0 or 15–86400 seconds for now playing.".into(),
        ));
    }
    let _now = state.spotify.now_gate.lock().await;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision")
        .fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::Conflict("Archive is in maintenance.".into()))?;
    db::set_setting(
        &mut tx,
        "spotify_intervals",
        &serde_json::to_string(&form).map_err(|_| AppError::Internal)?,
    )
    .await?;
    tx.commit().await?;
    *state.spotify.cache.lock().await = None;
    state.spotify.wake.notify_one();
    Ok(Json(form))
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
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM settings WHERE key IN (
            'spotify_intervals',
            'spotify_verified_at_ms',
            'retry_at_ms',
            'now_playing_retry_at_ms',
            'spotify_action_required',
            'spotify_error',
            'collection_error',
            'last_attempt_ms',
            'last_sync_ms',
            'last_sync_added',
            'now_playing_error',
            'now_playing_timing',
            'coverage_warning',
            'spotify_rate_limit_reason'
        )",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut map = std::collections::HashMap::with_capacity(rows.len());
    for (key, value) in rows {
        if !value.is_empty() {
            map.insert(key, value);
        }
    }
    fn timestamp(map: &std::collections::HashMap<String, String>, key: &str) -> Option<i64> {
        map.get(key).and_then(|v| v.parse::<i64>().ok())
    }
    let intervals = match map.get("spotify_intervals") {
        Some(value) => serde_json::from_str::<crate::dto::SpotifyIntervals>(value)
            .map_err(|_| AppError::Internal)?,
        None => crate::dto::SpotifyIntervals {
            sync_seconds: state.config.sync_seconds,
            now_playing_seconds: state.config.now_playing_seconds,
        },
    };
    let verified_at_ms = timestamp(&map, "spotify_verified_at_ms");
    let retry_at_ms = timestamp(&map, "retry_at_ms");
    let required = map.get("spotify_action_required").cloned();
    let last_error = map.remove("spotify_error");
    let collection_error = map.remove("collection_error");
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
    Ok(Json(crate::dto::SpotifyStatus {
        configured,
        connected,
        credentials_from_env: state.config.env_client_id.is_some(),
        redirect_uri: state.config.redirect_uri(),
        sync_seconds: intervals.sync_seconds,
        now_playing_seconds: intervals.now_playing_seconds,
        connection: connection.into(),
        collection: collection.into(),
        last_attempt_ms: timestamp(&map, "last_attempt_ms"),
        last_sync_ms: timestamp(&map, "last_sync_ms"),
        last_added: timestamp(&map, "last_sync_added"),
        verified_at_ms,
        last_error: collection_error.or(last_error),
        now_playing_error: map.remove("now_playing_error"),
        now_playing_timing: map
            .get("now_playing_timing")
            .and_then(|v| serde_json::from_str(v).ok()),
        coverage_warning: map.remove("coverage_warning"),
        retry_at_ms,
        rate_limit_reason: map.remove("spotify_rate_limit_reason"),
        review_count: sqlx::query_scalar(
            "SELECT COUNT(*) FROM listens WHERE needs_review=1 AND trashed_at_ms IS NULL",
        )
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
    if !state.config.spotify_redirect_allowed() {
        return Err(AppError::BadRequest("Spotify requires HTTPS on a private server. Follow the HTTPS guide in Connections, configure SCROBBL_EXTERNAL_ORIGIN, then sign in at that address.".into()));
    }
    let _gate = state.spotify.gate.lock().await;
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
    Ok(Json(
        json!({"url":url.to_string(), "attempt":auth::digest(&secret)}),
    ))
}

pub async fn connect_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt): Path<String>,
) -> Result<Json<Value>> {
    let session = auth::session(&state, &headers).await?;
    let row = sqlx::query("SELECT outcome,message,expires_at_ms FROM oauth_states WHERE digest=? AND session_digest=?")
        .bind(attempt).bind(session.digest).fetch_optional(&state.private_db).await?
        .ok_or(AppError::Forbidden)?;
    let outcome: String = row.get("outcome");
    if outcome == "pending" && row.get::<i64, _>("expires_at_ms") <= auth::now_ms() {
        return Ok(Json(
            json!({"outcome":"error", "message":"This connection attempt expired. Please try again."}),
        ));
    }
    Ok(Json(
        json!({"outcome":outcome,"message":row.get::<Option<String>,_>("message")}),
    ))
}

pub async fn cancel_connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(attempt): Path<String>,
) -> Result<Json<Value>> {
    let session = auth::require_write(&state, &headers).await?;
    let _gate = state.spotify.gate.lock().await;
    sqlx::query("UPDATE oauth_states SET outcome='error',message='Connection cancelled. You can try again.' WHERE digest=? AND session_digest=? AND outcome='pending'")
        .bind(attempt).bind(session.digest).execute(&state.private_db).await?;
    Ok(Json(json!({"ok":true})))
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
        "UPDATE oauth_states SET outcome='processing' WHERE digest = ? AND session_digest = ? AND expires_at_ms > ? AND outcome='pending'",
    )
    .bind(auth::digest(&secret))
    .bind(session.digest)
    .bind(auth::now_ms())
    .execute(&state.private_db)
    .await?;
    if used.rows_affected() != 1 {
        return Err(AppError::Forbidden);
    }
    let attempt = auth::digest(&secret);
    let result = if query.error.is_some() {
        Err(AppError::Spotify(
            "Spotify authorization was declined. You can try connecting again.".into(),
        ))
    } else {
        exchange_code(&state, query.code).await
    };
    let (outcome, message) = match result {
        Ok(()) => ("connected", None),
        Err(error) => ("error", Some(error.to_string())),
    };
    sqlx::query("UPDATE oauth_states SET outcome=?,message=? WHERE digest=?")
        .bind(outcome)
        .bind(&message)
        .bind(&attempt)
        .execute(&state.private_db)
        .await?;
    diagnostic(&state, "spotify_error", message.as_deref().unwrap_or("")).await?;
    if outcome == "connected" {
        state.spotify.wake.notify_one();
    }
    Ok(Redirect::to(&format!("/spotify/connect-result/{attempt}")))
}

async fn exchange_code(state: &AppState, code: Option<String>) -> Result<()> {
    let code = code.ok_or_else(|| AppError::BadRequest("Missing authorization code.".into()))?;
    let (id, secret) = credentials(state)
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
    let value = checked_json(state, response).await?;
    save_tokens(state, &value, None).await
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
    if previous_refresh.is_none() {
        sqlx::query(RESET_STATUS_SQL).execute(&state.db).await?;
    }
    Ok(())
}
pub(crate) fn parse_retry_after(header: &str, now_ms: i64) -> Option<i64> {
    let trimmed = header.trim();
    if let Ok(seconds) = trimmed.parse::<i64>() {
        return Some(seconds);
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(trimmed) {
        return Some(dt.timestamp() - (now_ms / 1000));
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(trimmed, "%a, %d %b %Y %H:%M:%S GMT") {
        return Some(dt.and_utc().timestamp() - (now_ms / 1000));
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(trimmed, "%a, %e %b %Y %H:%M:%S GMT") {
        return Some(dt.and_utc().timestamp() - (now_ms / 1000));
    }
    None
}

pub(crate) fn extract_rate_limit_reason(body: &Value) -> Option<&str> {
    let raw = body
        .get("error")
        .and_then(|e| e.get("reason"))
        .and_then(Value::as_str)
        .or_else(|| body.get("reason").and_then(Value::as_str))
        .map(str::trim);
    match raw {
        Some(s) if s.eq_ignore_ascii_case("QUOTA_EXCEEDED") => Some("QUOTA_EXCEEDED"),
        _ => None,
    }
}

async fn handle_rate_limit_response(
    state: &AppState,
    response: reqwest::Response,
    is_now_playing: bool,
) -> Result<Value> {
    let now = auth::now_ms();
    let wait = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| parse_retry_after(v, now))
        .unwrap_or(300)
        .clamp(1, 86_400);
    let proposed_at = now + wait * 1000;
    let body: Option<Value> = response.json().await.ok();
    let new_reason = body
        .as_ref()
        .and_then(extract_rate_limit_reason)
        .unwrap_or("");
    // Collection and now playing share one cooldown. A later 429 may arrive
    // while an earlier, longer pause is still active; it must never shorten it.
    let active_at = [
        db::setting(&state.db, "retry_at_ms").await?,
        db::setting(&state.db, "now_playing_retry_at_ms").await?,
    ]
    .into_iter()
    .flatten()
    .filter_map(|value| value.parse::<i64>().ok())
    .filter(|value| *value > now)
    .max();
    let (retry_at, reason) = match active_at {
        Some(active) => {
            let existing_reason = db::setting(&state.db, "spotify_rate_limit_reason")
                .await?
                .unwrap_or_default();
            let reason = if new_reason == "QUOTA_EXCEEDED" || existing_reason == "QUOTA_EXCEEDED" {
                "QUOTA_EXCEEDED".to_string()
            } else {
                new_reason.to_string()
            };
            (active.max(proposed_at), reason)
        }
        None => (proposed_at, new_reason.to_string()),
    };
    let remaining = ((retry_at - now + 999) / 1000).max(1);
    let error_msg = if is_now_playing {
        format!("Spotify player request limit reached. Retrying in {remaining} seconds.")
    } else {
        format!("Spotify request limit reached. Retrying in {remaining} seconds.")
    };
    let retry_at_string = retry_at.to_string();
    let mut entries = vec![
        ("retry_at_ms", retry_at_string.as_str()),
        ("now_playing_retry_at_ms", retry_at_string.as_str()),
        ("spotify_rate_limit_reason", reason.as_str()),
        // The last request did not succeed, so the badge must not claim that
        // authorization was verified by it. A later success sets it again.
        ("spotify_verified_at_ms", ""),
    ];
    if is_now_playing {
        entries.push(("now_playing_error", error_msg.as_str()));
    }
    diagnostics(state, &entries).await?;
    Err(AppError::Spotify(error_msg))
}

async fn checked_json(state: &AppState, response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS {
        return handle_rate_limit_response(state, response, false).await;
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
async fn checked_json_now_playing(state: &AppState, response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS {
        return handle_rate_limit_response(state, response, true).await;
    }
    if matches!(status.as_u16(), 400 | 401 | 403) {
        diagnostic(state, "spotify_action_required", "true").await?;
    }
    if !status.is_success() && status != StatusCode::NO_CONTENT {
        let message = match status.as_u16() {
            400 | 401 => {
                "Spotify authorization expired or was rejected. Please reconnect your account."
            }
            403 => {
                "Spotify denied access. Check Premium, the app allowlist and granted permissions."
            }
            _ => "Spotify player is temporarily unavailable.",
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
    let (value, _, _) = api_get_locked(state, path, params).await?;
    Ok(value)
}
async fn api_get_locked(
    state: &AppState,
    path: &str,
    params: &[(&str, String)],
) -> Result<(Value, i64, i64)> {
    let is_now_playing = path == "/me/player/currently-playing";
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
    let value = if is_now_playing {
        checked_json_now_playing(state, response).await?
    } else {
        checked_json(state, response).await?
    };
    let response_received_at_ms = auth::now_ms();
    diagnostics(
        state,
        &[
            ("spotify_verified_at_ms", &auth::now_ms().to_string()),
            ("spotify_action_required", ""),
            ("spotify_rate_limit_reason", ""),
        ],
    )
    .await?;
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
        duration_ms: row
            .get("duration_ms")
            .and_then(Value::as_i64)
            .filter(|v| *v >= 0),
        artists,
        structured: true,
    })
}
const SYNC_COALESCE_MS: i64 = 15_000;
const COALESCED_SYNC_MESSAGE: &str =
    "A collection attempt finished just now. Please wait a few seconds.";

async fn recently_synchronized(state: &AppState) -> Result<bool> {
    // Finished runs, successful or failed, share one short window so that a
    // waiting manual click or worker wake cannot start a second collection.
    Ok(db::setting(&state.db, "last_collection_finished_ms")
        .await?
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|at| auth::now_ms().saturating_sub(at) < SYNC_COALESCE_MS))
}

pub async fn sync(state: &AppState) -> Result<Value> {
    let _collection = state.spotify.collection_gate.lock().await;
    if recently_synchronized(state).await? {
        return Ok(json!({"skipped": "recently_synchronized"}));
    }
    sync_guarded(state).await
}

#[cfg(test)]
async fn sync_force(state: &AppState) -> Result<Value> {
    let _collection = state.spotify.collection_gate.lock().await;
    sync_guarded(state).await
}

async fn sync_guarded(state: &AppState) -> Result<Value> {
    if state.storage.directory.join("recovery.pending").exists()
        || crate::recovery::paused(&state.storage).await
    {
        return Err(AppError::Conflict(
            "Collection is paused after restore. Review Settings, then resume in Data & recovery."
                .into(),
        ));
    }
    if let Some(retry) = db::setting(&state.db, "retry_at_ms")
        .await?
        .and_then(|v| v.parse::<i64>().ok())
        && retry > auth::now_ms()
    {
        return Err(AppError::Spotify(
            "Waiting for Spotify's request limit to reset.".into(),
        ));
    }
    let intervals = intervals(state).await?;
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
    // Coalescing uses the end of the run, so a long collection still protects
    // a waiting manual click or worker wake, even when the run failed.
    let _ = diagnostic(
        state,
        "last_collection_finished_ms",
        &auth::now_ms().to_string(),
    )
    .await;
    if let Err(ref error) = result {
        if state.storage.job(&job.id).await?.state == "cancelled" {
            return Err(AppError::Conflict("Collection cancelled. Saved listens and the previous cursor are kept; future scheduled collections remain enabled.".into()));
        }
        diagnostic(state, "collection_error", &error.to_string()).await?;
        let retry_opt = db::setting(&state.db, "retry_at_ms")
            .await?
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v > auth::now_ms());
        let reason = db::setting(&state.db, "spotify_rate_limit_reason").await?;
        let retry = retry_opt.unwrap_or(auth::now_ms() + intervals.sync_seconds as i64 * 1000);
        let error_msg = if retry_opt.is_some() {
            if reason.as_deref() == Some("QUOTA_EXCEEDED") {
                "spotify_quota_exceeded: Spotify developer account quota exceeded".to_string()
            } else {
                let wait = ((retry - auth::now_ms() + 999) / 1000).max(1);
                format!(
                    "spotify_rate_limited: Spotify request limit reached. Retrying in {wait} seconds."
                )
            }
        } else {
            "spotify_collection_failed: see Connections for authorization, request limit or network details".to_string()
        };
        state.storage.pause(&job, Some(retry), &error_msg).await?;
    }
    result
}
async fn sync_locked(state: &AppState, job: &crate::jobs::Job) -> Result<Value> {
    if let Some(retry) = db::setting(&state.db, "retry_at_ms")
        .await?
        .and_then(|v| v.parse::<i64>().ok())
        && retry > auth::now_ms()
    {
        return Err(AppError::Spotify(
            "Waiting for Spotify's request limit to reset.".into(),
        ));
    }
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
        if page > 0 {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let mut checkpoint = state.db.begin().await?;
        crate::jobs::validate_lease(&mut checkpoint, job).await?;
        checkpoint.commit().await?;
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
    db::set_setting(&mut tx, "spotify_rate_limit_reason", "").await?;
    db::set_setting(&mut tx, "last_sync_added", &report.added.to_string()).await?;
    if let Some(warning) = warning {
        db::set_setting(&mut tx, "coverage_warning", warning).await?;
    }
    tx.commit().await?;
    Ok(json!(report))
}
pub async fn sync_now(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    let result = {
        // Coalesce after acquiring the collection gate so that concurrent
        // browser windows and the background worker cannot start two runs.
        let _collection = state.spotify.collection_gate.lock().await;
        if recently_synchronized(&state).await? {
            return Err(AppError::Conflict(COALESCED_SYNC_MESSAGE.into()));
        }
        sync_guarded(&state).await
    };
    if let Err(ref e) = result {
        diagnostic(&state, "spotify_error", &e.to_string()).await?;
    }
    result.map(Json)
}
pub async fn retry_now(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    let mut tx = state.db.begin().await?;
    // A manual retry is an explicit override of the local pause. It cannot
    // reset Spotify's developer quota, so the last real diagnosis stays
    // visible until a request actually succeeds.
    sqlx::query("DELETE FROM settings WHERE key IN ('retry_at_ms', 'now_playing_retry_at_ms', 'last_collection_finished_ms')")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE jobs SET retry_at_ms=0 WHERE kind='spotify_collection' AND state='waiting'",
    )
    .execute(&mut *tx)
    .await?;
    // The old messages may contain a countdown that is no longer running.
    let waiting = "Retry requested. Waiting for Spotify to accept a request.";
    db::set_setting(&mut tx, "collection_error", waiting).await?;
    db::set_setting(&mut tx, "now_playing_error", waiting).await?;
    tx.commit().await?;
    *state.spotify.cache.lock().await = None;
    state.spotify.wake_collection();
    Ok(Json(serde_json::json!({
        "status": "retry_requested",
        "message": "Retry requested. This does not reset your Spotify quota; if Spotify is still limiting requests, the cooldown will return."
    })))
}
pub async fn now_playing(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    now_playing_in_window(state, headers, None).await
}

async fn local_now_playing_links(
    db: &sqlx::SqlitePool,
    track_external: Option<&str>,
    album_external: Option<&str>,
) -> Result<(Option<String>, Option<String>)> {
    let mut track_id = None;
    let mut album_group_id = None;
    if let Some(external) = track_external
        && let Some((entity_id, group_id)) = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT l.entity_id, m.group_id
             FROM entity_links l
             LEFT JOIN tracks t ON t.id = l.entity_id
             LEFT JOIN name_group_members m ON m.kind = 'albums' AND m.entity_id = t.album_id
             WHERE l.kind = 'track' AND l.namespace = 'spotify' AND l.external_id = ?
             LIMIT 1",
        )
        .bind(external)
        .fetch_optional(db)
        .await?
    {
        track_id = Some(entity_id);
        album_group_id = group_id;
    }
    if album_group_id.is_none()
        && let Some(album_ext) = album_external
    {
        album_group_id = sqlx::query_scalar(
            "SELECT m.group_id
             FROM entity_links l
             JOIN name_group_members m ON m.kind = 'albums' AND m.entity_id = l.entity_id
             WHERE l.kind = 'album' AND l.namespace = 'spotify' AND l.external_id = ?
             LIMIT 1",
        )
        .bind(album_ext)
        .fetch_optional(db)
        .await?;
    }
    Ok((
        track_id,
        album_group_id.map(|id| format!("/api/v1/artwork/albums/{id}")),
    ))
}

// A fixed poll time lets cache tests exercise the same/new window independently
// of the wall clock crossing a boundary while the HTTP/SQLite calls run.
async fn now_playing_in_window(
    state: AppState,
    headers: HeaderMap,
    poll_at_ms: Option<i64>,
) -> Result<Json<Value>> {
    auth::session(&state, &headers).await?;
    if state.storage.directory.join("recovery.pending").exists()
        || crate::recovery::paused(&state.storage).await
    {
        return Ok(Json(Value::Null));
    }
    let intervals = intervals(&state).await?;
    if intervals.now_playing_seconds == 0 {
        return Ok(Json(Value::Null));
    }
    let requested_at_ms = auth::now_ms();
    let _now = state.spotify.now_gate.lock().await;
    let interval = intervals.now_playing_seconds as i64 * 1000;
    let now = poll_at_ms.unwrap_or(requested_at_ms);
    if let Some((time, value)) = &*state.spotify.cache.lock().await {
        let ttl = if value.is_null() {
            interval.max(60_000)
        } else {
            interval
        };
        if now.saturating_sub(*time) < ttl {
            let mut value = value.clone();
            let external = value["spotify_url"]
                .as_str()
                .and_then(|url| url.strip_prefix("https://open.spotify.com/track/"))
                .map(|id| format!("spotify:track:{id}"));
            let (track_id, cover_url) =
                local_now_playing_links(&state.read_db, external.as_deref(), None).await?;
            if let Some(object) = value.as_object_mut() {
                object.insert("served_at_ms".into(), json!(auth::now_ms()));
                object.insert("track_id".into(), json!(track_id));
                if cover_url.is_some() || !object.contains_key("cover_url") {
                    object.insert("cover_url".into(), json!(cover_url));
                }
            }
            return Ok(Json(value));
        }
    }
    if let Some(retry) = db::setting(&state.db, "now_playing_retry_at_ms")
        .await?
        .and_then(|v| v.parse::<i64>().ok())
        && retry > now
    {
        return Ok(Json(Value::Null));
    }
    if let Some(retry) = db::setting(&state.db, "retry_at_ms")
        .await?
        .and_then(|v| v.parse::<i64>().ok())
        && retry > now
    {
        return Ok(Json(Value::Null));
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
                let (track_id, cover_url) = local_now_playing_links(
                    &state.read_db,
                    Some(&track.id),
                    track.album_id.as_deref(),
                )
                .await?;
                json!({"track_id":track_id,"title":track.title,"artists":track.artists,"album":track.album,"cover_url":cover_url,"spotify_url":format!("https://open.spotify.com/track/{}",track.id.trim_start_matches("spotify:track:")),"duration_ms":track.duration_ms,"progress_ms":row["progress_ms"],"observed_at_ms":response_received_at_ms,"source_timestamp_ms":row["timestamp"].as_i64(),"requested_at_ms":requested_at_ms,"source_requested_at_ms":source_requested_at_ms,"response_received_at_ms":response_received_at_ms,"served_at_ms":auth::now_ms()})
            }
            Err(_) => Value::Null,
        }
    } else {
        Value::Null
    };
    let timing = json!({
        "requested_at_ms": requested_at_ms,
        "source_requested_at_ms": source_requested_at_ms,
        "response_received_at_ms": response_received_at_ms,
        "source_timestamp_ms": row["timestamp"].as_i64(),
    })
    .to_string();
    diagnostics(
        &state,
        &[("now_playing_timing", &timing), ("now_playing_error", "")],
    )
    .await?;
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
        if connected
            && !crate::recovery::paused(&state.storage).await
            && !state.storage.directory.join("recovery.pending").exists()
            && let Err(error) = sync(&state).await
        {
            let _ = diagnostic(&state, "spotify_error", &error.to_string()).await;
        }
        let seconds = intervals(&state)
            .await
            .map(|value| value.sync_seconds)
            .unwrap_or(state.config.sync_seconds);
        tokio::select! { _ = tokio::time::sleep(Duration::from_secs(seconds)) => {}, _ = state.spotify.wake.notified() => {} }
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
        fixture_with_intervals(replies, expired, 120, 30).await
    }
    async fn fixture_with_intervals(
        replies: Vec<Reply>,
        expired: bool,
        sync_seconds: u64,
        now_playing_seconds: u64,
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
            bind_ip: "127.0.0.1".parse().unwrap(),
            external_origin: None,
            port: 3210,
            data_dir: dir.path().to_owned(),
            sync_seconds,
            now_playing_seconds,
            import_limits: Default::default(),
            api_snapshot_limits: Default::default(),
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
    async fn wait_for_calls(mock: &Mock, expected: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while mock.calls.lock().await.len() < expected {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {expected} Spotify call(s)"));
    }
    async fn wait_for_setting(state: &AppState, key: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while db::setting(&state.db, key).await.unwrap().is_none() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for setting {key}"));
    }
    async fn call_count(mock: &Mock, path: &str) -> usize {
        mock.calls
            .lock()
            .await
            .iter()
            .filter(|(_, uri)| uri.split('?').next() == Some(path))
            .count()
    }
    fn replay_base_urls(from: &AppState) -> (String, String) {
        (
            from.spotify.api_base.clone(),
            from.spotify.token_url.clone(),
        )
    }
    #[test]
    fn parse_track_ignores_spotify_image_urls_and_keeps_structured_metadata() {
        let mut row = track();
        row["album"]["images"] = json!([
            {"url": "https://i.scdn.co/image/full", "width": 640},
            {"url": "https://i.scdn.co/image/thumb", "width": 64}
        ]);
        let parsed = parse_track(&row).unwrap();
        assert_eq!(parsed.id, "spotify:track:0000000000000000000001");
        assert_eq!(parsed.title, "Test track");
        assert_eq!(parsed.album_id.as_deref(), Some("spotify:album:album1"));
        assert_eq!(parsed.album.as_deref(), Some("An album"));
        assert_eq!(parsed.artists.len(), 1);
        assert!(parsed.structured);
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
    async fn saved_intervals_survive_restart_override_environment_defaults_and_reset_the_cache() {
        let (dir, state, _mock, handle) = fixture(
            vec![reply(
                "/v1/me/player/currently-playing",
                json!({"is_playing":true,"currently_playing_type":"track","item":track()}),
            )],
            false,
        )
        .await;
        let mut headers = owner_headers(&state).await;
        headers.insert("x-csrf-token", HeaderValue::from_static("csrf"));
        *state.spotify.cache.lock().await = Some((auth::now_ms(), json!({"title":"old sample"})));
        let configured = crate::dto::SpotifyIntervals {
            sync_seconds: 60,
            now_playing_seconds: 15,
        };
        assert_eq!(
            save_intervals(State(state.clone()), headers.clone(), Json(configured))
                .await
                .unwrap()
                .0,
            configured
        );
        assert!(state.spotify.cache.lock().await.is_none());
        let shown = status(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!((shown.sync_seconds, shown.now_playing_seconds), (60, 15));
        assert_eq!(
            now_playing(State(state.clone()), headers).await.unwrap().0["title"],
            "Test track"
        );
        let mut config = state.config.clone();
        config.sync_seconds = 600;
        config.now_playing_seconds = 300;
        state.storage.close().await;
        let store = db::open(dir.path()).await.unwrap();
        let restarted = AppState::new(store, config);
        assert_eq!(intervals(&restarted).await.unwrap(), configured);
        restarted.storage.close().await;
        handle.abort();
    }

    #[tokio::test]
    async fn interval_updates_validate_authorization_ranges_and_maintenance_and_wake_collection() {
        let (_dir, state, mock, handle) = fixture(
            vec![
                reply("/v1/me/player/recently-played", json!({"items":[]})),
                reply("/v1/me/player/recently-played", json!({"items":[]})),
            ],
            false,
        )
        .await;
        let mut headers = owner_headers(&state).await;
        let configured = crate::dto::SpotifyIntervals {
            sync_seconds: 60,
            now_playing_seconds: 15,
        };
        assert!(matches!(
            save_intervals(State(state.clone()), headers.clone(), Json(configured)).await,
            Err(AppError::Forbidden)
        ));
        assert!(matches!(
            save_intervals(State(state.clone()), HeaderMap::new(), Json(configured)).await,
            Err(AppError::Unauthorized)
        ));
        headers.insert("x-csrf-token", HeaderValue::from_static("csrf"));
        for (sync_seconds, now_playing_seconds) in [(59, 15), (60, 14), (86401, 15), (60, 86401)] {
            assert!(matches!(
                save_intervals(
                    State(state.clone()),
                    headers.clone(),
                    Json(crate::dto::SpotifyIntervals {
                        sync_seconds,
                        now_playing_seconds
                    })
                )
                .await,
                Err(AppError::BadRequest(_))
            ));
        }
        assert!(
            db::setting(&state.db, "spotify_intervals")
                .await
                .unwrap()
                .is_none()
        );
        sqlx::query("UPDATE archive_state SET maintenance=1")
            .execute(&state.db)
            .await
            .unwrap();
        assert!(matches!(
            save_intervals(State(state.clone()), headers.clone(), Json(configured)).await,
            Err(AppError::Conflict(_))
        ));
        sqlx::query("UPDATE archive_state SET maintenance=0")
            .execute(&state.db)
            .await
            .unwrap();
        let collecting = tokio::spawn(worker(state.clone()));
        wait_for_calls(&mock, 1).await;
        wait_for_setting(&state, "last_sync_ms").await;
        // Let the coalescing window of the just-finished collection pass; the
        // save must still wake the worker into a real collection.
        auth::advance_clock(SYNC_COALESCE_MS + 1_000);
        let _ = save_intervals(State(state.clone()), headers, Json(configured))
            .await
            .unwrap();
        wait_for_calls(&mock, 2).await;
        collecting.abort();
        let _ = collecting.await;
        handle.abort();
        state.storage.close().await;
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
        assert_eq!(sync_force(&restarted).await.unwrap()["duplicates"], 1);
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
        // The next cadence window must hit the persisted cooldown, not coalesce.
        auth::advance_clock(SYNC_COALESCE_MS + 1_000);
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
    async fn collection_resumes_after_persisted_rate_limit_without_losing_or_duplicating_listens() {
        let at = 1_749_000_000_000;
        let (dir, state, mock, handle) = fixture(
            vec![
                reply("/v1/me/player/recently-played", json!({"items":[item(at)]})),
                Reply {
                    path: "/v1/me/player/recently-played",
                    status: StatusCode::TOO_MANY_REQUESTS,
                    body: json!({}),
                    retry: Some("72000"),
                },
                reply(
                    "/v1/me/player/recently-played",
                    json!({"items":[item(at+180_000),item(at)]}),
                ),
                reply(
                    "/v1/me/player/recently-played",
                    json!({"items":[item(at+180_000),item(at)]}),
                ),
            ],
            false,
        )
        .await;
        sync(&state).await.unwrap();
        // The next background cadence window.
        auth::advance_clock(SYNC_COALESCE_MS + 1_000);
        assert!(sync(&state).await.is_err());
        assert_eq!(
            db::setting(&state.db, "recent_cursor_ms").await.unwrap(),
            Some(at.to_string())
        );
        let retry: i64 = db::setting(&state.db, "retry_at_ms")
            .await
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        assert!((retry - auth::now_ms() - 72_000_000).abs() < 1000);
        // Past the coalescing window, the restarted archive must stop at the
        // persisted pause instead of coalescing.
        auth::advance_clock(SYNC_COALESCE_MS + 1_000);
        let api = state.spotify.clone();
        let config = state.config.clone();
        state.storage.close().await;
        let mut restarted = AppState::new(db::open(dir.path()).await.unwrap(), config);
        restarted.spotify = api;
        assert!(sync(&restarted).await.is_err());
        assert_eq!(mock.calls.lock().await.len(), 2);
        assert_eq!(
            db::setting(&restarted.db, "retry_at_ms").await.unwrap(),
            Some(retry.to_string())
        );
        // Only the test archive's clocks advance; a real provider wait is never cleared.
        diagnostic(&restarted, "retry_at_ms", "0").await.unwrap();
        sqlx::query(
            "UPDATE jobs SET retry_at_ms=0 WHERE kind='spotify_collection' AND state='waiting'",
        )
        .execute(&restarted.db)
        .await
        .unwrap();
        assert_eq!(sync(&restarted).await.unwrap()["added"], 1);
        assert_eq!(sync_force(&restarted).await.unwrap()["duplicates"], 2);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens")
                .fetch_one(&restarted.db)
                .await
                .unwrap(),
            2
        );
        assert!(
            db::setting(&restarted.db, "collection_error")
                .await
                .unwrap()
                .is_none_or(|e| e.is_empty())
        );
        handle.abort();
        restarted.storage.close().await;
    }
    #[tokio::test]
    async fn cancelling_collection_stops_before_the_next_page_and_keeps_the_cursor() {
        let at = 1_749_000_000_000;
        let batch: Vec<_> = (0..50).map(|i| item(at - i * 180_000)).collect();
        let (_dir, state, mock, handle) = fixture(
            vec![reply(
                "/v1/me/player/recently-played",
                json!({"items":batch,"_test_delay_ms":200}),
            )],
            false,
        )
        .await;
        let collector = tokio::spawn({
            let state = state.clone();
            async move { sync(&state).await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while mock.calls.lock().await.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let id: String = sqlx::query_scalar(
            "SELECT id FROM jobs WHERE kind='spotify_collection' AND state='running'",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        state.storage.cancel(&id).await.unwrap();
        assert!(collector.await.unwrap().is_err());
        assert_eq!(mock.calls.lock().await.len(), 1);
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
        let status: String = sqlx::query_scalar("SELECT state FROM jobs WHERE id=?")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(status, "cancelled");
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
        auth::advance_clock(SYNC_COALESCE_MS + 1_000);
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
    async fn paginated_collection_paces_successive_pages_with_delay() {
        let at = 1_749_000_000_000;
        let batch: Vec<_> = (0..50).map(|i| item(at - i * 180_000)).collect();
        let (_dir, state, mock, handle) = fixture(
            vec![
                reply("/v1/me/player/recently-played", json!({"items":batch})),
                reply(
                    "/v1/me/player/recently-played",
                    json!({"items":[item(at - 50 * 180_000)]}),
                ),
            ],
            false,
        )
        .await;

        let start = tokio::time::Instant::now();
        let result = sync(&state).await.unwrap();
        let elapsed = start.elapsed();

        assert_eq!(result["added"], 51);
        let calls = mock.calls.lock().await;
        assert_eq!(calls.len(), 2);
        assert!(calls[1].1.contains("before="));
        assert!(
            elapsed >= Duration::from_millis(250),
            "expected at least 250ms elapsed due to pacing, got {:?}",
            elapsed
        );
        handle.abort();
    }
    #[tokio::test]
    async fn cancellation_during_pagination_pacing_stops_before_subsequent_page() {
        let at = 1_749_000_000_000;
        let batch: Vec<_> = (0..50).map(|i| item(at - i * 180_000)).collect();
        let (_dir, state, mock, handle) = fixture(
            vec![
                reply("/v1/me/player/recently-played", json!({"items":batch})),
                reply(
                    "/v1/me/player/recently-played",
                    json!({"items":[item(at - 50 * 180_000)]}),
                ),
            ],
            false,
        )
        .await;

        let collector = tokio::spawn({
            let state = state.clone();
            async move { sync(&state).await }
        });

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while mock.calls.lock().await.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let id: String = sqlx::query_scalar(
            "SELECT id FROM jobs WHERE kind='spotify_collection' AND state='running'",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();

        state.storage.cancel(&id).await.unwrap();

        assert!(collector.await.unwrap().is_err());
        assert_eq!(
            mock.calls.lock().await.len(),
            1,
            "subsequent page must not be fetched after cancellation"
        );
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
        let status: String = sqlx::query_scalar("SELECT state FROM jobs WHERE id=?")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(status, "cancelled");
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
        assert_eq!(
            connect_status(
                State(state.clone()),
                headers.clone(),
                Path(auth::digest(&secret))
            )
            .await
            .unwrap()
            .0["outcome"],
            "connected"
        );
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
    async fn begin_connect(state: &AppState, headers: &HeaderMap) -> (String, String) {
        let mut headers = headers.clone();
        headers.insert("x-csrf-token", HeaderValue::from_static("csrf"));
        let result = connect(State(state.clone()), headers).await.unwrap().0;
        let url = url::Url::parse(result["url"].as_str().unwrap()).unwrap();
        let secret = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        (result["attempt"].as_str().unwrap().to_owned(), secret)
    }
    #[tokio::test]
    async fn oauth_rejection_and_exchange_errors_belong_to_the_attempt_not_the_saved_connection() {
        let (_dir, state, mock, handle) = fixture(
            vec![Reply {
                path: "/api/token",
                status: StatusCode::BAD_GATEWAY,
                body: json!({"error":"secret-upstream-payload"}),
                retry: None,
            }],
            false,
        )
        .await;
        let headers = owner_headers(&state).await;
        let (attempt, secret) = begin_connect(&state, &headers).await;
        assert_eq!(
            connect_status(State(state.clone()), headers.clone(), Path(attempt.clone()))
                .await
                .unwrap()
                .0["outcome"],
            "pending"
        );
        let _ = callback(
            State(state.clone()),
            headers.clone(),
            Query(Callback {
                state: Some(secret),
                code: None,
                error: Some("access_denied".into()),
            }),
        )
        .await
        .unwrap();
        let result = connect_status(State(state.clone()), headers.clone(), Path(attempt))
            .await
            .unwrap()
            .0;
        assert_eq!(result["outcome"], "error");
        assert!(result["message"].as_str().unwrap().contains("declined"));
        assert!(mock.calls.lock().await.is_empty());
        let (attempt, secret) = begin_connect(&state, &headers).await;
        let _ = callback(
            State(state.clone()),
            headers.clone(),
            Query(Callback {
                state: Some(secret),
                code: Some("code".into()),
                error: None,
            }),
        )
        .await
        .unwrap();
        let result = connect_status(State(state.clone()), headers, Path(attempt))
            .await
            .unwrap()
            .0;
        assert_eq!(result["outcome"], "error");
        assert!(!result.to_string().contains("secret-upstream-payload"));
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT access_token FROM spotify_tokens")
                .fetch_one(&state.private_db)
                .await
                .unwrap(),
            "initial-access"
        );
        handle.abort();
        state.storage.close().await;
    }
    #[tokio::test]
    async fn oauth_attempts_reject_wrong_sessions_expiry_cancellation_and_superseded_callbacks() {
        let (_dir, state, mock, handle) = fixture(vec![], false).await;
        let mut headers = owner_headers(&state).await;
        let other = owner_headers(&state).await;
        let (attempt, secret) = begin_connect(&state, &headers).await;
        assert!(matches!(
            connect_status(State(state.clone()), other.clone(), Path(attempt.clone())).await,
            Err(AppError::Forbidden)
        ));
        assert!(matches!(
            callback(
                State(state.clone()),
                other,
                Query(Callback {
                    state: Some(secret.clone()),
                    code: Some("code".into()),
                    error: None
                })
            )
            .await,
            Err(AppError::Forbidden)
        ));
        assert!(matches!(
            cancel_connect(State(state.clone()), headers.clone(), Path(attempt.clone())).await,
            Err(AppError::Forbidden)
        ));
        headers.insert("x-csrf-token", HeaderValue::from_static("csrf"));
        let _ = cancel_connect(State(state.clone()), headers.clone(), Path(attempt.clone()))
            .await
            .unwrap();
        assert_eq!(
            connect_status(State(state.clone()), headers.clone(), Path(attempt))
                .await
                .unwrap()
                .0["outcome"],
            "error"
        );
        assert!(matches!(
            callback(
                State(state.clone()),
                headers.clone(),
                Query(Callback {
                    state: Some(secret),
                    code: Some("code".into()),
                    error: None
                })
            )
            .await,
            Err(AppError::Forbidden)
        ));
        let (attempt, secret) = begin_connect(&state, &headers).await;
        sqlx::query("UPDATE oauth_states SET expires_at_ms=0 WHERE digest=?")
            .bind(&attempt)
            .execute(&state.private_db)
            .await
            .unwrap();
        assert_eq!(
            connect_status(State(state.clone()), headers.clone(), Path(attempt))
                .await
                .unwrap()
                .0["outcome"],
            "error"
        );
        assert!(matches!(
            callback(
                State(state.clone()),
                headers.clone(),
                Query(Callback {
                    state: Some(secret),
                    code: Some("code".into()),
                    error: None
                })
            )
            .await,
            Err(AppError::Forbidden)
        ));
        let (_, secret) = begin_connect(&state, &headers).await;
        begin_connect(&state, &headers).await;
        assert!(matches!(
            callback(
                State(state.clone()),
                headers,
                Query(Callback {
                    state: Some(secret),
                    code: Some("code".into()),
                    error: None
                })
            )
            .await,
            Err(AppError::Forbidden)
        ));
        assert!(mock.calls.lock().await.is_empty());
        handle.abort();
        state.storage.close().await;
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
    async fn now_playing_refreshes_when_sample_is_older_than_the_interval() {
        let (_dir,state,mock,handle)=fixture(vec![reply("/v1/me/player/currently-playing",json!({"is_playing":true,"currently_playing_type":"track","item":track(),"timestamp":1234}))],false).await;
        let headers = owner_headers(&state).await;
        let now = auth::now_ms();
        *state.spotify.cache.lock().await = Some((now - 30_001, json!({"title":"stale sample"})));
        let sample = now_playing_in_window(state.clone(), headers, Some(now))
            .await
            .unwrap()
            .0;
        assert_eq!(sample["title"], "Test track");
        assert_eq!(mock.calls.lock().await.len(), 1);
        handle.abort();
        state.storage.close().await;
    }
    #[tokio::test]
    async fn test_spotify_429_diagnosis_and_coordination() {
        let at = 1_749_000_000_000;
        let (_dir, state, mock, handle) = fixture(
            vec![
                Reply {
                    path: "/v1/me/player/currently-playing",
                    status: StatusCode::TOO_MANY_REQUESTS,
                    body: json!({
                        "error": {
                            "status": 429,
                            "message": "Rate limit exceeded",
                            "reason": "QUOTA_EXCEEDED"
                        }
                    }),
                    retry: Some("3600"),
                },
                reply("/v1/me/player/recently-played", json!({"items":[item(at)]})),
            ],
            false,
        )
        .await;
        let headers = owner_headers(&state).await;

        // 1. A 429 on currently-playing sets both retry_at_ms and now_playing_retry_at_ms
        assert!(
            now_playing(State(state.clone()), headers.clone())
                .await
                .is_err()
        );

        let retry_at_str = db::setting(&state.db, "retry_at_ms")
            .await
            .unwrap()
            .expect("retry_at_ms should be set");
        let now_playing_retry_at_str = db::setting(&state.db, "now_playing_retry_at_ms")
            .await
            .unwrap()
            .expect("now_playing_retry_at_ms should be set");
        assert_eq!(retry_at_str, now_playing_retry_at_str);

        let retry_at_ms: i64 = retry_at_str.parse().unwrap();
        assert!(retry_at_ms > auth::now_ms());

        assert_eq!(
            db::setting(&state.db, "spotify_rate_limit_reason")
                .await
                .unwrap(),
            Some("QUOTA_EXCEEDED".to_string())
        );

        let current_status = status(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(current_status.collection, "waiting");
        assert_eq!(current_status.retry_at_ms, Some(retry_at_ms));
        assert_eq!(
            current_status.rate_limit_reason.as_deref(),
            Some("QUOTA_EXCEEDED")
        );
        assert!(current_status.now_playing_error.is_some());

        // mock called once for currently-playing
        assert_eq!(mock.calls.lock().await.len(), 1);

        // 2. Subsequent sync call MUST NOT make HTTP calls to Spotify while retry_at_ms is active
        let sync_err = sync(&state).await.unwrap_err();
        assert!(
            sync_err.to_string().contains("request limit")
                || sync_err.to_string().contains("waiting")
                || sync_err.to_string().contains("Waiting")
        );
        assert_eq!(mock.calls.lock().await.len(), 1);

        // 3. Reset or expiration clears the rate limit state consistently
        sqlx::query(RESET_STATUS_SQL)
            .execute(&state.db)
            .await
            .unwrap();

        assert!(
            db::setting(&state.db, "retry_at_ms")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db::setting(&state.db, "now_playing_retry_at_ms")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db::setting(&state.db, "spotify_rate_limit_reason")
                .await
                .unwrap()
                .is_none()
        );

        let reset_status = status(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(reset_status.collection, "active");
        assert!(reset_status.retry_at_ms.is_none());
        assert!(reset_status.rate_limit_reason.is_none());
        assert!(reset_status.now_playing_error.is_none());

        // Now collection sync can proceed and perform the HTTP request
        let report = sync(&state).await.unwrap();
        assert_eq!(report["added"], 1);
        assert_eq!(mock.calls.lock().await.len(), 2);

        handle.abort();
        state.storage.close().await;
    }
    #[tokio::test]
    async fn now_playing_disabled_when_interval_is_zero() {
        let (_dir, state, mock, handle) = fixture(vec![], false).await;
        let headers = owner_headers(&state).await;
        let configured = crate::dto::SpotifyIntervals {
            sync_seconds: 60,
            now_playing_seconds: 0,
        };
        let mut save_headers = headers.clone();
        save_headers.insert("x-csrf-token", HeaderValue::from_static("csrf"));
        let _ = save_intervals(State(state.clone()), save_headers, Json(configured))
            .await
            .unwrap();
        let sample = now_playing(State(state.clone()), headers).await.unwrap().0;
        assert!(sample.is_null());
        assert_eq!(mock.calls.lock().await.len(), 0);
        handle.abort();
        state.storage.close().await;
    }
    #[tokio::test]
    async fn now_playing_caches_null_for_at_least_sixty_seconds_when_idle() {
        let (_dir, state, mock, handle) = fixture(
            vec![Reply {
                path: "/v1/me/player/currently-playing",
                status: StatusCode::NO_CONTENT,
                body: json!(null),
                retry: None,
            }],
            false,
        )
        .await;
        let headers = owner_headers(&state).await;
        let first = now_playing(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert!(first.is_null());
        assert_eq!(mock.calls.lock().await.len(), 1);
        let poll_at = auth::now_ms() + 45_000;
        let second = now_playing_in_window(state.clone(), headers, Some(poll_at))
            .await
            .unwrap()
            .0;
        assert!(second.is_null());
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
        assert!(first.0["track_id"].is_null());
        sqlx::query("INSERT INTO tracks(id,title,metadata_rank) VALUES('local-now-track','Test track','spotify')").execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO entity_links VALUES('track','spotify','spotify:track:0000000000000000000001','local-now-track',0,'{}')").execute(&state.db).await.unwrap();
        let poll_at = first.0["source_requested_at_ms"].as_i64().unwrap();
        let cached = now_playing_in_window(state.clone(), headers, Some(poll_at))
            .await
            .unwrap();
        assert_eq!(cached.0["track_id"], "local-now-track");
        assert_eq!(mock.calls.lock().await.len(), 1);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM listens")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(count, 0);
        handle.abort();
    }
    #[tokio::test]
    async fn missing_artwork_never_uses_spotify_or_blocks_new_listens() {
        struct PublicMisses;
        impl crate::artwork_sources::ImageLookup for PublicMisses {
            async fn lookup(
                &self,
                _provider: crate::artwork_settings::ImageProvider,
                _kind: &str,
                _id: &str,
            ) -> Result<crate::artwork_sources::Lookup> {
                Ok(crate::artwork_sources::Lookup::Missing)
            }
        }
        let at = 1_749_000_000_000;
        let (_dir, state, mock, handle) = fixture(
            vec![
                reply("/v1/me/player/recently-played", json!({"items":[item(at)]})),
                reply(
                    "/v1/me/player/recently-played",
                    json!({"items":[item(at+180_000),item(at)]}),
                ),
                reply(
                    "/v1/me/player/currently-playing",
                    json!({"is_playing":true,"currently_playing_type":"track","item":track()}),
                ),
                reply(
                    "/v1/me/player/recently-played",
                    json!({"items":[item(at+180_000),item(at)]}),
                ),
            ],
            false,
        )
        .await;
        sync(&state).await.unwrap();
        let album: String =
            sqlx::query_scalar("SELECT id FROM name_groups WHERE kind='albums' LIMIT 1")
                .fetch_one(&state.db)
                .await
                .unwrap();
        let image =
            crate::library::ensure_artwork_with(&state, "albums", &album, &PublicMisses).await;
        let collection = sync_force(&state).await;
        assert!(
            collection.is_ok(),
            "Artwork must not block new listens: {collection:?}; image result: {image:?}"
        );
        assert_eq!(collection.unwrap()["added"], 1);
        assert_eq!(image.unwrap(), crate::library::ArtworkFetch::Missing);
        let headers = owner_headers(&state).await;
        let now = now_playing(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(now["title"], "Test track");
        assert_eq!(now["cover_url"], format!("/api/v1/artwork/albums/{album}"));
        assert_eq!(
            status(State(state.clone()), headers)
                .await
                .unwrap()
                .0
                .collection,
            "active"
        );
        assert!(
            db::setting(&state.db, "retry_at_ms")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(sync_force(&state).await.unwrap()["duplicates"], 2);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            2
        );
        assert_eq!(mock.calls.lock().await.len(), 4);
        handle.abort();
        state.storage.close().await;
    }
    #[test]
    fn extract_rate_limit_reason_sanitizes_strictly_and_rejects_arbitrary_strings() {
        assert_eq!(
            extract_rate_limit_reason(&json!({"error": {"reason": "QUOTA_EXCEEDED"}})),
            Some("QUOTA_EXCEEDED")
        );
        assert_eq!(
            extract_rate_limit_reason(&json!({"reason": "quota_exceeded"})),
            Some("QUOTA_EXCEEDED")
        );
        assert_eq!(
            extract_rate_limit_reason(&json!({"error": {"reason": "RATE_LIMIT_EXCEEDED"}})),
            None
        );
        assert_eq!(
            extract_rate_limit_reason(
                &json!({"error": {"reason": "Bearer token expired for user 123"}})
            ),
            None
        );
        assert_eq!(
            extract_rate_limit_reason(&json!({"reason": "internal_rate_limit_bucket_exhausted"})),
            None
        );
        assert_eq!(extract_rate_limit_reason(&json!({})), None);
    }

    #[test]
    fn parse_retry_after_supports_integers_and_http_dates() {
        let now_sec = 1_700_000_000i64;
        let now_ms = now_sec * 1000;

        // Integer seconds
        assert_eq!(parse_retry_after("120", now_ms), Some(120));
        assert_eq!(parse_retry_after("  45  ", now_ms), Some(45));
        assert_eq!(parse_retry_after("0", now_ms), Some(0));

        // RFC 7231 IMF-fixdate: 120 seconds in future
        let future_sec = now_sec + 120;
        let dt = chrono::DateTime::from_timestamp(future_sec, 0).unwrap();
        let imf_fixdate = dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        assert_eq!(parse_retry_after(&imf_fixdate, now_ms), Some(120));

        // RFC 2822 format: e.g. "Wed, 21 Oct 2026 07:28:00 +0000"
        let rfc2822 = dt.format("%a, %d %b %Y %H:%M:%S +0000").to_string();
        assert_eq!(parse_retry_after(&rfc2822, now_ms), Some(120));

        // Single digit day
        let single_digit = dt.format("%a, %e %b %Y %H:%M:%S GMT").to_string();
        assert_eq!(parse_retry_after(&single_digit, now_ms), Some(120));

        // Invalid format
        assert_eq!(parse_retry_after("not-a-date", now_ms), None);
        assert_eq!(parse_retry_after("", now_ms), None);
    }

    #[tokio::test]
    async fn quota_exceeded_rate_limit_sets_diagnostic_reason_and_status() {
        let (_dir, state, mock, handle) = fixture(
            vec![Reply {
                path: "/v1/me/player/recently-played",
                status: StatusCode::TOO_MANY_REQUESTS,
                body: json!({
                    "error": {
                        "status": 429,
                        "message": "API rate limit exceeded",
                        "reason": "QUOTA_EXCEEDED"
                    }
                }),
                retry: Some("60"),
            }],
            false,
        )
        .await;
        let headers = owner_headers(&state).await;
        let err = sync(&state).await.unwrap_err();
        assert!(err.to_string().contains("60"));

        // Verify setting persistence
        assert_eq!(
            db::setting(&state.db, "spotify_rate_limit_reason")
                .await
                .unwrap(),
            Some("QUOTA_EXCEEDED".to_string())
        );

        // Verify SpotifyStatus exposing rate_limit_reason
        let current_status = status(State(state.clone()), headers).await.unwrap().0;
        assert_eq!(
            current_status.rate_limit_reason.as_deref(),
            Some("QUOTA_EXCEEDED")
        );
        assert_eq!(current_status.collection, "waiting");
        assert!(current_status.retry_at_ms.unwrap() > auth::now_ms());

        let job: crate::jobs::Job =
            sqlx::query_as("SELECT * FROM jobs WHERE kind='spotify_collection'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(
            job.error_code.as_deref(),
            Some("spotify_quota_exceeded: Spotify developer account quota exceeded")
        );

        assert_eq!(mock.calls.lock().await.len(), 1);
        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn standard_rate_limit_clears_or_leaves_reason_empty() {
        let (_dir, state, mock, handle) = fixture(
            vec![
                Reply {
                    path: "/v1/me/player/recently-played",
                    status: StatusCode::TOO_MANY_REQUESTS,
                    body: json!({
                        "error": {
                            "status": 429,
                            "message": "API rate limit exceeded",
                            "reason": "QUOTA_EXCEEDED"
                        }
                    }),
                    retry: Some("60"),
                },
                Reply {
                    path: "/v1/me/player/recently-played",
                    status: StatusCode::TOO_MANY_REQUESTS,
                    body: json!({
                        "error": {
                            "status": 429,
                            "message": "Too Many Requests"
                        }
                    }),
                    retry: Some("30"),
                },
            ],
            false,
        )
        .await;
        let headers = owner_headers(&state).await;

        // First 429: QUOTA_EXCEEDED
        assert!(sync(&state).await.is_err());
        let job1: crate::jobs::Job =
            sqlx::query_as("SELECT * FROM jobs WHERE kind='spotify_collection'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(
            job1.error_code.as_deref(),
            Some("spotify_quota_exceeded: Spotify developer account quota exceeded")
        );
        assert_eq!(
            db::setting(&state.db, "spotify_rate_limit_reason")
                .await
                .unwrap(),
            Some("QUOTA_EXCEEDED".to_string())
        );

        // Reset both shared cooldown timers to bypass the wait for the next request.
        diagnostic(&state, "retry_at_ms", "0").await.unwrap();
        diagnostic(&state, "now_playing_retry_at_ms", "0")
            .await
            .unwrap();
        sqlx::query(
            "UPDATE jobs SET retry_at_ms=0 WHERE kind='spotify_collection' AND state='waiting'",
        )
        .execute(&state.db)
        .await
        .unwrap();
        auth::advance_clock(SYNC_COALESCE_MS + 1_000);

        // Second 429: standard (no reason)
        assert!(sync(&state).await.is_err());
        let job2: crate::jobs::Job =
            sqlx::query_as("SELECT * FROM jobs WHERE kind='spotify_collection'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert!(
            job2.error_code
                .as_deref()
                .unwrap()
                .starts_with("spotify_rate_limited: Spotify request limit reached. Retrying in")
        );
        assert!(
            db::setting(&state.db, "spotify_rate_limit_reason")
                .await
                .unwrap()
                .as_deref()
                .unwrap_or("")
                .is_empty()
        );

        let current_status = status(State(state.clone()), headers).await.unwrap().0;
        assert_eq!(current_status.rate_limit_reason, None);

        assert_eq!(mock.calls.lock().await.len(), 2);
        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn retry_after_http_date_parsing_and_clamping_at_mock_fixture() {
        let now_sec = auth::now_ms() / 1000;
        let wait_target_sec = 180;
        let future_sec = now_sec + wait_target_sec;
        let dt = chrono::DateTime::from_timestamp(future_sec, 0).unwrap();
        let http_date = dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        let leaked_date: &'static str = Box::leak(http_date.into_boxed_str());

        let (_dir, state, mock, handle) = fixture(
            vec![Reply {
                path: "/v1/me/player/recently-played",
                status: StatusCode::TOO_MANY_REQUESTS,
                body: json!({}),
                retry: Some(leaked_date),
            }],
            false,
        )
        .await;
        let headers = owner_headers(&state).await;
        assert!(sync(&state).await.is_err());

        let current_status = status(State(state.clone()), headers).await.unwrap().0;
        let retry_at_ms = current_status.retry_at_ms.unwrap();
        let expected_min_ms = auth::now_ms() + (wait_target_sec - 10) * 1000;
        let expected_max_ms = auth::now_ms() + (wait_target_sec + 10) * 1000;
        assert!(
            retry_at_ms >= expected_min_ms && retry_at_ms <= expected_max_ms,
            "retry_at_ms {retry_at_ms} was not within expected range [{expected_min_ms}, {expected_max_ms}]"
        );

        assert_eq!(mock.calls.lock().await.len(), 1);
        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn retry_after_clamping_to_24h() {
        let (_dir, state, mock, handle) = fixture(
            vec![Reply {
                path: "/v1/me/player/recently-played",
                status: StatusCode::TOO_MANY_REQUESTS,
                body: json!({}),
                retry: Some("100000"), // > 86_400 (24h)
            }],
            false,
        )
        .await;
        let headers = owner_headers(&state).await;
        assert!(sync(&state).await.is_err());

        let current_status = status(State(state.clone()), headers).await.unwrap().0;
        let retry_at_ms = current_status.retry_at_ms.unwrap();
        let max_expected_ms = auth::now_ms() + 86_400 * 1000 + 5000;
        let min_expected_ms = auth::now_ms() + 86_400 * 1000 - 5000;
        assert!(
            retry_at_ms >= min_expected_ms && retry_at_ms <= max_expected_ms,
            "retry_at_ms {retry_at_ms} should be clamped to ~86400s: [{min_expected_ms}, {max_expected_ms}]"
        );

        assert_eq!(mock.calls.lock().await.len(), 1);
        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn test_spotify_retry_now_overrides_local_cooldown_and_keeps_the_real_diagnosis() {
        let (_dir, state, _mock, handle) = fixture(
            vec![reply(
                "/v1/me/player/recently-played",
                json!({
                    "items": [{
                        "played_at": "2024-01-01T00:00:00.000Z",
                        "track": track()
                    }]
                }),
            )],
            false,
        )
        .await;

        // 1. Rejects unauthenticated requests
        let unauth_err = retry_now(State(state.clone()), HeaderMap::new()).await;
        assert!(matches!(unauth_err, Err(AppError::Unauthorized)));

        // 2. Rejects requests without CSRF token
        let headers_no_csrf = owner_headers(&state).await;
        let forbidden_err = retry_now(State(state.clone()), headers_no_csrf).await;
        assert!(matches!(forbidden_err, Err(AppError::Forbidden)));

        // Prepare valid owner headers with CSRF
        let mut valid_headers = owner_headers(&state).await;
        valid_headers.insert("x-csrf-token", HeaderValue::from_static("csrf"));

        // Simulate active rate limit settings and waiting collection job
        let future_retry = (auth::now_ms() + 300_000).to_string();
        diagnostics(
            &state,
            &[
                ("retry_at_ms", &future_retry),
                ("now_playing_retry_at_ms", &future_retry),
                ("spotify_rate_limit_reason", "QUOTA_EXCEEDED"),
                (
                    "now_playing_error",
                    "Spotify player request limit reached. Retrying in 300 seconds.",
                ),
                (
                    "collection_error",
                    "spotify_quota_exceeded: Spotify developer account quota exceeded",
                ),
                ("spotify_error", "stale diagnostic error"),
            ],
        )
        .await
        .unwrap();

        // Enqueue and pause a spotify_collection job with retry_at_ms in future
        state
            .storage
            .enqueue(
                "spotify_collection",
                None,
                Some("Spotify background collection"),
                &json!({}),
            )
            .await
            .unwrap();
        let claimed = state
            .storage
            .claim("spotify_collection")
            .await
            .unwrap()
            .unwrap();
        state
            .storage
            .pause(&claimed, Some(auth::now_ms() + 300_000), "rate_limited")
            .await
            .unwrap();

        // Set up listener for wake notification
        let wake = state.spotify.wake.clone();
        let wait_for_wake = tokio::spawn(async move {
            wake.notified().await;
        });

        // 3. Call retry_now endpoint
        let response = retry_now(State(state.clone()), valid_headers.clone())
            .await
            .unwrap();
        assert_eq!(response.0["status"], "retry_requested");
        assert!(
            response.0["message"]
                .as_str()
                .unwrap_or_default()
                .contains("does not reset")
        );

        // Verify wake was triggered
        tokio::time::timeout(Duration::from_millis(500), wait_for_wake)
            .await
            .expect("wake notification should be delivered")
            .unwrap();

        // The local cooldown is cleared, but the last real diagnosis stays
        // visible until Spotify actually answers successfully.
        assert!(
            db::setting(&state.db, "retry_at_ms")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db::setting(&state.db, "now_playing_retry_at_ms")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            db::setting(&state.db, "spotify_rate_limit_reason")
                .await
                .unwrap()
                .as_deref(),
            Some("QUOTA_EXCEEDED")
        );
        assert!(
            db::setting(&state.db, "now_playing_error")
                .await
                .unwrap()
                .is_some_and(|value| value.contains("Retry requested"))
        );
        assert!(
            db::setting(&state.db, "collection_error")
                .await
                .unwrap()
                .is_some_and(|value| value.contains("Retry requested"))
        );
        assert!(
            db::setting(&state.db, "spotify_error")
                .await
                .unwrap()
                .is_some()
        );

        let post_retry_status = status(State(state.clone()), valid_headers.clone())
            .await
            .unwrap()
            .0;
        assert!(post_retry_status.last_error.is_some());
        assert!(post_retry_status.now_playing_error.is_some());
        assert_eq!(
            post_retry_status.rate_limit_reason.as_deref(),
            Some("QUOTA_EXCEEDED")
        );
        assert_eq!(post_retry_status.connection, "unverified");

        // Verify collection job retry_at_ms is reset to 0 so it can be claimed immediately
        let waiting_retry: Option<i64> = sqlx::query_scalar(
            "SELECT retry_at_ms FROM jobs WHERE kind='spotify_collection' AND state='waiting'",
        )
        .fetch_optional(&state.db)
        .await
        .unwrap();
        assert_eq!(waiting_retry, Some(0));

        // The local cooldown is gone, but nothing claims a verified connection.
        let current_status = status(State(state.clone()), valid_headers).await.unwrap().0;
        assert!(current_status.retry_at_ms.is_none());
        assert_eq!(
            current_status.rate_limit_reason.as_deref(),
            Some("QUOTA_EXCEEDED")
        );
        assert_ne!(current_status.collection, "waiting");

        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn test_spotify_retry_now_router_endpoint() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let (_dir, state, _mock, handle) = fixture(vec![], false).await;
        let app = crate::web::router(state.clone());

        // 1. Unauthenticated request to /api/spotify/retry-now returns 401
        let req = Request::builder()
            .method("POST")
            .uri("/api/spotify/retry-now")
            .header("host", state.config.host())
            .header("origin", state.config.origin())
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // 2. Authenticated request with CSRF token returns 200 OK
        let cookie = auth::random_secret();
        sqlx::query("INSERT INTO sessions(digest,csrf,expires_at_ms) VALUES (?,'testcsrf',?)")
            .bind(auth::digest(&cookie))
            .bind(auth::now_ms() + 60000)
            .execute(&state.private_db)
            .await
            .unwrap();

        let req = Request::builder()
            .method("POST")
            .uri("/api/spotify/retry-now")
            .header("host", state.config.host())
            .header("origin", state.config.origin())
            .header("cookie", format!("scrobbl_session={cookie}"))
            .header("x-csrf-token", "testcsrf")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        handle.abort();
        state.storage.close().await;
    }

    fn quota_429(retry: &'static str) -> Reply {
        Reply {
            path: "/v1/me/player/recently-played",
            status: StatusCode::TOO_MANY_REQUESTS,
            body: json!({
                "error": {
                    "status": 429,
                    "message": "API rate limit exceeded",
                    "reason": "QUOTA_EXCEEDED"
                }
            }),
            retry: Some(retry),
        }
    }

    #[tokio::test]
    async fn quota_exceeded_on_collection_pauses_now_playing_without_http() {
        let (_dir, state, mock, handle) = fixture(vec![quota_429("7200")], false).await;
        let headers = owner_headers(&state).await;

        assert!(sync(&state).await.is_err());
        assert_eq!(mock.calls.lock().await.len(), 1);
        let retry: i64 = db::setting(&state.db, "retry_at_ms")
            .await
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        assert!(retry > auth::now_ms() + 7_000_000);

        for offset in [0, 60_000, 3_600_000] {
            let poll_at = auth::now_ms() + offset;
            let sample = now_playing_in_window(state.clone(), headers.clone(), Some(poll_at))
                .await
                .unwrap()
                .0;
            assert!(sample.is_null());
        }
        assert_eq!(
            mock.calls.lock().await.len(),
            1,
            "now playing must not contact Spotify during the shared cooldown"
        );
        assert_eq!(
            db::setting(&state.db, "spotify_rate_limit_reason")
                .await
                .unwrap()
                .as_deref(),
            Some("QUOTA_EXCEEDED")
        );

        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn quota_exceeded_on_now_playing_pauses_collection_without_http() {
        let (_dir, state, mock, handle) = fixture(
            vec![Reply {
                path: "/v1/me/player/currently-playing",
                status: StatusCode::TOO_MANY_REQUESTS,
                body: json!({
                    "error": {
                        "status": 429,
                        "message": "API rate limit exceeded",
                        "reason": "QUOTA_EXCEEDED"
                    }
                }),
                retry: Some("7200"),
            }],
            false,
        )
        .await;
        let headers = owner_headers(&state).await;

        assert!(
            now_playing(State(state.clone()), headers.clone())
                .await
                .is_err()
        );
        assert_eq!(mock.calls.lock().await.len(), 1);
        assert!(sync(&state).await.is_err());
        assert_eq!(
            mock.calls.lock().await.len(),
            1,
            "collection must not contact Spotify during the shared cooldown"
        );
        let shown = status(State(state.clone()), headers).await.unwrap().0;
        assert_eq!(shown.collection, "waiting");
        assert_eq!(shown.rate_limit_reason.as_deref(), Some("QUOTA_EXCEEDED"));

        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn rate_limit_clears_a_previous_verification_until_a_request_succeeds() {
        let (_dir, state, _mock, handle) = fixture(
            vec![
                reply("/v1/me/player/recently-played", json!({"items":[]})),
                quota_429("3600"),
            ],
            false,
        )
        .await;
        let headers = owner_headers(&state).await;

        sync(&state).await.unwrap();
        let verified = status(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(verified.connection, "verified");
        assert!(verified.verified_at_ms.is_some());

        auth::advance_clock(SYNC_COALESCE_MS + 1_000);
        assert!(sync(&state).await.is_err());
        let limited = status(State(state.clone()), headers).await.unwrap().0;
        assert_eq!(limited.connection, "unverified");
        assert!(limited.verified_at_ms.is_none());
        assert_eq!(limited.collection, "waiting");

        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn a_later_shorter_429_never_shortens_an_active_longer_pause() {
        let (_dir, state, _mock, handle) = fixture(
            vec![Reply {
                path: "/v1/me/player/recently-played",
                status: StatusCode::TOO_MANY_REQUESTS,
                body: json!({
                    "error": {"status": 429, "message": "Too Many Requests"}
                }),
                retry: Some("30"),
            }],
            false,
        )
        .await;
        let now = auth::now_ms();
        let long_pause = (now + 3_600_000).to_string();
        diagnostics(
            &state,
            &[
                ("retry_at_ms", &long_pause),
                ("now_playing_retry_at_ms", &long_pause),
                ("spotify_rate_limit_reason", "QUOTA_EXCEEDED"),
            ],
        )
        .await
        .unwrap();

        // A request that was already authorized before the longer pause (for
        // example a token exchange) can still return a shorter 429.
        let response = state
            .spotify
            .client
            .get(format!(
                "{}/me/player/recently-played",
                state.spotify.api_base
            ))
            .bearer_auth("test-token")
            .send()
            .await
            .unwrap();
        assert!(checked_json(&state, response).await.is_err());

        let retry: i64 = db::setting(&state.db, "retry_at_ms")
            .await
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            retry >= now + 3_500_000,
            "a later shorter 429 shortened the active pause to {} ms from now",
            retry - now
        );
        let now_playing_retry = db::setting(&state.db, "now_playing_retry_at_ms")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(now_playing_retry, retry.to_string());
        assert_eq!(
            db::setting(&state.db, "spotify_rate_limit_reason")
                .await
                .unwrap()
                .as_deref(),
            Some("QUOTA_EXCEEDED")
        );

        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn worker_resumes_collection_after_cooldown_and_restart_without_losing_or_duplicating() {
        let at = 1_749_000_000_000;
        let (dir, state, mock, handle) = fixture_with_intervals(
            vec![
                quota_429("3600"),
                // The controlled clock also crosses the one-hour token lifetime.
                reply(
                    "/api/token",
                    json!({"access_token":"access-after-cooldown","expires_in":3600}),
                ),
                reply("/v1/me/player/recently-played", json!({"items":[item(at)]})),
                reply("/v1/me/player/recently-played", json!({"items":[item(at)]})),
            ],
            false,
            1,
            30,
        )
        .await;
        let config = state.config.clone();
        let (api_base, token_url) = replay_base_urls(&state);

        let collecting = tokio::spawn(worker(state.clone()));
        wait_for_calls(&mock, 1).await;
        wait_for_setting(&state, "retry_at_ms").await;
        let retry: i64 = db::setting(&state.db, "retry_at_ms")
            .await
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        assert!(retry > auth::now_ms() + 3_000_000);
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        assert_eq!(
            mock.calls.lock().await.len(),
            1,
            "the worker must not contact Spotify during the cooldown"
        );
        assert!(
            db::setting(&state.db, "recent_cursor_ms")
                .await
                .unwrap()
                .is_none()
        );

        collecting.abort();
        let _ = collecting.await;
        // Let the coalescing window expire without expiring the cooldown, so
        // the restarted worker must stop at the persisted pause itself.
        auth::advance_clock(SYNC_COALESCE_MS + 1_000);
        state.storage.close().await;
        let mut restarted = AppState::new(db::open(dir.path()).await.unwrap(), config);
        restarted.spotify.api_base = api_base;
        restarted.spotify.token_url = token_url;

        let collecting = tokio::spawn(worker(restarted.clone()));
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        assert_eq!(
            mock.calls.lock().await.len(),
            1,
            "a restart must still respect the persisted cooldown"
        );
        assert!(
            db::setting(&restarted.db, "recent_cursor_ms")
                .await
                .unwrap()
                .is_none()
        );

        // Only the controlled clock moves: the worker keeps its normal schedule.
        auth::advance_clock(3_601_000);
        wait_for_calls(&mock, 2).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM listens")
                    .fetch_one(&restarted.db)
                    .await
                    .unwrap();
                if count == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            db::setting(&restarted.db, "recent_cursor_ms")
                .await
                .unwrap(),
            Some(at.to_string())
        );

        collecting.abort();
        let _ = collecting.await;

        // The next cadence window sees the same history again: it must be a duplicate.
        auth::advance_clock(SYNC_COALESCE_MS + 1_000);
        let report = sync(&restarted).await.unwrap();
        assert_eq!(report["added"], 0);
        assert_eq!(report["duplicates"], 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens")
                .fetch_one(&restarted.db)
                .await
                .unwrap(),
            1
        );

        handle.abort();
        restarted.storage.close().await;
    }

    #[tokio::test]
    async fn now_playing_resumes_after_cooldown_and_restart_without_duplicate_requests() {
        let (dir, state, mock, handle) = fixture(
            vec![
                Reply {
                    path: "/v1/me/player/currently-playing",
                    status: StatusCode::TOO_MANY_REQUESTS,
                    body: json!({
                        "error": {
                            "status": 429,
                            "message": "API rate limit exceeded",
                            "reason": "QUOTA_EXCEEDED"
                        }
                    }),
                    retry: Some("3600"),
                },
                // The controlled clock also crosses the one-hour token lifetime.
                reply(
                    "/api/token",
                    json!({"access_token":"access-after-cooldown","expires_in":3600}),
                ),
                reply(
                    "/v1/me/player/currently-playing",
                    json!({
                        "is_playing": true,
                        "currently_playing_type": "track",
                        "item": track()
                    }),
                ),
            ],
            false,
        )
        .await;
        let headers = owner_headers(&state).await;

        assert!(
            now_playing(State(state.clone()), headers.clone())
                .await
                .is_err()
        );
        assert_eq!(mock.calls.lock().await.len(), 1);

        let config = state.config.clone();
        let (api_base, token_url) = replay_base_urls(&state);
        state.storage.close().await;
        let mut restarted = AppState::new(db::open(dir.path()).await.unwrap(), config);
        restarted.spotify.api_base = api_base;
        restarted.spotify.token_url = token_url;

        let during = now_playing(State(restarted.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert!(during.is_null());
        assert_eq!(
            mock.calls.lock().await.len(),
            1,
            "now playing must not contact Spotify while the cooldown is active after restart"
        );

        auth::advance_clock(3_601_000);
        // The controlled clock also expires the short-lived test session.
        let headers = owner_headers(&restarted).await;
        let resumed = now_playing(State(restarted.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(resumed["title"], "Test track");
        assert_eq!(
            call_count(&mock, "/v1/me/player/currently-playing").await,
            2
        );
        assert_eq!(call_count(&mock, "/api/token").await, 1);
        let shown = status(State(restarted.clone()), headers).await.unwrap().0;
        assert!(shown.now_playing_error.is_none());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens")
                .fetch_one(&restarted.db)
                .await
                .unwrap(),
            0,
            "now playing must never create listens"
        );

        handle.abort();
        restarted.storage.close().await;
    }

    #[tokio::test]
    async fn concurrent_manual_and_background_syncs_are_coalesced_after_the_gate() {
        let (_dir, state, mock, handle) = fixture(
            vec![reply(
                "/v1/me/player/recently-played",
                json!({"items":[],"_test_delay_ms":250}),
            )],
            false,
        )
        .await;
        let mut first_headers = owner_headers(&state).await;
        first_headers.insert("x-csrf-token", HeaderValue::from_static("csrf"));
        let mut second_headers = owner_headers(&state).await;
        second_headers.insert("x-csrf-token", HeaderValue::from_static("csrf"));

        let first = tokio::spawn({
            let state = state.clone();
            let headers = first_headers;
            async move { sync_now(State(state), headers).await }
        });
        wait_for_calls(&mock, 1).await;

        let second = tokio::spawn({
            let state = state.clone();
            let headers = second_headers;
            async move { sync_now(State(state), headers).await }
        });
        let background = tokio::spawn({
            let state = state.clone();
            async move { sync(&state).await }
        });

        assert_eq!(first.await.unwrap().unwrap().0["added"], 0);
        assert!(matches!(second.await.unwrap(), Err(AppError::Conflict(_))));
        assert!(background.await.unwrap().is_ok());
        assert_eq!(
            mock.calls.lock().await.len(),
            1,
            "concurrent manual and background syncs must not run two collections"
        );

        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn a_failed_collection_still_coalesces_the_next_waiting_sync() {
        let (_dir, state, mock, handle) = fixture(
            vec![
                Reply {
                    path: "/v1/me/player/recently-played",
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    body: json!({"_test_delay_ms": 250}),
                    retry: None,
                },
                reply("/v1/me/player/recently-played", json!({"items":[]})),
            ],
            false,
        )
        .await;
        let mut headers = owner_headers(&state).await;
        headers.insert("x-csrf-token", HeaderValue::from_static("csrf"));

        let first = tokio::spawn({
            let state = state.clone();
            async move { sync_now(State(state), headers).await }
        });
        wait_for_calls(&mock, 1).await;
        let background = tokio::spawn({
            let state = state.clone();
            async move { sync(&state).await }
        });

        assert!(first.await.unwrap().is_err());
        assert!(background.await.unwrap().is_ok());
        assert_eq!(
            mock.calls.lock().await.len(),
            1,
            "a failed run must still coalesce a waiting sync"
        );

        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn manual_retry_keeps_the_last_diagnosis_and_restores_cooldown_after_another_429() {
        let (_dir, state, mock, handle) =
            fixture(vec![quota_429("3600"), quota_429("3600")], false).await;
        let mut headers = owner_headers(&state).await;
        headers.insert("x-csrf-token", HeaderValue::from_static("csrf"));

        assert!(sync(&state).await.is_err());
        assert_eq!(mock.calls.lock().await.len(), 1);
        let before = status(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(before.connection, "unverified");
        assert_eq!(before.collection, "waiting");

        let response = retry_now(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(response["status"], "retry_requested");
        let message = response["message"].as_str().unwrap_or_default();
        assert!(!message.to_ascii_lowercase().contains("verified"));
        assert!(message.to_ascii_lowercase().contains("quota"));
        assert!(message.to_ascii_lowercase().contains("not reset"));

        assert!(
            db::setting(&state.db, "retry_at_ms")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db::setting(&state.db, "now_playing_retry_at_ms")
                .await
                .unwrap()
                .is_none()
        );
        // The last real failure stays visible until Spotify actually answers.
        assert_eq!(
            db::setting(&state.db, "spotify_rate_limit_reason")
                .await
                .unwrap()
                .as_deref(),
            Some("QUOTA_EXCEEDED")
        );
        assert!(
            db::setting(&state.db, "collection_error")
                .await
                .unwrap()
                .is_some_and(|value| value.contains("Retry requested"))
        );
        let after_retry = status(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(after_retry.connection, "unverified");
        assert_ne!(after_retry.collection, "waiting");
        assert!(after_retry.rate_limit_reason.is_some());
        assert!(after_retry.last_error.is_some());

        // Spotify still answers 429: one attempt, then the real cooldown returns.
        assert!(sync(&state).await.is_err());
        assert_eq!(mock.calls.lock().await.len(), 2);
        let restored = status(State(state.clone()), headers.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(restored.collection, "waiting");
        assert_eq!(
            restored.rate_limit_reason.as_deref(),
            Some("QUOTA_EXCEEDED")
        );

        // No request loop while the cooldown is active again.
        auth::advance_clock(SYNC_COALESCE_MS + 1_000);
        assert!(sync(&state).await.is_err());
        assert_eq!(mock.calls.lock().await.len(), 2);

        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn recent_history_gap_is_reported_and_points_to_extended_import() {
        let at = 1_749_000_000_000;
        let (_dir, state, _mock, handle) = fixture(
            vec![
                reply("/v1/me/player/recently-played", json!({"items":[item(at)]})),
                reply(
                    "/v1/me/player/recently-played",
                    json!({"items":[item(at + 1_000_000)]}),
                ),
            ],
            false,
        )
        .await;

        sync(&state).await.unwrap();
        assert_eq!(
            db::setting(&state.db, "recent_cursor_ms").await.unwrap(),
            Some(at.to_string())
        );

        auth::advance_clock(SYNC_COALESCE_MS + 1_000);
        let report = sync(&state).await.unwrap();
        assert_eq!(report["added"], 1);
        let warning = db::setting(&state.db, "coverage_warning")
            .await
            .unwrap()
            .expect("a recent-history gap must be reported");
        assert!(warning.contains("extended-history import"), "{warning}");
        let headers = owner_headers(&state).await;
        let shown = status(State(state.clone()), headers).await.unwrap().0;
        assert_eq!(shown.coverage_warning.as_deref(), Some(warning.as_str()));

        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn request_consumption_is_measured_per_endpoint_and_deduplicated_across_windows() {
        let at = 1_749_000_000_000i64;
        let page1: Vec<_> = (0..50).map(|i| item(at - i * 180_000)).collect();
        let page2: Vec<_> = (50..100).map(|i| item(at - i * 180_000)).collect();
        let page3 = vec![item(at - 100 * 180_000)];
        let (_dir, state, mock, handle) = fixture(
            vec![
                reply(
                    "/api/token",
                    json!({"access_token":"new-access","expires_in":3600}),
                ),
                reply("/v1/me/player/recently-played", json!({"items":page1})),
                reply("/v1/me/player/recently-played", json!({"items":page2})),
                reply("/v1/me/player/recently-played", json!({"items":page3})),
                reply(
                    "/v1/me/player/currently-playing",
                    json!({
                        "is_playing": true,
                        "currently_playing_type": "track",
                        "item": track()
                    }),
                ),
            ],
            true,
        )
        .await;

        assert_eq!(sync(&state).await.unwrap()["added"], 101);
        let headers = owner_headers(&state).await;
        let mut windows = Vec::new();
        for _ in 0..8 {
            windows.push(tokio::spawn({
                let state = state.clone();
                let headers = headers.clone();
                async move { now_playing(State(state), headers).await }
            }));
        }
        for window in windows {
            assert!(window.await.unwrap().is_ok());
        }

        let token = call_count(&mock, "/api/token").await;
        let history = call_count(&mock, "/v1/me/player/recently-played").await;
        let player = call_count(&mock, "/v1/me/player/currently-playing").await;
        assert_eq!((token, history, player), (1, 3, 1));
        let intervals = intervals(&state).await.unwrap();
        println!(
            "Spotify request consumption: token_refresh={token}, history_pages={history}, now_playing={player}; configured sync={}s (~{}/h), now_playing={}s (~{}/h)",
            intervals.sync_seconds,
            3600 / intervals.sync_seconds,
            intervals.now_playing_seconds,
            3600 / intervals.now_playing_seconds
        );

        handle.abort();
        state.storage.close().await;
    }

    #[tokio::test]
    async fn rate_limit_mid_pagination_keeps_cursor_and_commits_no_partial_batch() {
        let at = 1_749_000_000_000;
        let batch: Vec<_> = (0..50).map(|i| item(at - i * 180_000)).collect();
        let (_dir, state, mock, handle) = fixture(
            vec![
                reply("/v1/me/player/recently-played", json!({"items":batch})),
                Reply {
                    path: "/v1/me/player/recently-played",
                    status: StatusCode::TOO_MANY_REQUESTS,
                    body: json!({
                        "error": {
                            "status": 429,
                            "message": "API rate limit exceeded",
                            "reason": "QUOTA_EXCEEDED"
                        }
                    }),
                    retry: Some("600"),
                },
            ],
            false,
        )
        .await;

        assert!(sync(&state).await.is_err());
        assert_eq!(mock.calls.lock().await.len(), 2);
        assert!(
            db::setting(&state.db, "recent_cursor_ms")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            0
        );
        let job: crate::jobs::Job =
            sqlx::query_as("SELECT * FROM jobs WHERE kind='spotify_collection'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(job.state, "waiting");
        assert!(job.retry_at_ms.unwrap() > auth::now_ms());

        let headers = owner_headers(&state).await;
        let sample = now_playing(State(state.clone()), headers).await.unwrap().0;
        assert!(sample.is_null());
        assert_eq!(
            mock.calls.lock().await.len(),
            2,
            "now playing must not contact Spotify during the cooldown"
        );

        handle.abort();
        state.storage.close().await;
    }
}
