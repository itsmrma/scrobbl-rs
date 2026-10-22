//! Read-only integration protocol. Cursors are random handles, never authorization.
use crate::{
    AppState, auth,
    consultation::{Identity, Snapshot},
    db::DurationKind,
};
use axum::{
    Json,
    extract::{Query, Request, State, rejection::QueryRejection},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};

type Result<T> = std::result::Result<T, ApiError>;
#[derive(Debug)]
pub struct ApiError(StatusCode, &'static str, &'static str);
impl From<sqlx::Error> for ApiError {
    fn from(_: sqlx::Error) -> Self {
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "The archive could not be read. Retry later.",
        )
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.0,
            Json(serde_json::json!({"error":{"code":self.1,"message":self.2}})),
        )
            .into_response();
        if self.0 == StatusCode::TOO_MANY_REQUESTS || self.0 == StatusCode::SERVICE_UNAVAILABLE {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        }
        response
    }
}
fn invalid() -> ApiError {
    ApiError(
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "Use valid UTC millisecond bounds, counted_only and a page size from 1 to 1000. With a cursor, omit selection parameters.",
    )
}
fn cursor_invalid() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "cursor_invalid",
        "This copy or cursor is expired or incompatible. Load a new complete copy.",
    )
}
fn unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "snapshot_unavailable",
        "A coherent copy could not be prepared. Retry later or load a new complete copy.",
    )
}
fn valid_token(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
pub fn is_external(path: &str) -> bool {
    matches!(path, "/api/v1/info" | "/api/v1/listens" | "/api/v1/changes")
}

async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<String> {
    let secret = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .filter(|v| valid_token(v))
        .ok_or(ApiError(
            StatusCode::UNAUTHORIZED,
            "invalid_key",
            "Provide a valid, unexpired read-only API key.",
        ))?;
    let digest = auth::digest(secret);
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_keys WHERE digest=? AND revoked_at_ms IS NULL AND (expires_at_ms IS NULL OR expires_at_ms>?))").bind(&digest).bind(auth::now_ms()).fetch_one(&state.private_db).await?;
    if !valid {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "invalid_key",
            "Provide a valid, unexpired read-only API key.",
        ));
    }
    Ok(digest)
}

/// Called inside the common Host check and recovery request gate, before owner CSRF rules.
pub async fn security(state: AppState, request: Request, next: Next) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    let allowed = match crate::api_keys::origins(&state).await {
        Ok(v) => v,
        Err(_) => return unavailable().into_response(),
    };
    let cors = origin
        .as_ref()
        .filter(|o| allowed.contains(o) || **o == state.config.origin());
    let mut response = if origin.is_some() && cors.is_none() {
        ApiError(
            StatusCode::FORBIDDEN,
            "origin_not_allowed",
            "This browser origin is not allowed.",
        )
        .into_response()
    } else if request.method() == Method::OPTIONS {
        let method = request
            .headers()
            .get(header::ACCESS_CONTROL_REQUEST_METHOD)
            .and_then(|h| h.to_str().ok());
        let headers = request
            .headers()
            .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        if method != Some("GET")
            || headers
                .split(',')
                .filter(|h| !h.trim().is_empty())
                .any(|h| !h.trim().eq_ignore_ascii_case("authorization"))
        {
            ApiError(
                StatusCode::FORBIDDEN,
                "forbidden",
                "Only read requests with Authorization are allowed.",
            )
            .into_response()
        } else {
            StatusCode::NO_CONTENT.into_response()
        }
    } else if request.method() != Method::GET && request.method() != Method::HEAD {
        ApiError(
            StatusCode::METHOD_NOT_ALLOWED,
            "read_only",
            "This API only supports reads.",
        )
        .into_response()
    } else {
        match authenticate(&state, request.headers()).await {
            Err(error) => error.into_response(),
            Ok(digest) => {
                let window = auth::now_ms() / 60000;
                let count=sqlx::query_scalar::<_,i64>("INSERT INTO api_rate_limits(key_digest,window_ms,requests) VALUES(?,?,1) ON CONFLICT(key_digest) DO UPDATE SET window_ms=excluded.window_ms,requests=CASE WHEN window_ms=excluded.window_ms THEN requests+1 ELSE 1 END RETURNING requests")
                    .bind(&digest).bind(window).fetch_one(&state.private_db).await;
                match count {
                    Ok(n) if n <= 120 => {
                        let response = next.run(request).await;
                        if response.status().is_success() {
                            let _ =
                                sqlx::query("UPDATE api_keys SET last_used_at_ms=? WHERE digest=?")
                                    .bind(auth::now_ms())
                                    .bind(digest)
                                    .execute(&state.private_db)
                                    .await;
                        }
                        response
                    }
                    Ok(_) => ApiError(
                        StatusCode::TOO_MANY_REQUESTS,
                        "rate_limited",
                        "Too many requests. Wait before retrying.",
                    )
                    .into_response(),
                    Err(e) => ApiError::from(e).into_response(),
                }
            }
        }
    };
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if let Some(origin) = cors.and_then(|o| HeaderValue::from_str(o).ok()) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, HEAD, OPTIONS"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Authorization"),
        );
        headers.insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static("Retry-After"),
        );
    }
    response
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ApiSelection {
    pub from_ms: Option<i64>,
    pub until_ms: Option<i64>,
    pub counted_only: bool,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameters {
    cursor: Option<String>,
    from_ms: Option<i64>,
    until_ms: Option<i64>,
    counted_only: Option<bool>,
    limit: Option<i64>,
}
#[derive(Clone, Serialize, Deserialize)]
enum Mode {
    Full,
    Delta { from_revision: i64 },
    Checkpoint,
}
#[derive(Clone, Serialize, Deserialize)]
struct Cursor {
    version: u32,
    identity: Identity,
    selection: ApiSelection,
    mode: Mode,
    snapshot: Option<Snapshot>,
    after_time: Option<i64>,
    after_id: Option<String>,
}
#[derive(Serialize, Deserialize, ts_rs::TS)]
pub struct ApiListen {
    pub id: String,
    pub played_at_ms: i64,
    pub track_id: String,
    pub title: String,
    pub album_id: Option<String>,
    pub album: Option<String>,
    pub artists: Vec<crate::db::Artist>,
    pub track_duration_ms: Option<i64>,
    pub listened_ms: Option<i64>,
    pub duration_kind: DurationKind,
    pub counted: bool,
    pub sources: Vec<String>,
}
#[derive(Serialize, ts_rs::TS)]
pub struct ApiRemoval {
    pub id: String,
    pub replacement_id: Option<String>,
}
#[derive(Serialize, ts_rs::TS)]
pub struct ApiPage {
    pub api_version: u32,
    pub instance_id: String,
    pub generation: String,
    pub from_revision: Option<i64>,
    pub revision: i64,
    pub retained_from: i64,
    pub snapshot_id: String,
    pub expires_at_ms: i64,
    pub selection: ApiSelection,
    pub items: Vec<ApiListen>,
    pub removed: Vec<ApiRemoval>,
    pub next_cursor: Option<String>,
    pub sync_cursor: Option<String>,
    pub complete: bool,
}
async fn identity(state: &AppState) -> Result<Identity> {
    let (value, maintenance): (Identity, bool) = {
        let mut tx = state.read_db.begin().await?;
        let value=sqlx::query_as("SELECT instance_id,generation,revision,retained_from FROM archive_state WHERE singleton=1").fetch_one(&mut *tx).await?;
        let maintenance: bool =
            sqlx::query_scalar("SELECT maintenance FROM archive_state WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        (value, maintenance)
    };
    if maintenance {
        return Err(unavailable());
    }
    Ok(value)
}
pub async fn info(State(state): State<AppState>) -> Result<Json<serde_json::Value>> {
    let identity = identity(&state).await?;
    Ok(Json(
        serde_json::json!({"api_version":1,"instance_id":identity.instance_id,"generation":identity.generation,"revision":identity.revision,"retained_from":identity.retained_from,"timestamp_unit":"unix_ms_utc","duration_unit":"ms","max_page_size":1000,"snapshot_ttl_seconds":state.config.api_snapshot_limits.ttl.as_secs(),"change_retention_revisions":10000}),
    ))
}
async fn save_cursor(state: &AppState, key: &str, cursor: &Cursor, expires: i64) -> Result<String> {
    let mut tx = state.private_db.begin().await?;
    sqlx::query("DELETE FROM api_cursors WHERE expires_at_ms<=?")
        .bind(auth::now_ms())
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_cursors")
        .fetch_one(&mut *tx)
        .await?;
    if count >= 10000 {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "cursor_quota",
            "Too many active cursors. Retry after older copies expire.",
        ));
    }
    let token = auth::random_secret();
    sqlx::query("INSERT INTO api_cursors VALUES(?,?,?,?)")
        .bind(&token)
        .bind(key)
        .bind(serde_json::to_string(cursor).map_err(|_| unavailable())?)
        .bind(expires)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(token)
}
async fn load_cursor(state: &AppState, key: &str, token: &str) -> Result<Cursor> {
    if !valid_token(token) {
        return Err(cursor_invalid());
    }
    let value: Option<String> = sqlx::query_scalar(
        "SELECT value FROM api_cursors WHERE token=? AND key_digest=? AND expires_at_ms>?",
    )
    .bind(token)
    .bind(key)
    .bind(auth::now_ms())
    .fetch_optional(&state.private_db)
    .await?;
    let cursor: Cursor =
        serde_json::from_str(&value.ok_or_else(cursor_invalid)?).map_err(|_| cursor_invalid())?;
    if cursor.version != 1 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "version_incompatible",
            "This cursor uses an incompatible API version. Load a new complete copy.",
        ));
    }
    Ok(cursor)
}
async fn frozen(state: &AppState) -> Result<(Snapshot, SqliteConnection, Identity)> {
    let snapshot = state
        .storage
        .materialize(&state.config.api_snapshot_limits)
        .await
        .map_err(|_| unavailable())?;
    let (conn, identity) = open_snapshot(state, &snapshot).await?;
    Ok((snapshot, conn, identity))
}
async fn open_snapshot(
    state: &AppState,
    snapshot: &Snapshot,
) -> Result<(SqliteConnection, Identity)> {
    if !valid_token(&snapshot.id) || snapshot.expires_at_ms <= auth::now_ms() {
        return Err(cursor_invalid());
    }
    let live = identity(state).await?;
    if live.instance_id != snapshot.instance_id || live.generation != snapshot.generation {
        return Err(cursor_invalid());
    }
    let path = state
        .storage
        .directory
        .join("snapshots")
        .join(format!("{}.db", snapshot.id));
    let mut conn =
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(path).read_only(true))
            .await
            .map_err(|_| cursor_invalid())?;
    let frozen: Identity = sqlx::query_as(
        "SELECT instance_id,generation,revision,retained_from FROM archive_state WHERE singleton=1",
    )
    .fetch_one(&mut conn)
    .await?;
    if frozen.instance_id != snapshot.instance_id
        || frozen.generation != snapshot.generation
        || frozen.revision != snapshot.revision
    {
        return Err(cursor_invalid());
    }
    Ok((conn, frozen))
}
pub async fn listens(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: std::result::Result<Query<Parameters>, QueryRejection>,
) -> Result<Json<ApiPage>> {
    page(state, headers, params.map_err(|_| invalid())?.0, false)
        .await
        .map(Json)
}
pub async fn changes(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: std::result::Result<Query<Parameters>, QueryRejection>,
) -> Result<Json<ApiPage>> {
    page(state, headers, params.map_err(|_| invalid())?.0, true)
        .await
        .map(Json)
}
async fn page(
    state: AppState,
    headers: HeaderMap,
    params: Parameters,
    delta: bool,
) -> Result<ApiPage> {
    let key = authenticate(&state, &headers).await?;
    let limit = params.limit.unwrap_or(500);
    if !(1..=1000).contains(&limit) {
        return Err(invalid());
    }
    let (mut cursor, mut conn) = if let Some(token) = params.cursor {
        if params.from_ms.is_some() || params.until_ms.is_some() || params.counted_only.is_some() {
            return Err(invalid());
        }
        let mut cursor = load_cursor(&state, &key, &token).await?;
        if matches!(cursor.mode, Mode::Checkpoint) {
            if !delta {
                return Err(cursor_invalid());
            }
            let live = identity(&state).await?;
            if cursor.identity.instance_id != live.instance_id
                || cursor.identity.generation != live.generation
                || cursor.identity.revision < live.retained_from
                || cursor.identity.revision > live.revision
            {
                return Err(cursor_invalid());
            }
            let (snapshot, conn, target) = frozen(&state).await?;
            if cursor.identity.revision < target.retained_from
                || cursor.identity.instance_id != target.instance_id
                || cursor.identity.generation != target.generation
            {
                return Err(cursor_invalid());
            }
            cursor.mode = Mode::Delta {
                from_revision: cursor.identity.revision,
            };
            cursor.identity = target;
            cursor.snapshot = Some(snapshot);
            cursor.after_time = None;
            cursor.after_id = None;
            (cursor, conn)
        } else {
            if delta != matches!(cursor.mode, Mode::Delta { .. }) {
                return Err(cursor_invalid());
            }
            let (conn, _) =
                open_snapshot(&state, cursor.snapshot.as_ref().ok_or_else(cursor_invalid)?).await?;
            (cursor, conn)
        }
    } else {
        if delta {
            return Err(invalid());
        }
        let selection = ApiSelection {
            from_ms: params.from_ms,
            until_ms: params.until_ms,
            counted_only: params.counted_only.unwrap_or(false),
        };
        if selection
            .from_ms
            .zip(selection.until_ms)
            .is_some_and(|(a, b)| a >= b)
            || [selection.from_ms, selection.until_ms]
                .into_iter()
                .flatten()
                .any(|v| v.unsigned_abs() > 8_640_000_000_000_000)
        {
            return Err(invalid());
        }
        let (snapshot, conn, identity) = frozen(&state).await?;
        (
            Cursor {
                version: 1,
                identity,
                selection,
                mode: Mode::Full,
                snapshot: Some(snapshot),
                after_time: None,
                after_id: None,
            },
            conn,
        )
    };
    // The outer row set includes changed IDs outside the selection so they become removals.
    let mut query = sqlx::QueryBuilder::new(format!(
        "{}SELECT l.*, t.title,t.album_id,t.duration_ms AS track_duration_ms,a.name AS album,(SELECT json_group_array(json_object('id',id,'name',name)) FROM (SELECT ar.id,ar.name FROM track_artists ta JOIN artists ar ON ar.id=ta.artist_id WHERE ta.track_id=l.track_id ORDER BY ta.ordinal,ar.id)) AS artists,(SELECT json_group_array(source) FROM (SELECT DISTINCT source FROM observations WHERE listen_id=l.id ORDER BY source)) AS sources",
        crate::library::METADATA
    ));
    match cursor.mode {
        Mode::Full => {
            query.push(", l.id AS changed_id, NULL AS replacement_id FROM listens l JOIN tracks t ON t.id=l.track_id LEFT JOIN albums a ON a.id=t.album_id WHERE l.trashed_at_ms IS NULL");
            selection_sql(&mut query, &cursor.selection);
            if let (Some(time), Some(id)) = (cursor.after_time, cursor.after_id.clone()) {
                query
                    .push(" AND (l.played_at_ms,l.id)>(")
                    .push_bind(time)
                    .push(",")
                    .push_bind(id)
                    .push(")");
            }
            query.push(" ORDER BY l.played_at_ms,l.id");
        }
        Mode::Delta { from_revision } => {
            query.push(", c.listen_id AS changed_id,c.replacement_id FROM (SELECT listen_id,MAX(revision) AS revision FROM changes WHERE revision>").push_bind(from_revision).push(" GROUP BY listen_id) latest JOIN changes c ON c.listen_id=latest.listen_id AND c.revision=latest.revision LEFT JOIN listens l ON l.id=c.listen_id LEFT JOIN tracks t ON t.id=l.track_id LEFT JOIN albums a ON a.id=t.album_id WHERE 1=1");
            if let Some(id) = cursor.after_id.clone() {
                query.push(" AND c.listen_id>").push_bind(id);
            }
            query.push(" ORDER BY c.listen_id");
        }
        Mode::Checkpoint => return Err(cursor_invalid()),
    }
    query.push(" LIMIT ").push_bind(limit + 1);
    let mut rows = query.build().fetch_all(&mut conn).await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let mut items = Vec::new();
    let mut removed = Vec::new();
    for row in rows {
        let id: String = row.get("changed_id");
        cursor.after_id = Some(id.clone());
        let time: Option<i64> = row.try_get("played_at_ms")?;
        cursor.after_time = time;
        let counted: Option<bool> = row.try_get("counted")?;
        let trashed: Option<i64> = row.try_get("trashed_at_ms")?;
        let selected = time.is_some_and(|t| {
            cursor.selection.from_ms.is_none_or(|f| t >= f)
                && cursor.selection.until_ms.is_none_or(|f| t < f)
        }) && trashed.is_none()
            && (!cursor.selection.counted_only || counted == Some(true));
        if !selected {
            removed.push(ApiRemoval {
                id,
                replacement_id: row.get("replacement_id"),
            });
            continue;
        }
        items.push(ApiListen {
            id,
            played_at_ms: time.ok_or_else(unavailable)?,
            track_id: row.get("track_id"),
            title: row.get("title"),
            album_id: row.get("album_id"),
            album: row.get("album"),
            artists: serde_json::from_str(&row.get::<String, _>("artists"))
                .map_err(|_| unavailable())?,
            track_duration_ms: row.get("track_duration_ms"),
            listened_ms: row.get("listened_ms"),
            duration_kind: serde_json::from_value(serde_json::Value::String(
                row.get("duration_kind"),
            ))
            .map_err(|_| unavailable())?,
            counted: counted.unwrap_or(false),
            sources: serde_json::from_str(&row.get::<String, _>("sources"))
                .map_err(|_| unavailable())?,
        });
    }
    conn.close().await?;
    let snapshot = cursor.snapshot.clone().ok_or_else(unavailable)?;
    let from_revision = match cursor.mode {
        Mode::Delta { from_revision } => Some(from_revision),
        _ => None,
    };
    let next_cursor = if more {
        Some(save_cursor(&state, &key, &cursor, snapshot.expires_at_ms).await?)
    } else {
        None
    };
    let sync_cursor = if more {
        None
    } else {
        let mut checkpoint = cursor.clone();
        checkpoint.mode = Mode::Checkpoint;
        checkpoint.snapshot = None;
        checkpoint.after_id = None;
        checkpoint.after_time = None;
        Some(save_cursor(&state, &key, &checkpoint, auth::now_ms() + 86_400_000).await?)
    };
    // A restore or a revocation during preparation must not publish an incompatible page.
    let live = identity(&state).await?;
    if live.instance_id != cursor.identity.instance_id
        || live.generation != cursor.identity.generation
    {
        return Err(cursor_invalid());
    }
    authenticate(&state, &headers).await?;
    if !more {
        let _guard = state.storage.snapshots.lock().await;
        let _ = std::fs::remove_file(
            state
                .storage
                .directory
                .join("snapshots")
                .join(format!("{}.db", snapshot.id)),
        );
    }
    Ok(ApiPage {
        api_version: 1,
        instance_id: cursor.identity.instance_id,
        generation: cursor.identity.generation,
        from_revision,
        revision: cursor.identity.revision,
        retained_from: cursor.identity.retained_from,
        snapshot_id: snapshot.id,
        expires_at_ms: snapshot.expires_at_ms,
        selection: cursor.selection,
        items,
        removed,
        next_cursor,
        sync_cursor,
        complete: !more,
    })
}
fn selection_sql(query: &mut sqlx::QueryBuilder<sqlx::Sqlite>, selection: &ApiSelection) {
    if let Some(from) = selection.from_ms {
        query.push(" AND l.played_at_ms>=").push_bind(from);
    }
    if let Some(until) = selection.until_ms {
        query.push(" AND l.played_at_ms<").push_bind(until);
    }
    if selection.counted_only {
        query.push(" AND l.counted=1");
    }
}
