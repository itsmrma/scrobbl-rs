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
    assert_eq!(widgets.len(), 4);
    assert_eq!(widgets[0]["type"], "totals");
    assert_eq!(widgets[0]["colSpan"], 6);
    assert_eq!(widgets[1]["type"], "unified_rankings");
    assert_eq!(widgets[1]["colSpan"], 6);
    assert_eq!(widgets[2]["type"], "activity_chart");
    assert_eq!(widgets[2]["colSpan"], 6);
    assert_eq!(widgets[3]["type"], "recent_listens");
    assert_eq!(widgets[3]["colSpan"], 6);

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

    // 6. Valid modified layout persists successfully
    let mut modified = default_layout.clone();
    modified["widgets"][0]["colSpan"] = json!(3);
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

    // 7. GET returns identical persisted layout
    let (status, read_back) = get(&state, &token, "/api/v1/dashboard/layout").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read_back["version"], 1);
    assert_eq!(read_back["widgets"][0]["colSpan"], 3);
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
    assert!(rankings["albums"].as_array().map_or(true, |a| a.is_empty()));
    assert!(rankings["tracks"].as_array().map_or(true, |a| a.is_empty()));

    // 3. GET /api/dashboard (full / default)
    let full = json_get(&state, &token, "/api/dashboard").await;
    assert_eq!(full["totals"]["listens"], 2);
    assert_eq!(full["rankings"]["artists"].as_array().unwrap().len(), 2);
    assert_eq!(full["rankings"]["albums"].as_array().unwrap().len(), 2);
    assert_eq!(full["rankings"]["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(full["recent"].as_array().unwrap().len(), 2);

    state.storage.close().await;
}
