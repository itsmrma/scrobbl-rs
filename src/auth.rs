use crate::{
    AppState,
    error::{AppError, Result},
};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, header},
    response::{IntoResponse, Response},
};
use rand::RngCore;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::time::{Duration, Instant};

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
pub fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| part.trim().strip_prefix("scrobbl_session="))
}
#[derive(Clone)]
pub struct Session {
    pub digest: String,
    pub csrf: String,
}
pub async fn session(state: &AppState, headers: &HeaderMap) -> Result<Session> {
    let token = session_cookie(headers).ok_or(AppError::Unauthorized)?;
    if token.len() != 64 {
        return Err(AppError::Unauthorized);
    }
    let digest = digest(token);
    let row = sqlx::query("SELECT csrf FROM sessions WHERE digest = ? AND expires_at_ms > ?")
        .bind(&digest)
        .bind(now_ms())
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::Unauthorized)?;
    Ok(Session {
        digest,
        csrf: row.get("csrf"),
    })
}
pub async fn require_write(state: &AppState, headers: &HeaderMap) -> Result<Session> {
    let session = session(state, headers).await?;
    if headers.get("x-csrf-token").and_then(|h| h.to_str().ok()) != Some(session.csrf.as_str()) {
        return Err(AppError::Forbidden);
    }
    Ok(session)
}
async fn new_session(state: &AppState) -> Result<Response> {
    let token = random_secret();
    let csrf = random_secret();
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at_ms <= ?")
        .bind(now_ms())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO sessions(digest, csrf, expires_at_ms) VALUES (?, ?, ?)")
        .bind(digest(&token))
        .bind(&csrf)
        .bind(now_ms() + 7 * 86_400_000)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let mut response = Json(json!({"authenticated":true,"csrf":csrf})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "scrobbl_session={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800"
        ))
        .map_err(|_| AppError::Internal)?,
    );
    Ok(response)
}
pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let setup_required = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM owner")
        .fetch_one(&state.db)
        .await?
        == 0;
    match session(&state, &headers).await {
        Ok(s) => Ok(Json(
            json!({"setup_required":setup_required,"authenticated":true,"csrf":s.csrf}),
        )),
        Err(AppError::Unauthorized) => Ok(Json(
            json!({"setup_required":setup_required,"authenticated":false}),
        )),
        Err(e) => Err(e),
    }
}
#[derive(Deserialize)]
pub struct PasswordForm {
    pub password: String,
}
async fn rate_limit(state: &AppState) -> Result<()> {
    let mut attempts = state.login_attempts.lock().await;
    let now = Instant::now();
    while attempts
        .front()
        .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60))
    {
        attempts.pop_front();
    }
    if attempts.len() >= 8 {
        return Err(AppError::TooManyRequests);
    }
    attempts.push_back(now);
    Ok(())
}
pub async fn setup(
    State(state): State<AppState>,
    Json(form): Json<PasswordForm>,
) -> Result<Response> {
    rate_limit(&state).await?;
    if form.password.chars().count() < 12 || form.password.len() > 1024 {
        return Err(AppError::BadRequest(
            "Choose a password of at least 12 characters (maximum 1024 bytes).".into(),
        ));
    }
    let hash = tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(form.password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|v| v.to_string())
            .map_err(|_| AppError::Internal)
    })
    .await
    .map_err(|_| AppError::Internal)??;
    let result = sqlx::query(
        "INSERT OR IGNORE INTO owner(id, password_hash, created_at_ms) VALUES (1, ?, ?)",
    )
    .bind(hash)
    .bind(now_ms())
    .execute(&state.db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::Conflict(
            "The owner password is already configured. Please sign in.".into(),
        ));
    }
    new_session(&state).await
}
pub async fn login(
    State(state): State<AppState>,
    Json(form): Json<PasswordForm>,
) -> Result<Response> {
    rate_limit(&state).await?;
    if form.password.len() > 1024 {
        return Err(AppError::Unauthorized);
    }
    let hash: Option<String> = sqlx::query_scalar("SELECT password_hash FROM owner WHERE id = 1")
        .fetch_optional(&state.db)
        .await?;
    let hash = hash.ok_or(AppError::Unauthorized)?;
    let valid = tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).is_ok_and(|hash| {
            Argon2::default()
                .verify_password(form.password.as_bytes(), &hash)
                .is_ok()
        })
    })
    .await
    .map_err(|_| AppError::Internal)?;
    if !valid {
        return Err(AppError::Unauthorized);
    }
    new_session(&state).await
}
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    let s = require_write(&state, &headers).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM oauth_states WHERE session_digest = ?")
        .bind(&s.digest)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE digest = ?")
        .bind(s.digest)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let mut response = Json(json!({"ok":true})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("scrobbl_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0"),
    );
    Ok(response)
}
