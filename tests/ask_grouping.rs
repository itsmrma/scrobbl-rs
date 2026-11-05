use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use scrobbl_rs::{
    AppState,
    ask::Ask,
    ask::dsl::Limit,
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

/// Eight counted listens across day/week/month/year boundaries, plus one short
/// listen below the counting threshold. Repeated plays of the same song reuse
/// the same source index so they share one track identity.
fn fixture() -> Vec<Event> {
    vec![
        // 2023-12-31 23:30 UTC = Monday 2024-01-01 00:30 in Europe/Rome.
        event(
            1,
            at("2023-12-31T23:30:00Z"),
            Some(200_000),
            DurationKind::Actual,
            "bjork",
            "Björk",
            "homogenic",
            "Homogenic",
            "Jóga",
        ),
        event(
            1,
            at("2024-01-01T09:00:00Z"),
            Some(180_000),
            DurationKind::Estimated,
            "bjork",
            "Björk",
            "homogenic",
            "Homogenic",
            "Jóga",
        ),
        // Sunday, same Monday-start week.
        event(
            3,
            at("2024-01-07T09:00:00Z"),
            Some(210_000),
            DurationKind::Actual,
            "radiohead",
            "Radiohead",
            "ok-computer",
            "OK Computer",
            "Karma Police",
        ),
        // Next Monday.
        event(
            4,
            at("2024-01-08T09:00:00Z"),
            Some(180_000),
            DurationKind::Actual,
            "sigur",
            "Sigur Rós",
            "agaetis",
            "Ágætis byrjun",
            "Hoppípolla",
        ),
        // 2024-01-31 23:30 UTC = Thursday 2024-02-01 00:30 in Europe/Rome.
        event(
            5,
            at("2024-01-31T23:30:00Z"),
            Some(200_000),
            DurationKind::Actual,
            "nina-1",
            "Nina Simone",
            "pastel",
            "Pastel Blues",
            "Sinnerman",
        ),
        event(
            6,
            at("2024-02-01T09:00:00Z"),
            None,
            DurationKind::Unknown,
            "nina-2",
            "Nina Simone",
            "spell",
            "I Put a Spell on You",
            "Feeling Good",
        ),
        // Year boundary, both later assigned to a Sunday-start calendar.
        event(
            4,
            at("2024-12-31T09:00:00Z"),
            Some(180_000),
            DurationKind::Actual,
            "sigur",
            "Sigur Rós",
            "agaetis",
            "Ágætis byrjun",
            "Hoppípolla",
        ),
        event(
            3,
            at("2025-01-01T09:00:00Z"),
            Some(210_000),
            DurationKind::Actual,
            "radiohead",
            "Radiohead",
            "ok-computer",
            "OK Computer",
            "Karma Police",
        ),
        // Below the counting threshold: present, never counted.
        event(
            1,
            at("2024-01-02T09:00:00Z"),
            Some(1_000),
            DurationKind::Actual,
            "bjork",
            "Björk",
            "homogenic",
            "Homogenic",
            "Jóga",
        ),
    ]
}

/// A second assigned calendar proves week grouping uses each listen's stored
/// week_date instead of one global week start.
async fn assign_sunday_calendar(state: &AppState) {
    sqlx::query("INSERT INTO calendars VALUES('sunday','Europe/Rome',6,1)")
        .execute(&state.storage.writer)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE listens SET calendar_id='sunday',week_date='2024-12-29' WHERE played_at_ms IN (?,?)",
    )
    .bind(at("2024-12-31T09:00:00Z"))
    .bind(at("2025-01-01T09:00:00Z"))
    .execute(&state.storage.writer)
    .await
    .unwrap();
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
        "sort_by": "date",
        "order": "asc",
        "limit": null,
    })
}

fn grouped(metric: &str, group_by: &str) -> Value {
    let mut query = query(metric);
    query["group_by"] = json!(group_by);
    query["sort_by"] = json!("date");
    query["order"] = json!("asc");
    query
}

async fn run(state: &AppState, token: &str, query: Value) -> Value {
    let (status, body) = post(state, token, "/api/v1/ask/query", json!({"query": query})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
    body
}

fn rows(body: &Value) -> Vec<(String, i64)> {
    body["result"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["label"].as_str().unwrap().to_owned(),
                row["plays"].as_i64().unwrap(),
            )
        })
        .collect()
}

fn row_map(body: &Value) -> std::collections::BTreeMap<String, i64> {
    rows(body).into_iter().collect()
}

#[tokio::test]
async fn temporal_grouping_follows_the_assigned_calendar() {
    let (_dir, state, token) = setup().await;
    save(&state.storage, &fixture()).await;
    assign_sunday_calendar(&state).await;

    let day = run(&state, &token, grouped("plays", "day")).await;
    assert_eq!(
        rows(&day),
        vec![
            ("2024-01-01".into(), 2),
            ("2024-01-07".into(), 1),
            ("2024-01-08".into(), 1),
            ("2024-02-01".into(), 2),
            ("2024-12-31".into(), 1),
            ("2025-01-01".into(), 1),
        ]
    );
    assert_eq!(day["result"]["total"], 6);
    // Day coverage: one actual and one estimated listen, no unknown duration.
    assert_eq!(day["result"]["rows"][0]["time"]["actual_ms"], 200_000);
    assert_eq!(day["result"]["rows"][0]["time"]["estimated_ms"], 180_000);
    assert_eq!(day["result"]["rows"][0]["time"]["unknown_listens"], 0);

    let week = run(&state, &token, grouped("plays", "week")).await;
    assert_eq!(
        row_map(&week),
        std::collections::BTreeMap::from([
            ("2024-01-01".into(), 3),
            ("2024-01-08".into(), 1),
            ("2024-01-29".into(), 2),
            // Sunday-start calendar: 31 Dec and 1 Jan both belong to 29 Dec.
            ("2024-12-29".into(), 2),
        ])
    );
    assert_eq!(week["result"]["total"], 4);

    let month = run(&state, &token, grouped("plays", "month")).await;
    assert_eq!(
        rows(&month),
        vec![
            ("2024-01".into(), 4),
            ("2024-02".into(), 2),
            ("2024-12".into(), 1),
            ("2025-01".into(), 1),
        ]
    );

    let year = run(&state, &token, grouped("plays", "year")).await;
    assert_eq!(rows(&year), vec![("2024".into(), 7), ("2025".into(), 1)]);

    let hour = run(&state, &token, grouped("plays", "hour")).await;
    assert_eq!(
        row_map(&hour),
        std::collections::BTreeMap::from([("00:00".into(), 2), ("10:00".into(), 6)])
    );

    let weekday = run(&state, &token, grouped("plays", "weekday")).await;
    assert_eq!(
        rows(&weekday),
        vec![
            ("Monday".into(), 3),
            ("Tuesday".into(), 1),
            ("Wednesday".into(), 1),
            ("Thursday".into(), 2),
            ("Sunday".into(), 1),
        ]
    );
}

#[tokio::test]
async fn categorical_grouping_merges_names_and_builds_links() {
    let (_dir, state, token) = setup().await;
    save(&state.storage, &fixture()).await;

    let mut artists_query = grouped("plays", "artist");
    artists_query["sort_by"] = json!("metric");
    artists_query["order"] = json!("desc");
    let artists = run(&state, &token, artists_query).await;
    assert_eq!(
        rows(&artists),
        vec![
            ("Björk".into(), 2),
            ("Nina Simone".into(), 2),
            ("Radiohead".into(), 2),
            ("Sigur Rós".into(), 2),
        ]
    );
    for row in artists["result"]["rows"].as_array().unwrap() {
        assert!(
            row["href"].as_str().unwrap().starts_with("/music/artists/"),
            "{row}"
        );
        assert!(!row["key"].as_str().unwrap().is_empty());
    }

    let albums = run(&state, &token, grouped("plays", "album")).await;
    assert_eq!(
        row_map(&albums),
        std::collections::BTreeMap::from([
            ("Ágætis byrjun".into(), 2),
            ("Homogenic".into(), 2),
            ("OK Computer".into(), 2),
            ("I Put a Spell on You".into(), 1),
            ("Pastel Blues".into(), 1),
        ])
    );

    let mut tracks_query = grouped("plays", "track");
    tracks_query["sort_by"] = json!("metric");
    tracks_query["order"] = json!("desc");
    let tracks = run(&state, &token, tracks_query).await;
    assert_eq!(
        rows(&tracks),
        vec![
            ("Hoppípolla".into(), 2),
            ("Jóga".into(), 2),
            ("Karma Police".into(), 2),
            ("Feeling Good".into(), 1),
            ("Sinnerman".into(), 1),
        ]
    );
    for row in tracks["result"]["rows"].as_array().unwrap() {
        assert!(
            row["href"].as_str().unwrap().starts_with("/music/tracks/"),
            "{row}"
        );
    }
}

#[tokio::test]
async fn ordering_and_limit_produce_a_deterministic_top_n() {
    let (_dir, state, token) = setup().await;
    save(&state.storage, &fixture()).await;

    let mut descending = grouped("plays", "track");
    descending["sort_by"] = json!("metric");
    descending["order"] = json!("desc");
    let top = run(&state, &token, descending).await;
    assert_eq!(
        rows(&top),
        vec![
            ("Hoppípolla".into(), 2),
            ("Jóga".into(), 2),
            ("Karma Police".into(), 2),
            ("Feeling Good".into(), 1),
            ("Sinnerman".into(), 1),
        ]
    );

    let mut ascending = grouped("plays", "track");
    ascending["sort_by"] = json!("metric");
    ascending["order"] = json!("asc");
    let bottom = run(&state, &token, ascending).await;
    assert_eq!(
        rows(&bottom),
        vec![
            ("Feeling Good".into(), 1),
            ("Sinnerman".into(), 1),
            ("Hoppípolla".into(), 2),
            ("Jóga".into(), 2),
            ("Karma Police".into(), 2),
        ]
    );

    let mut one = grouped("plays", "track");
    one["sort_by"] = json!("metric");
    one["order"] = json!("desc");
    one["limit"] = json!("one");
    let one = run(&state, &token, one).await;
    assert_eq!(rows(&one), vec![("Hoppípolla".into(), 2)]);
    assert_eq!(one["result"]["total"], 1);
    assert!(one["result"]["scalar"].is_null());

    let mut five = grouped("plays", "track");
    five["limit"] = json!("five");
    let five = run(&state, &token, five).await;
    assert_eq!(five["result"]["rows"].as_array().unwrap().len(), 5);
    assert_eq!(five["result"]["total"], 5);

    // The declared maximum is enforced by the enum and clamped defensively.
    assert_eq!(Limit::MAX, 100);
    for limit in [
        Limit::One,
        Limit::Five,
        Limit::Ten,
        Limit::TwentyFive,
        Limit::Fifty,
        Limit::Hundred,
    ] {
        assert!(limit.value() <= Limit::MAX);
    }
}

#[tokio::test]
async fn distinct_metrics_count_entities_in_and_out_of_groups() {
    let (_dir, state, token) = setup().await;
    save(&state.storage, &fixture()).await;

    for (metric, expected) in [
        ("distinct_tracks", 5),
        ("distinct_artists", 4),
        ("distinct_albums", 5),
    ] {
        let body = run(&state, &token, query(metric)).await;
        assert_eq!(body["result"]["scalar"], expected, "{metric}: {body}");
        assert!(body["result"]["rows"].as_array().unwrap().is_empty());
        assert_eq!(body["result"]["total"], expected);
    }

    let tracks_by_year = run(&state, &token, grouped("distinct_tracks", "year")).await;
    assert_eq!(
        row_map(&tracks_by_year),
        std::collections::BTreeMap::from([("2024".into(), 5), ("2025".into(), 1)])
    );

    let artists_by_month = run(&state, &token, grouped("distinct_artists", "month")).await;
    assert_eq!(
        row_map(&artists_by_month),
        std::collections::BTreeMap::from([
            ("2024-01".into(), 3),
            ("2024-02".into(), 1),
            ("2024-12".into(), 1),
            ("2025-01".into(), 1),
        ])
    );

    let albums_by_year = run(&state, &token, grouped("distinct_albums", "year")).await;
    assert_eq!(
        row_map(&albums_by_year),
        std::collections::BTreeMap::from([("2024".into(), 5), ("2025".into(), 1)])
    );

    // A top-N of distinct tracks by artist keeps the group distinct counts.
    let mut top = grouped("distinct_tracks", "artist");
    top["sort_by"] = json!("metric");
    top["order"] = json!("desc");
    top["limit"] = json!("one");
    let top = run(&state, &token, top).await;
    assert_eq!(top["result"]["rows"].as_array().unwrap().len(), 1);
    assert_eq!(top["result"]["rows"][0]["label"], "Nina Simone");
    assert_eq!(top["result"]["rows"][0]["plays"], 2);
}

#[tokio::test]
async fn ask_query_returns_grouped_rows_over_http() {
    let (_dir, state, token) = setup().await;
    save(&state.storage, &fixture()).await;

    let body = run(&state, &token, grouped("plays", "month")).await;
    assert_eq!(body["query"]["group_by"], "month");
    assert_eq!(body["query"]["sort_by"], "date");
    assert_eq!(body["query"]["order"], "asc");
    assert!(body["result"]["scalar"].is_null());
    assert_eq!(body["result"]["rows"].as_array().unwrap().len(), 4);
    assert_eq!(body["result"]["total"], 4);
    assert_eq!(body["result"]["rows"][0]["label"], "2024-01");
    assert_eq!(body["result"]["rows"][0]["plays"], 4);
    assert!(
        body["result"]["rows"][0]["time"]["known_ms"]
            .as_i64()
            .unwrap()
            > 0
    );
}
