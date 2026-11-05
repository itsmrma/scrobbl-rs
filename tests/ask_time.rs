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

async fn setup() -> (tempfile::TempDir, AppState, String) {
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
    sqlx::query("INSERT INTO settings(key,value) VALUES('ai_enabled','1')")
        .execute(&storage.writer)
        .await
        .unwrap();
    let mut state = AppState::new(storage, config);
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
        event(
            3,
            at("2024-05-02T09:00:00Z"),
            None,
            DurationKind::Unknown,
            "bjork",
            "Björk",
            "homogenic",
            "Homogenic",
            "Jóga",
        ),
        // Below the counting threshold: present in the archive, never counted.
        event(
            4,
            at("2024-05-03T12:00:00Z"),
            Some(1_000),
            DurationKind::Actual,
            "bjork",
            "Björk",
            "homogenic",
            "Homogenic",
            "Jóga",
        ),
        event(
            5,
            at("2025-06-10T09:00:00Z"),
            Some(120_000),
            DurationKind::Estimated,
            "bjork",
            "Björk",
            "homogenic",
            "Homogenic",
            "Jóga",
        ),
        event(
            6,
            at("2024-05-01T10:30:00Z"),
            Some(210_000),
            DurationKind::Actual,
            "radiohead",
            "Radiohead",
            "ok-computer",
            "OK Computer",
            "Karma Police",
        ),
        event(
            7,
            at("2024-05-02T09:30:00Z"),
            None,
            DurationKind::Unknown,
            "radiohead",
            "Radiohead",
            "ok-computer",
            "OK Computer",
            "Karma Police",
        ),
        event(
            8,
            at("2025-06-10T10:00:00Z"),
            Some(90_000),
            DurationKind::Actual,
            "radiohead",
            "Radiohead",
            "ok-computer",
            "OK Computer",
            "Karma Police",
        ),
    ]
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

async fn run(state: &AppState, token: &str, query: Value) -> Value {
    let (status, body) = post(state, token, "/api/v1/ask/query", json!({"query": query})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn assert_time(body: &Value, actual: i64, estimated: i64, unknown: i64, total: i64) {
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["metric"], "time_listened", "{body}");
    let time = &body["result"]["time"];
    assert_eq!(time["actual_ms"], actual, "{body}");
    assert_eq!(time["estimated_ms"], estimated, "{body}");
    assert_eq!(time["known_ms"], actual + estimated, "{body}");
    // The card's primary number is the known time; unknown is never zero.
    assert_eq!(body["result"]["scalar"], actual + estimated, "{body}");
    assert_eq!(time["unknown_listens"], unknown, "{body}");
    assert_eq!(body["result"]["total"], total, "{body}");
    let percent = time["unknown_percent"].as_f64().unwrap();
    let expected = if total == 0 {
        0.0
    } else {
        unknown as f64 / total as f64 * 100.0
    };
    assert!(
        (percent - expected).abs() < 1e-9,
        "unknown percent {percent} != {expected} in {body}"
    );
}

#[tokio::test]
async fn time_listened_sums_known_time_and_reports_unknown_separately() {
    let (_dir, state, token) = setup().await;
    save(&state.storage, &fixture_events()).await;

    // All counted listens: actual 200000 + 210000 + 90000, estimated
    // 180000 + 120000. The 1000 ms short listen never enters the selection.
    let body = run(&state, &token, query("time_listened")).await;
    assert_time(&body, 500_000, 300_000, 2, 7);
    assert_eq!(body["result"]["time"]["known_ms"], 800_000);
    assert_ne!(
        body["result"]["time"]["known_ms"],
        500_000 + 300_000 + 1_000,
        "unknown or uncounted time must not leak into the known total"
    );

    // Entity filtering keeps the same actual/estimated/unknown split.
    let mut scoped = query("time_listened");
    scoped["entity"] = artist_entity("Björk");
    let body = run(&state, &token, scoped).await;
    assert_time(&body, 200_000, 300_000, 1, 4);
    assert!(
        body["result"]["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["label"] == "View artist"
                && link["href"]
                    .as_str()
                    .unwrap()
                    .starts_with("/music/artists/"))
    );

    let mut scoped = query("time_listened");
    scoped["entity"] = artist_entity("Radiohead");
    let body = run(&state, &token, scoped).await;
    assert_time(&body, 300_000, 0, 1, 3);

    // Year filtering: 2024 excludes the 2025 listens, 2025 excludes unknown.
    let mut year = query("time_listened");
    year["year"] = json!(2024);
    let body = run(&state, &token, year).await;
    assert_time(&body, 410_000, 180_000, 2, 5);

    let mut year = query("time_listened");
    year["year"] = json!(2024);
    year["entity"] = artist_entity("Björk");
    let body = run(&state, &token, year).await;
    assert_time(&body, 200_000, 180_000, 1, 3);

    let mut year = query("time_listened");
    year["year"] = json!(2024);
    year["entity"] = artist_entity("Radiohead");
    let body = run(&state, &token, year).await;
    assert_time(&body, 210_000, 0, 1, 2);

    let mut year = query("time_listened");
    year["year"] = json!(2025);
    let body = run(&state, &token, year).await;
    assert_time(&body, 90_000, 120_000, 0, 2);
}

#[tokio::test]
async fn time_listened_unknown_is_zero_percent_when_every_counted_listen_is_known() {
    let (_dir, state, token) = setup().await;
    save(&state.storage, &fixture_events()).await;

    let mut year = query("time_listened");
    year["year"] = json!(2025);
    let body = run(&state, &token, year).await;
    assert_time(&body, 90_000, 120_000, 0, 2);
    assert_eq!(body["result"]["time"]["unknown_percent"], 0.0);
    assert_eq!(body["result"]["time"]["known_ms"], 210_000);
}

#[tokio::test]
async fn time_listened_filters_by_hour_in_the_assigned_calendar() {
    let (_dir, state, token) = setup().await;
    save(&state.storage, &fixture_events()).await;

    // 10:00 and 10:30 UTC are 12:00 and 12:30 in Europe/Rome.
    let mut hour = query("time_listened");
    hour["hour"] = json!(12);
    let body = run(&state, &token, hour).await;
    assert_time(&body, 500_000, 0, 0, 3);

    // 09:00 and 09:30 UTC are 11:00 and 11:30: one estimated and two unknown.
    let mut hour = query("time_listened");
    hour["hour"] = json!(11);
    let body = run(&state, &token, hour).await;
    assert_time(&body, 0, 120_000, 2, 3);
}

#[tokio::test]
async fn ask_with_the_stub_time_phrase_returns_the_time_coverage() {
    let (_dir, state, token) = setup().await;
    save(&state.storage, &fixture_events()).await;

    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "How many hours did I listen in total?", "context": null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["query"]["metric"], "time_listened", "{body}");
    assert_time(&body, 500_000, 300_000, 2, 7);

    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask",
        json!({"question": "Quanto tempo ho ascoltato?", "context": null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["query"]["metric"], "time_listened", "{body}");
    assert_time(&body, 500_000, 300_000, 2, 7);
}
