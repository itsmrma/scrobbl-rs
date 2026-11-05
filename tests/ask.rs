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

fn query(metric: &str) -> Value {
    json!({
        "metric": metric,
        "entity": null,
        "period": "all_time",
        "year": null,
        "month": null,
        "weekday": null,
        "hour": null,
        "source": null,
        "group_by": null,
        "sort_by": "metric",
        "order": "desc",
        "limit": null,
    })
}

fn artist_entity(name: &str) -> Value {
    json!({"kind": "artist", "name": name, "id": null})
}

#[tokio::test]
async fn ask_counts_and_first_last_match_the_fixture() {
    let (_dir, state, token) = setup(true).await;
    save(&state.storage, &fixture_events()).await;

    // The status endpoint is owned by the AI settings surface: this fixture
    // enables AI but has not downloaded or selected a model.
    let (status, body) = get(&state, &token, "/api/v1/ask/status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], true);
    assert_eq!(body["model_ready"], false);

    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "How many times did I listen to Björk?", "context": null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["result"]["scalar"], 3);
    assert_eq!(body["result"]["metric"], "plays");
    let chips = body["chips"].as_array().unwrap();
    assert!(chips.iter().any(|chip| chip["key"] == "metric"));
    assert!(chips.iter().any(|chip| chip["key"] == "entity"));
    assert!(
        chips
            .iter()
            .any(|chip| chip["key"] == "entity" && chip["value"] == "Björk")
    );

    // Italian phrasing reaches the same deterministic result.
    let (_, body) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "Quante volte ho ascoltato Björk?", "context": null}),
    )
    .await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["result"]["scalar"], 3);

    let mut plays = query("plays");
    plays["year"] = json!(2024);
    let (status, body) = post(&state, &token, "/api/v1/ask/query", json!({"query": plays})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["scalar"], 4);

    let mut plays = query("plays");
    plays["entity"] = artist_entity("Björk");
    plays["month"] = json!(5);
    let (_, body) = post(&state, &token, "/api/v1/ask/query", json!({"query": plays})).await;
    assert_eq!(body["result"]["scalar"], 3);

    // Assigned calendar weekday: the short listen is excluded, and the
    // 22:30 UTC listen belongs to Thursday 2 May, not Wednesday.
    let mut plays = query("plays");
    plays["weekday"] = json!("wednesday");
    let (_, body) = post(&state, &token, "/api/v1/ask/query", json!({"query": plays})).await;
    assert_eq!(body["result"]["scalar"], 2);

    // Hour 00 is the local hour in Europe/Rome, not the UTC hour.
    let mut plays = query("plays");
    plays["hour"] = json!(0);
    let (_, body) = post(&state, &token, "/api/v1/ask/query", json!({"query": plays})).await;
    assert_eq!(body["result"]["scalar"], 1);

    let mut last = query("last_play");
    last["entity"] = artist_entity("Björk");
    let (_, body) = post(&state, &token, "/api/v1/ask/query", json!({"query": last})).await;
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["scalar"], at("2024-05-02T09:00:00Z"));
    assert_eq!(body["result"]["last"]["title"], "Jóga");
    assert_eq!(body["result"]["last"]["artist"], "Björk");
    assert_eq!(body["result"]["last"]["album"], "Homogenic");
    assert!(
        body["result"]["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["href"].as_str().unwrap().starts_with("/music/tracks/"))
    );

    let mut first = query("first_play");
    first["entity"] = artist_entity("Radiohead");
    first["hour"] = json!(0);
    let (_, body) = post(&state, &token, "/api/v1/ask/query", json!({"query": first})).await;
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["scalar"], at("2024-05-01T22:30:00Z"));
    assert!(
        body["result"]["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["label"] == "Played on 2024-05-02")
    );

    let mut first = query("first_play");
    first["entity"] = artist_entity("Björk");
    let (_, body) = post(&state, &token, "/api/v1/ask/query", json!({"query": first})).await;
    assert_eq!(body["result"]["scalar"], at("2024-05-01T10:00:00Z"));
}

#[tokio::test]
async fn ask_ambiguous_name_returns_candidates() {
    let (_dir, state, token) = setup(true).await;
    save(
        &state.storage,
        &[
            event(
                1,
                at("2024-05-01T10:00:00Z"),
                Some(200_000),
                DurationKind::Actual,
                "cure-1",
                "Nina Simone",
                "album-1",
                "Album One",
                "Track One",
            ),
            event(
                2,
                at("2024-05-02T10:00:00Z"),
                Some(200_000),
                DurationKind::Actual,
                "cure-2",
                "Nina Hagen",
                "album-2",
                "Album Two",
                "Track Two",
            ),
        ],
    )
    .await;

    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "How many times did I listen to Nina?", "context": null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ambiguous");
    assert!(body["result"].is_null());
    let candidates = body["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate["kind"] == "artist")
    );
    let names: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Nina Simone"));
    assert!(names.contains(&"Nina Hagen"));

    // A candidate is an explicit user choice; the DSL then carries its ID.
    let mut chosen = body["query"].clone();
    chosen["entity"]["id"] = candidates[0]["id"].clone();
    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": chosen}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["result"]["scalar"], 1);
}

#[tokio::test]
async fn ask_refuses_unsupported_questions_and_metrics_explicitly() {
    let (_dir, state, token) = setup(true).await;
    save(&state.storage, &fixture_events()).await;

    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "What is the weather?", "context": null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "unsupported");
    assert!(body["query"].is_null());
    assert!(body["result"].is_null());
    let message = body["message"].as_str().unwrap();
    assert!(message.contains("play counts"));
    assert!(message.contains("first plays"));
    assert!(message.contains("last plays"));

    let (_, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": query("streak")}),
    )
    .await;
    assert_eq!(body["status"], "unsupported");
    assert!(body["query"].is_object());
    let message = body["message"].as_str().unwrap();
    assert!(message.contains("plays"));
    assert!(message.contains("first_play"));
    assert!(message.contains("last_play"));
}

#[tokio::test]
async fn ask_query_rejects_invalid_dsl() {
    let (_dir, state, token) = setup(true).await;
    let mut invalid = query("plays");
    invalid["month"] = json!(13);
    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": invalid}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let mut invalid = query("plays");
    invalid["hour"] = json!(24);
    let (status, _) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": invalid}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
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
        json!({"question": "How many times did I listen to Björk?", "context": null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "model_unavailable");
    assert!(body["result"].is_null());
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("Enable it in Settings")
    );
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
