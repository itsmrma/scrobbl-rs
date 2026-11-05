use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Europe::Rome;
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
    state.ask = Ask::stub();
    (dir, state, token)
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

fn rome_date(ms: i64) -> NaiveDate {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap()
        .with_timezone(&Rome)
        .date_naive()
}

/// Noon in the assigned calendar keeps the event on that local day across DST.
fn local_noon(date: NaiveDate) -> i64 {
    let naive = date.and_hms_opt(12, 0, 0).unwrap();
    Rome.from_local_datetime(&naive)
        .single()
        .unwrap()
        .timestamp_millis()
}

fn daily_events(dates: &[NaiveDate]) -> Vec<Event> {
    dates
        .iter()
        .enumerate()
        .map(|(index, date)| {
            event(
                index + 1,
                local_noon(*date),
                Some(200_000),
                DurationKind::Actual,
                "bjork",
                "Björk",
                "homogenic",
                "Homogenic",
                "Jóga",
            )
        })
        .collect()
}

fn milestone_counts(body: &Value) -> Vec<i64> {
    body["result"]["milestones"]
        .as_array()
        .unwrap()
        .iter()
        .map(|milestone| milestone["count"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn current_streak_ending_today_counts() {
    let (_dir, state, token) = setup(true).await;
    let today = rome_date(Utc::now().timestamp_millis());
    let start = today - Duration::days(2);
    save(
        &state.storage,
        &daily_events(&[start, today - Duration::days(1), today]),
    )
    .await;

    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": query("streak")}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["metric"], "streak");
    assert_eq!(body["result"]["streak"]["current"]["days"], 3);
    assert_eq!(
        body["result"]["streak"]["current"]["start"],
        start.to_string()
    );
    assert_eq!(
        body["result"]["streak"]["current"]["end"],
        today.to_string()
    );
    assert_eq!(body["result"]["streak"]["longest"]["days"], 3);
    assert_eq!(body["result"]["scalar"], 3);
    assert_eq!(body["result"]["total"], 3);
    assert!(
        body["result"]["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["href"].as_str().unwrap().starts_with("/history?from="))
    );
}

#[tokio::test]
async fn current_streak_ending_yesterday_counts() {
    let (_dir, state, token) = setup(true).await;
    let today = rome_date(Utc::now().timestamp_millis());
    let yesterday = today - Duration::days(1);
    save(
        &state.storage,
        &daily_events(&[today - Duration::days(2), yesterday]),
    )
    .await;

    let (_, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": query("streak")}),
    )
    .await;
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["streak"]["current"]["days"], 2);
    assert_eq!(
        body["result"]["streak"]["current"]["end"],
        yesterday.to_string()
    );
}

#[tokio::test]
async fn current_streak_ending_before_yesterday_is_zero() {
    let (_dir, state, token) = setup(true).await;
    let today = rome_date(Utc::now().timestamp_millis());
    let start = today - Duration::days(3);
    let end = today - Duration::days(2);
    save(&state.storage, &daily_events(&[start, end])).await;

    let (_, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": query("streak")}),
    )
    .await;
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["streak"]["current"]["days"], 0);
    assert!(body["result"]["streak"]["current"]["start"].is_null());
    assert!(body["result"]["streak"]["current"]["end"].is_null());
    assert_eq!(body["result"]["streak"]["longest"]["days"], 2);
    assert_eq!(
        body["result"]["streak"]["longest"]["start"],
        start.to_string()
    );
    assert_eq!(body["result"]["streak"]["longest"]["end"], end.to_string());
    assert_eq!(body["result"]["scalar"], 0);
    assert_eq!(body["result"]["total"], 0);
}

#[tokio::test]
async fn longest_streak_can_be_anywhere_and_gaps_split_runs() {
    let (_dir, state, token) = setup(true).await;
    let date = |day| NaiveDate::from_ymd_opt(2024, 1, day).unwrap();
    save(
        &state.storage,
        &daily_events(&[
            date(1),
            date(2),
            date(3),
            date(4),
            // Gap, then a shorter run.
            date(10),
            date(11),
            // Another gap, then a single day.
            date(20),
        ]),
    )
    .await;

    let (_, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": query("streak")}),
    )
    .await;
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["streak"]["current"]["days"], 0);
    assert_eq!(body["result"]["streak"]["longest"]["days"], 4);
    assert_eq!(
        body["result"]["streak"]["longest"]["start"],
        date(1).to_string()
    );
    assert_eq!(
        body["result"]["streak"]["longest"]["end"],
        date(4).to_string()
    );
}

#[tokio::test]
async fn empty_history_reports_zero_without_inventing_dates() {
    let (_dir, state, token) = setup(true).await;

    let (_, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": query("streak")}),
    )
    .await;
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["scalar"], 0);
    assert_eq!(body["result"]["total"], 0);
    assert_eq!(body["result"]["streak"]["current"]["days"], 0);
    assert!(body["result"]["streak"]["current"]["start"].is_null());
    assert!(body["result"]["streak"]["current"]["end"].is_null());
    assert_eq!(body["result"]["streak"]["longest"]["days"], 0);
    assert!(body["result"]["streak"]["longest"]["start"].is_null());
    assert!(body["result"]["streak"]["longest"]["end"].is_null());

    let (_, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": query("milestones")}),
    )
    .await;
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["scalar"], 0);
    assert_eq!(body["result"]["total"], 0);
    assert_eq!(body["result"]["milestones"], json!([]));
}

#[tokio::test]
async fn milestones_use_the_existing_thresholds_and_ignore_the_period() {
    let (_dir, state, token) = setup(true).await;
    let base = NaiveDate::from_ymd_opt(2001, 1, 1).unwrap();
    let first = local_noon(base);
    let mut events: Vec<Event> = (0..55)
        .map(|index| {
            event(
                index + 1,
                first + index as i64 * 3_600_000,
                Some(200_000),
                DurationKind::Actual,
                "aphex",
                "Aphex Twin",
                "saw",
                "Selected Ambient Works",
                "Xtal",
            )
        })
        .collect();
    // A later listen by another artist, outside the requested entity scope.
    events.push(event(
        100,
        first + 100 * 3_600_000,
        Some(200_000),
        DurationKind::Actual,
        "radiohead",
        "Radiohead",
        "ok-computer",
        "OK Computer",
        "Karma Police",
    ));
    save(&state.storage, &events).await;

    let mut milestones = query("milestones");
    milestones["entity"] = artist_entity("Aphex Twin");
    let (status, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": milestones}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["scalar"], 55);
    assert_eq!(body["result"]["total"], 55);
    assert_eq!(milestone_counts(&body), [10, 50]);

    let reached = body["result"]["milestones"].as_array().unwrap();
    assert_eq!(reached[0]["reached_at_ms"], first + 9 * 3_600_000);
    assert_eq!(reached[1]["reached_at_ms"], first + 49 * 3_600_000);
    assert!(
        body["result"]["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["href"]
                .as_str()
                .unwrap()
                .starts_with("/music/artists/"))
    );

    // The same thresholds must survive a period, year, month, weekday or hour
    // that does not contain the history: milestones are all-time.
    for (key, value) in [
        ("period", json!("today")),
        ("year", json!(1999)),
        ("month", json!(6)),
        ("weekday", json!("sunday")),
        ("hour", json!(3)),
    ] {
        let mut filtered = query("milestones");
        filtered["entity"] = artist_entity("Aphex Twin");
        filtered[key] = value;
        let (_, body) = post(
            &state,
            &token,
            "/api/v1/ask/query",
            json!({"query": filtered}),
        )
        .await;
        assert_eq!(body["status"], "ok", "{key}: {body}");
        assert_eq!(body["result"]["scalar"], 55, "{key}");
        assert_eq!(milestone_counts(&body), [10, 50], "{key}");
    }
}

#[tokio::test]
async fn milestones_keep_the_source_scope() {
    let (_dir, state, token) = setup(true).await;
    let base = NaiveDate::from_ymd_opt(2001, 2, 1).unwrap();
    let first = local_noon(base);
    let mut events: Vec<Event> = (0..12)
        .map(|index| {
            event(
                index + 1,
                first + index as i64 * 3_600_000,
                Some(200_000),
                DurationKind::Actual,
                "aphex",
                "Aphex Twin",
                "saw",
                "Selected Ambient Works",
                "Xtal",
            )
        })
        .collect();
    for index in 0..5 {
        let mut imported = event(
            50 + index,
            first + (24 + index) as i64 * 3_600_000,
            Some(200_000),
            DurationKind::Actual,
            "aphex",
            "Aphex Twin",
            "saw",
            "Selected Ambient Works",
            "Xtal",
        );
        imported.source = "spotify_import";
        events.push(imported);
    }
    save(&state.storage, &events).await;

    let mut all = query("milestones");
    all["entity"] = artist_entity("Aphex Twin");
    let (_, body) = post(&state, &token, "/api/v1/ask/query", json!({"query": all})).await;
    assert_eq!(body["result"]["scalar"], 17);
    assert_eq!(milestone_counts(&body), [10]);

    let mut api_only = query("milestones");
    api_only["entity"] = artist_entity("Aphex Twin");
    api_only["source"] = json!("spotify_api");
    let (_, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": api_only}),
    )
    .await;
    assert_eq!(body["result"]["scalar"], 12);
    assert_eq!(milestone_counts(&body), [10]);

    let mut imports_only = query("milestones");
    imports_only["entity"] = artist_entity("Aphex Twin");
    imports_only["source"] = json!("spotify_import");
    let (_, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": imports_only}),
    )
    .await;
    assert_eq!(body["result"]["scalar"], 5);
    assert_eq!(milestone_counts(&body), Vec::<i64>::new());
}

#[tokio::test]
async fn assigned_calendar_dates_define_streaks_across_dst() {
    let (_dir, state, token) = setup(true).await;
    save(
        &state.storage,
        &[
            // 00:30 on 31 March in Europe/Rome, before the DST change.
            event(
                1,
                at("2024-03-30T23:30:00Z"),
                Some(200_000),
                DurationKind::Actual,
                "bjork",
                "Björk",
                "homogenic",
                "Homogenic",
                "Jóga",
            ),
            // 00:30 on 1 April in Europe/Rome, after the DST change.
            event(
                2,
                at("2024-03-31T22:30:00Z"),
                Some(200_000),
                DurationKind::Actual,
                "bjork",
                "Björk",
                "homogenic",
                "Homogenic",
                "Jóga",
            ),
            event(
                3,
                at("2024-04-01T10:00:00Z"),
                Some(200_000),
                DurationKind::Actual,
                "bjork",
                "Björk",
                "homogenic",
                "Homogenic",
                "Jóga",
            ),
        ],
    )
    .await;

    let (_, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": query("streak")}),
    )
    .await;
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["streak"]["longest"]["days"], 2);
    assert_eq!(body["result"]["streak"]["longest"]["start"], "2024-03-31");
    assert_eq!(body["result"]["streak"]["longest"]["end"], "2024-04-01");
}

#[tokio::test]
async fn utc_days_that_share_an_assigned_day_do_not_extend_a_streak() {
    let (_dir, state, token) = setup(true).await;
    save(
        &state.storage,
        &[
            // Both of these are 2 May in Europe/Rome.
            event(
                1,
                at("2024-05-01T22:30:00Z"),
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
                at("2024-05-02T10:00:00Z"),
                Some(200_000),
                DurationKind::Actual,
                "bjork",
                "Björk",
                "homogenic",
                "Homogenic",
                "Jóga",
            ),
            event(
                3,
                at("2024-05-03T10:00:00Z"),
                Some(200_000),
                DurationKind::Actual,
                "bjork",
                "Björk",
                "homogenic",
                "Homogenic",
                "Jóga",
            ),
        ],
    )
    .await;

    let (_, body) = post(
        &state,
        &token,
        "/api/v1/ask/query",
        json!({"query": query("streak")}),
    )
    .await;
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["result"]["streak"]["longest"]["days"], 2);
    assert_eq!(body["result"]["streak"]["longest"]["start"], "2024-05-02");
    assert_eq!(body["result"]["streak"]["longest"]["end"], "2024-05-03");
}
