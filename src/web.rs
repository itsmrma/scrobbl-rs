use crate::{
    AppState, auth,
    error::{AppError, Result},
    imports, spotify,
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Europe::Rome;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{QueryBuilder, Row, Sqlite};

pub fn router(state: AppState) -> Router {
    let upload_limit =
        usize::try_from(state.config.import_limits.upload_bytes).unwrap_or(usize::MAX);
    Router::new()
        .route("/", get(index))
        .route("/healthz", get(health))
        .route("/license", get(license))
        .route("/licenses", get(licenses))
        .route("/app.js", get(js))
        .route("/style.css", get(css))
        .route("/favicon.svg", get(icon))
        .route("/settings", get(index))
        .route("/settings/{section}", get(index))
        .route("/spotify/connect-result/{attempt}", get(index))
        .route("/dashboard", get(index))
        .route("/history", get(index))
        .route("/import", get(index))
        .route("/activities", get(index))
        .route("/activities/{id}", get(index))
        .route(
            "/integrations/api-keys",
            get(|| async { Redirect::permanent("/settings/api-keys") }),
        )
        .route("/material-icons.woff2", get(icons_font))
        .route(
            "/api/v1/preferences",
            get(crate::preferences::get).post(crate::preferences::save),
        )
        .route("/api/v1/activities", get(crate::activities::list))
        .route(
            "/api/v1/recovery",
            get(crate::recovery_http::list).post(crate::recovery_http::start),
        )
        .route(
            "/api/v1/recovery/{id}",
            get(crate::recovery_http::download).post(crate::recovery_http::action),
        )
        .route(
            "/api/v1/recovery/restore",
            post(crate::recovery_http::restore),
        )
        .route(
            "/api/v1/recovery/resume",
            post(crate::recovery_http::resume),
        )
        .route(
            "/api/v1/activities/{id}",
            get(crate::activities::detail).post(crate::activities::action),
        )
        .route("/api/v1/import/limits", get(imports::limits))
        .route("/api/session", get(auth::status))
        .route("/api/setup", post(auth::setup))
        .route("/api/login", post(auth::login))
        .route("/api/logout", post(auth::logout))
        .route("/api/history", get(history))
        .route("/api/dashboard", get(dashboard))
        .route("/api/spotify/status", get(spotify::status))
        .route("/api/spotify/intervals", post(spotify::save_intervals))
        .route("/api/spotify/credentials", post(spotify::save_credentials))
        .route("/api/spotify/connect", post(spotify::connect))
        .route(
            "/api/spotify/connect/{attempt}",
            get(spotify::connect_status).post(spotify::cancel_connect),
        )
        .route("/api/spotify/disconnect", post(spotify::disconnect))
        .route("/api/spotify/sync", post(spotify::sync_now))
        .route("/api/spotify/now-playing", get(spotify::now_playing))
        .route("/auth/spotify/callback", get(spotify::callback))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .route(
            "/api/v1/recovery/upload",
            post(crate::recovery_http::upload).layer(DefaultBodyLimit::max(
                crate::recovery::PACKED_LIMIT as usize + 65536,
            )),
        )
        .route(
            "/api/import/spotify",
            post(imports::upload).layer(DefaultBodyLimit::max(
                upload_limit.saturating_add(state.config.import_limits.files.saturating_mul(1024)),
            )),
        )
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error":"Page not found."})),
            )
        })
        .layer(middleware::from_fn_with_state(state.clone(), security))
        .with_state(state)
}
async fn health(State(state): State<AppState>) -> Result<&'static str> {
    sqlx::query("SELECT 1").execute(&state.read_db).await?;
    Ok("ok")
}
async fn license() -> &'static str {
    include_str!("../LICENSE")
}
async fn licenses() -> &'static str {
    include_str!("../static/dist/third-party-notices.txt")
}
async fn index() -> Html<&'static str> {
    Html(include_str!("../static/dist/index.html"))
}
async fn js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../static/dist/app.js"),
    )
}
async fn css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/dist/style.css"),
    )
}
async fn icon() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        include_str!("../static/favicon.svg"),
    )
}
async fn icons_font() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "font/woff2")],
        include_bytes!("../static/dist/material-icons.woff2").as_slice(),
    )
}
async fn security(State(state): State<AppState>, request: Request<Body>, next: Next) -> Response {
    if request.uri().path().starts_with("/api/v1/recovery")
        && request.headers().contains_key(header::AUTHORIZATION)
    {
        return AppError::Forbidden.into_response();
    }
    // All in-flight HTTP requests drain before confirmation. The durable marker rejects
    // later mutations; startup activates only after this process has stopped.
    let gate = state.storage.requests.clone();
    let exclusive =
        request.uri().path() == "/api/v1/recovery/restore" && request.method() == Method::POST;
    let _read = if exclusive {
        None
    } else {
        Some(gate.read().await)
    };
    let _write = if exclusive {
        Some(gate.write().await)
    } else {
        None
    };
    if state.storage.directory.join("recovery.pending").exists()
        && ((request.method() != Method::GET && request.method() != Method::HEAD)
            || request.uri().path() == "/auth/spotify/callback")
    {
        return AppError::Conflict(
            "Restore is prepared. Restart the container, then sign in with the new password."
                .into(),
        )
        .into_response();
    }
    let expected_host = state.config.host();
    if request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        != Some(expected_host.as_str())
    {
        return AppError::Forbidden.into_response();
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        if request
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            != Some(state.config.origin().as_str())
        {
            return AppError::Forbidden.into_response();
        }
        if !matches!(request.uri().path(), "/api/login" | "/api/setup")
            && let Err(e) = auth::require_write(&state, request.headers()).await
        {
            return e.into_response();
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::CONTENT_SECURITY_POLICY,HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' https://i.scdn.co; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}
#[derive(Default, Deserialize, serde::Serialize, Clone)]
pub struct Filters {
    pub period: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub q: Option<String>,
    pub page: Option<u32>,
    pub status: Option<String>,
    pub sort: Option<String>,
}
fn midnight(date: NaiveDate) -> Result<i64> {
    Rome.from_local_datetime(&date.and_hms_opt(0, 0, 0).ok_or(AppError::Internal)?)
        .single()
        .map(|v| v.timestamp_millis())
        .ok_or_else(|| {
            AppError::BadRequest("This local date cannot be represented in Europe/Rome.".into())
        })
}
pub fn bounds(filters: &Filters) -> Result<(Option<i64>, Option<i64>)> {
    let today = Utc::now().with_timezone(&Rome).date_naive();
    let custom = filters.from.as_deref().filter(|v| !v.is_empty()).is_some()
        || filters.to.as_deref().filter(|v| !v.is_empty()).is_some();
    if custom || filters.period.as_deref() == Some("custom") {
        let parse = |s: &Option<String>| -> Result<Option<NaiveDate>> {
            s.as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| {
                    NaiveDate::parse_from_str(s, "%Y-%m-%d")
                        .map_err(|_| AppError::BadRequest("Use dates in YYYY-MM-DD format.".into()))
                })
                .transpose()
        };
        let from = parse(&filters.from)?;
        let to = parse(&filters.to)?;
        if from.zip(to).is_some_and(|(f, t)| f > t) {
            return Err(AppError::BadRequest(
                "The end date must be on or after the start date.".into(),
            ));
        }
        let end = to
            .map(|d| {
                d.succ_opt()
                    .ok_or_else(|| AppError::BadRequest("End date is out of range.".into()))
            })
            .transpose()?;
        return Ok((
            from.map(midnight).transpose()?,
            end.map(midnight).transpose()?,
        ));
    }
    let start = match filters.period.as_deref().unwrap_or("all") {
        "all" => return Ok((None, None)),
        "today" => today,
        "week" => today - Duration::days(today.weekday().num_days_from_monday().into()),
        "month" => today.with_day(1).ok_or(AppError::Internal)?,
        "year" => NaiveDate::from_ymd_opt(today.year(), 1, 1).ok_or(AppError::Internal)?,
        "7d" => {
            return Ok((
                Some(auth::now_ms() - 7 * 86_400_000),
                Some(auth::now_ms() + 1),
            ));
        }
        "30d" => {
            return Ok((
                Some(auth::now_ms() - 30 * 86_400_000),
                Some(auth::now_ms() + 1),
            ));
        }
        "365d" => {
            return Ok((
                Some(auth::now_ms() - 365 * 86_400_000),
                Some(auth::now_ms() + 1),
            ));
        }
        _ => return Err(AppError::BadRequest("Unknown period.".into())),
    };
    Ok((
        Some(midnight(start)?),
        Some(midnight(today.succ_opt().ok_or(AppError::Internal)?)?),
    ))
}
pub(crate) async fn selection(
    conn: &mut sqlx::SqliteConnection,
    filters: &Filters,
) -> Result<QueryBuilder<Sqlite>> {
    selection_projection(conn, filters, true).await
}
async fn selection_projection(
    conn: &mut sqlx::SqliteConnection,
    filters: &Filters,
    metadata: bool,
) -> Result<QueryBuilder<Sqlite>> {
    let mut b = QueryBuilder::new("WITH selected AS (SELECT l.*");
    if metadata {
        b.push(", t.title, t.album_id, t.spotify_url, a.name AS album, a.cover_url");
    }
    b.push(" FROM listens l");
    if metadata || filters.q.as_deref().is_some_and(|q| !q.trim().is_empty()) {
        b.push(" JOIN tracks t ON t.id=l.track_id LEFT JOIN albums a ON a.id=t.album_id");
    }
    b.push(" WHERE l.trashed_at_ms IS NULL");
    let parse = |s: &Option<String>| -> Result<Option<NaiveDate>> {
        s.as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| {
                NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .map_err(|_| AppError::BadRequest("Use dates in YYYY-MM-DD format.".into()))
            })
            .transpose()
    };
    let period = if filters.from.as_deref().is_some_and(|s| !s.is_empty())
        || filters.to.as_deref().is_some_and(|s| !s.is_empty())
        || filters.period.as_deref() == Some("custom")
    {
        crate::calendar::Period::Dates {
            from: parse(&filters.from)?,
            through: parse(&filters.to)?,
        }
    } else {
        match filters.period.as_deref().unwrap_or("all") {
            "all" => crate::calendar::Period::All,
            "today" => crate::calendar::Period::Today,
            "week" => crate::calendar::Period::Week,
            "month" => crate::calendar::Period::Month,
            "year" => crate::calendar::Period::Year,
            "7d" => crate::calendar::Period::RollingDays(7),
            "30d" => crate::calendar::Period::RollingDays(30),
            "365d" => crate::calendar::Period::RollingDays(365),
            _ => return Err(AppError::BadRequest("Unknown period.".into())),
        }
    };
    crate::calendar::filter(conn, &mut b, &period, auth::now_ms()).await?;
    if let Some(q) = filters.q.as_deref().filter(|s| !s.trim().is_empty()) {
        if q.len() > 500 {
            return Err(AppError::BadRequest(
                "Search is limited to 500 bytes.".into(),
            ));
        }
        let escaped = q
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        b.push(" AND (t.title LIKE ").push_bind(pattern.clone()).push(" ESCAPE '\\' OR a.name LIKE ").push_bind(pattern.clone()).push(" ESCAPE '\\' OR EXISTS(SELECT 1 FROM track_artists ta JOIN artists ar ON ar.id=ta.artist_id WHERE ta.track_id=t.id AND ar.name LIKE ").push_bind(pattern).push(" ESCAPE '\\'))");
    }
    match filters.status.as_deref().unwrap_or("all") {
        "all" => {}
        "counted" => {
            b.push(" AND l.counted=1");
        }
        "short" => {
            b.push(" AND l.counted=0");
        }
        "actual" => {
            b.push(" AND l.duration_kind='actual'");
        }
        "review" => {
            b.push(" AND l.needs_review=1");
        }
        "estimated" => {
            b.push(" AND l.duration_kind IN ('estimated','unknown')");
        }
        _ => return Err(AppError::BadRequest("Unknown listen filter.".into())),
    }
    b.push(") ");
    Ok(b)
}
fn listen_json(r: sqlx::sqlite::SqliteRow) -> Value {
    json!({"id":r.get::<String,_>("id"),"track_id":r.get::<String,_>("track_id"),"title":r.get::<String,_>("title"),"album":r.get::<Option<String>,_>("album"),"cover_url":r.get::<Option<String>,_>("cover_url"),"spotify_url":r.get::<Option<String>,_>("spotify_url"),"played_at_ms":r.get::<i64,_>("played_at_ms"),"listened_ms":r.get::<Option<i64>,_>("listened_ms"),"duration_kind":r.get::<String,_>("duration_kind"),"counted":r.get::<bool,_>("counted"),"needs_review":r.get::<bool,_>("needs_review"),
        "artists":serde_json::from_str::<Value>(&r.get::<String,_>("artists")).unwrap_or(json!([])),"sources":serde_json::from_str::<Value>(&r.get::<String,_>("sources")).unwrap_or(json!([]))})
}
const LISTEN_COLUMNS: &str = "SELECT s.*, (SELECT json_group_array(json_object('id',id,'name',name)) FROM (SELECT ar.id, ar.name FROM track_artists ta JOIN artists ar ON ar.id=ta.artist_id WHERE ta.track_id=s.track_id ORDER BY ta.ordinal)) AS artists, (SELECT json_group_array(source) FROM observations o WHERE o.listen_id=s.id) AS sources FROM selected s ";
pub async fn history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filters): Query<Filters>,
) -> Result<Json<crate::dto::History>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    let page = filters.page.unwrap_or(1).clamp(1, 1_000_000);
    let mut count = selection(&mut view.transaction, &filters).await?;
    count.push("SELECT COUNT(*) FROM selected");
    let total: i64 = count
        .build_query_scalar()
        .fetch_one(&mut *view.transaction)
        .await?;
    let mut query = selection(&mut view.transaction, &filters).await?;
    query
        .push(LISTEN_COLUMNS)
        .push("ORDER BY s.played_at_ms DESC, s.id DESC LIMIT 50 OFFSET ")
        .push_bind((page as i64 - 1) * 50);
    let items: Vec<_> = query
        .build()
        .fetch_all(&mut *view.transaction)
        .await?
        .into_iter()
        .map(listen_json)
        .collect();
    Ok(Json(
        serde_json::from_value(json!({"items":items,"total":total,"page":page,"page_size":50,"timezone":crate::preferences::read(&mut view.transaction).await?.timezone})).map_err(|_| AppError::Internal)?,
    ))
}
pub async fn dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filters): Query<Filters>,
) -> Result<Json<crate::dto::Dashboard>> {
    auth::session(&state, &headers).await?;
    let mut cache = state.dashboard_cache.lock().await;
    let mut view = state.storage.read_view().await?;
    let preferences = crate::preferences::read(&mut view.transaction).await?;
    // Absolute selections remain valid until archive identity/revision or preferences
    // change. Relative periods depend on the clock and are always recalculated.
    let cacheable = matches!(filters.period.as_deref().unwrap_or("all"), "all" | "custom")
        || filters.from.as_deref().is_some_and(|s| !s.is_empty())
        || filters.to.as_deref().is_some_and(|s| !s.is_empty());
    let key = serde_json::to_string(&(&view.identity, &preferences, &filters))
        .map_err(|_| AppError::Internal)?;
    if cacheable && let Some((_, value)) = cache.iter().find(|(stored, _)| stored == &key) {
        return Ok(Json(
            serde_json::from_value(value.clone()).map_err(|_| AppError::Internal)?,
        ));
    }
    let order = match filters.sort.as_deref().unwrap_or("plays") {
        "plays" => "plays DESC, name COLLATE NOCASE",
        "time" => "listened_ms DESC, name COLLATE NOCASE",
        _ => return Err(AppError::BadRequest("Unknown ranking order.".into())),
    };
    let mut query = selection_projection(&mut view.transaction, &filters, false).await?;
    query.push(include_str!("dashboard.sql").replace("/*ORDER*/", order));
    let aggregates: String = query
        .build_query_scalar()
        .fetch_one(&mut *view.transaction)
        .await?;
    let aggregates: Value = serde_json::from_str(&aggregates).map_err(|_| AppError::Internal)?;
    let totals = &aggregates["totals"];
    let rankings = &aggregates["rankings"];
    let mut recent = selection(&mut view.transaction, &filters).await?;
    recent
        .push(LISTEN_COLUMNS)
        .push(" ORDER BY s.played_at_ms DESC, s.id DESC LIMIT 8");
    let recent: Vec<_> = recent
        .build()
        .fetch_all(&mut *view.transaction)
        .await?
        .into_iter()
        .map(listen_json)
        .collect();
    let value = json!({"totals":totals,"rankings":rankings,"recent":recent,"timezone":preferences.timezone});
    let response = serde_json::from_value(value.clone()).map_err(|_| AppError::Internal)?;
    if cacheable {
        // At most sixteen small response projections; no originals or complete history.
        if cache.len() == 16 {
            cache.pop_front();
        }
        cache.push_back((key, value));
    }
    Ok(Json(response))
}
