use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use scrobbl_rs::{
    AppState, auth,
    config::Config,
    db::{self, Artist, Event},
    imports::parse_extended,
    spotify::parse_track,
    web,
};
use serde_json::{Value, json};
use tower::ServiceExt;

const URI: &str = "spotify:track:0000000000000000000001";
fn exported(end: &str, ms: i64) -> Value {
    json!({"ts":end,"ms_played":ms,"master_metadata_track_name":"A song","master_metadata_album_artist_name":"First artist","master_metadata_album_album_name":"An album","spotify_track_uri":URI,"spotify_episode_uri":null})
}
fn recent(at: i64) -> Event {
    let row = json!({"uri":URI,"type":"track","name":"A song","duration_ms":180000,"album":{"id":"album1","name":"An album","images":[]},"artists":[{"id":"artist1","name":"First artist"},{"id":"artist2","name":"Second artist"}]});
    let track = parse_track(&row).unwrap();
    Event {
        source: "spotify_api",
        source_key: format!("{URI}:{at}"),
        source_timestamp_ms: at,
        played_at_ms: at,
        listened_ms: track.duration_ms,
        duration_kind: db::DurationKind::Estimated,
        counted: true,
        track,
        raw: row,
    }
}
async fn state() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(dir.path()).await.unwrap();
    let config = Config {
        bind_ip: "127.0.0.1".parse().unwrap(),
        external_origin: None,
        port: 3210,
        data_dir: dir.path().to_owned(),
        sync_seconds: 120,
        now_playing_seconds: 30,
        import_limits: Default::default(),
        api_snapshot_limits: Default::default(),
        env_client_id: None,
        env_client_secret: None,
    };
    (dir, AppState::new(pool, config))
}
async fn save(state: &AppState, events: &[Event]) -> db::IngestReport {
    let mut tx = state.db.begin().await.unwrap();
    let report = db::ingest(&mut tx, events).await.unwrap();
    tx.commit().await.unwrap();
    report
}
async fn request(
    app: &Router,
    method: &str,
    path: &str,
    body: Value,
    cookie: Option<&str>,
    csrf: Option<&str>,
) -> (StatusCode, Value, Option<String>) {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:3210")
        .header("origin", "http://127.0.0.1:3210")
        .header("content-type", "application/json");
    if let Some(c) = cookie {
        r = r.header("cookie", c);
    }
    if let Some(c) = csrf {
        r = r.header("x-csrf-token", c);
    }
    let response = app
        .clone()
        .oneshot(r.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let code = response.status();
    let cookie = response
        .headers()
        .get("set-cookie")
        .map(|h| h.to_str().unwrap().split(';').next().unwrap().to_owned());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let data = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (code, data, cookie)
}
async fn sign_in(app: &Router) -> (String, String) {
    let (status, data, cookie) = request(
        app,
        "POST",
        "/api/setup",
        json!({"username":"owner","password":"a long local test password"}),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    (cookie.unwrap(), data["csrf"].as_str().unwrap().into())
}

#[tokio::test]
async fn preferences_api_preserves_calendars_and_queues_atomic_history_changes() {
    let (dir, state) = state().await;
    let app = web::router(state.clone());
    let (cookie, csrf) = sign_in(&app).await;
    assert_eq!(
        request(&app, "GET", "/api/v1/preferences", json!({}), None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (_, mut p, _) = request(
        &app,
        "GET",
        "/api/v1/preferences",
        json!({}),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(p["timezone"], "Europe/Rome");
    assert_eq!(p["initial_period"], "all");
    let first = exported("2025-01-02T00:30:00Z", 180000);
    let (events, _) = parse_extended(json!([first.clone()]).to_string().as_bytes()).unwrap();
    save(&state, &events).await;
    let old_calendar: String = sqlx::query_scalar("SELECT calendar_id FROM listens")
        .fetch_one(&state.read_db)
        .await
        .unwrap();
    p["name"] = json!("My archive");
    p["timezone"] = json!("UTC");
    p["week_start"] = json!(6);
    p["hour_format"] = json!("12");
    p["initial_period"] = json!("30d");
    let body = json!({"preferences":p,"calendar_scope":null});
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/v1/preferences",
            body.clone(),
            Some(&cookie),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/v1/preferences",
            body,
            Some(&cookie),
            Some(&csrf)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (code, saved, _) = request(
        &app,
        "POST",
        "/api/v1/preferences",
        json!({"preferences":p,"calendar_scope":"future"}),
        Some(&cookie),
        Some(&csrf),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(saved["preferences"]["name"], "My archive");
    assert!(saved["job_id"].is_null());
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/v1/preferences",
            json!({"preferences":p,"calendar_scope":"future"}),
            Some(&cookie),
            Some(&csrf)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (events, _) = parse_extended(
        json!([first, exported("2025-01-03T00:30:00Z", 180000)])
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    save(&state, &events).await;
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT calendar_id FROM listens ORDER BY played_at_ms LIMIT 1"
        )
        .fetch_one(&state.read_db)
        .await
        .unwrap(),
        old_calendar
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(DISTINCT calendar_id) FROM listens")
            .fetch_one(&state.read_db)
            .await
            .unwrap(),
        2
    );
    let originals: Vec<String> =
        sqlx::query_scalar("SELECT p.fingerprint FROM observation_variants v JOIN original_payloads p ON p.id=v.payload_id ORDER BY v.id")
            .fetch_all(&state.read_db)
            .await
            .unwrap();
    let instants: Vec<i64> = sqlx::query_scalar("SELECT played_at_ms FROM listens ORDER BY id")
        .fetch_all(&state.read_db)
        .await
        .unwrap();
    p = saved["preferences"].clone();
    p["timezone"] = json!("Asia/Tokyo");
    let (_, queued, _) = request(
        &app,
        "POST",
        "/api/v1/preferences",
        json!({"preferences":p,"calendar_scope":"all"}),
        Some(&cookie),
        Some(&csrf),
    )
    .await;
    let id = queued["job_id"].as_str().unwrap();
    assert_eq!(queued["preferences"]["timezone"], "UTC");
    let (_, activities, _) = request(
        &app,
        "GET",
        "/api/v1/activities",
        json!({}),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(activities[0]["state"], "queued");
    assert!(activities[0]["expected_rows"].is_null());
    assert!(!activities.to_string().contains("parameters_json"));
    state.storage.close().await;
    let resumed = AppState::new(db::open(dir.path()).await.unwrap(), state.config.clone());
    let resumed_app = web::router(resumed.clone());
    let (_, persisted, _) = request(
        &resumed_app,
        "GET",
        "/api/v1/preferences",
        json!({}),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(persisted["name"], "My archive");
    assert_eq!(persisted["hour_format"], "12");
    let lease = resumed.storage.claim("calendar").await.unwrap().unwrap();
    assert_eq!(lease.id, id);
    assert!(resumed.storage.calendar_step(&lease).await.unwrap());
    let (_, effective, _) = request(
        &resumed_app,
        "GET",
        "/api/v1/preferences",
        json!({}),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(effective["timezone"], "Asia/Tokyo");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(DISTINCT calendar_id) FROM listens")
            .fetch_one(&resumed.read_db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        originals,
        sqlx::query_scalar::<_, String>("SELECT p.fingerprint FROM observation_variants v JOIN original_payloads p ON p.id=v.payload_id ORDER BY v.id")
            .fetch_all(&resumed.read_db)
            .await
            .unwrap()
    );
    assert_eq!(
        instants,
        sqlx::query_scalar::<_, i64>("SELECT played_at_ms FROM listens ORDER BY id")
            .fetch_all(&resumed.read_db)
            .await
            .unwrap()
    );
    let (_, detail, _) = request(
        &resumed_app,
        "GET",
        &format!("/api/v1/activities/{id}"),
        json!({}),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(detail["activity"]["state"], "completed");
    assert_eq!(detail["activity"]["processed"], 2);
    resumed.storage.close().await;
}

#[tokio::test]
async fn preferences_validation_maintenance_and_activity_controls_are_guarded() {
    let (_dir, state) = state().await;
    let app = web::router(state.clone());
    let (cookie, csrf) = sign_in(&app).await;
    let (_, p, _) = request(
        &app,
        "GET",
        "/api/v1/preferences",
        json!({}),
        Some(&cookie),
        None,
    )
    .await;
    for (key, value) in [
        ("name", json!(" ")),
        ("timezone", json!("invalid/zone")),
        ("week_start", json!(7)),
        ("hour_format", json!("25")),
        ("initial_period", json!("anything")),
    ] {
        let mut invalid = p.clone();
        invalid[key] = value;
        assert_eq!(
            request(
                &app,
                "POST",
                "/api/v1/preferences",
                json!({"preferences":invalid,"calendar_scope":"all"}),
                Some(&cookie),
                Some(&csrf)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let id = state
        .storage
        .change_calendar("UTC", 0, true)
        .await
        .unwrap()
        .unwrap();
    let path = format!("/api/v1/activities/{id}");
    assert_eq!(
        request(&app, "GET", &path, json!({}), None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &path,
            json!({"action":"cancel"}),
            Some(&cookie),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &path,
            json!({"action":"cancel"}),
            Some(&cookie),
            Some(&csrf)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(state.storage.job(&id).await.unwrap().state, "cancelled");
    assert_eq!(
        request(
            &app,
            "POST",
            &path,
            json!({"action":"retry"}),
            Some(&cookie),
            Some(&csrf)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(state.storage.job(&id).await.unwrap().state, "queued");
    state.storage.cancel(&id).await.unwrap();
    let newer = state
        .storage
        .change_calendar("Asia/Tokyo", 0, true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        request(
            &app,
            "POST",
            &path,
            json!({"action":"retry"}),
            Some(&cookie),
            Some(&csrf)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(state.storage.job(&id).await.unwrap().state, "cancelled");
    state.storage.cancel(&newer).await.unwrap();
    state.storage.maintenance(true).await.unwrap();
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/v1/preferences",
            json!({"preferences":p,"calendar_scope":null}),
            Some(&cookie),
            Some(&csrf)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    state.storage.close().await;
}

#[tokio::test]
async fn import_is_idempotent_preserves_short_plays_and_filters_non_music() {
    let (_dir, state) = state().await;
    let input = json!([exported("2025-06-01T12:03:00Z",180000),exported("2025-06-01T12:04:00Z",10000),{"ts":"2025-06-01T12:05:00Z","ms_played":10000,"spotify_episode_uri":"spotify:episode:abc"},exported("bad date",12000)]);
    let (events, report) = parse_extended(input.to_string().as_bytes()).unwrap();
    assert_eq!(report.skipped, 1);
    assert_eq!(report.invalid, 1);
    assert_eq!(events.len(), 2);
    assert_eq!(save(&state, &events).await.added, 2);
    assert_eq!(save(&state, &events).await.duplicates, 2);
    let (total, counted, actual): (i64, i64, i64) =
        sqlx::query_as("SELECT COUNT(*),SUM(counted),SUM(listened_ms) FROM listens")
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!((total, counted, actual), (2, 1, 190000));
    let (raw, fingerprint): (String, String) = sqlx::query_as("SELECT p.raw_json,p.fingerprint FROM observations o JOIN original_payloads p ON p.id=o.payload_id WHERE o.source_key=?")
        .bind(&events[0].source_key)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(raw, "null");
    assert_eq!(fingerprint, db::payload_fingerprint(&events[0].raw));
}

#[tokio::test]
async fn overview_cache_tracks_revisions_selections_and_assigned_calendar_preferences() {
    let (_dir, state) = state().await;
    let app = web::router(state.clone());
    let (cookie, _csrf) = sign_in(&app).await;
    let first = recent(1748865600000);
    save(&state, std::slice::from_ref(&first)).await;
    let path = "/api/dashboard?period=all";
    let (_, initial, _) = request(&app, "GET", path, json!(null), Some(&cookie), None).await;
    let (_, cached, _) = request(&app, "GET", path, json!(null), Some(&cookie), None).await;
    assert_eq!(initial, cached);
    assert_eq!(cached["totals"]["records"], 1);
    save(&state, &[recent(first.played_at_ms + 180000)]).await;
    let (_, updated, _) = request(&app, "GET", path, json!(null), Some(&cookie), None).await;
    assert_eq!(updated["totals"]["records"], 2);
    state
        .storage
        .change_calendar("Asia/Tokyo", 6, false)
        .await
        .unwrap();
    let (_, preferences, _) = request(&app, "GET", path, json!(null), Some(&cookie), None).await;
    assert_eq!(preferences["timezone"], "Asia/Tokyo");
    let (_, empty, _) = request(
        &app,
        "GET",
        "/api/dashboard?from=2020-01-01&to=2020-01-01",
        json!(null),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(empty["totals"]["records"], 0);
    state.storage.close().await;
}
#[tokio::test]
async fn actual_duration_wins_in_both_orders_and_repeats_remain_separate() {
    for api_first in [true, false] {
        let (_dir, state) = state().await;
        let input = json!([exported("2025-06-01T12:03:00Z", 180000)]);
        let (events, _) = parse_extended(input.to_string().as_bytes()).unwrap();
        let at = events[0].played_at_ms + 900;
        let mut api = recent(at);
        api.listened_ms = Some(240000);
        if api_first {
            save(&state, &[api.clone()]).await;
            assert_eq!(save(&state, &events).await.reconciled, 1);
        } else {
            save(&state, &events).await;
            assert_eq!(save(&state, &[api.clone()]).await.reconciled, 1);
        }
        let (count, ms, kind): (i64, i64, String) =
            sqlx::query_as("SELECT COUNT(*),listened_ms,duration_kind FROM listens")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!((count, ms, kind), (1, 180000, "actual".into()));
        assert_eq!(save(&state, &[recent(at + 180000)]).await.added, 1);
        assert_eq!(save(&state, &[api]).await.duplicates, 1);
        let observations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM observations")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(observations, 3);
    }
}
#[tokio::test]
async fn short_actual_listen_replaces_api_count_without_losing_metadata() {
    let (_dir, state) = state().await;
    let (events, _) = parse_extended(
        json!([exported("2025-06-01T12:00:20Z", 20000)])
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    save(&state, &[recent(events[0].played_at_ms)]).await;
    save(&state, &events).await;
    let (counted, ms): (i64, i64) = sqlx::query_as("SELECT counted,listened_ms FROM listens")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!((counted, ms), (0, 20000));
    let credits: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM track_artists")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(credits, 2);
}
#[tokio::test]
async fn ambiguous_matches_are_kept_separate() {
    let (_dir, state) = state().await;
    let (events, _) = parse_extended(
        json!([exported("2025-06-01T12:03:00Z", 180000)])
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let at = events[0].played_at_ms;
    save(&state, &[recent(at - 500), recent(at + 500)]).await;
    let r = save(&state, &events).await;
    assert_eq!(r.ambiguous, 1);
    assert_eq!(r.added, 1);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM listens")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(count, 3);
}
#[tokio::test]
async fn authentication_csrf_revocation_and_restart() {
    let (dir, state) = state().await;
    let app = web::router(state.clone());
    assert_eq!(
        request(&app, "GET", "/api/history", json!(null), None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (cookie, csrf) = sign_in(&app).await;
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/setup",
            json!({"username":"owner","password":"another long password"}),
            None,
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&app, "POST", "/api/logout", json!({}), Some(&cookie), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/spotify/credentials",
            json!({"client_id":"abcdefghijk123456","client_secret":"testsecret123456789"}),
            Some(&cookie),
            Some(&csrf)
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, status, _) = request(
        &app,
        "GET",
        "/api/spotify/status",
        json!(null),
        Some(&cookie),
        None,
    )
    .await;
    assert!(!status.to_string().contains("testsecret"));
    assert_eq!(status["configured"], true);
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM owner")
        .fetch_one(&state.private_db)
        .await
        .unwrap();
    assert!(hash.starts_with("$argon2id$"));
    let stored: String = sqlx::query_scalar("SELECT digest FROM sessions")
        .fetch_one(&state.private_db)
        .await
        .unwrap();
    assert!(!cookie.contains(&stored));
    state.storage.close().await;
    let pool = db::open(dir.path()).await.unwrap();
    let reopened = AppState::new(pool, state.config.clone());
    let app = web::router(reopened);
    assert_eq!(
        request(
            &app,
            "GET",
            "/api/history",
            json!(null),
            Some(&cookie),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/logout",
            json!({}),
            Some(&cookie),
            Some(&csrf)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "GET",
            "/api/history",
            json!(null),
            Some(&cookie),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/login",
            json!({"username":"owner","password":"incorrect password"}),
            None,
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/login",
            json!({"username":"owner","password":"a long local test password"}),
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}
#[tokio::test]
async fn hostile_origins_hosts_expired_sessions_and_oauth_states_are_rejected() {
    let (_dir, state) = state().await;
    let app = web::router(state.clone());
    for (host, origin) in [
        ("evil.test:3210", "http://127.0.0.1:3210"),
        ("127.0.0.1:3210", "https://evil.test"),
        ("127.0.0.1:3210", "null"),
    ] {
        let r = Request::builder()
            .method("POST")
            .uri("/api/setup")
            .header("host", host)
            .header("origin", origin)
            .header("content-type", "application/json")
            .body(Body::from(
                "{\"username\":\"owner\",\"password\":\"a long local test password\"}",
            ))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(r).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    let (cookie, _) = sign_in(&app).await;
    assert_eq!(
        request(
            &app,
            "GET",
            "/auth/spotify/callback?state=invalid&code=invalid",
            json!(null),
            Some(&cookie),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE sessions SET expires_at_ms = 0")
        .execute(&state.private_db)
        .await
        .unwrap();
    assert_eq!(
        request(
            &app,
            "GET",
            "/api/dashboard",
            json!(null),
            Some(&cookie),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn statistics_count_each_credit_once_and_search_is_literal() {
    let (_dir, state) = state().await;
    let mut event = recent(Utc::now().timestamp_millis());
    event.track.title = "100% <script>alert(1)</script>".into();
    event.track.artists.push(Artist {
        id: event.track.artists[0].id.clone(),
        name: "First artist".into(),
    });
    save(&state, &[event]).await;
    let app = web::router(state);
    let (cookie, _) = sign_in(&app).await;
    let (code, data, _) = request(
        &app,
        "GET",
        "/api/dashboard?period=all",
        json!(null),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(data["totals"]["listens"], 1);
    assert_eq!(data["rankings"]["artists"].as_array().unwrap().len(), 2);
    assert_eq!(data["rankings"]["artists"][0]["plays"], 1);
    let (_, data, _) = request(
        &app,
        "GET",
        "/api/history?q=100%25",
        json!(null),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(data["total"], 1);
    let (_, data, _) = request(
        &app,
        "GET",
        "/api/history?q=_",
        json!(null),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(data["total"], 0);
    assert_eq!(
        request(
            &app,
            "GET",
            "/api/history?from=2025-02-01&to=2025-01-01",
            json!(null),
            Some(&cookie),
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
#[test]
fn rome_custom_days_follow_daylight_saving_and_end_date_is_inclusive() {
    let f = web::Filters {
        from: Some("2025-03-30".into()),
        to: Some("2025-03-30".into()),
        ..Default::default()
    };
    let (from, to) = web::bounds(&f).unwrap();
    assert_eq!(to.unwrap() - from.unwrap(), 23 * 3600 * 1000);
    assert_eq!(
        from.unwrap(),
        Utc.with_ymd_and_hms(2025, 3, 29, 23, 0, 0)
            .unwrap()
            .timestamp_millis()
    );
    let f = web::Filters {
        from: Some("2025-10-26".into()),
        to: Some("2025-10-26".into()),
        ..Default::default()
    };
    let (from, to) = web::bounds(&f).unwrap();
    assert_eq!(to.unwrap() - from.unwrap(), 25 * 3600 * 1000);
}
#[test]
fn rejects_wrong_format_and_preserves_unsplit_artist_names() {
    assert!(parse_extended(b"{\"streaming\":[]}").is_err());
    let mut row = exported("2025-06-01T12:03:00Z", 180000);
    row["master_metadata_album_artist_name"] = json!("Earth, Wind & Fire");
    let (events, _) = parse_extended(json!([row]).to_string().as_bytes()).unwrap();
    assert_eq!(events[0].track.artists.len(), 1);
    assert_eq!(events[0].track.artists[0].name, "Earth, Wind & Fire");
    assert!(events[0].played_at_ms < auth::now_ms());
}

#[tokio::test]
async fn close_repeats_in_the_incoming_batch_do_not_claim_one_counterpart() {
    let (_dir, state) = state().await;
    let input = json!([
        exported("2025-06-01T12:03:00Z", 180000),
        exported("2025-06-01T12:03:00.500Z", 180000)
    ]);
    let (events, _) = parse_extended(input.to_string().as_bytes()).unwrap();
    save(&state, &[recent(events[0].played_at_ms + 250)]).await;
    let report = save(&state, &events).await;
    assert_eq!(
        (report.added, report.ambiguous, report.reconciled),
        (2, 2, 0)
    );
    let reviews: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM listens WHERE needs_review=1")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(reviews, 3);
    let app = web::router(state);
    let (cookie, _) = sign_in(&app).await;
    let (code, data, _) = request(
        &app,
        "GET",
        "/api/history?status=review",
        json!(null),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(data["total"], 3);
}

#[tokio::test]
async fn later_batches_expose_ambiguity_without_preserving_a_false_pair() {
    for api_first in [true, false] {
        let (_dir, state) = state().await;
        let (events, _) = parse_extended(
            json!([
                exported("2025-06-01T12:03:00Z", 180000),
                exported("2025-06-01T12:03:00.500Z", 180000)
            ])
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let api = recent(events[0].played_at_ms + 250);
        if api_first {
            save(&state, &[api]).await;
            save(&state, &events[..1]).await;
        } else {
            save(&state, &events[..1]).await;
            save(&state, &[api]).await;
        }
        let before = state.storage.read_view().await.unwrap().identity;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            1
        );
        let result = save(&state, &events[1..]).await;
        assert_eq!(result.added, 2);
        assert_eq!(result.ambiguous, 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens WHERE needs_review=1")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            3
        );
        assert_eq!(
            state.storage.changes(&before, 100).await.unwrap().1.len(),
            3
        );
    }
}

#[tokio::test]
async fn reverse_ambiguity_uses_the_counterpart_time_not_the_merged_time() {
    let (_dir, state) = state().await;
    let (events, _) = parse_extended(
        json!([exported("2025-06-01T12:03:00Z", 180000)])
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let at = events[0].played_at_ms;
    save(&state, &events).await;
    assert_eq!(save(&state, &[recent(at - 1800)]).await.reconciled, 1);
    assert_eq!(save(&state, &[recent(at + 1800)]).await.ambiguous, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens WHERE needs_review=1")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        3
    );
}

#[tokio::test]
async fn owner_setup_requires_username_and_rejects_guessable_passwords_on_the_server() {
    let (_dir, state) = state().await;
    let app = web::router(state.clone());
    for body in [
        json!({"password":"a long local test password"}),
        json!({"username":"owner","password":"short"}),
        json!({"username":"owner","password":"aaaaaaaaaaaaaaa"}),
        json!({"username":"owner","password":"Password123456789!"}),
    ] {
        assert_eq!(
            request(&app, "POST", "/api/setup", body, None, None)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let (cookie, csrf) = sign_in(&app).await;
    let session = request(
        &app,
        "GET",
        "/api/session",
        Value::Null,
        Some(&cookie),
        None,
    )
    .await
    .1;
    assert_eq!(session["username"], "owner");
    assert_eq!(session["username_setup_required"], false);
    for username in [None, Some("someone-else")] {
        assert_eq!(
            request(
                &app,
                "POST",
                "/api/login",
                json!({"username":username,"password":"a long local test password"}),
                None,
                None
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/login",
            json!({"username":"OWNER","password":"a long local test password"}),
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/owner/username",
            json!({"username":"changed"}),
            Some(&cookie),
            Some(&csrf)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn existing_owner_can_add_username_only_with_a_valid_session_and_csrf() {
    let (_dir, state) = state().await;
    let app = web::router(state.clone());
    let (cookie, csrf) = sign_in(&app).await;
    sqlx::query("DELETE FROM settings WHERE key='owner_username'")
        .execute(&state.private_db)
        .await
        .unwrap();
    let session = request(
        &app,
        "GET",
        "/api/session",
        Value::Null,
        Some(&cookie),
        None,
    )
    .await
    .1;
    assert_eq!(session["username_setup_required"], true);
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/login",
            json!({"password":"a long local test password"}),
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    for (cookie, csrf, status) in [
        (None, None, StatusCode::UNAUTHORIZED),
        (Some(cookie.as_str()), None, StatusCode::FORBIDDEN),
        (Some(cookie.as_str()), Some(csrf.as_str()), StatusCode::OK),
    ] {
        assert_eq!(
            request(
                &app,
                "POST",
                "/api/owner/username",
                json!({"username":"listener"}),
                cookie,
                csrf
            )
            .await
            .0,
            status
        );
    }
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/login",
            json!({"password":"a long local test password"}),
            None,
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/login",
            json!({"username":"listener","password":"a long local test password"}),
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn imported_artist_and_album_rankings_combine_tracks_with_the_same_name() {
    let (_dir, state) = state().await;
    let app = web::router(state.clone());
    let (cookie, csrf) = sign_in(&app).await;
    let one = exported("2025-05-01T12:03:00Z", 180000);
    let mut two = one.clone();
    two["spotify_track_uri"] = json!("spotify:track:0000000000000000000002");
    two["master_metadata_track_name"] = json!("A different song");
    let (events, _) = parse_extended(&serde_json::to_vec(&json!([one, two])).unwrap()).unwrap();
    save(&state, &events).await;
    for kind in ["artists", "albums"] {
        let (status, page, _) = request(
            &app,
            "GET",
            &format!("/api/v1/consultation/rankings/{kind}"),
            Value::Null,
            Some(&cookie),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["total"], 1, "{kind}: {page}");
        assert_eq!(page["items"][0]["plays"], 2);
        let id = page["items"][0]["id"].as_str().unwrap();
        let music = request(
            &app,
            "GET",
            &format!("/api/v1/consultation/music/{kind}/{id}"),
            Value::Null,
            Some(&cookie),
            None,
        )
        .await
        .1;
        assert_eq!(music["totals"]["listens"], 2);
        let group_path = format!("/api/v1/library/{kind}/{id}/group");
        let members = request(&app, "GET", &group_path, Value::Null, Some(&cookie), None)
            .await
            .1;
        assert_eq!(members["total"], 2);
        let member = members["members"][0]["id"].as_str().unwrap();
        let separate = json!({"action":"separate","member_ids":[member]});
        assert_eq!(
            request(
                &app,
                "POST",
                &group_path,
                separate.clone(),
                Some(&cookie),
                None
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        let (status, changed, _) = request(
            &app,
            "POST",
            &group_path,
            separate,
            Some(&cookie),
            Some(&csrf),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        let detached = changed["group_id"].as_str().unwrap();
        let separated = request(
            &app,
            "GET",
            &format!("/api/v1/consultation/rankings/{kind}"),
            Value::Null,
            Some(&cookie),
            None,
        )
        .await
        .1;
        assert_eq!(separated["total"], 2);
        assert!(
            separated["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["plays"] == 1)
        );
        save(&state, &events).await;
        let separated = request(
            &app,
            "GET",
            &format!("/api/v1/consultation/rankings/{kind}"),
            Value::Null,
            Some(&cookie),
            None,
        )
        .await
        .1;
        assert_eq!(separated["total"], 2, "reimport must preserve separation");
        let rejoined = request(
            &app,
            "POST",
            &format!("/api/v1/library/{kind}/{detached}/group"),
            json!({"action":"rejoin","member_ids":[member]}),
            Some(&cookie),
            Some(&csrf),
        )
        .await;
        assert_eq!(rejoined.0, StatusCode::OK, "{}", rejoined.1);
        let combined = request(
            &app,
            "GET",
            &format!("/api/v1/consultation/rankings/{kind}"),
            Value::Null,
            Some(&cookie),
            None,
        )
        .await
        .1;
        assert_eq!(combined["total"], 1);
        assert_eq!(combined["items"][0]["plays"], 2);
    }
}

#[tokio::test]
async fn name_group_upgrade_preserves_source_ids_and_existing_personal_data() {
    let (dir, state) = state().await;
    let mut two = exported("2025-05-01T12:03:00Z", 180000);
    two["spotify_track_uri"] = json!("spotify:track:0000000000000000000002");
    two["master_metadata_track_name"] = json!("Another song");
    let (events, _) = parse_extended(
        &serde_json::to_vec(&json!([exported("2025-05-01T12:03:00Z", 180000), two])).unwrap(),
    )
    .unwrap();
    save(&state, &events).await;
    let before: Vec<String> = sqlx::query_scalar("SELECT id FROM listens ORDER BY id")
        .fetch_all(&state.read_db)
        .await
        .unwrap();
    let artist: String = sqlx::query_scalar("SELECT id FROM artists ORDER BY id LIMIT 1")
        .fetch_one(&state.read_db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO local_corrections VALUES('artist',?,'name','\"Personal artist\"')")
        .bind(&artist)
        .execute(&state.db)
        .await
        .unwrap();
    for sql in [
        "DROP TABLE name_group_members",
        "DROP TABLE name_groups",
        "DELETE FROM _sqlx_migrations WHERE version=12",
    ] {
        sqlx::query(sql).execute(&state.db).await.unwrap();
    }
    state.storage.close().await;
    let reopened = db::open(dir.path()).await.unwrap();
    let after: Vec<String> = sqlx::query_scalar("SELECT id FROM listens ORDER BY id")
        .fetch_all(&reopened.readers)
        .await
        .unwrap();
    assert_eq!(before, after);
    let group: String = sqlx::query_scalar(
        "SELECT group_id FROM name_group_members WHERE entity_id=? AND kind='artists'",
    )
    .bind(&artist)
    .fetch_one(&reopened.readers)
    .await
    .unwrap();
    let members: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM name_group_members WHERE group_id=?")
            .bind(&group)
            .fetch_one(&reopened.readers)
            .await
            .unwrap();
    assert_eq!(members, 2);
    let personal: String = sqlx::query_scalar(
        "SELECT value_json FROM local_corrections WHERE entity_id=? AND field='name'",
    )
    .bind(&group)
    .fetch_one(&reopened.readers)
    .await
    .unwrap();
    assert_eq!(personal, "\"Personal artist\"");
    let originals: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM observations")
        .fetch_one(&reopened.readers)
        .await
        .unwrap();
    assert_eq!(originals, 2);
    let app = web::router(AppState::new(reopened.clone(), state.config.clone()));
    let (cookie, csrf) = sign_in(&app).await;
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("/api/v1/library/artists/{group}"),
            json!({"name":"Combined group","rating":null}),
            Some(&cookie),
            Some(&csrf)
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, separated, _) = request(
        &app,
        "POST",
        &format!("/api/v1/library/artists/{group}/group"),
        json!({"action":"separate","member_ids":[artist]}),
        Some(&cookie),
        Some(&csrf),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{separated}");
    let group = separated["group_id"].as_str().unwrap().to_owned();
    let details = request(
        &app,
        "GET",
        &format!("/api/v1/library/artists/{group}"),
        Value::Null,
        Some(&cookie),
        None,
    )
    .await
    .1;
    assert_eq!(
        details["name"], "Personal artist",
        "separation restores an older source-specific correction"
    );
    let work = tempfile::tempdir().unwrap();
    let package = work.path().join("group-backup.zip");
    scrobbl_rs::recovery::create_backup(&reopened, work.path(), &package)
        .await
        .unwrap();
    let trial = tempfile::tempdir().unwrap();
    scrobbl_rs::recovery::trial_restore(&package, trial.path())
        .await
        .unwrap();
    let mut restored = scrobbl_rs::recovery::connect(&trial.path().join("archive.db"), true)
        .await
        .unwrap();
    let retained: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM name_group_members WHERE group_id=?")
            .bind(&group)
            .fetch_one(&mut restored)
            .await
            .unwrap();
    assert_eq!(retained, 1);
    reopened.close().await;
}
