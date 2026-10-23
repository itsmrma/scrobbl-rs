use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use scrobbl_rs::{
    AppState, auth,
    config::Config,
    db::{self, DurationKind, Event},
    recovery, storage, web,
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn setup() -> (tempfile::TempDir, AppState, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
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
    sqlx::query("INSERT INTO sessions VALUES(?,'csrf',9999999999999)")
        .bind(auth::digest(&token))
        .execute(&store.private)
        .await
        .unwrap();
    let state = AppState::new(store, config);
    seed(&state, "Source title").await;
    (dir, state, token)
}
async fn seed(state: &AppState, title: &str) {
    let raw = json!({"uri":"spotify:track:0000000000000000000001","type":"track","name":title,"duration_ms":180000,"album":{"id":"0000000000000000000002","name":"Source album","images":[]},"artists":[{"id":"0000000000000000000003","name":"Source artist"}]});
    let event = Event {
        source: "spotify_api",
        source_key: "one".into(),
        source_timestamp_ms: 1000000,
        played_at_ms: 1000000,
        listened_ms: Some(180000),
        duration_kind: DurationKind::Estimated,
        counted: true,
        track: scrobbl_rs::spotify::parse_track(&raw).unwrap(),
        raw,
    };
    let mut tx = state.db.begin().await.unwrap();
    db::ingest(&mut tx, &[event]).await.unwrap();
    tx.commit().await.unwrap();
}
async fn call(
    state: &AppState,
    token: &str,
    method: &str,
    path: &str,
    body: Vec<u8>,
    csrf: bool,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:3210")
        .header("origin", "http://127.0.0.1:3210")
        .header("cookie", format!("scrobbl_session={token}"))
        .header("content-type", "application/json");
    if csrf {
        request = request.header("X-CSRF-Token", "csrf");
    }
    let response = web::router(state.clone())
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    (
        response.status(),
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
}
async fn get(state: &AppState, token: &str, path: &str) -> Value {
    let (status, body) = call(state, token, "GET", path, vec![], false).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_slice(&body).unwrap()
}
#[tokio::test]
async fn corrections_overlay_sources_propagate_to_reads_survive_reingest_and_can_be_removed() {
    let (_dir, state, token) = setup().await;
    let dash = get(&state, &token, "/api/dashboard?period=all").await;
    for kind in ["artists", "albums", "tracks"] {
        let id = dash["rankings"][kind][0]["id"].as_str().unwrap();
        let path = format!("/api/v1/library/{kind}/{id}");
        let original = get(&state, &token, &path).await;
        let body =
            serde_json::to_vec(&json!({"name":format!("Personal {kind}"),"rating":4.5})).unwrap();
        assert_eq!(
            call(&state, &token, "POST", &path, body.clone(), false)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(&state, &token, "POST", &path, body, true).await.0,
            StatusCode::OK
        );
        let edited = get(&state, &token, &path).await;
        assert_eq!(edited["rating"], 4.5);
        assert_eq!(edited["original_name"], original["original_name"]);
        seed(&state, "Updated by provider").await;
        let dash = get(&state, &token, "/api/dashboard?period=all").await;
        assert_eq!(
            dash["rankings"][kind][0]["name"],
            format!("Personal {kind}")
        );
        if kind == "artists" {
            let history = get(&state, &token, "/api/history?period=all").await;
            assert_eq!(
                history["items"][0]["artists"][0]["name"],
                "Personal artists"
            );
            let choices = get(
                &state,
                &token,
                "/api/v1/consultation/choices/artists?q=Personal",
            )
            .await;
            assert_eq!(choices[0]["name"], "Personal artists");
        }
        assert_eq!(
            call(
                &state,
                &token,
                "POST",
                &path,
                serde_json::to_vec(&json!({"name":null,"rating":null})).unwrap(),
                true
            )
            .await
            .0,
            StatusCode::OK
        );
        let reset = get(&state, &token, &path).await;
        assert_eq!(reset["name"], reset["original_name"]);
        assert!(reset["rating"].is_null());
    }
    state.storage.close().await;
}
#[tokio::test]
async fn ratings_validate_half_steps_and_image_urls_reject_local_targets_without_changing_saved_image()
 {
    let (_dir, state, token) = setup().await;
    let dash = get(&state, &token, "/api/dashboard").await;
    let id = dash["rankings"]["artists"][0]["id"].as_str().unwrap();
    for rating in [0.0, -0.5, 5.5, 3.3] {
        assert_eq!(
            call(
                &state,
                &token,
                "POST",
                &format!("/api/v1/library/artists/{id}"),
                serde_json::to_vec(&json!({"name":null,"rating":rating})).unwrap(),
                true
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            &format!("/api/v1/library/artists/{id}"),
            serde_json::to_vec(&json!({"name":null,"rating":0.5})).unwrap(),
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    // Valid image signature; active document types cannot be stored or served.
    let png = b"\x89PNG\r\n\x1a\nlocal image bytes".to_vec();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            &format!("/api/v1/artwork/artists/{id}/upload"),
            png.clone(),
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    for url in [
        "https://127.0.0.1/secret",
        "http://example.com/image.png",
        "https://user:pass@example.com/image.png",
    ] {
        assert_eq!(
            call(
                &state,
                &token,
                "POST",
                &format!("/api/v1/artwork/artists/{id}"),
                serde_json::to_vec(&json!({"url":url})).unwrap(),
                true
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            &format!("/api/v1/artwork/artists/{id}/upload"),
            b"<svg onload='alert(1)'/>".to_vec(),
            true
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let image = call(
        &state,
        &token,
        "GET",
        &format!("/api/v1/artwork/artists/{id}"),
        vec![],
        false,
    )
    .await;
    assert_eq!(image.0, StatusCode::OK);
    assert_eq!(image.1, png);
    sqlx::query("UPDATE archive_state SET maintenance=1")
        .execute(&state.db)
        .await
        .unwrap();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            &format!("/api/v1/artwork/artists/{id}/upload"),
            png,
            true
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    state.storage.close().await;
}
#[tokio::test]
async fn artwork_precedence_expiry_and_portable_backup_preserve_owner_data() {
    let (_dir, state, token) = setup().await;
    let dash = get(&state, &token, "/api/dashboard").await;
    let id = dash["rankings"]["artists"][0]["id"].as_str().unwrap();
    sqlx::query("INSERT INTO artwork VALUES('artists',?,'spotify',?,'image/png',?,?)")
        .bind(id)
        .bind(b"\x89PNG\r\n\x1a\ncached provider image".as_slice())
        .bind(auth::now_ms())
        .bind(auth::now_ms() + 86400000)
        .execute(&state.db)
        .await
        .unwrap();
    let path = format!("/api/v1/artwork/artists/{id}");
    for _ in 0..3 {
        assert_eq!(
            call(
                &state,
                &token,
                "GET",
                &format!("{path}?cached=true"),
                vec![],
                false
            )
            .await
            .1,
            b"\x89PNG\r\n\x1a\ncached provider image"
        );
        assert_eq!(
            call(&state, &token, "GET", &path, vec![], false).await.1,
            b"\x89PNG\r\n\x1a\ncached provider image"
        );
    }
    let png = b"\x89PNG\r\n\x1a\nowner bytes".to_vec();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            &format!("{path}/upload"),
            png.clone(),
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, &token, "GET", &path, vec![], false).await.1,
        png
    );
    sqlx::query("UPDATE artwork SET expires_at_ms=1 WHERE source='spotify'")
        .execute(&state.db)
        .await
        .unwrap();
    scrobbl_rs::retention::cleanup(&state.storage)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artwork WHERE source='spotify'")
        .fetch_one(&state.read_db)
        .await
        .unwrap();
    assert_eq!(left, 0);
    call(
        &state,
        &token,
        "POST",
        &format!("/api/v1/library/artists/{id}"),
        serde_json::to_vec(&json!({"name":"Personal artist","rating":2.5})).unwrap(),
        true,
    )
    .await;
    let work = tempfile::tempdir().unwrap();
    let package = work.path().join("backup.zip");
    recovery::create_backup(&state.storage, work.path(), &package)
        .await
        .unwrap();
    let trial = tempfile::tempdir().unwrap();
    recovery::trial_restore(&package, trial.path())
        .await
        .unwrap();
    let mut copy = recovery::connect(&trial.path().join("archive.db"), true)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_corrections WHERE entity_id=?")
        .bind(id)
        .fetch_one(&mut copy)
        .await
        .unwrap();
    assert_eq!(count, 2);
    let saved: Vec<u8> = sqlx::query_scalar("SELECT bytes FROM artwork WHERE source='custom'")
        .fetch_one(&mut copy)
        .await
        .unwrap();
    assert_eq!(saved, png);
    state.storage.close().await;
}

#[tokio::test]
async fn temporary_spotify_limit_does_not_cache_missing_artwork() {
    let (_dir, state, token) = setup().await;
    sqlx::query("INSERT INTO spotify_tokens VALUES(1,'unused','unused',9999999999999)")
        .execute(&state.private_db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO settings(key,value) VALUES('retry_at_ms',?)")
        .bind((auth::now_ms() + 60000).to_string())
        .execute(&state.db)
        .await
        .unwrap();
    let dash = get(&state, &token, "/api/dashboard").await;
    for kind in ["artists", "albums"] {
        let id = dash["rankings"][kind][0]["id"].as_str().unwrap();
        let (status, _) = call(
            &state,
            &token,
            "GET",
            &format!("/api/v1/artwork/{kind}/{id}"),
            vec![],
            false,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let cached: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM artwork WHERE kind=? AND entity_id=?")
                .bind(kind)
                .bind(id)
                .fetch_one(&state.read_db)
                .await
                .unwrap();
        assert_eq!(
            cached, 0,
            "temporary failure must not become a missing-image cache entry"
        );
    }
    state.storage.close().await;
}

#[tokio::test]
async fn artwork_cache_probe_never_contacts_spotify_and_upgrade_preserves_custom_images() {
    let (_dir, state, token) = setup().await;
    let dash = get(&state, &token, "/api/dashboard").await;
    let id = dash["rankings"]["albums"][0]["id"].as_str().unwrap();
    let path = format!("/api/v1/artwork/albums/{id}?cached=true");
    assert_eq!(
        call(&state, &token, "GET", &path, vec![], false).await.0,
        StatusCode::NOT_FOUND
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artwork")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    for source in ["custom", "spotify"] {
        sqlx::query("INSERT INTO artwork VALUES('albums',?,?,?,'image/png',0,9999999999999)")
            .bind(id)
            .bind(source)
            .bind(b"\x89PNG\r\n\x1a\nimage".as_slice())
            .execute(&state.db)
            .await
            .unwrap();
    }
    sqlx::raw_sql(include_str!(
        "../release-migrations/0013_album_artwork_quality.sql"
    ))
    .execute(&state.db)
    .await
    .unwrap();
    assert_eq!(
        call(&state, &token, "GET", &path, vec![], false).await.1,
        b"\x89PNG\r\n\x1a\nimage"
    );
    let sources: Vec<String> = sqlx::query_scalar("SELECT source FROM artwork")
        .fetch_all(&state.db)
        .await
        .unwrap();
    assert_eq!(sources, vec!["custom"]);
    state.storage.close().await;
}

#[tokio::test]
async fn downloaded_spotify_artwork_survives_cleanup_restart_and_backup() {
    let (dir, state, token) = setup().await;
    let dash = get(&state, &token, "/api/dashboard").await;
    let id = dash["rankings"]["artists"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let png = b"\x89PNG\r\n\x1a\npermanent provider image";
    sqlx::query("INSERT INTO artwork VALUES('artists',?,'spotify',?,'image/png',0,1)")
        .bind(&id)
        .bind(png.as_slice())
        .execute(&state.db)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../release-migrations/0014_persistent_spotify_artwork.sql"
    ))
    .execute(&state.db)
    .await
    .unwrap();
    scrobbl_rs::retention::cleanup(&state.storage)
        .await
        .unwrap();
    let expires: Option<i64> =
        sqlx::query_scalar("SELECT expires_at_ms FROM artwork WHERE entity_id=?")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert!(expires.is_none());
    let config = state.config.clone();
    state.storage.close().await;
    let store = storage::open(dir.path()).await.unwrap();
    let state = AppState::new(store, config);
    assert_eq!(
        call(
            &state,
            &token,
            "GET",
            &format!("/api/v1/artwork/artists/{id}?cached=true"),
            vec![],
            false
        )
        .await
        .1,
        png
    );
    let work = tempfile::tempdir().unwrap();
    let backup = work.path().join("artwork.zip");
    recovery::create_backup(&state.storage, work.path(), &backup)
        .await
        .unwrap();
    let restored = tempfile::tempdir().unwrap();
    recovery::trial_restore(&backup, restored.path())
        .await
        .unwrap();
    let mut copy = recovery::connect(&restored.path().join("archive.db"), true)
        .await
        .unwrap();
    let saved: Vec<u8> =
        sqlx::query_scalar("SELECT bytes FROM artwork WHERE entity_id=? AND source='spotify'")
            .bind(&id)
            .fetch_one(&mut copy)
            .await
            .unwrap();
    assert_eq!(saved, png);
    state.storage.close().await;
}
