use crate::{
    AppState, auth,
    error::{AppError, Result},
};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Running,
    Restart,
    Stop,
}

#[derive(Clone)]
pub struct Application {
    pub action: tokio::sync::watch::Sender<Action>,
    pub boot_id: String,
}
impl Default for Application {
    fn default() -> Self {
        Self::new()
    }
}
impl Application {
    pub fn new() -> Self {
        let (action, _) = tokio::sync::watch::channel(Action::Running);
        Self {
            action,
            boot_id: auth::random_secret(),
        }
    }
}

pub async fn profile(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::session(&state, &headers).await?;
    let username: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='owner_username'")
            .fetch_optional(&state.private_db)
            .await?;
    let name: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='profile_name'")
            .fetch_optional(&state.private_db)
            .await?;
    let avatar: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='profile_avatar'")
            .fetch_optional(&state.private_db)
            .await?;
    Ok(Json(
        json!({"username":username,"name":name.unwrap_or_default(),"avatar_url":avatar.map(|v|format!("/api/v1/account/avatar?v={}",auth::digest(&v)))}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileEdit {
    pub name: String,
}
pub async fn edit_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<ProfileEdit>,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    let name = form.name.trim();
    if name.chars().count() > 80 {
        return Err(AppError::BadRequest(
            "Use a display name of 80 characters or fewer.".into(),
        ));
    }
    sqlx::query("INSERT INTO settings(key,value) VALUES('profile_name',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value").bind(name).execute(&state.private_db).await?;
    profile(State(state), headers).await
}

pub async fn upload_avatar(
    State(state): State<AppState>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    crate::library::image_type(&bytes)?;
    sqlx::query("INSERT INTO settings(key,value) VALUES('profile_avatar',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value").bind(auth::encode_hex(&bytes)).execute(&state.private_db).await?;
    profile(State(state), headers).await
}
pub async fn remove_avatar(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    sqlx::query("DELETE FROM settings WHERE key='profile_avatar'")
        .execute(&state.private_db)
        .await?;
    profile(State(state), headers).await
}
pub async fn avatar(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    auth::session(&state, &headers).await?;
    let value: String = sqlx::query_scalar("SELECT value FROM settings WHERE key='profile_avatar'")
        .fetch_optional(&state.private_db)
        .await?
        .ok_or(AppError::NotFound)?;
    let bytes = value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .ok_or(AppError::Internal)
        })
        .collect::<Result<Vec<_>>>()?;
    let mime = crate::library::image_type(&bytes)?;
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasswordEdit {
    pub current_password: String,
    pub password: String,
}
pub async fn password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<PasswordEdit>,
) -> Result<Json<Value>> {
    let session = auth::require_write(&state, &headers).await?;
    auth::rate_limit(&state).await?;
    let username: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='owner_username'")
            .fetch_optional(&state.private_db)
            .await?;
    auth::validate_password(&form.password, username.as_deref())?;
    let original: String = sqlx::query_scalar("SELECT password_hash FROM owner WHERE id=1")
        .fetch_one(&state.private_db)
        .await?;
    let previous = original.clone();
    let hash = tokio::task::spawn_blocking(move || {
        let old = PasswordHash::new(&original).map_err(|_| AppError::Internal)?;
        Argon2::default()
            .verify_password(form.current_password.as_bytes(), &old)
            .map_err(|_| AppError::BadRequest("The current password is incorrect.".into()))?;
        Argon2::default()
            .hash_password(form.password.as_bytes())
            .map(|v| v.to_string())
            .map_err(|_| AppError::Internal)
    })
    .await
    .map_err(|_| AppError::Internal)??;
    let mut tx = state.private_db.begin().await?;
    if sqlx::query("UPDATE owner SET password_hash=? WHERE id=1 AND password_hash=?")
        .bind(hash)
        .bind(previous)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        != 1
    {
        return Err(AppError::Conflict(
            "The password changed elsewhere. Reload and try again.".into(),
        ));
    }
    sqlx::query("DELETE FROM oauth_states WHERE session_digest != ?")
        .bind(&session.digest)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE digest != ?")
        .bind(&session.digest)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"saved":true})))
}

pub async fn application(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::session(&state, &headers).await?;
    Ok(Json(json!({"boot_id":state.application.boot_id})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    pub action: Action,
    pub confirmed: bool,
}
pub async fn control(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<Control>,
) -> Result<Response> {
    auth::require_write(&state, &headers).await?;
    if !form.confirmed || form.action == Action::Running {
        return Err(AppError::BadRequest(
            "Confirm restart or stop explicitly.".into(),
        ));
    }
    state.application.action.send_if_modified(|current| {
        if *current == Action::Running {
            *current = form.action;
            true
        } else {
            false
        }
    });
    if *state.application.action.borrow() != form.action {
        return Err(AppError::Conflict(
            "Another application action is already in progress.".into(),
        ));
    }
    Ok((StatusCode::ACCEPTED, Json(json!({"accepted":true}))).into_response())
}
