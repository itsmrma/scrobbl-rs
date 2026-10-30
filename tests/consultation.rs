use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use scrobbl_rs::{
    AppState, auth, browse,
    config::Config,
    db::{self, DurationKind, Event},
    spotify::parse_track,
    storage::Storage,
    web::{self, Filters},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use tower::ServiceExt;

fn at(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .unwrap()
        .timestamp_millis()
}
fn event(
    index: usize,
    when: i64,
    ms: Option<i64>,
    kind: DurationKind,
    artists: &[(&str, &str)],
) -> Event {
    let raw = json!({"uri":format!("spotify:track:{index:022}"),"type":"track","name":format!("Track {index:03}"),"duration_ms":240000,"album":{"id":format!("album{index}"),"name":format!("Album {index:03}"),"images":[]},"artists":artists.iter().map(|(id,name)|json!({"id":id,"name":name})).collect::<Vec<_>>()});
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
    (dir, AppState::new(storage, config), token)
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
async fn json_get(state: &AppState, token: &str, path: &str) -> Value {
    let (status, value) = get(state, token, path).await;
    assert_eq!(status, StatusCode::OK, "{path}: {value}");
    value
}

#[tokio::test]
async fn listening_patterns_preserve_context_reimport_and_assigned_dst_without_double_counting() {
    let (_dir, state, token) = setup().await;
    let mut first = event(
        1,
        at("2025-10-26T00:30:00Z"),
        Some(180000),
        DurationKind::Actual,
        &[("a", "Artist")],
    );
    first.source = "spotify_import";
    first.raw["platform"] = json!("Android");
    first.raw["shuffle"] = json!(false);
    first.raw["offline"] = json!(true);
    first.raw["skipped"] = json!(false);
    first.raw["conn_country"] = json!("IT");
    first.raw["reason_end"] = json!("trackdone");
    first.raw["ip_addr"] = json!("192.0.2.99");
    first.raw["username"] = json!("private user");
    let mut second = first.clone();
    second.source_key = "second".into();
    second.played_at_ms = at("2025-10-26T01:30:00Z");
    second.source_timestamp_ms = second.played_at_ms;
    second.raw["platform"] = json!("Windows");
    second.raw["shuffle"] = json!(true);
    let unknown = event(
        3,
        at("2025-10-27T23:30:00Z"),
        None,
        DurationKind::Unknown,
        &[("a", "Artist")],
    );
    let short = event(
        4,
        at("2025-10-26T02:30:00Z"),
        Some(1000),
        DurationKind::Actual,
        &[("a", "Artist")],
    );
    save(
        &state.storage,
        &[first.clone(), second.clone(), unknown, short],
    )
    .await;
    let path = "/api/v1/consultation/patterns?period=all";
    assert_eq!(
        get(&state, "invalid", path).await.0,
        StatusCode::UNAUTHORIZED
    );
    let patterns = json_get(&state, &token, path).await;
    assert_eq!(patterns["total"], 3);
    for bins in patterns["series"].as_object().unwrap().values() {
        assert_eq!(
            bins.as_array()
                .unwrap()
                .iter()
                .map(|b| b["count"].as_i64().unwrap())
                .sum::<i64>(),
            3
        );
    }
    assert_eq!(patterns["series"]["hours"][2]["count"], 2); // Repeated 02:30 across DST fallback.
    assert_eq!(patterns["series"]["hours"][0]["count"], 1);
    assert_eq!(patterns["series"]["weekdays"][6]["count"], 2);
    assert_eq!(patterns["coverage"]["devices"]["known"], 2);
    assert_eq!(patterns["coverage"]["shuffle"]["unknown"], 1);
    let saved: Vec<String> = sqlx::query_scalar("SELECT context_json FROM observation_variants")
        .fetch_all(&state.db)
        .await
        .unwrap();
    assert!(
        saved
            .iter()
            .all(|s| !s.contains("private user") && !s.contains("192.0.2.99"))
    );
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM listens ORDER BY id")
        .fetch_all(&state.db)
        .await
        .unwrap();
    sqlx::query("UPDATE observation_variants SET context_json='{}'")
        .execute(&state.db)
        .await
        .unwrap();
    save(&state.storage, &[first.clone(), second.clone()]).await;
    assert_eq!(
        json_get(&state, &token, path).await["coverage"]["devices"]["known"],
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT id FROM listens ORDER BY id")
            .fetch_all(&state.db)
            .await
            .unwrap(),
        ids
    );
    let dashboard = json_get(&state, &token, "/api/dashboard?period=all").await;
    for (kind, filter) in [
        ("artists", "artist"),
        ("albums", "album"),
        ("tracks", "track"),
    ] {
        let id = dashboard["rankings"][kind][0]["id"].as_str().unwrap();
        let scoped = json_get(&state, &token, &format!("{path}&{filter}={id}")).await;
        assert_eq!(scoped["total"], if kind == "artists" { 3 } else { 2 });
    }
    let mut conflict = first.clone();
    conflict.raw["platform"] = json!("iOS");
    save(&state.storage, &[conflict]).await;
    assert_eq!(
        json_get(&state, &token, path).await["coverage"]["devices"]["conflicting"],
        1
    );
    assert_eq!(
        json_get(&state, &token, &format!("{path}&source=spotify_import")).await["total"],
        2
    );
    assert_eq!(
        json_get(&state, &token, &format!("{path}&from=2026-01-01")).await["total"],
        0
    );
    let frozen = _dir.path().join("patterns-snapshot.db");
    scrobbl_rs::recovery::freeze(&state.storage, &frozen)
        .await
        .unwrap();
    let exported = _dir.path().join("patterns-export.json");
    scrobbl_rs::exports::write(&frozen, &exported, "json", &Default::default())
        .await
        .unwrap();
    let exported = std::fs::read_to_string(exported).unwrap();
    assert!(exported.contains("\"platform\":\"Android\""));
    assert!(!exported.contains("192.0.2.99") && !exported.contains("private user"));
    let mut tx = state.db.begin().await.unwrap();
    let id: String = sqlx::query_scalar("SELECT id FROM listens WHERE played_at_ms=?")
        .bind(first.played_at_ms)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    db::set_trashed(&mut tx, &[id], Some(auth::now_ms()))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(json_get(&state, &token, path).await["total"], 2);
}

#[tokio::test]
async fn rankings_keep_primary_ties_across_pages_secondary_order_precision_and_full_attribution() {
    let (_dir, state, token) = setup().await;
    let mut events = Vec::new();
    let base = at("2025-01-01T12:00:00Z");
    for i in 0..105 {
        for n in 0..if i < 9 {
            3
        } else if i < 11 {
            2
        } else {
            1
        } {
            events.push(event(
                i,
                base + n * 300000,
                Some(60000 + i as i64),
                DurationKind::Actual,
                &[("one", "Shared artist"), ("two", "Other artist")],
            ));
        }
    }
    save(&state.storage, &events).await;
    let first = json_get(
        &state,
        &token,
        "/api/v1/consultation/rankings/tracks?page_size=10",
    )
    .await;
    let second = json_get(
        &state,
        &token,
        "/api/v1/consultation/rankings/tracks?page_size=10&page=2",
    )
    .await;
    assert_eq!(first["total"], 105);
    assert_eq!(first["items"][9]["position"], 10);
    assert_eq!(second["items"][0]["position"], 10);
    assert_eq!(second["items"][1]["position"], 12);
    // Secondary precise milliseconds order rows without breaking the primary tie.
    assert_eq!(first["items"][9]["name"], "Track 010");
    assert_eq!(second["items"][0]["name"], "Track 009");
    let mut identities = BTreeSet::new();
    for size in [10, 25, 50, 100] {
        identities.clear();
        for page in 1..=((105 + size - 1) / size) {
            let data = json_get(
                &state,
                &token,
                &format!("/api/v1/consultation/rankings/tracks?page_size={size}&page={page}"),
            )
            .await;
            for item in data["items"].as_array().unwrap() {
                assert!(identities.insert(item["id"].as_str().unwrap().to_owned()));
            }
        }
        assert_eq!(identities.len(), 105);
    }
    let timed = json_get(
        &state,
        &token,
        "/api/v1/consultation/rankings/tracks?sort=time",
    )
    .await;
    assert_eq!(timed["items"][0]["name"], "Track 008");
    assert_eq!(timed["items"][0]["position"], 1);
    assert_eq!(timed["items"][1]["position"], 2);
    let dashboard = json_get(&state, &token, "/api/dashboard").await;
    let artists = json_get(&state, &token, "/api/v1/consultation/rankings/artists").await;
    for artist in artists["items"].as_array().unwrap() {
        assert_eq!(artist["plays"], dashboard["totals"]["listens"]);
        assert_eq!(artist["actual_ms"], dashboard["totals"]["actual_ms"]);
    }
    assert_eq!(dashboard["rankings"]["tracks"][0], first["items"][0]);
    assert_eq!(
        get(
            &state,
            &token,
            "/api/v1/consultation/rankings/tracks?page_size=11"
        )
        .await
        .0,
        400
    );
    assert_eq!(
        get(&state, &token, "/api/v1/consultation/rankings/nope")
            .await
            .0,
        400
    );
    state.storage.close().await;
}
#[tokio::test]
async fn browsing_combines_filters_hides_short_plays_and_distinguishes_missing_time() {
    let (_dir, state, token) = setup().await;
    let base = at("2025-06-01T22:30:00Z");
    let a = event(
        1,
        base,
        Some(240000),
        DurationKind::Actual,
        &[("one", "Artist One"), ("two", "Artist Two")],
    );
    let mut unknown = event(
        2,
        base + 86400000,
        None,
        DurationKind::Unknown,
        &[("one", "Artist One")],
    );
    unknown.source = "listenbrainz";
    let estimated = event(
        3,
        base + 2 * 86400000,
        Some(10000),
        DurationKind::Estimated,
        &[("one", "Artist One")],
    );
    let short = event(
        4,
        base + 3 * 86400000,
        Some(29000),
        DurationKind::Actual,
        &[("one", "Artist One")],
    );
    let counted = event(
        5,
        base + 3 * 86400000,
        Some(30000),
        DurationKind::Actual,
        &[("one", "Artist One")],
    );
    save(&state.storage, &[a, unknown, estimated, short, counted]).await;
    let history = json_get(&state, &token, "/api/history").await;
    assert_eq!(history["total"], 4);
    assert_eq!(
        json_get(&state, &token, "/api/history?status=all").await["total"],
        5
    );
    assert_eq!(
        json_get(&state, &token, "/api/history?status=short").await["total"],
        1
    );
    let dashboard = json_get(&state, &token, "/api/dashboard").await;
    assert_eq!(dashboard["totals"]["listens"], 4);
    assert_eq!(dashboard["totals"]["actual_ms"], 270000);
    assert_eq!(dashboard["totals"]["estimated_ms"], 10000);
    assert_eq!(dashboard["totals"]["unknown_durations"], 1);
    let listen = &history["items"][3];
    assert_eq!(listen["timezone"], "Europe/Rome");
    assert_eq!(listen["local_date"], "2025-06-02");
    let artist = listen["artists"][0]["id"].as_str().unwrap();
    let album = listen["album_id"].as_str().unwrap();
    let track = listen["track_id"].as_str().unwrap();
    let filtered=json_get(&state,&token,&format!("/api/history?from=2025-06-02&to=2025-06-02&artist={artist}&album={album}&q=Track&source=spotify_api")).await;
    assert_eq!(filtered["total"], 1);
    assert!(
        filtered["items"][0]["first_acquired_at_ms"]
            .as_i64()
            .unwrap()
            > 0
    );
    assert_eq!(
        json_get(
            &state,
            &token,
            &format!("/api/history?artist={artist}&source=listenbrainz")
        )
        .await["total"],
        1
    );
    for (kind, id) in [("artists", artist), ("albums", album), ("tracks", track)] {
        let page = json_get(
            &state,
            &token,
            &format!("/api/v1/consultation/music/{kind}/{id}"),
        )
        .await;
        assert!(page["totals"]["listens"].as_i64().unwrap() > 0);
        if kind != "artists" {
            assert_eq!(page["artists"].as_array().unwrap().len(), 2);
        }
        if kind == "tracks" {
            assert_eq!(page["album_id"], album);
        }
    }
    let charts = json_get(
        &state,
        &token,
        "/api/v1/consultation/activity?period=custom&from=2025-06-01&to=2025-06-07&grain=day",
    )
    .await;
    let buckets = charts["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 7);
    assert_eq!(buckets[0]["listens"], 0);
    assert_eq!(buckets[1]["actual_ms"], 240000);
    assert_eq!(buckets[2]["unknown_durations"], 1);
    assert_eq!(buckets[2]["actual_ms"], 0);
    assert_eq!(
        buckets
            .iter()
            .map(|b| b["listens"].as_i64().unwrap())
            .sum::<i64>(),
        4
    );
    for grain in ["day", "week", "month", "year"] {
        let chart = json_get(
            &state,
            &token,
            &format!("/api/v1/consultation/activity?grain={grain}&artist={artist}"),
        )
        .await;
        let mut seen = BTreeSet::new();
        for bucket in chart["buckets"].as_array().unwrap() {
            let start = bucket["start"].as_str().unwrap();
            let page = json_get(
                &state,
                &token,
                &format!("/api/history?artist={artist}&bucket={grain}&bucket_start={start}"),
            )
            .await;
            assert_eq!(page["total"], bucket["listens"], "{grain} {start}");
            for item in page["items"].as_array().unwrap() {
                assert!(seen.insert(item["id"].as_str().unwrap().to_owned()));
            }
        }
        assert_eq!(seen.len(), 4);
    }
    let empty = json_get(
        &state,
        &token,
        &format!(
            "/api/v1/consultation/rankings/albums?artist={artist}&from=2020-01-01&to=2020-01-01"
        ),
    )
    .await;
    assert_eq!(empty["total"], 0);
    assert_eq!(
        get(&state, &token, "/api/v1/consultation/music/tracks/missing")
            .await
            .0,
        404
    );
    assert_eq!(
        get(&state, "invalid", "/api/v1/consultation/activity")
            .await
            .0,
        401
    );
    state.storage.close().await;
}
#[tokio::test]
async fn assigned_calendars_rolling_dst_current_periods_and_recalculation_stay_coherent() {
    let (_dir, state, token) = setup().await;
    let now = at("2025-03-30T10:00:00Z"); // European DST changes during the rolling window.
    let artists = &[("one", "One")];
    save(
        &state.storage,
        &[
            event(
                1,
                now - 7 * 86400000 - 1,
                Some(60000),
                DurationKind::Actual,
                artists,
            ),
            event(
                2,
                now - 7 * 86400000,
                Some(60000),
                DurationKind::Actual,
                artists,
            ),
            event(
                3,
                at("2025-03-29T23:30:00Z"),
                Some(60000),
                DurationKind::Actual,
                artists,
            ),
            event(4, now + 1, Some(60000), DurationKind::Actual, artists),
        ],
    )
    .await;
    state
        .storage
        .change_calendar("UTC", 6, false)
        .await
        .unwrap();
    save(
        &state.storage,
        &[event(
            5,
            at("2025-03-29T23:30:00Z"),
            Some(60000),
            DurationKind::Actual,
            artists,
        )],
    )
    .await;
    let mut conn = state.storage.read_view().await.unwrap();
    let rolling = Filters {
        period: Some("7d".into()),
        ..Default::default()
    };
    assert_eq!(
        browse::ranking_page(&mut conn.transaction, "tracks", &rolling, now)
            .await
            .unwrap()
            .total,
        3
    );
    let today = Filters {
        period: Some("today".into()),
        ..Default::default()
    };
    assert_eq!(
        browse::ranking_page(&mut conn.transaction, "tracks", &today, now)
            .await
            .unwrap()
            .total,
        1
    ); // Rome listen, not UTC yesterday or a future instant.
    let custom = Filters {
        period: Some("custom".into()),
        from: Some("2025-03-30".into()),
        to: Some("2025-03-30".into()),
        grain: Some("day".into()),
        ..Default::default()
    };
    let chart = browse::activity_chart(&mut conn.transaction, &custom, now)
        .await
        .unwrap();
    assert_eq!(chart.buckets[0].listens, 2); // Whole inclusive date includes the future fixture.
    // The GUI's automatic scale must never combine overlapping assigned weeks.
    for (from, to, expected) in [
        ("2025-03-01", "2025-04-30", "day"),
        ("2020-01-01", "2025-12-31", "month"),
        ("1980-01-01", "2025-12-31", "year"),
    ] {
        let automatic = Filters {
            period: Some("custom".into()),
            from: Some(from.into()),
            to: Some(to.into()),
            ..Default::default()
        };
        let automatic = browse::activity_chart(&mut conn.transaction, &automatic, now)
            .await
            .unwrap();
        assert_eq!(automatic.grain, expected);
        assert!(!automatic.mixed_week_starts);
    }
    let weeks = Filters {
        grain: Some("week".into()),
        ..Default::default()
    };
    let chart = browse::activity_chart(&mut conn.transaction, &weeks, now)
        .await
        .unwrap();
    assert!(chart.mixed_week_starts);
    assert_eq!(chart.buckets.iter().map(|b| b.listens).sum::<i64>(), 5);
    let current = browse::activity_chart(&mut conn.transaction, &today, now)
        .await
        .unwrap();
    assert!(current.partial);
    assert!(current.buckets[0].partial);
    drop(conn);
    // A weekly bar drills into the stored week_date, never a blanket current-calendar UTC range.
    for bucket in chart.buckets {
        let history = json_get(
            &state,
            &token,
            &format!("/api/history?bucket=week&bucket_start={}", bucket.start),
        )
        .await;
        assert_eq!(history["total"], bucket.listens);
    }
    let job = state
        .storage
        .change_calendar("UTC", 0, true)
        .await
        .unwrap()
        .unwrap();
    let lease = state.storage.claim("calendar").await.unwrap().unwrap();
    assert_eq!(job, lease.id);
    while !state.storage.calendar_step(&lease).await.unwrap() {}
    let mut view = state.storage.read_view().await.unwrap();
    let after = browse::activity_chart(&mut view.transaction, &weeks, now)
        .await
        .unwrap();
    assert!(!after.mixed_week_starts);
    assert_eq!(after.buckets.iter().map(|b| b.listens).sum::<i64>(), 5);
    assert_eq!(
        browse::ranking_page(&mut view.transaction, "tracks", &today, now)
            .await
            .unwrap()
            .total,
        0
    );
    drop(view);
    state.storage.close().await;
}
#[tokio::test]
async fn empty_buckets_partial_edges_and_chart_limits_are_explicit() {
    let (_dir, state, token) = setup().await;
    let chart = json_get(
        &state,
        &token,
        "/api/v1/consultation/activity?from=2025-01-02&to=2025-02-02&grain=month",
    )
    .await;
    assert_eq!(chart["buckets"].as_array().unwrap().len(), 2);
    assert_eq!(chart["buckets"][0]["listens"], 0);
    assert_eq!(chart["buckets"][0]["partial"], true);
    assert_eq!(chart["buckets"][1]["partial"], true);
    assert_eq!(
        get(
            &state,
            &token,
            "/api/v1/consultation/activity?from=1900-01-01&to=2025-01-01&grain=day"
        )
        .await
        .0,
        400
    );
    for path in [
        "/api/v1/consultation/activity?grain=nonsense",
        "/api/history?bucket=month&bucket_start=2025-01-02",
        "/api/history?bucket=week",
        "/api/history?from=2025-02-01&to=2025-01-01",
    ] {
        assert_eq!(get(&state, &token, path).await.0, 400, "{path}");
    }
    state.storage.close().await;
}

#[tokio::test]
async fn competition_positions_use_only_the_primary_measure_and_names_break_complete_ties() {
    let (_dir, state, token) = setup().await;
    let base = at("2025-02-01T12:00:00Z");
    let mut rows = Vec::new();
    for (index, plays, ms, name) in [
        (0, 3, 120000, "Lead"),
        (1, 2, 120000, "Beta"),
        (2, 2, 120000, "Alpha"),
        (3, 1, 240000, "Last"),
    ] {
        for n in 0..plays {
            let mut row = event(
                index,
                base + n * 500000,
                Some(ms),
                DurationKind::Actual,
                &[("one", "One")],
            );
            row.track.title = name.into();
            rows.push(row);
        }
    }
    save(&state.storage, &rows).await;
    let plays = json_get(&state, &token, "/api/v1/consultation/rankings/tracks").await;
    assert_eq!(
        plays["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["position"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 2, 4]
    );
    assert_eq!(plays["items"][1]["name"], "Alpha");
    assert_eq!(plays["items"][2]["name"], "Beta");
    let time = json_get(
        &state,
        &token,
        "/api/v1/consultation/rankings/tracks?sort=time",
    )
    .await;
    assert_eq!(
        time["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["position"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 2, 2]
    );
    assert_eq!(time["items"][1]["name"], "Alpha");
    assert_eq!(time["items"][3]["name"], "Last");
    state.storage.close().await;
}

#[tokio::test]
async fn library_search_groups_editions_and_orders_by_counted_listens() {
    let (_dir, state, token) = setup().await;
    let mut rows = Vec::new();
    for (index, title, plays) in [(1, "Normal", 1), (2, "Normal", 3), (3, "Normal (Live)", 2)] {
        for n in 0..plays {
            let mut row = event(
                index,
                at("2025-01-01T12:00:00Z") + n * 300000,
                Some(180000),
                DurationKind::Actual,
                &[("a", "Artist")],
            );
            row.track.title = title.into();
            rows.push(row);
        }
    }
    save(&state.storage, &rows).await;
    let found = json_get(
        &state,
        &token,
        "/api/v1/consultation/search/tracks?q=Normal",
    )
    .await;
    assert_eq!(
        found.as_array().unwrap().len(),
        2,
        "same song across albums must appear once"
    );
    assert_eq!(found[0]["name"], "Normal");
    assert_eq!(found[0]["plays"], 4);
    assert_eq!(found[0]["editions"].as_array().unwrap().len(), 2);
    assert_eq!(found[0]["editions"][0]["plays"], 3);
    assert_eq!(found[0]["id"], found[0]["editions"][0]["id"]);
    assert!(
        found[0]["cover_url"]
            .as_str()
            .unwrap()
            .starts_with("/api/v1/artwork/albums/")
    );
    assert_eq!(found[1]["name"], "Normal (Live)");
    assert_eq!(found[1]["plays"], 2);
    let albums = json_get(&state, &token, "/api/v1/consultation/search/albums?q=Album").await;
    assert_eq!(albums[0]["name"], "Album 002");
    assert_eq!(albums[0]["plays"], 3);
    assert_eq!(albums[1]["plays"], 2);
    let artists = json_get(
        &state,
        &token,
        "/api/v1/consultation/search/artists?q=Artist",
    )
    .await;
    assert_eq!(artists[0]["plays"], 6);
    let mut other = event(
        4,
        at("2025-01-01T12:00:00Z"),
        Some(180000),
        DurationKind::Actual,
        &[("other", "Other artist")],
    );
    other.track.title = "Normal".into();
    save(&state.storage, &[other]).await;
    let found = json_get(
        &state,
        &token,
        "/api/v1/consultation/search/tracks?q=Normal",
    )
    .await;
    assert_eq!(
        found.as_array().unwrap().len(),
        3,
        "same title by a different artist stays separate"
    );
    assert_eq!(found[0]["editions"].as_array().unwrap().len(), 2);
    state.storage.close().await;
}

#[tokio::test]
async fn library_search_is_bounded_and_validates_categories_and_queries() {
    let (_dir, state, token) = setup().await;
    let events: Vec<_> = (0..55)
        .map(|index| {
            event(
                index,
                at("2025-01-01T12:00:00Z"),
                Some(180000),
                DurationKind::Actual,
                &[("a", "Artist")],
            )
        })
        .collect();
    save(&state.storage, &events).await;
    let matches = json_get(&state, &token, "/api/v1/consultation/search/tracks?q=Track").await;
    assert_eq!(matches.as_array().unwrap().len(), 50);
    assert_eq!(matches[0]["name"], "Track 000");
    assert_eq!(matches[49]["name"], "Track 049");
    assert_eq!(
        get(
            &state,
            &token,
            "/api/v1/consultation/search/unknown?q=Track"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(
            &state,
            &token,
            &format!("/api/v1/consultation/search/tracks?q={}", "a".repeat(501))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    state.storage.close().await;
}

#[tokio::test]
async fn library_search_finds_corrected_music_with_short_listens_and_excludes_trash() {
    let (_dir, state, token) = setup().await;
    save(
        &state.storage,
        &[event(
            1,
            at("2025-01-01T12:00:00Z"),
            Some(29000),
            DurationKind::Actual,
            &[("short", "Short % artist")],
        )],
    )
    .await;
    let artist = json_get(&state, &token, "/api/v1/consultation/search/artists?q=%25").await;
    assert_eq!(artist.as_array().unwrap().len(), 1);
    assert_eq!(artist[0]["name"], "Short % artist");
    let album = json_get(&state, &token, "/api/v1/consultation/search/albums?q=Album").await;
    assert_eq!(album[0]["subtitle"], "Short % artist");
    let tracks = json_get(&state, &token, "/api/v1/consultation/search/tracks?q=Track").await;
    assert_eq!(tracks.as_array().unwrap().len(), 1);
    assert_eq!(tracks[0]["subtitle"], "Short % artist · Album 001");
    let track = tracks[0]["id"].as_str().unwrap();
    let music = json_get(
        &state,
        &token,
        &format!("/api/v1/consultation/music/tracks/{track}"),
    )
    .await;
    assert_eq!(music["name"], tracks[0]["name"]);
    assert_eq!(
        get(
            &state,
            "missing",
            "/api/v1/consultation/search/tracks?q=Track"
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("INSERT INTO local_corrections(entity_kind,entity_id,field,value_json) VALUES('track',?,'name','\"Corrected _ title\"')")
        .bind(track).execute(&state.db).await.unwrap();
    assert_eq!(
        json_get(&state, &token, "/api/v1/consultation/search/tracks?q=_").await[0]["name"],
        "Corrected _ title"
    );
    assert!(
        json_get(&state, &token, "/api/v1/consultation/search/tracks?q=Track")
            .await
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        json_get(&state, &token, "/api/v1/consultation/search/tracks?q=%20")
            .await
            .as_array()
            .unwrap()
            .is_empty()
    );
    let id: String = sqlx::query_scalar("SELECT id FROM listens")
        .fetch_one(&state.db)
        .await
        .unwrap();
    let mut tx = state.db.begin().await.unwrap();
    db::set_trashed(&mut tx, &[id], Some(auth::now_ms()))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    for kind in ["artists", "albums", "tracks"] {
        assert!(
            json_get(
                &state,
                &token,
                &format!("/api/v1/consultation/search/{kind}?q=o")
            )
            .await
            .as_array()
            .unwrap()
            .is_empty()
        );
    }
    state.storage.close().await;
}

#[tokio::test]
async fn history_typeahead_can_select_identities_with_only_short_listens_and_excludes_trash() {
    let (_dir, state, token) = setup().await;
    let row = event(
        1,
        at("2025-01-01T12:00:00Z"),
        Some(29000),
        DurationKind::Actual,
        &[("short", "Only short % artist")],
    );
    save(&state.storage, &[row]).await;
    assert_eq!(
        json_get(&state, &token, "/api/v1/consultation/rankings/artists").await["total"],
        0
    );
    let choices = json_get(&state, &token, "/api/v1/consultation/choices/artists?q=%25").await;
    assert_eq!(choices.as_array().unwrap().len(), 1);
    let artist = choices[0]["id"].as_str().unwrap();
    assert_eq!(
        json_get(&state, &token, &format!("/api/history?artist={artist}")).await["total"],
        0
    );
    assert_eq!(
        json_get(
            &state,
            &token,
            &format!("/api/history?artist={artist}&status=all")
        )
        .await["total"],
        1
    );
    let album = json_get(
        &state,
        &token,
        "/api/v1/consultation/choices/albums?q=Album%20001",
    )
    .await;
    assert_eq!(album.as_array().unwrap().len(), 1);
    assert_eq!(
        get(
            &state,
            "missing",
            "/api/v1/consultation/choices/artists?q=a"
        )
        .await
        .0,
        401
    );
    assert_eq!(
        get(&state, &token, "/api/v1/consultation/choices/tracks?q=a")
            .await
            .0,
        400
    );
    let id: String = sqlx::query_scalar("SELECT id FROM listens")
        .fetch_one(&state.read_db)
        .await
        .unwrap();
    let mut tx = state.db.begin().await.unwrap();
    db::set_trashed(&mut tx, &[id], Some(auth::now_ms()))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        json_get(&state, &token, "/api/v1/consultation/choices/artists?q=%25")
            .await
            .as_array()
            .unwrap()
            .len(),
        0
    );
    state.storage.close().await;
}

#[tokio::test]
#[ignore = "requires SCROBBL_BROWSE_BENCH_DB pointing to a read-only archive"]
async fn music_browse_latency() {
    use sqlx::{Connection, Row};
    let path = std::env::var("SCROBBL_BROWSE_BENCH_DB").expect("archive path");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .pragma("query_only", "ON");
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    let row = sqlx::query(
        "SELECT track_id FROM listens GROUP BY track_id ORDER BY COUNT(*) DESC LIMIT 1",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    let track: String = row.get("track_id");
    let album: String = sqlx::query_scalar("SELECT group_id FROM name_group_members WHERE kind='albums' AND entity_id=(SELECT album_id FROM main.tracks WHERE id=?)").bind(&track).fetch_one(&mut conn).await.unwrap();
    let artist: String = sqlx::query_scalar("SELECT group_id FROM name_group_members WHERE kind='artists' AND entity_id=(SELECT artist_id FROM main.track_artists WHERE track_id=? LIMIT 1)").bind(&track).fetch_one(&mut conn).await.unwrap();
    for filters in [
        Filters {
            track: Some(track),
            ..Default::default()
        },
        Filters {
            album: Some(album),
            ..Default::default()
        },
        Filters {
            artist: Some(artist),
            ..Default::default()
        },
    ] {
        let start = std::time::Instant::now();
        let chart = browse::activity_chart(&mut conn, &filters, auth::now_ms())
            .await
            .unwrap();
        eprintln!(
            "activity: {:?}, {} buckets",
            start.elapsed(),
            chart.buckets.len()
        );
        // This opt-in benchmark reports host-dependent timings; the deterministic
        // provider-queue and delayed-navigation regressions enforce responsiveness.
        assert!(!chart.buckets.is_empty());
    }
}

async fn post(
    state: &AppState,
    token: Option<&str>,
    csrf: Option<&str>,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", "127.0.0.1:3210")
        .header("origin", "http://127.0.0.1:3210")
        .header("content-type", "application/json");
    if let Some(t) = token {
        builder = builder.header("cookie", format!("scrobbl_session={t}"));
    }
    if let Some(c) = csrf {
        builder = builder.header("x-csrf-token", c);
    }
    let response = web::router(state.clone())
        .oneshot(
            builder
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
async fn dashboard_layout_persistence_schema_validation_and_concurrency() {
    let (_dir, state, token) = setup().await;

    // 1. Unauthenticated request rejected
    let (status, _) = get(&state, "invalid-token", "/api/v1/dashboard/layout").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 2. Unconfigured layout returns deterministic V1 default layout
    let (status, default_layout) = get(&state, &token, "/api/v1/dashboard/layout").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(default_layout["version"], 0);
    let widgets = default_layout["widgets"].as_array().expect("widgets array");
    assert_eq!(widgets.len(), 8);
    assert_eq!(widgets[0]["type"], "now_playing");
    assert_eq!(widgets[0]["colSpan"], 6);
    assert_eq!(widgets[1]["type"], "totals");
    assert_eq!(widgets[1]["colSpan"], 6);
    assert_eq!(widgets[2]["type"], "top_artists");
    assert_eq!(widgets[2]["colSpan"], 2);
    assert_eq!(widgets[3]["type"], "top_albums");
    assert_eq!(widgets[3]["colSpan"], 2);
    assert_eq!(widgets[4]["type"], "top_tracks");
    assert_eq!(widgets[4]["colSpan"], 2);
    assert_eq!(widgets[5]["type"], "activity_chart");
    assert_eq!(widgets[5]["colSpan"], 6);
    assert_eq!(widgets[6]["type"], "listening_patterns");
    assert_eq!(widgets[6]["colSpan"], 3);
    assert_eq!(widgets[7]["type"], "recent_listens");
    assert_eq!(widgets[7]["colSpan"], 3);

    // 3. POST without CSRF is rejected
    let (status, _) = post(
        &state,
        Some(&token),
        None,
        "/api/v1/dashboard/layout",
        default_layout.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 4. Invalid column span (e.g. 5) is rejected
    let mut invalid_span = default_layout.clone();
    invalid_span["widgets"][0]["colSpan"] = json!(5);
    let (status, _) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout",
        invalid_span,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 5. Unknown widget type is rejected
    let mut invalid_type = default_layout.clone();
    invalid_type["widgets"][0]["type"] = json!("non_existent_widget");
    let (status, _) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout",
        invalid_type,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 5b. Invalid period override is rejected
    let mut invalid_period = default_layout.clone();
    invalid_period["widgets"][0]["periodOverride"] = json!("invalid_period_foo");
    let (status, _) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout",
        invalid_period,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 6. Valid modified layout with period override persists successfully
    let mut modified = default_layout.clone();
    modified["widgets"][0]["colSpan"] = json!(3);
    modified["widgets"][0]["periodOverride"] = json!("30d");
    modified["widgets"][1]["colSpan"] = json!(3);
    let (status, saved) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout",
        modified.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["widgets"][0]["colSpan"], 3);
    assert_eq!(saved["widgets"][0]["periodOverride"], "30d");

    // 7. GET returns identical persisted layout
    let (status, read_back) = get(&state, &token, "/api/v1/dashboard/layout").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_back["version"], 1);
    assert_eq!(read_back["widgets"][0]["colSpan"], 3);
    assert_eq!(read_back["widgets"][0]["periodOverride"], "30d");
    assert_eq!(read_back["widgets"][1]["colSpan"], 3);

    // 8. Direct database check: settings table stores JSON under 'dashboard_layout'
    let stored_json: String =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='dashboard_layout'")
            .fetch_one(&state.read_db)
            .await
            .unwrap();
    let parsed: Value = serde_json::from_str(&stored_json).unwrap();
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["widgets"][0]["colSpan"], 3);
    assert_eq!(parsed["widgets"][0]["periodOverride"], "30d");

    // 9. Stale version update rejected with 409 Conflict
    let mut stale = default_layout.clone();
    stale["version"] = json!(0); // current is 1
    let (status, _) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout",
        stale,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn dashboard_parametric_widget_filtering_calculates_only_requested_components() {
    let (_dir, state, token) = setup().await;
    let event1 = event(
        1,
        at("2025-06-01T12:00:00Z"),
        Some(180000),
        DurationKind::Actual,
        &[("ar1", "Artist One")],
    );
    let event2 = event(
        2,
        at("2025-06-01T13:00:00Z"),
        Some(200000),
        DurationKind::Actual,
        &[("ar2", "Artist Two")],
    );
    save(&state.storage, &[event1, event2]).await;

    // 1. GET /api/dashboard?widgets=totals
    let totals_only = json_get(&state, &token, "/api/dashboard?widgets=totals").await;
    assert_eq!(totals_only["totals"]["listens"], 2);
    assert_eq!(totals_only["totals"]["actual_ms"], 380000);
    assert!(totals_only.get("rankings").is_none() || totals_only["rankings"].is_null());
    assert!(totals_only.get("recent").is_none() || totals_only["recent"].is_null());

    // 2. GET /api/dashboard?widgets=top_artists
    let artists_only = json_get(&state, &token, "/api/dashboard?widgets=top_artists").await;
    assert!(artists_only.get("totals").is_none() || artists_only["totals"].is_null());
    assert!(artists_only.get("recent").is_none() || artists_only["recent"].is_null());
    let rankings = &artists_only["rankings"];
    assert!(!rankings.is_null());
    let artists = rankings["artists"].as_array().expect("artists array");
    assert_eq!(artists.len(), 2);
    assert_eq!(artists[0]["name"], "Artist Two");
    assert_eq!(artists[1]["name"], "Artist One");
    assert!(rankings["albums"].as_array().is_none_or(|a| a.is_empty()));
    assert!(rankings["tracks"].as_array().is_none_or(|a| a.is_empty()));

    // 3. GET /api/dashboard (full / default)
    let full = json_get(&state, &token, "/api/dashboard").await;
    assert_eq!(full["totals"]["listens"], 2);
    assert_eq!(full["rankings"]["artists"].as_array().unwrap().len(), 2);
    assert_eq!(full["rankings"]["albums"].as_array().unwrap().len(), 2);
    assert_eq!(full["rankings"]["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(full["recent"].as_array().unwrap().len(), 2);

    state.storage.close().await;
}

#[tokio::test]
async fn discovery_ratio_over_period_and_empty_state() {
    let (_dir, state, token) = setup().await;

    // Period 1: January 2025
    // Track 1 listened on 2025-01-10 (historical)
    // Track 2 listened on 2025-01-15 (historical)
    let historical_1 = event(
        1,
        at("2025-01-10T10:00:00Z"),
        Some(180000),
        DurationKind::Actual,
        &[("art1", "Artist 1")],
    );
    let historical_2 = event(
        2,
        at("2025-01-15T12:00:00Z"),
        Some(200000),
        DurationKind::Actual,
        &[("art2", "Artist 2")],
    );

    // Period 2: February 2025 (target period: 2025-02-01 to 2025-02-28)
    // Track 1 played again -> Repeat
    let repeat_1 = event(
        1,
        at("2025-02-10T14:00:00Z"),
        Some(180000),
        DurationKind::Actual,
        &[("art1", "Artist 1")],
    );
    // Track 3 played for the first time -> Discovery
    let discovery_3 = event(
        3,
        at("2025-02-12T09:00:00Z"),
        Some(210000),
        DurationKind::Actual,
        &[("art3", "Artist 3")],
    );
    // Track 4 played twice for the first time -> Discovery (counted once as a discovered track)
    let discovery_4a = event(
        4,
        at("2025-02-14T15:00:00Z"),
        Some(190000),
        DurationKind::Actual,
        &[("art4", "Artist 4")],
    );
    let discovery_4b = event(
        4,
        at("2025-02-15T16:00:00Z"),
        Some(190000),
        DurationKind::Actual,
        &[("art4", "Artist 4")],
    );
    // Track 5 played with short listen (<30s) -> uncounted, should be ignored
    let uncounted_5 = event(
        5,
        at("2025-02-16T18:00:00Z"),
        Some(10000),
        DurationKind::Actual,
        &[("art5", "Artist 5")],
    );

    save(
        &state.storage,
        &[
            historical_1,
            historical_2,
            repeat_1,
            discovery_3,
            discovery_4a,
            discovery_4b,
            uncounted_5,
        ],
    )
    .await;

    // Query February 2025
    let (status, body) = get(
        &state,
        &token,
        "/api/v1/consultation/discovery-ratio?from=2025-02-01&to=2025-02-28",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["discoveries"], 2); // Track 3, Track 4
    assert_eq!(body["repeats"], 1); // Track 1
    assert_eq!(body["total"], 3);
    let percent = body["discovery_percent"].as_f64().unwrap();
    assert!((percent - (2.0 / 3.0 * 100.0)).abs() < 1e-6);

    // Query an empty period (March 2025)
    let (status, empty_body) = get(
        &state,
        &token,
        "/api/v1/consultation/discovery-ratio?from=2025-03-01&to=2025-03-31",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty_body["discoveries"], 0);
    assert_eq!(empty_body["repeats"], 0);
    assert_eq!(empty_body["total"], 0);
    assert_eq!(empty_body["discovery_percent"], 0.0);

    // Layout validation accepts discovery_ratio widget
    let layout_with_discovery = json!({
        "version": 0,
        "widgets": [
            {
                "id": "dr-1",
                "type": "discovery_ratio",
                "colSpan": 3
            }
        ]
    });
    let (status, saved_layout) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout",
        layout_with_discovery,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved_layout["widgets"][0]["type"], "discovery_ratio");
}

#[tokio::test]
async fn music_page_template_layout_persistence_matrix_validation_and_concurrency() {
    let (_dir, state, token) = setup().await;

    // 1. Unconfigured template layouts return deterministic defaults
    let (status, artist_default) =
        get(&state, &token, "/api/v1/dashboard/layout?template=artist").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(artist_default["version"], 0);
    let artist_widgets = artist_default["widgets"].as_array().expect("widgets array");
    assert_eq!(artist_widgets.len(), 6);
    assert_eq!(artist_widgets[0]["type"], "totals");
    assert_eq!(artist_widgets[0]["colSpan"], 6);
    assert_eq!(artist_widgets[1]["type"], "activity_chart");
    assert_eq!(artist_widgets[1]["colSpan"], 6);
    assert_eq!(artist_widgets[2]["type"], "listening_patterns");
    assert_eq!(artist_widgets[2]["colSpan"], 6);
    assert_eq!(artist_widgets[3]["type"], "top_albums");
    assert_eq!(artist_widgets[3]["colSpan"], 3);
    assert_eq!(artist_widgets[4]["type"], "top_tracks");
    assert_eq!(artist_widgets[4]["colSpan"], 3);
    assert_eq!(artist_widgets[5]["type"], "history");
    assert_eq!(artist_widgets[5]["colSpan"], 6);

    let (status, album_default) =
        get(&state, &token, "/api/v1/dashboard/layout?template=album").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(album_default["version"], 0);
    let album_widgets = album_default["widgets"].as_array().expect("widgets array");
    assert_eq!(album_widgets.len(), 5);
    assert_eq!(album_widgets[0]["type"], "totals");
    assert_eq!(album_widgets[0]["colSpan"], 6);
    assert_eq!(album_widgets[1]["type"], "activity_chart");
    assert_eq!(album_widgets[1]["colSpan"], 6);
    assert_eq!(album_widgets[2]["type"], "listening_patterns");
    assert_eq!(album_widgets[2]["colSpan"], 6);
    assert_eq!(album_widgets[3]["type"], "top_tracks");
    assert_eq!(album_widgets[3]["colSpan"], 6);
    assert_eq!(album_widgets[4]["type"], "history");
    assert_eq!(album_widgets[4]["colSpan"], 6);

    let (status, track_default) =
        get(&state, &token, "/api/v1/dashboard/layout?template=track").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(track_default["version"], 0);
    let track_widgets = track_default["widgets"].as_array().expect("widgets array");
    assert_eq!(track_widgets.len(), 4);
    assert_eq!(track_widgets[0]["type"], "totals");
    assert_eq!(track_widgets[0]["colSpan"], 6);
    assert_eq!(track_widgets[1]["type"], "activity_chart");
    assert_eq!(track_widgets[1]["colSpan"], 6);
    assert_eq!(track_widgets[2]["type"], "listening_patterns");
    assert_eq!(track_widgets[2]["colSpan"], 6);
    assert_eq!(track_widgets[3]["type"], "history");
    assert_eq!(track_widgets[3]["colSpan"], 6);

    // 2. Unknown template rejected with 400 Bad Request
    let (status, _) = get(&state, &token, "/api/v1/dashboard/layout?template=invalid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=invalid",
        artist_default.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 3. Matrix validation: reject home-only widgets on artist template
    for forbidden_type in &["now_playing", "top_artists", "unified_rankings"] {
        let mut invalid_artist = artist_default.clone();
        invalid_artist["widgets"][0]["type"] = json!(forbidden_type);
        let (status, _) = post(
            &state,
            Some(&token),
            Some("test-csrf"),
            "/api/v1/dashboard/layout?template=artist",
            invalid_artist,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "Expected {forbidden_type} to be rejected on artist template"
        );
    }

    // Matrix validation: reject music-only widget (history) on home template
    let (status, home_default) = get(&state, &token, "/api/v1/dashboard/layout").await;
    assert_eq!(status, StatusCode::OK);
    let mut invalid_home = home_default.clone();
    invalid_home["widgets"][0]["type"] = json!("history");
    let (status, _) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout",
        invalid_home,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Expected history widget to be rejected on home template"
    );

    // 4. Modify and persist artist template layout
    let mut modified_artist = artist_default.clone();
    modified_artist["widgets"] = json!([
        {
            "id": "totals-custom",
            "type": "totals",
            "colSpan": 6
        },
        {
            "id": "history-custom",
            "type": "history",
            "colSpan": 6
        }
    ]);
    let (status, saved_artist) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=artist",
        modified_artist.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved_artist["version"], 1);
    assert_eq!(saved_artist["widgets"].as_array().unwrap().len(), 2);

    // 5. Verify direct database storage and isolation:
    // settings has 'dashboard_layout_artist', while 'dashboard_layout' and others are untouched.
    let stored_artist_json: String =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='dashboard_layout_artist'")
            .fetch_one(&state.read_db)
            .await
            .unwrap();
    let parsed_artist: Value = serde_json::from_str(&stored_artist_json).unwrap();
    assert_eq!(parsed_artist["version"], 1);

    let stored_home: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='dashboard_layout'")
            .fetch_optional(&state.read_db)
            .await
            .unwrap();
    assert!(stored_home.is_none());

    let (status, read_home) = get(&state, &token, "/api/v1/dashboard/layout").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_home["version"], 0);

    let (status, read_album) = get(&state, &token, "/api/v1/dashboard/layout?template=album").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_album["version"], 0);

    // 6. Optimistic concurrency conflict on artist template
    let mut stale_artist = artist_default.clone();
    stale_artist["version"] = json!(0);
    let (status, _) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=artist",
        stale_artist,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // 7. Empty music layout is permitted and persisted (does NOT fall back to default)
    let empty_artist = json!({
        "version": 1,
        "widgets": []
    });
    let (status, saved_empty) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=artist",
        empty_artist,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved_empty["version"], 2);
    assert_eq!(saved_empty["widgets"].as_array().unwrap().len(), 0);

    let (status, read_empty) =
        get(&state, &token, "/api/v1/dashboard/layout?template=artist").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_empty["version"], 2);
    assert_eq!(read_empty["widgets"].as_array().unwrap().len(), 0);

    // Empty home layout is rejected
    let empty_home = json!({
        "version": 0,
        "widgets": []
    });
    let (status, _) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=home",
        empty_home,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn album_template_layout_persistence_matrix_validation_and_concurrency() {
    let (_dir, state, token) = setup().await;

    // 1. Unconfigured album template layout returns deterministic default
    let (status, album_default) =
        get(&state, &token, "/api/v1/dashboard/layout?template=album").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(album_default["version"], 0);
    let album_widgets = album_default["widgets"].as_array().expect("widgets array");
    assert_eq!(album_widgets.len(), 5);
    assert_eq!(album_widgets[0]["type"], "totals");
    assert_eq!(album_widgets[0]["colSpan"], 6);
    assert_eq!(album_widgets[1]["type"], "activity_chart");
    assert_eq!(album_widgets[1]["colSpan"], 6);
    assert_eq!(album_widgets[2]["type"], "listening_patterns");
    assert_eq!(album_widgets[2]["colSpan"], 6);
    assert_eq!(album_widgets[3]["type"], "top_tracks");
    assert_eq!(album_widgets[3]["colSpan"], 6);
    assert_eq!(album_widgets[4]["type"], "history");
    assert_eq!(album_widgets[4]["colSpan"], 6);

    // 2. Matrix validation: reject forbidden widgets on album template
    // forbidden: top_albums, top_artists, now_playing, unified_rankings
    for forbidden_type in &[
        "top_albums",
        "top_artists",
        "now_playing",
        "unified_rankings",
    ] {
        let mut invalid_album = album_default.clone();
        invalid_album["widgets"][0]["type"] = json!(forbidden_type);
        let (status, _) = post(
            &state,
            Some(&token),
            Some("test-csrf"),
            "/api/v1/dashboard/layout?template=album",
            invalid_album,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "Expected {forbidden_type} to be rejected on album template"
        );
    }

    // 3. Matrix validation: allow valid widgets on album template
    // allowed: totals, activity_chart, listening_patterns, recent_listens, history, top_tracks
    let valid_album = json!({
        "version": 0,
        "widgets": [
            { "id": "w1", "type": "totals", "colSpan": 6 },
            { "id": "w2", "type": "activity_chart", "colSpan": 6 },
            { "id": "w3", "type": "listening_patterns", "colSpan": 6 },
            { "id": "w4", "type": "top_tracks", "colSpan": 6 },
            { "id": "w5", "type": "recent_listens", "colSpan": 3 },
            { "id": "w6", "type": "history", "colSpan": 6 }
        ]
    });
    let (status, saved_valid) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=album",
        valid_album,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved_valid["version"], 1);
    assert_eq!(saved_valid["widgets"].as_array().unwrap().len(), 6);

    // 4. Persistence isolation: settings table has 'dashboard_layout_album',
    // while 'dashboard_layout', 'dashboard_layout_artist', 'dashboard_layout_track' remain absent.
    let stored_album_json: String =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='dashboard_layout_album'")
            .fetch_one(&state.read_db)
            .await
            .unwrap();
    let parsed_album: Value = serde_json::from_str(&stored_album_json).unwrap();
    assert_eq!(parsed_album["version"], 1);

    let stored_artist: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='dashboard_layout_artist'")
            .fetch_optional(&state.read_db)
            .await
            .unwrap();
    assert!(stored_artist.is_none());

    let (status, read_home) = get(&state, &token, "/api/v1/dashboard/layout").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_home["version"], 0);

    let (status, read_artist) =
        get(&state, &token, "/api/v1/dashboard/layout?template=artist").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_artist["version"], 0);

    // 5. Optimistic concurrency conflict on album template
    let mut stale_album = album_default.clone();
    stale_album["version"] = json!(0);
    let (status, _) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=album",
        stale_album,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // 6. Empty album layout is permitted and persisted (does NOT fall back to default)
    let empty_album = json!({
        "version": 1,
        "widgets": []
    });
    let (status, saved_empty) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=album",
        empty_album,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved_empty["version"], 2);
    assert_eq!(saved_empty["widgets"].as_array().unwrap().len(), 0);

    let (status, read_empty) = get(&state, &token, "/api/v1/dashboard/layout?template=album").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_empty["version"], 2);
    assert_eq!(read_empty["widgets"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn track_template_layout_persistence_matrix_validation_and_concurrency() {
    let (_dir, state, token) = setup().await;

    // 1. GET default track template layout
    let (status, track_default) =
        get(&state, &token, "/api/v1/dashboard/layout?template=track").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(track_default["version"], 0);
    let widgets = track_default["widgets"].as_array().expect("widgets array");
    assert_eq!(widgets.len(), 4);
    assert_eq!(widgets[0]["type"], "totals");
    assert_eq!(widgets[0]["colSpan"], 6);
    assert_eq!(widgets[1]["type"], "activity_chart");
    assert_eq!(widgets[1]["colSpan"], 6);
    assert_eq!(widgets[2]["type"], "listening_patterns");
    assert_eq!(widgets[2]["colSpan"], 6);
    assert_eq!(widgets[3]["type"], "history");
    assert_eq!(widgets[3]["colSpan"], 6);

    // 2. Matrix validation: reject forbidden widgets on track template
    for forbidden_type in &[
        "top_albums",
        "top_tracks",
        "top_artists",
        "now_playing",
        "unified_rankings",
        "discovery_ratio",
    ] {
        let mut invalid_track = track_default.clone();
        invalid_track["widgets"][0]["type"] = json!(forbidden_type);
        let (status, _) = post(
            &state,
            Some(&token),
            Some("test-csrf"),
            "/api/v1/dashboard/layout?template=track",
            invalid_track,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "Expected '{forbidden_type}' to be rejected on track template"
        );
    }

    // 3. Matrix validation: allow all compatible widgets on track template
    let valid_track_widgets = json!([
        { "id": "totals-1", "type": "totals", "colSpan": 6 },
        { "id": "activity-1", "type": "activity_chart", "colSpan": 6 },
        { "id": "patterns-1", "type": "listening_patterns", "colSpan": 6 },
        { "id": "recent-1", "type": "recent_listens", "colSpan": 6 },
        { "id": "history-1", "type": "history", "colSpan": 6 }
    ]);
    let mut modified_track = track_default.clone();
    modified_track["widgets"] = valid_track_widgets;

    let (status, saved_track) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=track",
        modified_track,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved_track["version"], 1);
    assert_eq!(saved_track["widgets"].as_array().unwrap().len(), 5);

    // 4. Persistence and isolation check in SQLite
    let stored_track_json: String =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='dashboard_layout_track'")
            .fetch_one(&state.read_db)
            .await
            .unwrap();
    let parsed_track: Value = serde_json::from_str(&stored_track_json).unwrap();
    assert_eq!(parsed_track["version"], 1);
    assert_eq!(parsed_track["widgets"].as_array().unwrap().len(), 5);

    // Isolation: home, artist, album keys are not affected
    let stored_home: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='dashboard_layout'")
            .fetch_optional(&state.read_db)
            .await
            .unwrap();
    assert!(stored_home.is_none());

    let stored_artist: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='dashboard_layout_artist'")
            .fetch_optional(&state.read_db)
            .await
            .unwrap();
    assert!(stored_artist.is_none());

    // 5. Stale version rejected with 409 Conflict
    let mut stale_track = track_default.clone();
    stale_track["version"] = json!(0);
    let (status, _) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=track",
        stale_track,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // 6. Empty track layout is permitted and persisted without fallback to default
    let empty_track = json!({
        "version": 1,
        "widgets": []
    });
    let (status, saved_empty) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=track",
        empty_track,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved_empty["version"], 2);
    assert_eq!(saved_empty["widgets"].as_array().unwrap().len(), 0);

    let (status, read_empty) = get(&state, &token, "/api/v1/dashboard/layout?template=track").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_empty["version"], 2);
    assert_eq!(read_empty["widgets"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn album_scoped_consultation_aggregations_and_cache_isolation() {
    let (_dir, state, token) = setup().await;

    let e1 = Event {
        source: "spotify_api",
        source_key: "t1:1".into(),
        source_timestamp_ms: at("2025-06-01T10:00:00Z"),
        played_at_ms: at("2025-06-01T10:00:00Z"),
        listened_ms: Some(200000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000001",
            "type": "track",
            "name": "Track One",
            "duration_ms": 200000,
            "album": { "id": "album-a", "name": "Album Alpha", "images": [] },
            "artists": [{ "id": "art-1", "name": "Artist 1" }]
        }))
        .unwrap(),
        raw: json!({}),
    };

    let e2 = Event {
        source: "spotify_api",
        source_key: "t1:2".into(),
        source_timestamp_ms: at("2025-06-01T12:00:00Z"),
        played_at_ms: at("2025-06-01T12:00:00Z"),
        listened_ms: Some(200000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000001",
            "type": "track",
            "name": "Track One",
            "duration_ms": 200000,
            "album": { "id": "album-a", "name": "Album Alpha", "images": [] },
            "artists": [{ "id": "art-1", "name": "Artist 1" }]
        }))
        .unwrap(),
        raw: json!({}),
    };

    let e3 = Event {
        source: "spotify_api",
        source_key: "t2:1".into(),
        source_timestamp_ms: at("2025-06-01T14:00:00Z"),
        played_at_ms: at("2025-06-01T14:00:00Z"),
        listened_ms: Some(180000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000002",
            "type": "track",
            "name": "Track Two",
            "duration_ms": 180000,
            "album": { "id": "album-a", "name": "Album Alpha", "images": [] },
            "artists": [{ "id": "art-1", "name": "Artist 1" }]
        }))
        .unwrap(),
        raw: json!({}),
    };

    let e4 = Event {
        source: "spotify_api",
        source_key: "t3:1".into(),
        source_timestamp_ms: at("2025-06-01T16:00:00Z"),
        played_at_ms: at("2025-06-01T16:00:00Z"),
        listened_ms: Some(150000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000003",
            "type": "track",
            "name": "Track Three",
            "duration_ms": 150000,
            "album": { "id": "album-b", "name": "Album Beta", "images": [] },
            "artists": [{ "id": "art-2", "name": "Artist 2" }]
        }))
        .unwrap(),
        raw: json!({}),
    };

    save(&state.storage, &[e1, e2, e3, e4]).await;

    let album_a_id: String = sqlx::query_scalar("SELECT id FROM albums WHERE name='Album Alpha'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();
    let album_b_id: String = sqlx::query_scalar("SELECT id FROM albums WHERE name='Album Beta'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();

    // Query Album A
    let (status, album_a) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/music/albums/{album_a_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(album_a["totals"]["listens"], 3);
    assert_eq!(album_a["totals"]["tracks"], 2);
    assert_eq!(album_a["totals"]["actual_ms"], 580000);

    // Rankings for tracks in Album A
    let (status, tracks_a) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/rankings/tracks?album={album_a_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items_a = tracks_a["items"].as_array().expect("items array");
    assert_eq!(items_a.len(), 2);
    assert_eq!(items_a[0]["name"], "Track One");
    assert_eq!(items_a[0]["plays"], 2);
    assert_eq!(items_a[1]["name"], "Track Two");
    assert_eq!(items_a[1]["plays"], 1);

    // Query Album B
    let (status, album_b) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/music/albums/{album_b_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(album_b["totals"]["listens"], 1);
    assert_eq!(album_b["totals"]["tracks"], 1);
    assert_eq!(album_b["totals"]["actual_ms"], 150000);

    // Rankings for tracks in Album B
    let (status, tracks_b) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/rankings/tracks?album={album_b_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items_b = tracks_b["items"].as_array().expect("items array");
    assert_eq!(items_b.len(), 1);
    assert_eq!(items_b[0]["name"], "Track Three");
    assert_eq!(items_b[0]["plays"], 1);

    // Cache isolation: Query Album A again and ensure results are identical
    let (status, album_a_cached) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/music/albums/{album_a_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(album_a_cached["totals"]["listens"], 3);
    assert_eq!(album_a_cached["totals"]["tracks"], 2);
    assert_eq!(album_a_cached["totals"]["actual_ms"], 580000);
}

#[tokio::test]
async fn calendar_milestones_and_discovery_ratio_layout_matrix_validation() {
    let (_dir, state, token) = setup().await;

    // 1. Home template rejects listening_calendar and listening_milestones
    let (status, home_layout) = get(&state, &token, "/api/v1/dashboard/layout").await;
    assert_eq!(status, StatusCode::OK);

    for widget_type in &["listening_calendar", "listening_milestones"] {
        let mut invalid_home = home_layout.clone();
        invalid_home["widgets"][0]["type"] = json!(widget_type);
        let (status, _) = post(
            &state,
            Some(&token),
            Some("test-csrf"),
            "/api/v1/dashboard/layout",
            invalid_home,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "Expected {widget_type} to be rejected on home template"
        );
    }

    // 2. Artist template allows listening_calendar, listening_milestones, discovery_ratio
    let (status, artist_layout) =
        get(&state, &token, "/api/v1/dashboard/layout?template=artist").await;
    assert_eq!(status, StatusCode::OK);

    let mut valid_artist = artist_layout.clone();
    valid_artist["widgets"] = json!([
        { "id": "cal-1", "type": "listening_calendar", "colSpan": 6 },
        { "id": "ms-1", "type": "listening_milestones", "colSpan": 6 },
        { "id": "dr-1", "type": "discovery_ratio", "colSpan": 3 }
    ]);
    let (status, saved_artist) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=artist",
        valid_artist,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved_artist["widgets"].as_array().unwrap().len(), 3);

    // 3. Album template allows listening_calendar, listening_milestones, discovery_ratio
    let (status, album_layout) =
        get(&state, &token, "/api/v1/dashboard/layout?template=album").await;
    assert_eq!(status, StatusCode::OK);

    let mut valid_album = album_layout.clone();
    valid_album["widgets"] = json!([
        { "id": "cal-alb", "type": "listening_calendar", "colSpan": 6 },
        { "id": "ms-alb", "type": "listening_milestones", "colSpan": 6 },
        { "id": "dr-alb", "type": "discovery_ratio", "colSpan": 3 }
    ]);
    let (status, saved_album) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=album",
        valid_album,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved_album["widgets"].as_array().unwrap().len(), 3);

    // 4. Track template allows listening_calendar and listening_milestones, but REJECTS discovery_ratio
    let (status, track_layout) =
        get(&state, &token, "/api/v1/dashboard/layout?template=track").await;
    assert_eq!(status, StatusCode::OK);

    // discovery_ratio rejected on track template
    let mut invalid_track = track_layout.clone();
    invalid_track["widgets"] = json!([
        { "id": "dr-trk", "type": "discovery_ratio", "colSpan": 3 }
    ]);
    let (status, _) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=track",
        invalid_track,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Expected discovery_ratio to be rejected on track template"
    );

    // listening_calendar and listening_milestones accepted on track template
    let mut valid_track = track_layout.clone();
    valid_track["widgets"] = json!([
        { "id": "cal-trk", "type": "listening_calendar", "colSpan": 6 },
        { "id": "ms-trk", "type": "listening_milestones", "colSpan": 6 }
    ]);
    let (status, saved_track) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=track",
        valid_track,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved_track["widgets"].as_array().unwrap().len(), 2);

    // 5. listening_milestones MUST NOT accept period_override (reject with 400 Bad Request)
    for template_name in &["artist", "album", "track"] {
        let (status, base_layout) = get(
            &state,
            &token,
            &format!("/api/v1/dashboard/layout?template={template_name}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let mut invalid_override = base_layout.clone();
        invalid_override["widgets"] = json!([
            {
                "id": "ms-override",
                "type": "listening_milestones",
                "colSpan": 6,
                "periodOverride": "30d"
            }
        ]);
        let (status, _) = post(
            &state,
            Some(&token),
            Some("test-csrf"),
            &format!("/api/v1/dashboard/layout?template={template_name}"),
            invalid_override,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "Expected listening_milestones with period_override to be rejected on {template_name} template"
        );
    }
}

#[tokio::test]
async fn discovery_ratio_scoped_and_rejected_for_track() {
    let (_dir, state, token) = setup().await;

    // Seed events:
    // Artist 1 / Album 1:
    // - Track 1: historical play in Jan 2025
    // - Track 1: repeated play in Feb 2025
    // - Track 2: discovery play in Feb 2025
    // Artist 2 / Album 2:
    // - Track 3: historical play in Jan 2025
    // - Track 3: repeat in Feb 2025
    let e1 = Event {
        source: "spotify_api",
        source_key: "t1:1".into(),
        source_timestamp_ms: at("2025-01-10T10:00:00Z"),
        played_at_ms: at("2025-01-10T10:00:00Z"),
        listened_ms: Some(200000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000001",
            "type": "track",
            "name": "Track One",
            "duration_ms": 200000,
            "album": { "id": "alb-1", "name": "Album One", "images": [] },
            "artists": [{ "id": "art-1", "name": "Artist One" }]
        }))
        .unwrap(),
        raw: json!({}),
    };
    let e2 = Event {
        source: "spotify_api",
        source_key: "t1:2".into(),
        source_timestamp_ms: at("2025-02-10T10:00:00Z"),
        played_at_ms: at("2025-02-10T10:00:00Z"),
        listened_ms: Some(200000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000001",
            "type": "track",
            "name": "Track One",
            "duration_ms": 200000,
            "album": { "id": "alb-1", "name": "Album One", "images": [] },
            "artists": [{ "id": "art-1", "name": "Artist One" }]
        }))
        .unwrap(),
        raw: json!({}),
    };
    let e3 = Event {
        source: "spotify_api",
        source_key: "t2:1".into(),
        source_timestamp_ms: at("2025-02-15T12:00:00Z"),
        played_at_ms: at("2025-02-15T12:00:00Z"),
        listened_ms: Some(180000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000002",
            "type": "track",
            "name": "Track Two",
            "duration_ms": 180000,
            "album": { "id": "alb-1", "name": "Album One", "images": [] },
            "artists": [{ "id": "art-1", "name": "Artist One" }]
        }))
        .unwrap(),
        raw: json!({}),
    };
    let e4 = Event {
        source: "spotify_api",
        source_key: "t3:1".into(),
        source_timestamp_ms: at("2025-01-05T10:00:00Z"),
        played_at_ms: at("2025-01-05T10:00:00Z"),
        listened_ms: Some(220000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000003",
            "type": "track",
            "name": "Track Three",
            "duration_ms": 220000,
            "album": { "id": "alb-2", "name": "Album Two", "images": [] },
            "artists": [{ "id": "art-2", "name": "Artist Two" }]
        }))
        .unwrap(),
        raw: json!({}),
    };
    let e5 = Event {
        source: "spotify_api",
        source_key: "t3:2".into(),
        source_timestamp_ms: at("2025-02-20T10:00:00Z"),
        played_at_ms: at("2025-02-20T10:00:00Z"),
        listened_ms: Some(220000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000003",
            "type": "track",
            "name": "Track Three",
            "duration_ms": 220000,
            "album": { "id": "alb-2", "name": "Album Two", "images": [] },
            "artists": [{ "id": "art-2", "name": "Artist Two" }]
        }))
        .unwrap(),
        raw: json!({}),
    };

    save(&state.storage, &[e1, e2, e3, e4, e5]).await;

    let art1_id: String = sqlx::query_scalar("SELECT id FROM artists WHERE name='Artist One'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();
    let art2_id: String = sqlx::query_scalar("SELECT id FROM artists WHERE name='Artist Two'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();
    let alb1_id: String = sqlx::query_scalar("SELECT id FROM albums WHERE name='Album One'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();
    let trk1_id: String = sqlx::query_scalar("SELECT id FROM tracks WHERE title='Track One'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();

    // 1. Scoped to Artist One in Feb 2025: 1 discovery (Track 2), 1 repeat (Track 1)
    let (status, dr_art1) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/discovery-ratio?kind=artists&id={art1_id}&from=2025-02-01&to=2025-02-28"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dr_art1["discoveries"], 1);
    assert_eq!(dr_art1["repeats"], 1);
    assert_eq!(dr_art1["total"], 2);
    assert_eq!(dr_art1["discovery_percent"], 50.0);

    // 2. Scoped to Artist Two in Feb 2025: 0 discoveries, 1 repeat (Track 3)
    let (status, dr_art2) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/discovery-ratio?kind=artists&id={art2_id}&from=2025-02-01&to=2025-02-28"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dr_art2["discoveries"], 0);
    assert_eq!(dr_art2["repeats"], 1);
    assert_eq!(dr_art2["total"], 1);
    assert_eq!(dr_art2["discovery_percent"], 0.0);

    // 3. Scoped to Album One in Feb 2025
    let (status, dr_alb1) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/discovery-ratio?kind=albums&id={alb1_id}&from=2025-02-01&to=2025-02-28"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dr_alb1["discoveries"], 1);
    assert_eq!(dr_alb1["repeats"], 1);

    // 4. Discovery ratio REJECTED for track scope (400 Bad Request)
    let (status, _) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/discovery-ratio?kind=tracks&id={trk1_id}"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Expected discovery ratio with kind=tracks to return 400"
    );

    let (status, _) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/discovery-ratio?track={trk1_id}"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Expected discovery ratio with track param to return 400"
    );

    // 5. Invalid kind rejected (400)
    let (status, _) = get(
        &state,
        &token,
        "/api/v1/consultation/discovery-ratio?kind=invalid&id=123",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 6. Kind without id rejected (400)
    let (status, _) = get(
        &state,
        &token,
        "/api/v1/consultation/discovery-ratio?kind=artists",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 7. Id without kind rejected (400)
    let (status, _) = get(
        &state,
        &token,
        "/api/v1/consultation/discovery-ratio?id=123",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn listening_calendar_scoping_zeros_dst_and_years() {
    let (_dir, state, token) = setup().await;

    // Seed listens for Artist A across June 2024:
    // - 2024-06-10: 2 counted listens
    // - 2024-06-11: 1 counted listen
    // - 2024-06-12: 0 listens (empty day)
    // - 2024-06-13: 1 counted listen
    // - 2024-06-14: 1 uncounted short listen (15s) -> count must be 0!
    // - 2024-06-15: 1 listen for Artist B (must NOT be counted for Artist A)
    // - 2025-01-15: 1 counted listen in 2025 (multi-year interval)
    let e1 = Event {
        source: "spotify_api",
        source_key: "cal:1".into(),
        source_timestamp_ms: at("2024-06-10T08:00:00Z"),
        played_at_ms: at("2024-06-10T08:00:00Z"),
        listened_ms: Some(180000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000010",
            "type": "track",
            "name": "Track A1",
            "duration_ms": 180000,
            "album": { "id": "cal-alb-a", "name": "Album Cal A", "images": [] },
            "artists": [{ "id": "cal-art-a", "name": "Artist Cal A" }]
        }))
        .unwrap(),
        raw: json!({}),
    };
    let e2 = Event {
        source: "spotify_api",
        source_key: "cal:2".into(),
        source_timestamp_ms: at("2024-06-10T20:00:00Z"),
        played_at_ms: at("2024-06-10T20:00:00Z"),
        listened_ms: Some(180000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000010",
            "type": "track",
            "name": "Track A1",
            "duration_ms": 180000,
            "album": { "id": "cal-alb-a", "name": "Album Cal A", "images": [] },
            "artists": [{ "id": "cal-art-a", "name": "Artist Cal A" }]
        }))
        .unwrap(),
        raw: json!({}),
    };
    let e3 = Event {
        source: "spotify_api",
        source_key: "cal:3".into(),
        source_timestamp_ms: at("2024-06-11T12:00:00Z"),
        played_at_ms: at("2024-06-11T12:00:00Z"),
        listened_ms: Some(180000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000010",
            "type": "track",
            "name": "Track A1",
            "duration_ms": 180000,
            "album": { "id": "cal-alb-a", "name": "Album Cal A", "images": [] },
            "artists": [{ "id": "cal-art-a", "name": "Artist Cal A" }]
        }))
        .unwrap(),
        raw: json!({}),
    };
    let e4 = Event {
        source: "spotify_api",
        source_key: "cal:4".into(),
        source_timestamp_ms: at("2024-06-13T14:00:00Z"),
        played_at_ms: at("2024-06-13T14:00:00Z"),
        listened_ms: Some(180000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000010",
            "type": "track",
            "name": "Track A1",
            "duration_ms": 180000,
            "album": { "id": "cal-alb-a", "name": "Album Cal A", "images": [] },
            "artists": [{ "id": "cal-art-a", "name": "Artist Cal A" }]
        }))
        .unwrap(),
        raw: json!({}),
    };
    // Uncounted short play (<30s)
    let e5 = Event {
        source: "spotify_api",
        source_key: "cal:5".into(),
        source_timestamp_ms: at("2024-06-14T10:00:00Z"),
        played_at_ms: at("2024-06-14T10:00:00Z"),
        listened_ms: Some(15000),
        duration_kind: DurationKind::Actual,
        counted: false,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000010",
            "type": "track",
            "name": "Track A1",
            "duration_ms": 180000,
            "album": { "id": "cal-alb-a", "name": "Album Cal A", "images": [] },
            "artists": [{ "id": "cal-art-a", "name": "Artist Cal A" }]
        }))
        .unwrap(),
        raw: json!({}),
    };
    // Play for Artist B
    let e6 = Event {
        source: "spotify_api",
        source_key: "cal:6".into(),
        source_timestamp_ms: at("2024-06-15T10:00:00Z"),
        played_at_ms: at("2024-06-15T10:00:00Z"),
        listened_ms: Some(180000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000020",
            "type": "track",
            "name": "Track B1",
            "duration_ms": 180000,
            "album": { "id": "cal-alb-b", "name": "Album Cal B", "images": [] },
            "artists": [{ "id": "cal-art-b", "name": "Artist Cal B" }]
        }))
        .unwrap(),
        raw: json!({}),
    };
    // 2025 play for Artist A
    let e7 = Event {
        source: "spotify_api",
        source_key: "cal:7".into(),
        source_timestamp_ms: at("2025-01-15T10:00:00Z"),
        played_at_ms: at("2025-01-15T10:00:00Z"),
        listened_ms: Some(180000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&json!({
            "uri": "spotify:track:0000000000000000000010",
            "type": "track",
            "name": "Track A1",
            "duration_ms": 180000,
            "album": { "id": "cal-alb-a", "name": "Album Cal A", "images": [] },
            "artists": [{ "id": "cal-art-a", "name": "Artist Cal A" }]
        }))
        .unwrap(),
        raw: json!({}),
    };

    save(&state.storage, &[e1, e2, e3, e4, e5, e6, e7]).await;

    let art_a_id: String = sqlx::query_scalar("SELECT id FROM artists WHERE name='Artist Cal A'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();
    let alb_a_id: String = sqlx::query_scalar("SELECT id FROM albums WHERE name='Album Cal A'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();
    let trk_a_id: String = sqlx::query_scalar("SELECT id FROM tracks WHERE title='Track A1'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();

    // 1. Missing entity scope rejected (400 Bad Request)
    let (status, _) = get(&state, &token, "/api/v1/consultation/calendar").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 2. Query June 10 - June 14, 2024 for Artist A
    let (status, cal) = get(
        &state,
        &token,
        &format!(
            "/api/v1/consultation/calendar?kind=artists&id={art_a_id}&from=2024-06-10&to=2024-06-14"
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cal["total_listens"], 4);
    let days = cal["days"].as_array().expect("days array");
    assert_eq!(days.len(), 5);

    // Day 0: 2024-06-10 (2 counted listens)
    assert_eq!(days[0]["date"], "2024-06-10");
    assert_eq!(days[0]["count"], 2);

    // Day 1: 2024-06-11 (1 counted listen)
    assert_eq!(days[1]["date"], "2024-06-11");
    assert_eq!(days[1]["count"], 1);

    // Day 2: 2024-06-12 (0 listens - included explicitly!)
    assert_eq!(days[2]["date"], "2024-06-12");
    assert_eq!(days[2]["count"], 0);

    // Day 3: 2024-06-13 (1 counted listen)
    assert_eq!(days[3]["date"], "2024-06-13");
    assert_eq!(days[3]["count"], 1);

    // Day 4: 2024-06-14 (1 uncounted listen -> count is 0)
    assert_eq!(days[4]["date"], "2024-06-14");
    assert_eq!(days[4]["count"], 0);

    // 3. Years list contains all active years for Artist A
    let years = cal["years"].as_array().expect("years array");
    assert_eq!(years.len(), 2);
    assert_eq!(years[0], 2025);
    assert_eq!(years[1], 2024);

    // 4. Year navigation: query year=2024
    let (status, cal_2024) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/calendar?kind=artists&id={art_a_id}&year=2024"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // 2024 is a leap year (366 days)
    assert_eq!(cal_2024["days"].as_array().unwrap().len(), 366);
    assert_eq!(cal_2024["total_listens"], 4);

    // 5. Scoped to album
    let (status, cal_alb) = get(
        &state,
        &token,
        &format!(
            "/api/v1/consultation/calendar?kind=albums&id={alb_a_id}&from=2024-06-10&to=2024-06-14"
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cal_alb["total_listens"], 4);

    // 6. Scoped to track
    let (status, cal_trk) = get(
        &state,
        &token,
        &format!(
            "/api/v1/consultation/calendar?kind=tracks&id={trk_a_id}&from=2024-06-10&to=2024-06-14"
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cal_trk["total_listens"], 4);

    // 7. Empty entity returns totalListens = 0 and valid structure
    let (status, empty_cal) = get(
        &state,
        &token,
        "/api/v1/consultation/calendar?kind=artists&id=non-existent-artist",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty_cal["total_listens"], 0);
    assert_eq!(empty_cal["days"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn listening_milestones_thresholds_streaks_and_all_time() {
    let (_dir, state, token) = setup().await;

    // Seed 12 events for Artist MS:
    // First listen on 2024-01-01 10:00:00: "First Song"
    // Streaks:
    // - 2024-01-01: 1 listen
    // - 2024-01-02: 2 listens (same day - does not lengthen streak!)
    // - 2024-01-03: 1 listen -> streak is 3 days (Jan 1, Jan 2, Jan 3)
    // - Gap: Jan 4 no listens (breaks streak!)
    // - 2024-01-05: 1 listen
    // - 2024-01-06: 1 listen -> streak is 2 days
    // - Jan 7 to Jan 12: 1 listen each day to reach 12 listens total
    //   Threshold 10 reached on Jan 10!
    // Last listen on 2024-01-12: "Last Song"
    let mut events = Vec::new();

    let titles = [
        "First Song",
        "Second Song",
        "Third Song",
        "Fourth Song",
        "Fifth Song",
        "Sixth Song",
        "Seventh Song",
        "Eighth Song",
        "Ninth Song",
        "Tenth Song",
        "Eleventh Song",
        "Last Song",
    ];

    let timestamps = [
        "2024-01-01T10:00:00Z",
        "2024-01-02T09:00:00Z",
        "2024-01-02T20:00:00Z", // multiple listens on Jan 2
        "2024-01-03T10:00:00Z", // streak end (3 days)
        // gap Jan 4
        "2024-01-05T10:00:00Z",
        "2024-01-06T10:00:00Z",
        "2024-01-07T10:00:00Z",
        "2024-01-08T10:00:00Z",
        "2024-01-09T10:00:00Z",
        "2024-01-10T10:00:00Z", // 10th listen!
        "2024-01-11T10:00:00Z",
        "2024-01-12T10:00:00Z", // 12th listen!
    ];

    for i in 0..12 {
        let ts = at(timestamps[i]);
        events.push(Event {
            source: "spotify_api",
            source_key: format!("ms:{i}"),
            source_timestamp_ms: ts,
            played_at_ms: ts,
            listened_ms: Some(200000),
            duration_kind: DurationKind::Actual,
            counted: true,
            track: parse_track(&json!({
                "uri": format!("spotify:track:{:022}", i + 100),
                "type": "track",
                "name": titles[i],
                "duration_ms": 200000,
                "album": { "id": "ms-alb", "name": "Milestone Album", "images": [] },
                "artists": [{ "id": "ms-art", "name": "Artist MS" }]
            }))
            .unwrap(),
            raw: json!({}),
        });
    }

    save(&state.storage, &events).await;

    let art_ms_id: String = sqlx::query_scalar("SELECT id FROM artists WHERE name='Artist MS'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();

    // 1. Missing scope rejected (400)
    let (status, _) = get(&state, &token, "/api/v1/consultation/milestones").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 2. Query milestones
    let (status, ms) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/milestones?kind=artists&id={art_ms_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ms["total_listens"], 12);

    // First listen
    assert_eq!(ms["first_listen"]["track_title"], "First Song");
    assert_eq!(
        ms["first_listen"]["played_at_ms"],
        at("2024-01-01T10:00:00Z")
    );

    // Last listen
    assert_eq!(ms["last_listen"]["track_title"], "Last Song");
    assert_eq!(
        ms["last_listen"]["played_at_ms"],
        at("2024-01-12T10:00:00Z")
    );

    // Threshold 10
    let thresholds = ms["thresholds"].as_array().expect("thresholds array");
    assert_eq!(thresholds.len(), 1);
    assert_eq!(thresholds[0]["count"], 10);
    assert_eq!(thresholds[0]["reached_at_ms"], at("2024-01-10T10:00:00Z"));

    // Longest streak: Jan 5 - Jan 12 is 8 consecutive calendar days!
    // (Jan 1, 2, 3 was 3 days. Jan 5, 6, 7, 8, 9, 10, 11, 12 is 8 days!)
    assert_eq!(ms["longest_streak"]["days"], 8);
    assert_eq!(ms["longest_streak"]["start_date"], "2024-01-05");
    assert_eq!(ms["longest_streak"]["end_date"], "2024-01-12");

    // Current streak: ended in Jan 2024 (well before today/yesterday) -> 0 days
    assert_eq!(ms["current_streak"]["days"], 0);

    // 3. All-time behavior: passing ?period=today or custom dates still evaluates whole history!
    let (status, ms_filtered) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/milestones?kind=artists&id={art_ms_id}&period=today"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ms_filtered["total_listens"], 12);
    assert_eq!(ms_filtered["first_listen"]["track_title"], "First Song");
    assert_eq!(ms_filtered["last_listen"]["track_title"], "Last Song");

    // 4. Empty entity returns zero/None state without error
    let (status, ms_empty) = get(
        &state,
        &token,
        "/api/v1/consultation/milestones?kind=artists&id=non-existent-artist",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ms_empty["total_listens"], 0);
    assert!(ms_empty["first_listen"].is_null());
    assert!(ms_empty["last_listen"].is_null());
    assert_eq!(ms_empty["thresholds"].as_array().unwrap().len(), 0);
    assert_eq!(ms_empty["longest_streak"]["days"], 0);
    assert_eq!(ms_empty["current_streak"]["days"], 0);
}

#[tokio::test]
async fn dashboard_layout_rejects_duplicate_widget_ids_and_duplicate_widget_types() {
    let (_dir, state, token) = setup().await;

    // 1. Duplicate widget ID
    let layout_dup_id = json!({
        "version": 0,
        "widgets": [
            { "id": "dup-id", "type": "totals", "colSpan": 6 },
            { "id": "dup-id", "type": "activity_chart", "colSpan": 6 }
        ]
    });
    let (status, err) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=home",
        layout_dup_id,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        err["error"]
            .as_str()
            .unwrap_or("")
            .contains("Duplicate widget ID 'dup-id'")
    );

    // 2. Duplicate widget type
    let layout_dup_type = json!({
        "version": 0,
        "widgets": [
            { "id": "totals-1", "type": "totals", "colSpan": 6 },
            { "id": "totals-2", "type": "totals", "colSpan": 6 }
        ]
    });
    let (status, err) = post(
        &state,
        Some(&token),
        Some("test-csrf"),
        "/api/v1/dashboard/layout?template=home",
        layout_dup_type,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        err["error"]
            .as_str()
            .unwrap_or("")
            .contains("Duplicate widget type 'totals'")
    );
}

#[tokio::test]
async fn music_page_selective_totals_skips_heavy_query_when_disabled() {
    let (_dir, state, token) = setup().await;
    save(
        &state.storage,
        &[event(
            1,
            at("2024-06-10T12:00:00Z"),
            Some(180_000),
            DurationKind::Actual,
            &[("art-select-totals", "Selective Artist")],
        )],
    )
    .await;
    let art_id: String = sqlx::query_scalar("SELECT id FROM artists WHERE name='Selective Artist'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();

    // Default request includes totals
    let (status, with_totals) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/music/artists/{art_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(with_totals["totals"]["records"], 1);
    assert_eq!(with_totals["totals"]["listens"], 1);

    // include_totals=false skips query and returns zeroed Totals
    let (status, without_totals) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/music/artists/{art_id}?include_totals=false"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(without_totals["totals"]["records"], 0);
    assert_eq!(without_totals["totals"]["listens"], 0);
    assert_eq!(without_totals["totals"]["tracks"], 0);
    assert_eq!(without_totals["totals"]["actual_ms"], 0);
}

#[tokio::test]
async fn calendar_long_history_defaults_to_most_recent_active_year_without_failing() {
    let (_dir, state, token) = setup().await;
    // 2012 and 2025 are ~13 years apart (>4700 days!)
    save(
        &state.storage,
        &[
            event(
                1,
                at("2012-01-15T12:00:00Z"),
                Some(180_000),
                DurationKind::Actual,
                &[("art-long-history", "Archive Legend")],
            ),
            event(
                2,
                at("2025-05-20T14:00:00Z"),
                Some(200_000),
                DurationKind::Actual,
                &[("art-long-history", "Archive Legend")],
            ),
        ],
    )
    .await;
    let art_id: String = sqlx::query_scalar("SELECT id FROM artists WHERE name='Archive Legend'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();

    // Initial load with default "all" and no year param must succeed, defaulting to 2025
    let (status, cal) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/calendar?kind=artists&id={art_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let years = cal["years"].as_array().expect("years array");
    assert_eq!(years.len(), 2);
    assert_eq!(years[0], 2025);
    assert_eq!(years[1], 2012);

    // Days are returned for the most recent year (2025)
    let days = cal["days"].as_array().expect("days array");
    assert!(!days.is_empty());
    assert!(days[0]["date"].as_str().unwrap().starts_with("2025"));
    assert_eq!(cal["total_listens"], 1);

    // Year navigation to 2012 works as expected
    let (status, cal_2012) = get(
        &state,
        &token,
        &format!("/api/v1/consultation/calendar?kind=artists&id={art_id}&year=2012"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let days_2012 = cal_2012["days"].as_array().expect("days array");
    assert!(!days_2012.is_empty());
    assert!(days_2012[0]["date"].as_str().unwrap().starts_with("2012"));
    assert_eq!(cal_2012["total_listens"], 1);
}
