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
use serde_json::{Value, json};
use sqlx::Row;

pub fn router(state: AppState) -> Router {
    let upload_limit =
        usize::try_from(state.config.import_limits.upload_bytes).unwrap_or(usize::MAX);
    Router::new()
        .route("/", get(index))
        .route("/healthz", get(health))
        .route("/api/owner/username", post(auth::set_username))
        .route(
            "/api/v1/account",
            get(crate::account::profile).post(crate::account::edit_profile),
        )
        .route("/api/v1/account/password", post(crate::account::password))
        .route(
            "/api/v1/account/avatar/remove",
            post(crate::account::remove_avatar),
        )
        .route(
            "/api/v1/application",
            get(crate::account::application).post(crate::account::control),
        )
        .route("/license", get(license))
        .route("/licenses", get(licenses))
        .route("/app.js", get(js))
        .route("/style.css", get(css))
        .route("/favicon.svg", get(icon))
        .route("/settings", get(index))
        .route("/settings/{section}", get(index))
        .route("/spotify/connect-result/{attempt}", get(index))
        .route("/dashboard", get(index))
        .route("/search", get(index))
        .route("/history", get(index))
        .route("/review", get(index))
        .route(
            "/api/v1/review",
            get(crate::review::list).post(crate::review::decide),
        )
        .route("/api/v1/review/{id}/undo", post(crate::review::undo))
        .route("/api/v1/review/trash", get(crate::review::trash))
        .route(
            "/api/v1/consultation/patterns",
            get(crate::listening_patterns::get),
        )
        .route(
            "/api/v1/review/automatic",
            get(crate::review::automatic_preview).post(crate::review::automatic_execute),
        )
        .route("/rankings/{kind}", get(index))
        .route(
            "/api/v1/consultation/choices/{kind}",
            get(crate::browse::choices),
        )
        .route(
            "/api/v1/consultation/search/{kind}",
            get(crate::browse::search),
        )
        .route("/music/now-playing", get(index))
        .route("/music/{kind}/{id}", get(index))
        .route(
            "/api/v1/consultation/rankings/{kind}",
            get(crate::browse::rankings),
        )
        .route(
            "/api/v1/consultation/music/{kind}/{id}",
            get(crate::browse::music),
        )
        .route(
            "/api/v1/consultation/activity",
            get(crate::browse::activity),
        )
        .route(
            "/api/v1/consultation/discovery-ratio",
            get(crate::browse::discovery_ratio),
        )
        .route(
            "/api/v1/consultation/calendar",
            get(crate::browse::calendar),
        )
        .route(
            "/api/v1/consultation/milestones",
            get(crate::browse::milestones),
        )
        .route("/music/{kind}/{id}/edit", get(index))
        .route(
            "/api/v1/library/{kind}/{id}",
            get(crate::library::details).post(crate::library::edit),
        )
        .route(
            "/api/v1/artwork/{kind}/{id}",
            get(crate::library::artwork).post(crate::library::image_url),
        )
        .route(
            "/api/v1/artwork/{kind}/{id}/credit",
            get(crate::library::artwork_credit),
        )
        .route(
            "/api/v1/library/{kind}/{id}/group",
            get(crate::name_groups::members).post(crate::name_groups::change),
        )
        .route("/import", get(index))
        .route("/activities", get(index))
        .route("/activities/{id}", get(index))
        .route(
            "/integrations/api-keys",
            get(|| async { Redirect::permanent("/settings/api-keys") }),
        )
        .route("/material-icons.woff2", get(icons_font))
        .route(
            "/api/v1/artwork-providers",
            get(crate::artwork_settings::get).post(crate::artwork_settings::save),
        )
        .route(
            "/api/v1/preferences",
            get(crate::preferences::get).post(crate::preferences::save),
        )
        .route(
            "/api/v1/dashboard/layout",
            get(crate::dashboard_layout::get).post(crate::dashboard_layout::save),
        )
        .route(
            "/api/v1/listenbrainz/limits",
            get(crate::listenbrainz::limits),
        )
        .route(
            "/api/v1/listenbrainz/fetch",
            post(crate::listenbrainz::fetch),
        )
        .route(
            "/api/v1/listenbrainz/jobs/{id}/originals",
            get(crate::listenbrainz::originals),
        )
        .route(
            "/api/v1/listenbrainz/jobs/{id}/files/{file_id}",
            get(crate::listenbrainz::original_file),
        )
        .route("/api/v1/activities", get(crate::activities::list))
        .route("/api/v1/info", get(crate::read_api::info))
        .route("/api/v1/listens", get(crate::read_api::listens))
        .route("/api/v1/changes", get(crate::read_api::changes))
        .route(
            "/api/v1/api-keys",
            get(crate::api_keys::list).post(crate::api_keys::create),
        )
        .route(
            "/api/v1/api-keys/{id}/revoke",
            post(crate::api_keys::revoke),
        )
        .route("/api/v1/api-keys/cors", post(crate::api_keys::save_origins))
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
        .route(
            "/api/v1/activities/{id}/cancel",
            post(crate::activities::cancel),
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
            "/api/v1/account/avatar",
            get(crate::account::avatar)
                .post(crate::account::upload_avatar)
                .layer(DefaultBodyLimit::max(5 * 1024 * 1024)),
        )
        .route(
            "/api/v1/recovery/upload",
            post(crate::recovery_http::upload).layer(DefaultBodyLimit::max(
                crate::recovery::PACKED_LIMIT as usize + 65536,
            )),
        )
        .route(
            "/api/v1/artwork/{kind}/{id}/upload",
            post(crate::library::upload_image).layer(DefaultBodyLimit::max(5 * 1024 * 1024)),
        )
        .route(
            "/api/import/spotify",
            post(imports::upload).layer(DefaultBodyLimit::max(
                upload_limit.saturating_add(state.config.import_limits.files.saturating_mul(1024)),
            )),
        )
        .route(
            "/api/v1/listenbrainz/upload",
            post(crate::listenbrainz::upload).layer(DefaultBodyLimit::disable()),
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
    if !crate::read_api::is_external(request.uri().path())
        && request.headers().contains_key(header::AUTHORIZATION)
    {
        return AppError::Forbidden.into_response();
    }
    // All in-flight HTTP requests drain before confirmation. The durable marker rejects
    // later mutations; startup activates only after this process has stopped.
    let gate = state.storage.requests.clone();
    let exclusive =
        request.uri().path() == "/api/v1/recovery/restore" && request.method() == Method::POST;
    // Status reads and owner/CSRF-protected cancellation only touch job controls,
    // so they remain available during restore preparation's exclusive barrier.
    let activity_control = (request.method() == Method::GET
        && (request.uri().path() == "/api/v1/activities"
            || request.uri().path().starts_with("/api/v1/activities/")))
        || (request.method() == Method::POST
            && request.uri().path().starts_with("/api/v1/activities/")
            && request.uri().path().ends_with("/cancel"));
    let _read = if exclusive || activity_control {
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
    if crate::read_api::is_external(request.uri().path()) {
        return crate::read_api::security(state, request, next).await;
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
    headers.insert(header::CONTENT_SECURITY_POLICY,HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' blob:; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'"));
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
pub use crate::consultation::Filters;
use crate::consultation::selection_at;
pub(crate) fn listen_json(r: sqlx::sqlite::SqliteRow) -> Value {
    json!({"id":r.get::<String,_>("id"),"track_id":r.get::<String,_>("track_id"),"title":r.get::<String,_>("title"),"album_id":r.get::<Option<String>,_>("album_id"),"timezone":r.get::<String,_>("timezone"),"local_date":r.get::<String,_>("local_date"),"week_date":r.get::<String,_>("week_date"),"album":r.get::<Option<String>,_>("album"),"cover_url":r.get::<Option<String>,_>("cover_url"),"spotify_url":r.get::<Option<String>,_>("spotify_url"),"played_at_ms":r.get::<i64,_>("played_at_ms"),"first_acquired_at_ms":r.get::<i64,_>("first_acquired_at_ms"),"listened_ms":r.get::<Option<i64>,_>("listened_ms"),"duration_kind":r.get::<String,_>("duration_kind"),"counted":r.get::<bool,_>("counted"),"needs_review":r.get::<bool,_>("needs_review"),
        "artists":serde_json::from_str::<Value>(&r.get::<String,_>("artists")).unwrap_or(json!([])),"sources":serde_json::from_str::<Value>(&r.get::<String,_>("sources")).unwrap_or(json!([]))})
}
pub(crate) const LISTEN_COLUMNS: &str = "SELECT s.*, (SELECT timezone FROM calendars WHERE id=s.calendar_id) AS timezone, (SELECT json_group_array(json_object('id',id,'name',name)) FROM (SELECT ar.id, ar.name FROM track_artists ta JOIN artists ar ON ar.id=ta.artist_id WHERE ta.track_id=s.track_id ORDER BY ta.ordinal)) AS artists, (SELECT json_group_array(source) FROM observations o WHERE o.listen_id=s.id) AS sources FROM selected s ";
pub async fn history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(mut filters): Query<Filters>,
) -> Result<Json<crate::dto::History>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    if filters.status.is_none() {
        filters.status = Some("counted".into());
    }
    let now = auth::now_ms();
    let (page, size) = crate::browse::paging(&filters)?;
    let has_search = filters.q.as_deref().is_some_and(|q| !q.trim().is_empty());
    let mut count = selection_at(&mut view.transaction, &filters, has_search, now).await?;
    count.push("SELECT COUNT(*) FROM selected");
    let total: i64 = count
        .build_query_scalar()
        .fetch_one(&mut *view.transaction)
        .await?;
    let mut query = selection_at(&mut view.transaction, &filters, true, now).await?;
    query
        .push(LISTEN_COLUMNS)
        .push("ORDER BY s.played_at_ms DESC, s.id DESC LIMIT ")
        .push_bind(size)
        .push(" OFFSET ")
        .push_bind((page - 1) * size);
    let items: Vec<_> = query
        .build()
        .fetch_all(&mut *view.transaction)
        .await?
        .into_iter()
        .map(listen_json)
        .collect();
    Ok(Json(
        serde_json::from_value(json!({"items":items,"total":total,"page":page,"page_size":size,"timezone":crate::preferences::read(&mut view.transaction).await?.timezone})).map_err(|_| AppError::Internal)?,
    ))
}
pub async fn dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filters): Query<Filters>,
) -> Result<Json<crate::dto::Dashboard>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    let preferences = crate::preferences::read(&mut view.transaction).await?;
    // Absolute selections remain valid until archive identity/revision or preferences
    // change. Relative periods depend on the clock and are always recalculated.
    let cacheable = matches!(filters.period.as_deref().unwrap_or("all"), "all" | "custom")
        || filters.from.as_deref().is_some_and(|s| !s.is_empty())
        || filters.to.as_deref().is_some_and(|s| !s.is_empty());
    let key = if cacheable {
        let key = serde_json::to_string(&(&view.identity, &preferences, &filters))
            .map_err(|_| AppError::Internal)?;
        let cache = state.dashboard_cache.lock().await;
        if let Some((_, value)) = cache.iter().find(|(stored, _)| stored == &key) {
            return Ok(Json(
                serde_json::from_value(value.clone()).map_err(|_| AppError::Internal)?,
            ));
        }
        Some(key)
    } else {
        None
    };
    let (need_totals, need_artists, need_albums, need_tracks, need_recent) =
        match filters.widgets.as_deref() {
            Some(w) => {
                let set: std::collections::HashSet<&str> = w
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();
                (
                    set.contains("totals"),
                    set.contains("top_artists") || set.contains("unified_rankings"),
                    set.contains("top_albums") || set.contains("unified_rankings"),
                    set.contains("top_tracks") || set.contains("unified_rankings"),
                    set.contains("recent_listens"),
                )
            }
            None => (true, true, true, true, true),
        };
    let need_aggregates = need_totals || need_artists || need_albums || need_tracks;
    let (totals, rankings) = if need_aggregates {
        let order = match filters.sort.as_deref().unwrap_or("plays") {
            "plays" => "plays DESC, listened_ms DESC, name COLLATE NOCASE, id",
            "time" => "listened_ms DESC, plays DESC, name COLLATE NOCASE, id",
            _ => return Err(AppError::BadRequest("Unknown ranking order.".into())),
        };
        let now = auth::now_ms();
        let mut query = selection_at(&mut view.transaction, &filters, false, now).await?;
        let mut sql = String::from(
            r#", by_track AS MATERIALIZED (
                SELECT track_id, COUNT(*) AS records, SUM(counted) AS plays,
                       COALESCE(SUM(CASE WHEN counted=1 THEN listened_ms ELSE 0 END),0) AS listened_ms,
                       COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='actual' THEN listened_ms ELSE 0 END),0) AS actual_ms,
                       COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='estimated' THEN listened_ms ELSE 0 END),0) AS estimated_ms,
                       SUM(CASE WHEN counted=1 AND duration_kind='unknown' THEN 1 ELSE 0 END) AS unknown_durations
                FROM selected GROUP BY track_id
            )"#,
        );
        if need_artists {
            sql.push_str(
                r#", artist_ranks AS (
                    SELECT ar.id, ar.name, CASE WHEN COUNT(*)=1 THEN MIN(t.title) ELSE CAST(COUNT(*) AS TEXT) || ' tracks' END AS subtitle, '/api/v1/artwork/artists/'||ar.id AS cover_url,
                           SUM(b.plays) AS plays, SUM(b.listened_ms) AS listened_ms, SUM(b.actual_ms) AS actual_ms, SUM(b.estimated_ms) AS estimated_ms, SUM(b.unknown_durations) AS unknown_durations
                    FROM by_track b JOIN track_artists ta ON ta.track_id=b.track_id
                    JOIN artists ar ON ar.id=ta.artist_id JOIN tracks t ON t.id=b.track_id
                    WHERE b.plays>0 GROUP BY ar.id
                )"#,
            );
        }
        if need_albums {
            sql.push_str(
                r#", album_ranks AS (
                    SELECT a.id, a.name, NULL AS subtitle, a.cover_url,
                           SUM(b.plays) AS plays, SUM(b.listened_ms) AS listened_ms, SUM(b.actual_ms) AS actual_ms, SUM(b.estimated_ms) AS estimated_ms, SUM(b.unknown_durations) AS unknown_durations
                    FROM by_track b JOIN tracks t ON t.id=b.track_id JOIN albums a ON a.id=t.album_id
                    WHERE b.plays>0 GROUP BY a.id
                )"#,
            );
        }
        if need_tracks {
            sql.push_str(
                r#", track_ranks AS (
                    SELECT t.id, t.title AS name, a.name AS subtitle, a.cover_url, b.plays, b.listened_ms, b.actual_ms, b.estimated_ms, b.unknown_durations
                    FROM by_track b JOIN tracks t ON t.id=b.track_id LEFT JOIN albums a ON a.id=t.album_id
                    WHERE b.plays>0
                )"#,
            );
        }
        sql.push_str(" SELECT json_object(");
        if need_totals {
            sql.push_str(
                r#"'totals', (SELECT json_object(
                    'records',COALESCE(SUM(records),0), 'listens',COALESCE(SUM(plays),0),
                    'tracks',COALESCE(SUM(plays>0),0), 'actual_ms',COALESCE(SUM(actual_ms),0),
                    'estimated_ms',COALESCE(SUM(estimated_ms),0), 'unknown_durations',COALESCE(SUM(unknown_durations),0)
                ) FROM by_track),"#,
            );
        } else {
            sql.push_str("'totals', NULL,");
        }
        if need_artists || need_albums || need_tracks {
            let artists_expr = if need_artists {
                "(SELECT json_group_array(json_object('id',id,'name',name,'subtitle',subtitle,'cover_url',cover_url,'plays',plays,'listened_ms',listened_ms,'actual_ms',actual_ms,'estimated_ms',estimated_ms,'unknown_durations',unknown_durations,'position',position)) FROM (SELECT *, RANK() OVER (ORDER BY /*PRIMARY*/ DESC) AS position FROM artist_ranks ORDER BY /*ORDER*/ LIMIT 5))"
            } else {
                "json_array()"
            };
            let albums_expr = if need_albums {
                "(SELECT json_group_array(json_object('id',id,'name',name,'subtitle',subtitle,'cover_url',cover_url,'plays',plays,'listened_ms',listened_ms,'actual_ms',actual_ms,'estimated_ms',estimated_ms,'unknown_durations',unknown_durations,'position',position)) FROM (SELECT *, RANK() OVER (ORDER BY /*PRIMARY*/ DESC) AS position FROM album_ranks ORDER BY /*ORDER*/ LIMIT 5))"
            } else {
                "json_array()"
            };
            let tracks_expr = if need_tracks {
                "(SELECT json_group_array(json_object('id',id,'name',name,'subtitle',subtitle,'cover_url',cover_url,'plays',plays,'listened_ms',listened_ms,'actual_ms',actual_ms,'estimated_ms',estimated_ms,'unknown_durations',unknown_durations,'position',position)) FROM (SELECT *, RANK() OVER (ORDER BY /*PRIMARY*/ DESC) AS position FROM track_ranks ORDER BY /*ORDER*/ LIMIT 5))"
            } else {
                "json_array()"
            };
            sql.push_str(&format!(
                "'rankings', json_object('artists', {artists_expr}, 'albums', {albums_expr}, 'tracks', {tracks_expr})"
            ));
        } else {
            sql.push_str("'rankings', NULL");
        }
        sql.push(')');
        let primary = if filters.sort.as_deref() == Some("time") {
            "listened_ms"
        } else {
            "plays"
        };
        let sql_str = sql
            .replace("/*ORDER*/", order)
            .replace("/*PRIMARY*/", primary);
        query.push(sql_str);
        let aggregates: String = query
            .build_query_scalar()
            .fetch_one(&mut *view.transaction)
            .await?;
        let aggregates: Value =
            serde_json::from_str(&aggregates).map_err(|_| AppError::Internal)?;
        let totals: Option<crate::dto::Totals> = if need_totals {
            serde_json::from_value(aggregates["totals"].clone()).ok()
        } else {
            None
        };
        let rankings: Option<crate::dto::Rankings> = if need_artists || need_albums || need_tracks {
            serde_json::from_value(aggregates["rankings"].clone()).ok()
        } else {
            None
        };
        (totals, rankings)
    } else {
        (None, None)
    };
    let recent: Option<Vec<crate::dto::Listen>> = if need_recent {
        let mut recent_filters = filters.clone();
        if recent_filters.status.is_none() {
            recent_filters.status = Some("counted".into());
        }
        let now = auth::now_ms();
        let mut recent = selection_at(&mut view.transaction, &recent_filters, true, now).await?;
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
        Some(serde_json::from_value(Value::Array(recent)).map_err(|_| AppError::Internal)?)
    } else {
        None
    };
    let response = crate::dto::Dashboard {
        totals,
        rankings,
        recent,
        timezone: preferences.timezone,
    };
    if let Some(key) = key {
        let value = serde_json::to_value(&response).map_err(|_| AppError::Internal)?;
        let mut cache = state.dashboard_cache.lock().await;
        if !cache.iter().any(|(stored, _)| stored == &key) {
            if cache.len() == 16 {
                cache.pop_front();
            }
            cache.push_back((key, value));
        }
    }
    Ok(Json(response))
}
