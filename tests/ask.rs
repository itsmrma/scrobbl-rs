#![allow(clippy::unwrap_used, clippy::disallowed_methods)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use scrobbl_rs::{
    AppState,
    ask::Ask,
    auth,
    config::Config,
    db::{self, DurationKind, Event},
    spotify::parse_track,
    storage::Storage,
    web,
};
use serde_json::{Value, json};
use tower::ServiceExt;

fn at(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .unwrap()
        .timestamp_millis()
}

#[allow(clippy::too_many_arguments)]
fn event(
    index: usize,
    when: i64,
    ms: Option<i64>,
    kind: DurationKind,
    artist_id: &str,
    artist_name: &str,
    album_id: &str,
    album_name: &str,
    title: &str,
) -> Event {
    let raw = json!({
        "uri": format!("spotify:track:{index:022}"),
        "type": "track",
        "name": title,
        "duration_ms": 240000,
        "album": {"id": album_id, "name": album_name, "images": []},
        "artists": [{"id": artist_id, "name": artist_name}],
    });
    Event {
        source: "spotify_api",
        source_key: format!("{index}:{when}"),
        source_timestamp_ms: when,
        played_at_ms: when,
        listened_ms: ms,
        duration_kind: kind,
        counted: kind != DurationKind::Actual || ms.unwrap_or(0) >= 30000,
        track: parse_track(&raw).unwrap(),
        raw,
    }
}

async fn save(store: &Storage, events: &[Event]) {
    let mut tx = store.writer.begin().await.unwrap();
    db::ingest(&mut tx, events).await.unwrap();
    tx.commit().await.unwrap();
}

async fn setup(ai_enabled: bool) -> (tempfile::TempDir, AppState, String) {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let config = Config {
        bind_ip: "127.0.0.1".parse().unwrap(),
        external_origin: None,
        port: 3210,
        data_dir: dir.path().into(),
        sync_seconds: 120,
        now_playing_seconds: 30,
        import_limits: Default::default(),
        api_snapshot_limits: Default::default(),
        env_client_id: None,
        env_client_secret: None,
    };
    let token = auth::random_secret();
    sqlx::query("INSERT INTO sessions VALUES(?,'test-csrf',9999999999999)")
        .bind(auth::digest(&token))
        .execute(&storage.private)
        .await
        .unwrap();
    if ai_enabled {
        sqlx::query("INSERT INTO settings(key,value) VALUES('ai_enabled','1')")
            .execute(&storage.writer)
            .await
            .unwrap();
    }
    let mut state = AppState::new(storage, config);
    // Tests always use the deterministic stub; the production AppState keeps
    // the unavailable parser until #178 provides the local model runtime.
    state.ask = Ask::stub();
    (dir, state, token)
}

fn fixture_events() -> Vec<Event> {
    vec![
        event(
            1,
            at("2024-05-01T10:00:00Z"),
            Some(200_000),
            DurationKind::Actual,
            "bjork",
            "Björk",
            "homogenic",
            "Homogenic",
            "Jóga",
        ),
        event(
            2,
            at("2024-05-01T11:00:00Z"),
            Some(180_000),
            DurationKind::Estimated,
            "bjork",
            "Björk",
            "homogenic",
            "Homogenic",
            "Jóga",
        ),
        // Below the counting threshold: present in the archive, never counted.
        event(
            3,
            at("2024-05-01T12:00:00Z"),
            Some(1_000),
            DurationKind::Actual,
            "radiohead",
            "Radiohead",
            "ok-computer",
            "OK Computer",
            "Karma Police",
        ),
        event(
            4,
            at("2024-05-02T09:00:00Z"),
            None,
            DurationKind::Unknown,
            "bjork",
            "Björk",
            "homogenic",
            "Homogenic",
            "Jóga",
        ),
        // 22:30 UTC on 1 May is 00:30 on 2 May in Europe/Rome.
        event(
            5,
            at("2024-05-01T22:30:00Z"),
            Some(210_000),
            DurationKind::Actual,
            "radiohead",
            "Radiohead",
            "ok-computer",
            "OK Computer",
            "Karma Police",
        ),
    ]
}

async fn get(state: &AppState, token: &str, path: &str) -> (StatusCode, Value) {
    let response = web::router(state.clone())
        .oneshot(
            Request::builder()
                .uri(path)
                .header("host", "127.0.0.1:3210")
                .header("cookie", format!("scrobbl_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

async fn post(state: &AppState, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("content-type", "application/json")
                .header("cookie", format!("scrobbl_session={token}"))
                .header("x-csrf-token", "test-csrf")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn ask_is_model_unavailable_when_ai_is_off() {
    let (_dir, state, token) = setup(false).await;
    save(&state.storage, &fixture_events()).await;

    let (status, body) = get(&state, &token, "/api/v1/ask/status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], false);
    assert_eq!(body["model_ready"], false);

    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "How many times did I listen to Björk?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "model_unavailable");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("Enable it in Settings")
    );
}

#[tokio::test]
async fn ask_failure_modes_produce_clear_messages() {
    let (_dir, mut state, token) = setup(true).await;
    save(&state.storage, &fixture_events()).await;

    // 1. Model missing when model_ready is false
    state.ask.model_ready = false;
    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "How many times did I listen to Björk?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "model_missing");
    let msg = body["message"].as_str().unwrap();
    assert!(msg.contains("Download and select"), "{msg}");

    // 2. Model loading
    state.ask.model_ready = true;
    struct LoadingParser;
    impl scrobbl_rs::ask::parser::QueryParser for LoadingParser {
        fn parse<'a>(&'a self, _q: &'a str) -> scrobbl_rs::ask::parser::ParseFuture<'a> {
            Box::pin(async {
                Ok(scrobbl_rs::ask::parser::ParsedQuestion::Unsupported {
                    reason: "loading".into(),
                })
            })
        }
        fn model_loading(&self) -> bool {
            true
        }
    }
    state.ask = Ask::with_parser(std::sync::Arc::new(LoadingParser), true, true, false);
    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "How many times did I listen to Björk?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "model_unavailable");
    let msg = body["message"].as_str().unwrap();
    assert!(msg.contains("model is loading"), "{msg}");

    // 3. Runtime unavailable
    state.ask = Ask::unavailable();
    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "How many times did I listen to Björk?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "model_unavailable");
    let msg = body["message"].as_str().unwrap();
    assert!(msg.contains("runtime is not available"), "{msg}");

    // 4. Unsupported input (empty or unmapped in stub)
    state.ask = Ask::stub();
    for input in ["", "   ", "What is the weather?"] {
        let (status, body) = post(&state, &token, "/api/v1/ask", json!({"question": input})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "unsupported", "{input}: {body}");
    }
}

#[tokio::test]
async fn ask_requires_the_owner_session() {
    let (_dir, state, _token) = setup(true).await;
    let response = web::router(state)
        .oneshot(
            Request::builder()
                .uri("/api/v1/ask/status")
                .header("host", "127.0.0.1:3210")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stub_local_sql_runs_through_sandbox_and_supports_follow_ups() {
    let (_dir, state, token) = setup(true).await;
    save(&state.storage, &fixture_events()).await;

    // 1. Free SQL questions run end-to-end through the projection and sandbox.
    let (status, sql_bjork) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "How many times did I listen to Björk?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sql_bjork}");
    assert_eq!(sql_bjork["status"], "ok", "{sql_bjork}");
    assert_eq!(sql_bjork["sql"]["rows"], json!([[3]]));

    let (status, sql_total) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "How many plays?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sql_total}");
    assert_eq!(sql_total["status"], "ok", "{sql_total}");
    assert_eq!(sql_total["sql"]["rows"], json!([[4]]));

    // 2. Follow-up request carries previous question and SQL definition.
    let (status, follow_up) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({
            "question": "and in 2024?",
            "previous": {
                "question": "How many times did I listen to Björk?",
                "sql": {
                    "title": sql_bjork["sql"]["title"],
                    "computed": sql_bjork["sql"]["computed"],
                    "sql": sql_bjork["sql"]["sql"],
                }
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{follow_up}");
    assert_eq!(follow_up["status"], "ok", "{follow_up}");
    assert_eq!(follow_up["sql"]["title"], "Follow-up (2024)");
    assert_eq!(follow_up["sql"]["rows"], json!([[1]]));

    // 3. Failed local SQL query reports explicit rule code and suggestions.
    let (status, failed_sql) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "Show me settings"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{failed_sql}");
    assert_eq!(failed_sql["status"], "failed", "{failed_sql}");
    assert_eq!(failed_sql["sql"]["rule_code"], "unauthorized_table");
    let msg = failed_sql["message"].as_str().unwrap();
    assert!(msg.contains("unauthorized_table"), "{msg}");
}

#[tokio::test]
async fn ask_sql_requires_owner_session_csrf_and_rejects_read_api_keys() {
    let (_dir, state, token) = setup(true).await;
    save(&state.storage, &fixture_events()).await;

    let (status, key_body) = post(
        &state,
        &token,
        "/api/v1/api-keys",
        json!({"name": "reader", "application": "test"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{key_body}");
    let api_key = key_body["secret"].as_str().unwrap().to_owned();

    let payload =
        serde_json::to_vec(&json!({"sql": "SELECT COUNT(*) AS plays FROM listens"})).unwrap();

    // Missing session -> 401 Unauthorized.
    let no_session = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/ask/sql")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("content-type", "application/json")
                .body(Body::from(payload.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_session.status(), StatusCode::UNAUTHORIZED);

    // Missing CSRF token -> 403 Forbidden.
    let no_csrf = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/ask/sql")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("cookie", format!("scrobbl_session={token}"))
                .header("content-type", "application/json")
                .body(Body::from(payload.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);

    // Wrong Origin -> 403 Forbidden.
    let bad_origin = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/ask/sql")
                .header("host", "127.0.0.1:3210")
                .header("origin", "https://evil.example")
                .header("cookie", format!("scrobbl_session={token}"))
                .header("x-csrf-token", "test-csrf")
                .header("content-type", "application/json")
                .body(Body::from(payload.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_origin.status(), StatusCode::FORBIDDEN);

    // Read API key alone -> 403 Forbidden.
    let read_key_only = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/ask/sql")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("authorization", format!("Bearer {api_key}"))
                .header("content-type", "application/json")
                .body(Body::from(payload.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_key_only.status(), StatusCode::FORBIDDEN);

    // Read API key even alongside owner cookie & CSRF -> 403 Forbidden.
    let read_key_with_cookie = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/ask/sql")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("cookie", format!("scrobbl_session={token}"))
                .header("x-csrf-token", "test-csrf")
                .header("authorization", format!("Bearer {api_key}"))
                .header("content-type", "application/json")
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_key_with_cookie.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn ask_mode_requires_owner_session_csrf_and_rejects_read_api_keys() {
    let (_dir, state, token) = setup(true).await;
    save(&state.storage, &fixture_events()).await;

    let (status, key_body) = post(
        &state,
        &token,
        "/api/v1/api-keys",
        json!({"name": "reader", "application": "test"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{key_body}");
    let api_key = key_body["secret"].as_str().unwrap().to_owned();

    let payload = serde_json::to_vec(&json!({
        "question": "How many times did I listen to Björk?",
    }))
    .unwrap();

    // Missing session -> 401 Unauthorized.
    let no_session = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/ask")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("content-type", "application/json")
                .body(Body::from(payload.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_session.status(), StatusCode::UNAUTHORIZED);

    // Missing CSRF token -> 403 Forbidden.
    let no_csrf = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/ask")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("cookie", format!("scrobbl_session={token}"))
                .header("content-type", "application/json")
                .body(Body::from(payload.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);

    // Read API key alone -> 403 Forbidden.
    let read_key_only = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/ask")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("authorization", format!("Bearer {api_key}"))
                .header("content-type", "application/json")
                .body(Body::from(payload.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_key_only.status(), StatusCode::FORBIDDEN);

    // Read API key even alongside owner cookie & CSRF -> 403 Forbidden.
    let read_key_with_cookie = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/ask")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("cookie", format!("scrobbl_session={token}"))
                .header("x-csrf-token", "test-csrf")
                .header("authorization", format!("Bearer {api_key}"))
                .header("content-type", "application/json")
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_key_with_cookie.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn ask_sql_executes_valid_sql_and_rejects_unsafe_sql_with_rule_code() {
    let (_dir, state, token) = setup(true).await;
    save(&state.storage, &fixture_events()).await;

    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask/sql",
        json!({
            "sql": "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1",
            "title": "Total plays",
            "answer_kind": "value",
            "columns": [{"name": "plays", "format": "count"}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["sql"]["answer_kind"], "value");
    assert_eq!(body["sql"]["title"], "Total plays");
    assert_eq!(body["sql"]["rows"], json!([[4]]));
    assert_eq!(
        body["sql"]["columns"],
        json!([{"name": "plays", "format": "count"}])
    );
    assert!(
        body["sql"]["as_of_ms"].as_i64().is_some_and(|ms| ms > 0),
        "{body}"
    );

    // Forbidden SQL is rejected with its sandbox rule and no rows.
    for (sql, expected_rule) in [
        ("SELECT value FROM settings", "unauthorized_table"),
        ("DELETE FROM listens", "not_select"),
    ] {
        let (status, body) = post(&state, &token, "/api/v1/ask/sql", json!({"sql": sql})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "failed", "{sql}: {body}");
        assert_eq!(body["sql"]["rule_code"], expected_rule, "{sql}: {body}");
        assert_eq!(body["sql"]["sql"], sql, "{sql}: {body}");
        assert_eq!(body["sql"]["rows"], json!([]), "{sql}: {body}");
        assert!(
            !body["message"].as_str().unwrap_or("").is_empty(),
            "{sql}: {body}"
        );
    }
}

#[tokio::test]
async fn ask_sql_resolves_entity_links_through_projection_keys() {
    let (_dir, state, token) = setup(true).await;
    // Two distinct tracks sharing the same title "Intro" by different artists/albums,
    // plus a unique track whose title contains HTML and SQL.
    let events = vec![
        event(
            10,
            at("2024-05-03T10:00:00Z"),
            Some(180_000),
            DurationKind::Actual,
            "ar-alpha",
            "Alpha",
            "al-one",
            "First Album",
            "Intro",
        ),
        event(
            11,
            at("2024-05-03T11:00:00Z"),
            Some(180_000),
            DurationKind::Actual,
            "ar-beta",
            "Beta",
            "al-two",
            "Second Album",
            "Intro",
        ),
        event(
            12,
            at("2024-05-03T12:00:00Z"),
            Some(180_000),
            DurationKind::Actual,
            "ar-gamma",
            "Gamma",
            "al-three",
            "Third Album",
            "<img src=x onerror=alert(1)>'; DROP TABLE listens;--",
        ),
    ];
    save(&state.storage, &events).await;

    // Unique artist, album, and track (including HTML+SQL title) resolve to /music/... links
    // even when the SQL query does not project ID columns.
    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask/sql",
        json!({
            "sql": "SELECT ar.name AS artist, al.name AS album, t.title AS track, COUNT(*) AS plays \
                    FROM listens l \
                    JOIN tracks t ON t.id = l.track_id \
                    JOIN albums al ON al.id = t.album_id \
                    JOIN track_artists ta ON ta.track_id = t.id \
                    JOIN artists ar ON ar.id = ta.artist_id \
                    WHERE l.counted = 1 AND ar.name = 'Gamma' \
                    GROUP BY t.id",
            "columns": [
                {"name": "artist", "format": "artist"},
                {"name": "album", "format": "album"},
                {"name": "track", "format": "track"},
                {"name": "plays", "format": "count"}
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let links = body["sql"]["cell_links"].as_array().unwrap();
    assert_eq!(links.len(), 1, "{body}");
    let cells = links[0].as_array().unwrap();
    assert!(
        cells[0].as_str().unwrap().starts_with("/music/artists/"),
        "{cells:?}"
    );
    assert!(
        cells[1].as_str().unwrap().starts_with("/music/albums/"),
        "{cells:?}"
    );
    assert!(
        cells[2].as_str().unwrap().starts_with("/music/tracks/"),
        "{cells:?}"
    );
    assert!(cells[3].is_null(), "{cells:?}");

    // Ambiguous title "Intro" without artist/album/id is NOT guessed (null link).
    let (_, ambig) = post(
        &state,
        &token,
        "/api/v1/ask/sql",
        json!({
            "sql": "SELECT t.title AS track FROM tracks t WHERE t.title = 'Intro'",
            "columns": [{"name": "track", "format": "track"}]
        }),
    )
    .await;
    assert_eq!(ambig["sql"]["cell_links"], json!([[null], [null]]));

    // Disambiguating by artist in the same row resolves each "Intro" to its own track page.
    let (_, by_artist) = post(
        &state,
        &token,
        "/api/v1/ask/sql",
        json!({
            "sql": "SELECT ar.name AS artist, t.title AS track \
                    FROM tracks t JOIN track_artists ta ON ta.track_id = t.id JOIN artists ar ON ar.id = ta.artist_id \
                    WHERE t.title = 'Intro' ORDER BY ar.name ASC",
            "columns": [
                {"name": "artist", "format": "artist"},
                {"name": "track", "format": "track"}
            ]
        }),
    )
    .await;
    let by_artist_links = by_artist["sql"]["cell_links"].as_array().unwrap();
    assert_ne!(by_artist_links[0][1], by_artist_links[1][1]);
    assert!(
        by_artist_links[0][1]
            .as_str()
            .unwrap()
            .starts_with("/music/tracks/")
    );
    assert!(
        by_artist_links[1][1]
            .as_str()
            .unwrap()
            .starts_with("/music/tracks/")
    );

    // Including an explicit track_id column also resolves to that exact track ID.
    let (_, disambig) = post(
        &state,
        &token,
        "/api/v1/ask/sql",
        json!({
            "sql": "SELECT t.id AS track_id, ar.name AS artist, t.title AS track \
                    FROM tracks t JOIN track_artists ta ON ta.track_id = t.id JOIN artists ar ON ar.id = ta.artist_id \
                    WHERE t.title = 'Intro' ORDER BY ar.name ASC",
            "columns": [
                {"name": "track_id", "format": "text"},
                {"name": "artist", "format": "artist"},
                {"name": "track", "format": "track"}
            ]
        }),
    )
    .await;
    let rows = disambig["sql"]["rows"].as_array().unwrap();
    let dlinks = disambig["sql"]["cell_links"].as_array().unwrap();
    assert_eq!(
        dlinks[0][2].as_str().unwrap(),
        format!("/music/tracks/{}", rows[0][0].as_str().unwrap())
    );
    assert_eq!(
        dlinks[1][2].as_str().unwrap(),
        format!("/music/tracks/{}", rows[1][0].as_str().unwrap())
    );
}

#[tokio::test]
async fn ask_sql_maps_exact_history_filters_and_omits_unmappable_queries() {
    let (_dir, state, token) = setup(true).await;
    save(&state.storage, &fixture_events()).await;

    let (_, bjork) = post(
        &state,
        &token,
        "/api/v1/ask/sql",
        json!({
            "sql": "SELECT COUNT(DISTINCT l.id) AS plays FROM listens l \
                    JOIN track_artists ta ON ta.track_id = l.track_id \
                    JOIN artists a ON a.id = ta.artist_id \
                    WHERE l.counted = 1 AND LOWER(a.name) = LOWER('Björk')"
        }),
    )
    .await;
    let bjork_href = bjork["sql"]["history_href"].as_str().unwrap();
    assert!(bjork_href.starts_with("/history?"), "{bjork_href}");
    assert!(bjork_href.contains("period=all"), "{bjork_href}");
    assert!(bjork_href.contains("status=counted"), "{bjork_href}");
    assert!(bjork_href.contains("artist="), "{bjork_href}");

    for (sql, expected) in [
        (
            "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1",
            "/history?period=all&status=counted",
        ),
        (
            "SELECT COUNT(*) AS plays FROM listens",
            "/history?period=all&status=all",
        ),
        (
            "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1 AND local_year = 2024",
            "/history?period=all&year=2024&status=counted",
        ),
        (
            "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1 AND local_date >= '2024-05-01' AND local_date <= '2024-05-02'",
            "/history?period=custom&from=2024-05-01&to=2024-05-02&status=counted",
        ),
        (
            "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1 AND local_month = 5 AND weekday = 3",
            "/history?period=all&month=5&weekday=wednesday&status=counted",
        ),
    ] {
        let (status, body) = post(&state, &token, "/api/v1/ask/sql", json!({"sql": sql})).await;
        assert_eq!(status, StatusCode::OK, "{sql}: {body}");
        assert_eq!(
            body["sql"]["history_href"].as_str(),
            Some(expected),
            "{sql}: {body}"
        );
    }

    // Queries whose filters do not map exactly to /history filters must omit the link.
    for unmappable in [
        "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1 AND local_hour = 0",
        "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1 OR needs_review = 1",
        "SELECT t.title AS track, COUNT(*) AS plays FROM listens l JOIN tracks t ON t.id = l.track_id WHERE l.counted = 1 GROUP BY t.id HAVING COUNT(*) > 1",
        "SELECT COUNT(*) AS plays FROM listens l JOIN tracks t ON t.id = l.track_id WHERE t.title LIKE '%Jóga%'",
        "WITH c AS (SELECT * FROM listens WHERE counted = 1) SELECT COUNT(*) FROM c",
        "SELECT COUNT(*) AS plays FROM listens WHERE local_year IN (2023, 2024)",
        "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1 AND duration_kind = 'actual'",
    ] {
        let (status, body) = post(
            &state,
            &token,
            "/api/v1/ask/sql",
            json!({"sql": unmappable}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{unmappable}: {body}");
        assert!(
            body["sql"]["history_href"].is_null(),
            "expected no history link for `{unmappable}`, got {body}"
        );
    }
}
