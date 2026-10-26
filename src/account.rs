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
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, CASE WHEN key='profile_avatar' THEN '1' ELSE value END FROM settings WHERE key IN ('owner_username','profile_name','profile_avatar_etag','profile_avatar')",
    )
    .fetch_all(&state.private_db)
    .await?;
    let mut username = None;
    let mut name = None;
    let mut avatar_etag = None;
    let mut has_avatar = false;
    for (key, value) in rows {
        match key.as_str() {
            "owner_username" => username = Some(value),
            "profile_name" => name = Some(value),
            "profile_avatar_etag" => avatar_etag = Some(value),
            "profile_avatar" => has_avatar = true,
            _ => {}
        }
    }
    if has_avatar
        && avatar_etag.is_none()
        && let Some(raw) =
            sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key='profile_avatar'")
                .fetch_optional(&state.private_db)
                .await?
    {
        let etag = auth::digest(&raw);
        let _ = sqlx::query(
            "INSERT INTO settings(key,value) VALUES('profile_avatar_etag',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        )
        .bind(&etag)
        .execute(&state.private_db)
        .await;
        avatar_etag = Some(etag);
    }
    let avatar_url = if has_avatar {
        avatar_etag.map(|etag| format!("/api/v1/account/avatar?v={etag}"))
    } else {
        None
    };
    Ok(Json(
        json!({"username":username,"name":name.unwrap_or_default(),"avatar_url":avatar_url}),
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
    let encoded = auth::encode_hex(&bytes);
    let etag = auth::digest(&encoded);
    let mut tx = state.private_db.begin().await?;
    sqlx::query("INSERT INTO settings(key,value) VALUES('profile_avatar',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
        .bind(&encoded)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO settings(key,value) VALUES('profile_avatar_etag',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
        .bind(&etag)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    profile(State(state), headers).await
}
pub async fn remove_avatar(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    sqlx::query("DELETE FROM settings WHERE key IN ('profile_avatar','profile_avatar_etag')")
        .execute(&state.private_db)
        .await?;
    profile(State(state), headers).await
}
fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
pub async fn avatar(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    auth::session(&state, &headers).await?;
    let value: String = sqlx::query_scalar("SELECT value FROM settings WHERE key='profile_avatar'")
        .fetch_optional(&state.private_db)
        .await?
        .ok_or(AppError::NotFound)?;
    if !value.len().is_multiple_of(2) {
        return Err(AppError::Internal);
    }
    let bytes = value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[hi, lo]| {
            hex_nibble(hi)
                .zip(hex_nibble(lo))
                .map(|(h, l)| (h << 4) | l)
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
