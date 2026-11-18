#![allow(clippy::unwrap_used, clippy::disallowed_methods)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::TimeZone;
use chrono_tz::Europe::Rome;
use http_body_util::BodyExt;
use scrobbl_rs::{
    AppState,
    ask::{
        projection,
        sandbox::{self, SandboxLimits, SqlValue},
    },
    auth, browse,
    config::Config,
    consultation::Filters,
    db::{self, DurationKind, Event},
    spotify::parse_track,
    web,
};
use serde_json::json;
use tower::ServiceExt;

fn ts(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .unwrap()
        .timestamp_millis()
}

fn scalar(proj: &std::path::Path, sql: &str) -> Vec<SqlValue> {
    let out = sandbox::execute(proj, sql, SandboxLimits::default()).unwrap();
    assert_eq!(
        out.rows.len(),
        1,
        "expected 1 row for `{sql}`, got {:?}",
        out.rows
    );
    out.rows.into_iter().next().unwrap()
}

#[allow(clippy::too_many_arguments)]
fn make_event(
    source: &'static str,
    key: &str,
    when: i64,
    ms: Option<i64>,
    kind: DurationKind,
    track_uri: &str,
    title: &str,
    album: Option<(&str, &str)>,
    artists: &[(&str, &str)],
    raw_extra: serde_json::Value,
) -> Event {
    let mut raw = json!({
        "uri": track_uri,
        "type": "track",
        "name": title,
        "duration_ms": 240_000,
        "album": album.map(|(id, name)| json!({"id": id, "name": name, "images": []})).unwrap_or(json!({"id": "none", "name": "", "images": []})),
        "artists": artists.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
    });
    if let (Some(dst), Some(src)) = (raw.as_object_mut(), raw_extra.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    let mut track = parse_track(&raw).unwrap();
    if album.is_none() {
        track.album_id = None;
        track.album = None;
    }
    Event {
        source,
        source_key: key.to_owned(),
        source_timestamp_ms: when,
        played_at_ms: when,
        listened_ms: ms,
        duration_kind: kind,
        counted: kind != DurationKind::Actual || ms.unwrap_or(0) >= 30_000,
        track,
        raw,
    }
}

async fn post_json(state: &AppState, token: &str, uri: &str, body: serde_json::Value) {
    let response = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
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
    assert_eq!(response.status(), StatusCode::OK, "POST {uri} failed");
}

async fn get_json(state: &AppState, token: &str, uri: &str) -> serde_json::Value {
    let response = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("host", "127.0.0.1:3210")
                .header("cookie", format!("scrobbl_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "GET {uri} failed");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn projection_sql_matches_consultation_across_metrics() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let token = auth::random_secret();
    sqlx::query("INSERT INTO sessions VALUES(?,'test-csrf',9999999999999)")
        .bind(auth::digest(&token))
        .execute(&storage.private)
        .await
        .unwrap();
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
    let state = AppState::new(storage.clone(), config);

    // Anchor "today" for streaks relative to real current date in Europe/Rome.
    let now_ms = scrobbl_rs::auth::now_ms();
    let today_rome = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now_ms)
        .unwrap()
        .with_timezone(&Rome)
        .date_naive();
    let noon = |d: chrono::NaiveDate| -> i64 {
        Rome.from_local_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
            .single()
            .unwrap()
            .timestamp_millis()
    };

    // Synthetic fixture covering:
    // - Name groups (two distinct source artist IDs both named "Ado" auto-grouped)
    // - Local corrections (renaming "Bjork" -> "Björk", album "Homogenic" -> "Homogenic (Corrected)", track "Joga" -> "Jóga")
    // - Track editions (two editions of "Show" by Ado on Single and Zanmu)
    // - Multi-artist track with guest ("Boy With Luv" by BTS + Halsey)
    // - Short listen (<30s actual -> counted = 0)
    // - Unknown duration, estimated duration, actual duration
    // - Trashed listen (must be excluded everywhere)
    // - Historical 3-day streak (2024-01-01..03) and active 2-day streak ending yesterday
    let events = vec![
        // Day 1 historical: 2023-12-31 23:30 UTC = 2024-01-01 00:30 Europe/Rome (Monday)
        make_event(
            "spotify_import",
            "k1",
            ts("2023-12-31T23:30:00Z"),
            Some(200_000),
            DurationKind::Actual,
            "spotify:track:0000000000000000000001",
            "Joga",
            Some(("al-homogenic", "Homogenic")),
            &[("ar-bjork", "Bjork")],
            json!({"platform": "android", "conn_country": "IT", "shuffle": false}),
        ),
        // Reconciled spotify_api observation on k1
        make_event(
            "spotify_api",
            "k1-api",
            ts("2023-12-31T23:30:00Z"),
            Some(210_000),
            DurationKind::Estimated,
            "spotify:track:0000000000000000000001",
            "Joga",
            Some(("al-homogenic", "Homogenic")),
            &[("ar-bjork", "Bjork")],
            json!({}),
        ),
        // Day 2 historical: 2024-01-02 (Tuesday) - Ado edition 1 (single)
        make_event(
            "spotify_api",
            "k2",
            ts("2024-01-02T14:00:00Z"),
            Some(190_000),
            DurationKind::Estimated,
            "spotify:track:0000000000000000000002",
            "Show",
            Some(("al-show-single", "Show - Single")),
            &[("ar-ado-1", "Ado")],
            json!({}),
        ),
        // Day 3 historical: 2024-01-03 (Wednesday) - Ado edition 2 (album Zanmu, different source artist ID same name)
        make_event(
            "listenbrainz",
            "k3",
            ts("2024-01-03T15:00:00Z"),
            None,
            DurationKind::Unknown,
            "spotify:track:0000000000000000000003",
            "Show",
            Some(("al-zanmu", "Zanmu")),
            &[("ar-ado-2", "Ado")],
            json!({}),
        ),
        // Same day 2024-01-03: short play (<30s actual -> counted = 0)
        make_event(
            "spotify_import",
            "k4-short",
            ts("2024-01-03T16:00:00Z"),
            Some(12_000),
            DurationKind::Actual,
            "spotify:track:0000000000000000000004",
            "Odo",
            Some(("al-zanmu", "Zanmu")),
            &[("ar-ado-2", "Ado")],
            json!({}),
        ),
        // Same day 2024-01-03: trashed listen (must never count)
        make_event(
            "spotify_api",
            "k5-trash",
            ts("2024-01-03T17:00:00Z"),
            Some(220_000),
            DurationKind::Actual,
            "spotify:track:0000000000000000000005",
            "Trashed Song",
            Some(("al-trash", "Trashed Album")),
            &[("ar-trash", "Trashed Artist")],
            json!({}),
        ),
        // 2024-02-15: multi-artist collab (BTS + Halsey)
        make_event(
            "spotify_api",
            "k6",
            ts("2024-02-15T18:00:00Z"),
            Some(230_000),
            DurationKind::Actual,
            "spotify:track:0000000000000000000006",
            "Boy With Luv",
            Some(("al-persona", "Persona")),
            &[("ar-bts", "BTS"), ("ar-halsey", "Halsey")],
            json!({}),
        ),
        // Current streak ending yesterday: 2 consecutive days (today - 2, today - 1)
        make_event(
            "spotify_api",
            "k7",
            noon(today_rome - chrono::Duration::days(2)),
            Some(180_000),
            DurationKind::Actual,
            "spotify:track:0000000000000000000001",
            "Joga",
            Some(("al-homogenic", "Homogenic")),
            &[("ar-bjork", "Bjork")],
            json!({}),
        ),
        make_event(
            "spotify_api",
            "k8",
            noon(today_rome - chrono::Duration::days(1)),
            Some(180_000),
            DurationKind::Actual,
            "spotify:track:0000000000000000000004",
            "Odo",
            Some(("al-zanmu", "Zanmu")),
            &[("ar-ado-2", "Ado")],
            json!({}),
        ),
    ];

    let mut tx = storage.writer.begin().await.unwrap();
    db::ingest(&mut tx, &events).await.unwrap();
    sqlx::query("UPDATE listens SET trashed_at_ms = 1 WHERE id IN (SELECT listen_id FROM observations WHERE source_key = 'k5-trash')")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Apply local corrections to artist, album, and track names.
    let bjork_group: String = sqlx::query_scalar(
        "SELECT m.group_id FROM name_group_members m JOIN artists a ON a.id = m.entity_id WHERE m.kind='artists' AND a.name='Bjork' LIMIT 1",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let homogenic_group: String = sqlx::query_scalar(
        "SELECT m.group_id FROM name_group_members m JOIN albums a ON a.id = m.entity_id WHERE m.kind='albums' AND a.name='Homogenic' LIMIT 1",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let joga_id: String = sqlx::query_scalar("SELECT id FROM tracks WHERE title='Joga' LIMIT 1")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Apply local corrections to artist, album, and track names through the library API.
    post_json(
        &state,
        &token,
        &format!("/api/v1/library/artists/{bjork_group}"),
        json!({"name": "Björk", "rating": null}),
    )
    .await;
    post_json(
        &state,
        &token,
        &format!("/api/v1/library/albums/{homogenic_group}"),
        json!({"name": "Homogenic (Corrected)", "rating": null}),
    )
    .await;
    post_json(
        &state,
        &token,
        &format!("/api/v1/library/tracks/{joga_id}"),
        json!({"name": "Jóga", "rating": null}),
    )
    .await;

    let gate = projection::ProjectionGate::default();
    let proj = projection::ensure(&storage, &gate).await.unwrap();

    let dashboard = get_json(&state, &token, "/api/dashboard?period=all").await;
    let totals = &dashboard["totals"];

    let mut view = storage.read_view().await.unwrap();
    let conn = &mut *view.transaction;

    // 1. Plays and Time listened vs /api/dashboard totals
    assert_eq!(
        scalar(&proj, "SELECT COUNT(*) FROM listens"),
        vec![SqlValue::Integer(totals["records"].as_i64().unwrap())]
    );
    assert_eq!(
        scalar(&proj, "SELECT SUM(counted) FROM listens"),
        vec![SqlValue::Integer(totals["listens"].as_i64().unwrap())]
    );
    assert_eq!(
        scalar(
            &proj,
            "SELECT \
             COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='actual' THEN listened_ms END), 0), \
             COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='estimated' THEN listened_ms END), 0), \
             COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='unknown' THEN 1 ELSE 0 END), 0), \
             COALESCE(SUM(CASE WHEN counted=1 AND duration_kind IN ('actual','estimated') THEN listened_ms END), 0) \
             FROM listens"
        ),
        vec![
            SqlValue::Integer(totals["actual_ms"].as_i64().unwrap()),
            SqlValue::Integer(totals["estimated_ms"].as_i64().unwrap()),
            SqlValue::Integer(totals["unknown_durations"].as_i64().unwrap()),
            SqlValue::Integer(
                totals["actual_ms"].as_i64().unwrap() + totals["estimated_ms"].as_i64().unwrap()
            ),
        ]
    );
    assert_eq!(
        scalar(
            &proj,
            "SELECT COUNT(DISTINCT l.track_id) FROM listens l WHERE l.counted = 1"
        ),
        vec![SqlValue::Integer(totals["tracks"].as_i64().unwrap())]
    );

    // 2. Top-N rankings (consultation::ranking_page vs projection SQL)
    let artist_rank = browse::ranking_page(conn, "artists", &Filters::default(), now_ms)
        .await
        .unwrap();
    assert_eq!(
        scalar(
            &proj,
            "SELECT COUNT(DISTINCT ta.artist_id) FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id WHERE l.counted = 1"
        ),
        vec![SqlValue::Integer(artist_rank.items.len() as i64)]
    );
    let sql_artist_rank = sandbox::execute(
        &proj,
        "SELECT a.id, a.name, COUNT(DISTINCT l.id) AS plays \
         FROM listens l \
         JOIN track_artists ta ON ta.track_id = l.track_id \
         JOIN artists a ON a.id = ta.artist_id \
         WHERE l.counted = 1 \
         GROUP BY a.id \
         ORDER BY plays DESC, \
                  COALESCE(SUM(CASE WHEN l.duration_kind IN ('actual','estimated') THEN l.listened_ms END), 0) DESC, \
                  a.name COLLATE NOCASE ASC, a.id ASC",
        SandboxLimits::default(),
    )
    .unwrap();
    let expected_artists: Vec<Vec<SqlValue>> = artist_rank
        .items
        .iter()
        .map(|r| {
            vec![
                SqlValue::Text(r.id.clone()),
                SqlValue::Text(r.name.clone()),
                SqlValue::Integer(r.plays),
            ]
        })
        .collect();
    assert_eq!(sql_artist_rank.rows, expected_artists);

    let album_rank = browse::ranking_page(conn, "albums", &Filters::default(), now_ms)
        .await
        .unwrap();
    let sql_album_rank = sandbox::execute(
        &proj,
        "SELECT al.id, al.name, COUNT(*) AS plays \
         FROM listens l \
         JOIN tracks t ON t.id = l.track_id \
         JOIN albums al ON al.id = t.album_id \
         WHERE l.counted = 1 \
         GROUP BY al.id \
         ORDER BY plays DESC, \
                  COALESCE(SUM(CASE WHEN l.duration_kind IN ('actual','estimated') THEN l.listened_ms END), 0) DESC, \
                  al.name COLLATE NOCASE ASC, al.id ASC",
        SandboxLimits::default(),
    )
    .unwrap();
    let expected_albums: Vec<Vec<SqlValue>> = album_rank
        .items
        .iter()
        .map(|r| {
            vec![
                SqlValue::Text(r.id.clone()),
                SqlValue::Text(r.name.clone()),
                SqlValue::Integer(r.plays),
            ]
        })
        .collect();
    assert_eq!(sql_album_rank.rows, expected_albums);

    let track_rank = browse::ranking_page(conn, "tracks", &Filters::default(), now_ms)
        .await
        .unwrap();
    let sql_top_tracks = sandbox::execute(
        &proj,
        "SELECT t.id, t.title, COUNT(*) AS plays \
         FROM listens l \
         JOIN tracks t ON t.id = l.track_id \
         WHERE l.counted = 1 \
         GROUP BY t.id \
         ORDER BY plays DESC, \
                  COALESCE(SUM(CASE WHEN l.duration_kind IN ('actual','estimated') THEN l.listened_ms END), 0) DESC, \
                  t.title COLLATE NOCASE ASC, t.id ASC",
        SandboxLimits::default(),
    )
    .unwrap();
    let expected_tracks: Vec<Vec<SqlValue>> = track_rank
        .items
        .iter()
        .map(|r| {
            vec![
                SqlValue::Text(r.id.clone()),
                SqlValue::Text(r.name.clone()),
                SqlValue::Integer(r.plays),
            ]
        })
        .collect();
    assert_eq!(sql_top_tracks.rows, expected_tracks);

    // 3. Corrected artist & track names reflected in projection
    assert!(
        scalar(
            &proj,
            &format!("SELECT COUNT(*) FROM artists WHERE id = '{bjork_group}' AND name = 'Björk'")
        ) == vec![SqlValue::Integer(1)]
    );

    // 4. Streaks (longest and current) via calendar + listens vs browse::listening_milestones
    let milestones = browse::listening_milestones(conn, &Filters::default(), now_ms)
        .await
        .unwrap();
    let streak_sql = "
        WITH active_days AS (
            SELECT DISTINCT c.local_date,
                   ROW_NUMBER() OVER (ORDER BY c.local_date) AS seq
            FROM calendar c
            WHERE EXISTS (SELECT 1 FROM listens l WHERE l.local_date = c.local_date AND l.counted = 1)
        ),
        grouped AS (
            SELECT local_date,
                   date(local_date, '-' || seq || ' days') AS grp
            FROM active_days
        ),
        runs AS (
            SELECT COUNT(*) AS days, MIN(local_date) AS start_date, MAX(local_date) AS end_date
            FROM grouped
            GROUP BY grp
        )
        SELECT
            (SELECT days FROM runs ORDER BY days DESC, start_date ASC LIMIT 1) AS longest_days,
            (SELECT start_date FROM runs ORDER BY days DESC, start_date ASC LIMIT 1) AS longest_start,
            (SELECT end_date FROM runs ORDER BY days DESC, start_date ASC LIMIT 1) AS longest_end,
            COALESCE((SELECT days FROM runs WHERE end_date >= date((SELECT MAX(local_date) FROM calendar), '-1 day') ORDER BY end_date DESC LIMIT 1), 0) AS current_days
    ";
    assert_eq!(
        scalar(&proj, streak_sql),
        vec![
            SqlValue::Integer(milestones.longest_streak.days),
            SqlValue::Text(milestones.longest_streak.start_date.unwrap()),
            SqlValue::Text(milestones.longest_streak.end_date.unwrap()),
            SqlValue::Integer(milestones.current_streak.days),
        ]
    );

    // 5. Name group separate -> rebuild -> parity still holds
    drop(view);
    let before_separate = scalar(
        &proj,
        "SELECT COUNT(DISTINCT ta.artist_id) FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id WHERE l.counted = 1",
    );
    let ado_2: String =
        sqlx::query_scalar("SELECT id FROM artists WHERE name = 'Ado' ORDER BY id DESC LIMIT 1")
            .fetch_one(&storage.readers)
            .await
            .unwrap();
    let ado_group: String = sqlx::query_scalar(
        "SELECT group_id FROM name_group_members WHERE kind='artists' AND entity_id = ?",
    )
    .bind(&ado_2)
    .fetch_one(&storage.readers)
    .await
    .unwrap();
    post_json(
        &state,
        &token,
        &format!("/api/v1/library/artists/{ado_group}/group"),
        json!({"action": "separate", "member_ids": [ado_2]}),
    )
    .await;

    let proj = projection::ensure(&storage, &gate).await.unwrap();
    let mut view = storage.read_view().await.unwrap();
    let distinct_after = browse::ranking_page(
        &mut view.transaction,
        "artists",
        &Filters::default(),
        now_ms,
    )
    .await
    .unwrap()
    .items
    .len() as i64;
    assert_eq!(
        scalar(
            &proj,
            "SELECT COUNT(DISTINCT ta.artist_id) FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id WHERE l.counted = 1"
        ),
        vec![SqlValue::Integer(distinct_after)]
    );
    assert_ne!(before_separate, vec![SqlValue::Integer(distinct_after)]);
}
