use axum::{
    Json, Router,
    body::Body,
    http::{Request, StatusCode},
    routing::post,
};
use http_body_util::BodyExt;
use scrobbl_rs::{
    AppState,
    ask::{Ask, parser::ModelParser, projection},
    auth,
    config::Config,
    db::{self, DurationKind, Event},
    exports::{self, ExportSelection},
    recovery,
    spotify::parse_track,
    web,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

#[derive(Clone, Default)]
struct MockSql {
    reply: Arc<Mutex<String>>,
}

async fn mock_serve(mock: MockSql) -> String {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(_body): Json<Value>| {
            let reply = mock.reply.lock().unwrap().clone();
            async move {
                Json(json!({
                    "choices": [{"message": {"role": "assistant", "content": reply}}]
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/v1")
}

async fn req(
    state: &AppState,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:3210")
        .header("cookie", format!("scrobbl_session={token}"));
    if method != "GET" {
        builder = builder
            .header("origin", "http://127.0.0.1:3210")
            .header("x-csrf-token", "test-csrf")
            .header("content-type", "application/json");
    }
    let bytes = body
        .map(|v| serde_json::to_vec(&v).unwrap())
        .unwrap_or_default();
    let response = web::router(state.clone())
        .oneshot(builder.body(Body::from(bytes)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let raw = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&raw).unwrap_or(Value::Null))
}

async fn setup_remote_sql(mock_url: &str) -> (tempfile::TempDir, AppState, String) {
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
    let mut state = AppState::new(storage, config);
    state.ask = Ask::with_parser(
        Arc::new(ModelParser::local(
            state.db.clone(),
            dir.path().to_path_buf(),
        )),
        false,
        true,
        true,
    );
    let (status, body) = req(
        &state,
        &token,
        "POST",
        "/api/v1/ai/settings",
        Some(json!({
            "enabled": true,
            "provider": "remote",
            "remote_url": mock_url,
            "remote_model": "test-sql",
            "active_model": null,
            "threads": 4,
            "unload_minutes": 15,
            "model_dir": null,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    (dir, state, token)
}

fn make_play(source: &'static str, key: &str, ts_str: &str, artist: &str, title: &str) -> Event {
    let when = chrono::DateTime::parse_from_rfc3339(ts_str)
        .unwrap()
        .timestamp_millis();
    let raw = json!({
        "uri": "spotify:track:0000000000000000000042",
        "type": "track",
        "name": title,
        "duration_ms": 200_000,
        "album": {"id": "al-1", "name": "Debut", "images": []},
        "artists": [{"id": "ar-1", "name": artist}],
    });
    Event {
        source,
        source_key: key.to_owned(),
        source_timestamp_ms: when,
        played_at_ms: when,
        listened_ms: Some(200_000),
        duration_kind: DurationKind::Actual,
        counted: true,
        track: parse_track(&raw).unwrap(),
        raw,
    }
}

#[tokio::test]
async fn ask_answers_reflect_import_review_correction_and_calendar_changes_with_updated_as_of_time()
{
    let mock = MockSql::default();
    let url = mock_serve(mock.clone()).await;
    let (_dir, state, token) = setup_remote_sql(&url).await;

    // 1. Initial ingest of 2 plays on the same track.
    let mut tx = state.storage.writer.begin().await.unwrap();
    db::ingest(
        &mut tx,
        &[
            make_play("spotify_api", "p1", "2024-06-01T10:00:00Z", "Bjork", "Joga"),
            make_play("spotify_api", "p2", "2024-06-01T10:05:00Z", "Bjork", "Joga"),
        ],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    *mock.reply.lock().unwrap() = json!({
        "sql": "SELECT a.name AS artist, COUNT(DISTINCT l.id) AS plays FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id WHERE l.counted = 1 GROUP BY a.id",
        "title": "Artist plays",
        "computed": "Counted plays per artist.",
        "answer_kind": "table",
        "columns": [{"name": "artist", "format": "artist"}, {"name": "plays", "format": "count"}]
    })
    .to_string();

    let (status, body1) = req(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "Artist plays?"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body1}");
    assert_eq!(body1["sql"]["rows"], json!([["Bjork", 2]]));
    let as_of_1 = body1["sql"]["as_of_ms"]
        .as_i64()
        .expect("as_of_ms should be present");

    // 2. Local correction renames Bjork -> Björk.
    let artist_id: String = sqlx::query_scalar(
        "SELECT group_id FROM name_group_members WHERE kind = 'artists' LIMIT 1",
    )
    .fetch_one(&state.read_db)
    .await
    .unwrap();
    let (status, _) = req(
        &state,
        &token,
        "POST",
        &format!("/api/v1/library/artists/{artist_id}"),
        Some(json!({"name": "Björk", "rating": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body2) = req(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "Artist plays?"})),
    )
    .await;
    assert_eq!(body2["sql"]["rows"], json!([["Björk", 2]]));
    let as_of_2 = body2["sql"]["as_of_ms"].as_i64().unwrap();
    assert!(as_of_2 > as_of_1, "{as_of_2} <= {as_of_1}");

    // 3. Review decision trashes one duplicate play.
    let listen_ids: Vec<(String, i64)> =
        sqlx::query_as("SELECT id, revision FROM listens ORDER BY played_at_ms ASC")
            .fetch_all(&state.read_db)
            .await
            .unwrap();
    sqlx::query("UPDATE listens SET needs_review = 1")
        .execute(&state.db)
        .await
        .unwrap();
    let (status, _) = req(
        &state,
        &token,
        "POST",
        "/api/v1/review",
        Some(json!({
            "ids": [listen_ids[0].0, listen_ids[1].0],
            "revisions": [listen_ids[0].1, listen_ids[1].1],
            "keep_id": listen_ids[0].0,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body3) = req(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "Artist plays?"})),
    )
    .await;
    assert_eq!(body3["sql"]["rows"], json!([["Björk", 1]]));
    let as_of_3 = body3["sql"]["as_of_ms"].as_i64().unwrap();
    assert!(as_of_3 > as_of_2, "{as_of_3} <= {as_of_2}");

    // 4. Calendar change updates timezone and week_start in meta.
    let (_, prefs) = req(&state, &token, "GET", "/api/v1/preferences", None).await;
    let (status, saved_prefs) = req(
        &state,
        &token,
        "POST",
        "/api/v1/preferences",
        Some(json!({
            "preferences": {
                "name": prefs["name"],
                "initial_period": prefs["initial_period"],
                "hour_format": prefs["hour_format"],
                "timezone": "America/New_York",
                "week_start": 6,
                "version": prefs["version"],
            },
            "calendar_scope": "future",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved_prefs}");

    *mock.reply.lock().unwrap() = json!({
        "sql": "SELECT timezone, week_start FROM meta",
        "title": "Calendar",
        "computed": "Current projection timezone and week start.",
        "answer_kind": "table",
        "columns": [{"name": "timezone", "format": "text"}, {"name": "week_start", "format": "count"}]
    })
    .to_string();
    let (_, body4) = req(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "What calendar?"})),
    )
    .await;
    assert_eq!(body4["sql"]["rows"], json!([["America/New_York", 6]]));
    let as_of_4 = body4["sql"]["as_of_ms"].as_i64().unwrap();
    assert!(as_of_4 > as_of_3, "{as_of_4} <= {as_of_3}");
}

#[tokio::test]
async fn disabling_ai_deletes_projection_and_storage_and_backups_stay_clean() {
    let mock = MockSql::default();
    *mock.reply.lock().unwrap() = json!({
        "sql": "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1",
        "title": "Plays",
        "computed": "Total counted plays.",
        "answer_kind": "value",
        "columns": [{"name": "plays", "format": "count"}]
    })
    .to_string();
    let url = mock_serve(mock).await;
    let (dir, state, token) = setup_remote_sql(&url).await;

    let mut tx = state.storage.writer.begin().await.unwrap();
    db::ingest(
        &mut tx,
        &[make_play(
            "spotify_api",
            "p1",
            "2024-06-01T10:00:00Z",
            "Björk",
            "Jóga",
        )],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Ask once so the projection is built on disk.
    let (status, body) = req(
        &state,
        &token,
        "POST",
        "/api/v1/ask",
        Some(json!({"question": "Plays?"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let proj_path = projection::path(dir.path());
    assert!(proj_path.is_file());
    let actual_size = std::fs::metadata(&proj_path).unwrap().len();
    assert!(actual_size > 0);

    // Storage endpoint reports projection_bytes and includes it in total_storage_bytes.
    let (status, app) = req(&state, &token, "GET", "/api/v1/application", None).await;
    assert_eq!(status, StatusCode::OK, "{app}");
    assert_eq!(app["projection_bytes"], actual_size);
    assert!(
        app["total_storage_bytes"].as_u64().unwrap()
            >= app["archive_bytes"].as_u64().unwrap() + actual_size
    );

    // Backups must exclude ask_projection.db.
    let work = tempfile::tempdir().unwrap();
    let backup_zip = work.path().join("backup.zip");
    recovery::create_backup(&state.storage, work.path(), &backup_zip)
        .await
        .unwrap();
    let archive = zip::ZipArchive::new(std::fs::File::open(&backup_zip).unwrap()).unwrap();
    for name in archive.file_names() {
        let name = name.unwrap();
        assert!(
            name == "archive.db" || name == "manifest.json" || name.starts_with("spool/"),
            "unexpected backup entry {name}"
        );
    }

    // Exports must exclude projection tables.
    let frozen = work.path().join("frozen.db");
    recovery::freeze(&state.storage, &frozen).await.unwrap();
    let export_json = work.path().join("export.json");
    exports::write(&frozen, &export_json, "json", &ExportSelection::default())
        .await
        .unwrap();
    let exported = std::fs::read_to_string(&export_json).unwrap();
    assert!(!exported.contains("ask_projection"));
    assert!(!exported.contains("_projection_meta"));

    // Disabling AI immediately deletes ask_projection.db and reports 0 bytes.
    let (status, _) = req(
        &state,
        &token,
        "POST",
        "/api/v1/ai/settings",
        Some(json!({
            "enabled": false,
            "provider": "remote",
            "remote_url": url,
            "remote_model": "test-sql",
            "active_model": null,
            "threads": 4,
            "unload_minutes": 15,
            "model_dir": null,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!proj_path.exists());

    let (_, app_after) = req(&state, &token, "GET", "/api/v1/application", None).await;
    assert_eq!(app_after["projection_bytes"], 0);
}

#[tokio::test]
async fn restore_activation_removes_stale_projection() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let mut tx = storage.writer.begin().await.unwrap();
    db::set_setting(&mut tx, "ai_enabled", "1").await.unwrap();
    db::ingest(
        &mut tx,
        &[make_play(
            "spotify_api",
            "old-1",
            "2024-01-01T10:00:00Z",
            "Old Artist",
            "Old Track",
        )],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Build initial projection and create a backup of it.
    let gate = projection::ProjectionGate::default();
    let proj_path = projection::ensure(&storage, &gate).await.unwrap();
    assert!(proj_path.is_file());

    let backup_dir = tempfile::tempdir().unwrap();
    let backup_zip = backup_dir.path().join("backup.zip");
    recovery::create_backup(&storage, backup_dir.path(), &backup_zip)
        .await
        .unwrap();

    // Import a backup asset and prepare a restore onto the same data dir.
    let asset_id = recovery::enqueue_asset(&storage, "backup", "zip", json!({}))
        .await
        .unwrap();
    let asset = recovery::asset(&storage, &asset_id).await.unwrap();
    let dest = recovery::asset_path(&storage, &asset).unwrap();
    std::fs::copy(&backup_zip, &dest).unwrap();
    sqlx::query("UPDATE recovery_assets SET state='ready' WHERE id=?")
        .bind(&asset_id)
        .execute(&storage.writer)
        .await
        .unwrap();
    // Built at runtime from the asset id: not a credential, and not a literal.
    let restore_password = format!("{asset_id}-Restored-Pass-9!");
    recovery::prepare_restore(&storage, &asset_id, restore_password)
        .await
        .unwrap();
    storage.close().await;

    // Opening storage activates the pending restore and must remove the stale projection.
    assert!(dir.path().join("recovery.pending").exists());
    let reopened = db::open(dir.path()).await.unwrap();
    assert!(
        !projection::path(dir.path()).exists(),
        "stale projection must be removed during restore activation"
    );
    reopened.close().await;
}
