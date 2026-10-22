//! Owner-only access management. Integration secrets never enter the music archive.
use crate::{
    AppState, auth,
    error::{AppError, Result},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Serialize, ts_rs::TS)]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    pub application: String,
    pub permissions: String,
    pub created_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
    pub expires_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}
#[derive(Serialize, ts_rs::TS)]
pub struct ApiKeySettings {
    pub base_url: String,
    pub cors_origins: Vec<String>,
    pub keys: Vec<ApiKey>,
}
#[derive(Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct CreateApiKey {
    pub name: String,
    pub application: String,
    pub expires_at_ms: Option<i64>,
}
#[derive(Serialize, ts_rs::TS)]
pub struct CreatedApiKey {
    pub id: String,
    pub secret: String,
}
#[derive(Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ApiCorsSettings {
    pub origins: Vec<String>,
}
pub async fn origins(state: &AppState) -> Result<Vec<String>> {
    let value = crate::db::setting(&state.private_db, "api_cors_origins").await?;
    value
        .map(|v| serde_json::from_str(&v).map_err(|_| AppError::Internal))
        .unwrap_or_else(|| Ok(vec![]))
}
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiKeySettings>> {
    auth::session(&state, &headers).await?;
    let rows = sqlx::query("SELECT * FROM api_keys ORDER BY created_at_ms DESC,id")
        .fetch_all(&state.private_db)
        .await?;
    Ok(Json(ApiKeySettings {
        base_url: format!("{}/api/v1", state.config.origin()),
        cors_origins: origins(&state).await?,
        keys: rows
            .into_iter()
            .map(|r| ApiKey {
                id: r.get("id"),
                name: r.get("name"),
                application: r.get("application"),
                permissions: "history:read".into(),
                created_at_ms: r.get("created_at_ms"),
                last_used_at_ms: r.get("last_used_at_ms"),
                expires_at_ms: r.get("expires_at_ms"),
                revoked_at_ms: r.get("revoked_at_ms"),
            })
            .collect(),
    }))
}
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<CreateApiKey>,
) -> Result<Json<CreatedApiKey>> {
    auth::require_write(&state, &headers).await?;
    let name = form.name.trim();
    let application = form.application.trim();
    if name.is_empty()
        || name.len() > 120
        || application.is_empty()
        || application.len() > 120
        || form
            .expires_at_ms
            .is_some_and(|t| t <= auth::now_ms() || t > 8_640_000_000_000_000)
    {
        return Err(AppError::BadRequest(
            "Enter a name, an application and an optional future expiry.".into(),
        ));
    }
    let secret = auth::random_secret();
    let id = auth::random_secret();
    sqlx::query("INSERT INTO api_keys(digest,id,name,application,created_at_ms,expires_at_ms) VALUES(?,?,?,?,?,?)")
        .bind(auth::digest(&secret)).bind(&id).bind(name).bind(application).bind(auth::now_ms()).bind(form.expires_at_ms).execute(&state.private_db).await?;
    Ok(Json(CreatedApiKey { id, secret }))
}
pub async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    auth::require_write(&state, &headers).await?;
    let result =
        sqlx::query("UPDATE api_keys SET revoked_at_ms=COALESCE(revoked_at_ms,?) WHERE id=?")
            .bind(auth::now_ms())
            .bind(&id)
            .execute(&state.private_db)
            .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({"revoked":true})))
}
pub async fn save_origins(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<ApiCorsSettings>,
) -> Result<Json<ApiCorsSettings>> {
    auth::require_write(&state, &headers).await?;
    if form.origins.len() > 20 {
        return Err(AppError::BadRequest(
            "Use at most 20 explicit origins.".into(),
        ));
    }
    let mut origins = Vec::new();
    for value in form.origins {
        let url = crate::config::parse_origin(value.trim()).map_err(|_| {
            AppError::BadRequest("Use explicit HTTPS origins without a path or wildcard.".into())
        })?;
        let loopback = matches!(url.host(),Some(url::Host::Ipv4(ip)) if ip.is_loopback())
            || matches!(url.host(),Some(url::Host::Ipv6(ip)) if ip.is_loopback());
        if url.host_str().is_some_and(|h| h.contains('*')) || (url.scheme() != "https" && !loopback)
        {
            return Err(AppError::BadRequest(
                "Use HTTPS; HTTP is supported only on loopback for development.".into(),
            ));
        }
        let origin = url.origin().ascii_serialization();
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }
    crate::db::set_setting(
        &mut *state.private_db.acquire().await?,
        "api_cors_origins",
        &serde_json::to_string(&origins).map_err(|_| AppError::Internal)?,
    )
    .await?;
    Ok(Json(ApiCorsSettings { origins }))
}
