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
async fn disconnect_retires_new_spotify_cache_but_preserves_legacy_and_personal_images() {
    let (_dir, state, token) = setup().await;
    for (id, source, expires, provenance) in [
        (
            "new",
            "spotify",
            Some(auth::now_ms() + 86400000),
            Some("{}"),
        ),
        ("legacy", "spotify", None, None),
        ("personal", "custom", None, None),
    ] {
        sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms,provenance_json) VALUES('artists',?,?,?,'image/png',0,?,?)")
            .bind(id).bind(source).bind(b"\x89PNG\r\n\x1a\nimage".as_slice()).bind(expires).bind(provenance).execute(&state.db).await.unwrap();
    }
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/spotify/disconnect",
            b"{}".to_vec(),
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    let kept: Vec<String> = sqlx::query_scalar("SELECT entity_id FROM artwork ORDER BY entity_id")
        .fetch_all(&state.db)
        .await
        .unwrap();
    assert_eq!(kept, vec!["legacy", "personal"]);
    state.storage.close().await;
}
#[tokio::test]
async fn image_provider_settings_require_owner_csrf_validate_order_and_survive_restart() {
    let (dir, state, token) = setup().await;
    let path = "/api/v1/artwork-providers";
    assert_eq!(
        call(&state, "invalid", "GET", path, vec![], false).await.0,
        StatusCode::UNAUTHORIZED
    );
    let mut settings = get(&state, &token, path).await;
    assert_eq!(settings["version"], 0);
    let original = serde_json::to_vec(&settings).unwrap();
    assert_eq!(
        call(&state, &token, "POST", path, original.clone(), false)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let mut invalid = settings.clone();
    invalid["providers"][0] = invalid["providers"][1].clone();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            path,
            serde_json::to_vec(&invalid).unwrap(),
            true
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut invalid = settings.clone();
    invalid["providers"].as_array_mut().unwrap().pop();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            path,
            serde_json::to_vec(&invalid).unwrap(),
            true
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    settings["providers"].as_array_mut().unwrap().swap(1, 2);
    for provider in settings["providers"].as_array_mut().unwrap() {
        provider["enabled"] = json!(false);
    }
    // Saved images and personal images must stay byte-for-byte intact when saving policy.
    sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms) VALUES('artists','saved','spotify',?,'image/png',0),('artists','custom','custom',?,'image/png',0)").bind(b"\x89PNG\r\n\x1a\nsaved".as_slice()).bind(b"\x89PNG\r\n\x1a\ncustom".as_slice()).execute(&state.db).await.unwrap();
    let before: Vec<(String, Vec<u8>)> =
        sqlx::query_as("SELECT entity_id,bytes FROM artwork ORDER BY entity_id")
            .fetch_all(&state.db)
            .await
            .unwrap();
    let (status, body) = call(
        &state,
        &token,
        "POST",
        path,
        serde_json::to_vec(&settings).unwrap(),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    settings["version"] = json!(1);
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), settings);
    assert_eq!(
        call(&state, &token, "POST", path, original, true).await.0,
        StatusCode::CONFLICT
    );
    let after: Vec<(String, Vec<u8>)> =
        sqlx::query_as("SELECT entity_id,bytes FROM artwork ORDER BY entity_id")
            .fetch_all(&state.db)
            .await
            .unwrap();
    assert_eq!(before, after);
    // Disabling all providers lets the real background queue complete without a Spotify connection or network.
    let lease = state.storage.claim("artwork").await.unwrap().unwrap();
    assert!(scrobbl_rs::artwork::step(&state, &lease).await.unwrap());
    let misses: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artwork_misses")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(misses, 1);
    sqlx::query("UPDATE archive_state SET maintenance=1")
        .execute(&state.db)
        .await
        .unwrap();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            path,
            serde_json::to_vec(&settings).unwrap(),
            true
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE archive_state SET maintenance=0")
        .execute(&state.db)
        .await
        .unwrap();
    let config = state.config.clone();
    state.storage.close().await;
    let state = AppState::new(storage::open(dir.path()).await.unwrap(), config);
    assert_eq!(get(&state, &token, path).await, settings);
    state.storage.close().await;
}
#[tokio::test]
async fn account_profile_and_application_controls_require_owner_and_csrf() {
    let (_dir, state, token) = setup().await;
    for path in [
        "/api/v1/account",
        "/api/v1/account/avatar",
        "/api/v1/application",
    ] {
        assert_eq!(
            call(&state, "invalid", "GET", path, vec![], false).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    let edit = serde_json::to_vec(&json!({"name":"Music owner"})).unwrap();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/account",
            edit.clone(),
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&state, &token, "POST", "/api/v1/account", edit, true)
            .await
            .0,
        StatusCode::OK
    );
    let mut png = b"\x89PNG\r\n\x1a\nprofile image".to_vec();
    png.resize(32 * 1024, 0);
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/account/avatar",
            b"<svg/>".to_vec(),
            true
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/account/avatar",
            png.clone(),
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/account/avatar",
            png.clone(),
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            &token,
            "GET",
            "/api/v1/account/avatar",
            vec![],
            false
        )
        .await
        .1,
        png
    );
    let profile = get(&state, &token, "/api/v1/account").await;
    assert_eq!(profile["name"], "Music owner");
    assert!(
        profile["avatar_url"]
            .as_str()
            .unwrap()
            .starts_with("/api/v1/account/avatar?v=")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM settings WHERE key LIKE 'profile_%'")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        0
    );
    for (body, status) in [
        (
            json!({"action":"restart","confirmed":false}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"action":"running","confirmed":true}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(
            call(
                &state,
                &token,
                "POST",
                "/api/v1/application",
                serde_json::to_vec(&body).unwrap(),
                true
            )
            .await
            .0,
            status
        );
    }
    let body = serde_json::to_vec(&json!({"action":"restart","confirmed":true})).unwrap();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/application",
            body.clone(),
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        *state.application.action.borrow(),
        scrobbl_rs::account::Action::Running
    );
    assert_eq!(
        call(&state, &token, "POST", "/api/v1/application", body, true)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        *state.application.action.borrow(),
        scrobbl_rs::account::Action::Restart
    );
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/account/avatar/remove",
            b"{}".to_vec(),
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            &token,
            "GET",
            "/api/v1/account/avatar",
            vec![],
            false
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn changing_owner_password_revokes_other_sessions_and_preserves_current_access() {
    use argon2::PasswordHasher;
    let (_dir, state, token) = setup().await;
    let old = "a local browser test password 7!";
    let new = "new personal music pass 9!";
    let hash = argon2::Argon2::default()
        .hash_password(old.as_bytes())
        .unwrap()
        .to_string();
    sqlx::query("INSERT INTO owner VALUES(1,?,0)")
        .bind(hash)
        .execute(&state.private_db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO settings VALUES('owner_username','owner')")
        .execute(&state.private_db)
        .await
        .unwrap();
    let other = auth::random_secret();
    sqlx::query("INSERT INTO sessions VALUES(?,'other-csrf',9999999999999)")
        .bind(auth::digest(&other))
        .execute(&state.private_db)
        .await
        .unwrap();
    let body = serde_json::to_vec(&json!({"current_password":"wrong","password":new})).unwrap();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/account/password",
            body,
            true
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&state, &other, "GET", "/api/v1/account", vec![], false)
            .await
            .0,
        StatusCode::OK
    );
    let body = serde_json::to_vec(&json!({"current_password":old,"password":new})).unwrap();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/account/password",
            body.clone(),
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/account/password",
            body,
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, &token, "GET", "/api/v1/account", vec![], false)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&state, &other, "GET", "/api/v1/account", vec![], false)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    for (password, status) in [(old, StatusCode::UNAUTHORIZED), (new, StatusCode::OK)] {
        let body = serde_json::to_vec(&json!({"username":"owner","password":password})).unwrap();
        assert_eq!(
            call(&state, "", "POST", "/api/login", body, false).await.0,
            status
        );
    }
}

#[tokio::test]
async fn duplicate_review_is_explicit_recoverable_and_keeps_originals() {
    let (_dir, state, token) = setup().await;
    let raw = json!({"uri":"spotify:track:0000000000000000000001","type":"track","name":"Source title","duration_ms":180000,"album":{"id":"0000000000000000000002","name":"Source album","images":[]},"artists":[{"id":"0000000000000000000003","name":"Source artist"}]});
    let event = Event {
        source: "listenbrainz",
        source_key: "lb:one:0".into(),
        source_timestamp_ms: 1000000,
        played_at_ms: 1000000,
        listened_ms: None,
        duration_kind: DurationKind::Unknown,
        counted: true,
        track: scrobbl_rs::spotify::parse_track(&raw).unwrap(),
        raw,
    };
    let mut tx = state.db.begin().await.unwrap();
    db::ingest(&mut tx, std::slice::from_ref(&event))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let review = get(&state, &token, "/api/v1/review").await;
    assert_eq!(review["total"], 2);
    let items = review["groups"][0]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let ids: Vec<_> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    let revisions: Vec<_> = items
        .iter()
        .map(|i| i["revision"].as_i64().unwrap())
        .collect();
    let action = json!({"ids":ids,"revisions":revisions,"keep_id":ids[0]});
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/review",
            serde_json::to_vec(&action).unwrap(),
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, body) = call(
        &state,
        &token,
        "POST",
        "/api/v1/review",
        serde_json::to_vec(&action).unwrap(),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let decision: Value = serde_json::from_slice(&body).unwrap();
    let trash = get(&state, &token, "/api/v1/review/trash").await;
    assert_eq!(trash["total"], 1);
    assert_eq!(trash["items"][0]["decision_id"], decision["decision_id"]);
    assert_eq!(trash["items"][0]["title"], "Source title");
    assert_eq!(
        get(&state, &token, "/api/v1/review/trash?page=1").await["items"],
        json!([])
    );
    assert_eq!(
        call(
            &state,
            "invalid",
            "GET",
            "/api/v1/review/trash",
            vec![],
            false
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(get(&state, &token, "/api/v1/review").await["total"], 0);
    assert_eq!(
        get(&state, &token, "/api/history?period=all&status=all").await["total"],
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM observations")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM observation_variants")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        2
    );
    // Replaying either source must not resurrect the discarded record.
    let mut tx = state.db.begin().await.unwrap();
    db::ingest(&mut tx, &[event]).await.unwrap();
    tx.commit().await.unwrap();
    seed(&state, "Source title").await;
    assert_eq!(get(&state, &token, "/api/v1/review").await["total"], 0);
    let track = items[0]["track_id"].as_str().unwrap();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            &format!("/api/v1/library/tracks/{track}"),
            serde_json::to_vec(&json!({"name":"Personal reviewed title","rating":4.5})).unwrap(),
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    let undo = format!(
        "/api/v1/review/{}/undo",
        decision["decision_id"].as_str().unwrap()
    );
    assert_eq!(
        call(&state, &token, "POST", &undo, b"{}".to_vec(), true)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(get(&state, &token, "/api/v1/review").await["total"], 2);
    assert_eq!(
        get(&state, &token, "/api/v1/review/trash").await["total"],
        0
    );
    assert_eq!(
        get(&state, &token, "/api/history?period=all&status=all").await["total"],
        2
    );
    assert_eq!(
        get(&state, &token, "/api/history?period=all&status=all").await["items"][0]["title"],
        "Personal reviewed title"
    );
    assert_eq!(
        call(&state, &token, "POST", &undo, b"{}".to_vec(), true)
            .await
            .0,
        StatusCode::CONFLICT
    );
    // The old comparison cannot overwrite a newer revision after undo.
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/review",
            serde_json::to_vec(&action).unwrap(),
            true
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let review = get(&state, &token, "/api/v1/review").await;
    let items = review["groups"][0]["items"].as_array().unwrap();
    let separate = json!({"ids":items.iter().map(|i|i["id"].clone()).collect::<Vec<_>>(), "revisions":items.iter().map(|i|i["revision"].clone()).collect::<Vec<_>>(),"keep_id":null});
    let before_revision: i64 =
        sqlx::query_scalar("SELECT revision FROM archive_state WHERE singleton=1")
            .fetch_one(&state.db)
            .await
            .unwrap();
    let mut stale = separate.clone();
    stale["revisions"][1] = json!(-1);
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/review",
            serde_json::to_vec(&stale).unwrap(),
            true
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT revision FROM archive_state WHERE singleton=1")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        before_revision
    );
    assert_eq!(get(&state, &token, "/api/v1/review").await["total"], 2);
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/review",
            serde_json::to_vec(&separate).unwrap(),
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        get(&state, &token, "/api/history?period=all&status=all").await["total"],
        2
    );
    assert_eq!(get(&state, &token, "/api/v1/review").await["total"], 0);
    let current = get(&state, &token, "/api/v1/review").await;
    let latest = current["recent"][0]["id"].as_str().unwrap();
    seed(&state, "Changed source evidence").await;
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            &format!("/api/v1/review/{latest}/undo"),
            b"{}".to_vec(),
            true
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(get(&state, &token, "/api/v1/review").await["total"], 0);
}
async fn review_copy(
    state: &AppState,
    source: &'static str,
    key: &str,
    at: i64,
    title: &str,
    spotify: &str,
) {
    let raw = json!({"uri":spotify,"type":"track","name":title,"duration_ms":180000,"album":{"id":"0000000000000000000002","name":"Source album","images":[]},"artists":[{"id":"0000000000000000000003","name":"Source artist"}]});
    let event = Event {
        source,
        source_key: key.into(),
        source_timestamp_ms: at,
        played_at_ms: at,
        listened_ms: None,
        duration_kind: DurationKind::Unknown,
        counted: true,
        track: scrobbl_rs::spotify::parse_track(&raw).unwrap(),
        raw,
    };
    let mut tx = state.db.begin().await.unwrap();
    db::ingest(&mut tx, &[event]).await.unwrap();
    tx.commit().await.unwrap();
    // Fixtures also exercise candidates deliberately left for manual review.
    sqlx::query("UPDATE listens SET needs_review=1")
        .execute(&state.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn automatic_review_revalidates_preserves_originals_and_undoes_whole_batch() {
    let (_dir, state, token) = setup().await;
    review_copy(
        &state,
        "listenbrainz",
        "auto:one:0",
        1000000,
        "Source title",
        "spotify:track:0000000000000000000001",
    )
    .await;
    review_copy(
        &state,
        "spotify_api",
        "auto:two",
        2000000,
        "Source title",
        "spotify:track:0000000000000000000001",
    )
    .await;
    review_copy(
        &state,
        "listenbrainz",
        "auto:two:0",
        2000999,
        "Source title",
        "spotify:track:0000000000000000000001",
    )
    .await;
    let preview = get(&state, &token, "/api/v1/review/automatic").await;
    assert_eq!(preview["groups"], 2);
    assert_eq!(preview["records"], 4);
    assert_eq!(preview["remaining"], 0);
    assert_eq!(get(&state, &token, "/api/v1/review").await["total"], 4); // GET never changes anything.
    let action = serde_json::to_vec(&json!({"token":preview["token"]})).unwrap();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/review/automatic",
            action.clone(),
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let first: String = sqlx::query_scalar("SELECT id FROM listens ORDER BY id LIMIT 1")
        .fetch_one(&state.db)
        .await
        .unwrap();
    sqlx::query("UPDATE listens SET revision=revision+1 WHERE id=?")
        .bind(&first)
        .execute(&state.db)
        .await
        .unwrap();
    assert_eq!(
        call(
            &state,
            &token,
            "POST",
            "/api/v1/review/automatic",
            action,
            true
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(get(&state, &token, "/api/v1/review").await["total"], 4);
    let fresh = get(&state, &token, "/api/v1/review/automatic").await;
    let before: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id,observation_id,fingerprint FROM observation_variants ORDER BY id",
    )
    .fetch_all(&state.db)
    .await
    .unwrap();
    let (status, body) = call(
        &state,
        &token,
        "POST",
        "/api/v1/review/automatic",
        serde_json::to_vec(&json!({"token":fresh["token"]})).unwrap(),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["removed"], 2);
    let trash = get(&state, &token, "/api/v1/review/trash").await;
    assert_eq!(trash["total"], 2);
    assert!(
        trash["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["decision_id"] == result["decision_id"])
    );
    assert_eq!(get(&state, &token, "/api/v1/review").await["total"], 0);
    assert_eq!(
        get(&state, &token, "/api/history?period=all&status=all").await["total"],
        2
    );
    let retained: String = sqlx::query_scalar(
        "SELECT duration_kind FROM listens WHERE played_at_ms=1000000 AND trashed_at_ms IS NULL",
    )
    .fetch_one(&state.db)
    .await
    .unwrap();
    assert_eq!(retained, "estimated");
    let after: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id,observation_id,fingerprint FROM observation_variants ORDER BY id",
    )
    .fetch_all(&state.db)
    .await
    .unwrap();
    assert_eq!(before, after);
    seed(&state, "Source title").await; // Replaying originals must not undo the decision.
    assert_eq!(get(&state, &token, "/api/v1/review").await["total"], 0);
    let undo = format!(
        "/api/v1/review/{}/undo",
        result["decision_id"].as_str().unwrap()
    );
    assert_eq!(
        call(&state, &token, "POST", &undo, b"{}".to_vec(), true)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(get(&state, &token, "/api/v1/review").await["total"], 4);
    assert_eq!(
        call(&state, &token, "POST", &undo, b"{}".to_vec(), true)
            .await
            .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn automatic_review_excludes_boundary_repeats_conflicts_and_distinct_identities() {
    for case in [
        "one_second",
        "competing",
        "identity",
        "evidence",
        "countability",
        "actual_duration",
    ] {
        let (_dir, state, token) = setup().await;
        let at = if case == "one_second" {
            1001000
        } else {
            1000000
        };
        let source = if case == "same_source" {
            "spotify_api"
        } else {
            "listenbrainz"
        };
        let spotify = if case == "identity" {
            "spotify:track:0000000000000000000009"
        } else {
            "spotify:track:0000000000000000000001"
        };
        review_copy(
            &state,
            source,
            "excluded:one:0",
            at,
            "Source title",
            spotify,
        )
        .await;
        if case == "competing" {
            review_copy(
                &state,
                "listenbrainz",
                "excluded:two:0",
                1000500,
                "Source title",
                spotify,
            )
            .await;
        }
        if case == "evidence" {
            sqlx::query("INSERT INTO conflicts(id,listen_id,kind,evidence_json) SELECT 'conflict',id,'actual_duration','{}' FROM listens LIMIT 1").execute(&state.db).await.unwrap();
        }
        if case == "countability" {
            sqlx::query("UPDATE listens SET counted=0 WHERE anchor_source='listenbrainz'")
                .execute(&state.db)
                .await
                .unwrap();
        }
        if case == "actual_duration" {
            sqlx::query("UPDATE listens SET duration_kind='actual',listened_ms=CASE WHEN anchor_source='listenbrainz' THEN 90000 ELSE 180000 END").execute(&state.db).await.unwrap();
        }
        assert_eq!(
            get(&state, &token, "/api/v1/review/automatic").await["groups"],
            0,
            "{case}"
        );
    }
}

#[tokio::test]
async fn automatic_review_includes_same_source_duplicates_within_one_second() {
    let (_dir, state, token) = setup().await;
    review_copy(
        &state,
        "spotify_import",
        "same_source:one",
        3000000,
        "Same source title",
        "spotify:track:0000000000000000000001",
    )
    .await;
    review_copy(
        &state,
        "spotify_import",
        "same_source:two",
        3000400,
        "Same source title",
        "spotify:track:0000000000000000000001",
    )
    .await;
    let preview = get(&state, &token, "/api/v1/review/automatic").await;
    assert_eq!(preview["groups"], 1);
    assert_eq!(preview["records"], 2);
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
    sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms) VALUES('artists',?,'spotify',?,'image/png',?,?)")
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
        sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms) VALUES('albums',?,?,?,'image/png',0,9999999999999)")
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
async fn legacy_spotify_artwork_survives_cleanup_and_restart_but_is_excluded_from_backup() {
    let (dir, state, token) = setup().await;
    let dash = get(&state, &token, "/api/dashboard").await;
    let id = dash["rankings"]["artists"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let png = b"\x89PNG\r\n\x1a\npermanent provider image";
    sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms) VALUES('artists',?,'spotify',?,'image/png',0,1)")
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
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artwork WHERE source='spotify'")
        .fetch_one(&mut copy)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let live: Vec<u8> =
        sqlx::query_scalar("SELECT bytes FROM artwork WHERE entity_id=? AND source='spotify'")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(live, png);
    state.storage.close().await;
}

#[tokio::test]
async fn owner_can_cancel_every_job_kind() {
    let (_dir, state, token) = setup().await;
    for kind in [
        "calendar",
        "spotify_import",
        "listenbrainz_file",
        "listenbrainz_fetch",
        "artwork",
        "spotify_collection",
        "recovery_export",
        "recovery_backup",
        "recovery_verify",
        "recovery_inspect",
        "recovery_restore",
    ] {
        let id = state
            .storage
            .enqueue(kind, None, None, &json!({}))
            .await
            .unwrap();
        let uri = format!("/api/v1/activities/{id}/cancel");
        assert_eq!(
            call(&state, &token, "POST", &uri, vec![], false).await.0,
            StatusCode::FORBIDDEN
        );

        assert_eq!(
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                call(&state, &token, "POST", &uri, vec![], true)
            )
            .await
            .unwrap()
            .0,
            StatusCode::OK
        );
        if kind == "recovery_restore" {
            assert_eq!(
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    call(
                        &state,
                        &token,
                        "GET",
                        &format!("/api/v1/activities/{id}"),
                        vec![],
                        false
                    )
                )
                .await
                .unwrap()
                .0,
                StatusCode::OK
            );
        }

        assert_eq!(state.storage.job(&id).await.unwrap().state, "cancelled");
    }
    state.storage.close().await;
}
