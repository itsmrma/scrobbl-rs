use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use http_body_util::BodyExt;
use scrobbl_rs::{
    AppState, auth,
    config::Config,
    db::{self, Event},
    imports::parse_extended,
    storage::Storage,
    web,
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn setup() -> (tempfile::TempDir, AppState, String) {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let config = Config {
        bind_ip: "127.0.0.1".parse().unwrap(),
        port: 3210,
        external_origin: None,
        data_dir: dir.path().into(),
        sync_seconds: 120,
        now_playing_seconds: 30,
        import_limits: Default::default(),
        api_snapshot_limits: Default::default(),
        env_client_id: None,
        env_client_secret: None,
    };
    let owner = auth::random_secret();
    sqlx::query("INSERT INTO sessions VALUES(?,'csrf',9999999999999)")
        .bind(auth::digest(&owner))
        .execute(&storage.private)
        .await
        .unwrap();
    (dir, AppState::new(storage, config), owner)
}
async fn request(
    state: &AppState,
    method: &str,
    path: &str,
    owner: Option<&str>,
    key: Option<&str>,
    origin: Option<&str>,
    body: Value,
) -> (StatusCode, HeaderMap, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:3210");
    if let Some(owner) = owner {
        builder = builder
            .header("cookie", format!("scrobbl_session={owner}"))
            .header("x-csrf-token", "csrf");
    }
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    if let Some(origin) = origin {
        builder = builder.header("origin", origin);
    }
    let response = web::router(state.clone())
        .oneshot(
            builder
                .header("content-type", "application/json")
                .body(if body.is_null() {
                    Body::empty()
                } else {
                    Body::from(body.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn owner_post(state: &AppState, owner: &str, path: &str, body: Value) -> Value {
    let (status, _, value) = request(
        state,
        "POST",
        path,
        Some(owner),
        None,
        Some("http://127.0.0.1:3210"),
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value
}
async fn key(state: &AppState, owner: &str) -> Value {
    owner_post(
        state,
        owner,
        "/api/v1/api-keys",
        json!({"name":"Analysis","application":"spotify_analysis","expires_at_ms":null}),
    )
    .await
}
async fn read(state: &AppState, key: &str, path: &str) -> Value {
    let (status, _, value) = request(state, "GET", path, None, Some(key), None, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{path}: {value}");
    value
}
fn event(index: usize, when: i64, ms: i64) -> Event {
    let end = chrono::DateTime::from_timestamp_millis(when + ms)
        .unwrap()
        .to_rfc3339();
    parse_extended(json!([{"ts":end,"ms_played":ms,"master_metadata_track_name":format!("Song {}",index%5000),"master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":format!("spotify:track:{:022}",index%5000)}]).to_string().as_bytes()).unwrap().0.remove(0)
}
async fn save(store: &Storage, events: &[Event]) {
    let mut tx = store.writer.begin().await.unwrap();
    db::ingest(&mut tx, events).await.unwrap();
    tx.commit().await.unwrap();
}
async fn ids(state: &AppState) -> Vec<String> {
    sqlx::query_scalar("SELECT id FROM listens ORDER BY played_at_ms,id")
        .fetch_all(&state.db)
        .await
        .unwrap()
}
async fn trash(state: &AppState, ids: &[String], at: Option<i64>) {
    let mut tx = state.db.begin().await.unwrap();
    db::set_trashed(&mut tx, ids, at).await.unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn keys_are_private_one_time_read_only_and_never_fall_back_to_owner_cookies() {
    let (_dir, state, owner) = setup().await;
    let created = key(&state, &owner).await;
    let secret = created["secret"].as_str().unwrap();
    assert_eq!(secret.len(), 64);
    let (_, _, listed) = request(
        &state,
        "GET",
        "/api/v1/api-keys",
        Some(&owner),
        None,
        None,
        Value::Null,
    )
    .await;
    assert!(!listed.to_string().contains(secret));
    assert_eq!(listed["keys"][0]["permissions"], "history:read");
    let digest: String = sqlx::query_scalar("SELECT digest FROM api_keys")
        .fetch_one(&state.private_db)
        .await
        .unwrap();
    assert_eq!(digest, auth::digest(secret));
    let tables: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name='api_keys'")
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(tables, 0);
    for path in [
        "/api/v1/preferences",
        "/api/v1/api-keys",
        "/api/v1/recovery",
        "/api/spotify/status",
        "/api/history",
    ] {
        assert_eq!(
            request(
                &state,
                "GET",
                path,
                Some(&owner),
                Some(secret),
                None,
                Value::Null
            )
            .await
            .0,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    assert_eq!(
        request(
            &state,
            "POST",
            "/api/v1/preferences",
            Some(&owner),
            Some(secret),
            Some("http://127.0.0.1:3210"),
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    for invalid in [
        None,
        Some("bad"),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    ] {
        let (status, _, value) = request(
            &state,
            "GET",
            "/api/v1/info",
            Some(&owner),
            invalid,
            None,
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(value["error"]["code"], "invalid_key");
    }
    read(&state, secret, "/api/v1/info").await;
    let last: Option<i64> = sqlx::query_scalar("SELECT last_used_at_ms FROM api_keys")
        .fetch_one(&state.private_db)
        .await
        .unwrap();
    assert!(last.is_some());
    owner_post(
        &state,
        &owner,
        &format!(
            "/api/v1/api-keys/{}/revoke",
            created["id"].as_str().unwrap()
        ),
        json!({}),
    )
    .await;
    assert_eq!(
        request(
            &state,
            "GET",
            "/api/v1/info",
            Some(&owner),
            Some(secret),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    state.storage.close().await;
}

#[tokio::test]
async fn full_copy_is_coherent_under_concurrent_writes_with_equal_times_and_precise_utc_filters() {
    let (_dir, state, owner) = setup().await;
    let created = key(&state, &owner).await;
    let secret = created["secret"].as_str().unwrap();
    let start = 1743292800000;
    save(
        &state.storage,
        &[
            event(0, start, 180000),
            event(1, start, 10000),
            event(2, start + 1, 180000),
            event(3, start + 2, 180000),
        ],
    )
    .await;
    let before = ids(&state).await;
    let first = read(
        &state,
        secret,
        &format!(
            "/api/v1/listens?from_ms={start}&until_ms={}&limit=1",
            start + 2
        ),
    )
    .await;
    let mut all = first["items"].as_array().unwrap().clone();
    let mut page = first.clone();
    save(&state.storage, &[event(4, start - 1, 180000)]).await;
    trash(&state, &[before[1].clone()], Some(auth::now_ms())).await;
    while let Some(cursor) = page["next_cursor"].as_str() {
        page = read(
            &state,
            secret,
            &format!("/api/v1/listens?cursor={cursor}&limit=1"),
        )
        .await;
        assert_eq!(page["revision"], first["revision"]);
        all.extend(page["items"].as_array().unwrap().clone());
    }
    assert_eq!(all.len(), 3);
    assert_eq!(
        all.iter()
            .map(|v| v["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        before[..3].iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert_eq!(all.iter().filter(|v| v["counted"] == false).count(), 1);
    assert!(page["complete"].as_bool().unwrap());
    assert!(page["sync_cursor"].is_string());
    assert!(!page.to_string().contains("raw_json"));
    let counted = read(&state, secret, "/api/v1/listens?counted_only=true").await;
    assert_eq!(counted["items"].as_array().unwrap().len(), 4);
    assert!(
        counted["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["counted"] == true)
    );
    for bad in [
        "limit=0",
        "limit=1001",
        "from_ms=9&until_ms=8",
        "from_ms=broken",
        "counted_only=yes",
        "secret=no",
    ] {
        let (status, _, error) = request(
            &state,
            "GET",
            &format!("/api/v1/listens?{bad}"),
            None,
            Some(secret),
            None,
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["error"]["code"], "invalid_request");
    }
    state.storage.close().await;
}

#[tokio::test]
async fn incrementals_include_backdated_updates_selection_exits_removals_and_frozen_targets() {
    let (_dir, state, owner) = setup().await;
    let created = key(&state, &owner).await;
    let secret = created["secret"].as_str().unwrap();
    let start = 1743292800000;
    save(
        &state.storage,
        &[event(1, start + 100, 180000), event(2, start + 200, 180000)],
    )
    .await;
    let original = read(
        &state,
        secret,
        &format!("/api/v1/listens?from_ms={start}&counted_only=true"),
    )
    .await;
    let old = ids(&state).await;
    trash(&state, &[old[0].clone()], Some(auth::now_ms())).await;
    save(&state.storage, &[event(3, start + 50, 180000)]).await;
    // Model the same atomic publication future correction tools use: a selection exit.
    let mut tx = state.db.begin().await.unwrap();
    sqlx::query("UPDATE listens SET played_at_ms=? WHERE id=?")
        .bind(start - 1)
        .bind(&old[1])
        .execute(&mut *tx)
        .await
        .unwrap();
    db::set_trashed(&mut tx, &[old[1].clone()], Some(1))
        .await
        .unwrap();
    db::set_trashed(&mut tx, &[old[1].clone()], None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut page = read(
        &state,
        secret,
        &format!(
            "/api/v1/changes?cursor={}&limit=1",
            original["sync_cursor"].as_str().unwrap()
        ),
    )
    .await;
    let target = page["revision"].clone();
    let mut upserts = page["items"].as_array().unwrap().clone();
    let mut removed = page["removed"].as_array().unwrap().clone();
    save(&state.storage, &[event(4, start + 10, 180000)]).await;
    while let Some(cursor) = page["next_cursor"].as_str() {
        page = read(
            &state,
            secret,
            &format!("/api/v1/changes?cursor={cursor}&limit=1"),
        )
        .await;
        assert_eq!(page["revision"], target);
        upserts.extend(page["items"].as_array().unwrap().clone());
        removed.extend(page["removed"].as_array().unwrap().clone());
    }
    assert_eq!(upserts.len(), 1);
    assert_eq!(upserts[0]["played_at_ms"], start + 50);
    assert_eq!(removed.len(), 2);
    let later = read(
        &state,
        secret,
        &format!(
            "/api/v1/changes?cursor={}",
            page["sync_cursor"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(later["items"].as_array().unwrap().len(), 1);
    assert_eq!(later["items"][0]["played_at_ms"], start + 10);
    let empty = read(
        &state,
        secret,
        &format!(
            "/api/v1/changes?cursor={}",
            later["sync_cursor"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(empty["items"], json!([]));
    assert_eq!(empty["removed"], json!([]));
    assert_eq!(empty["revision"], later["revision"]);
    state.storage.close().await;
}

#[tokio::test]
async fn cursors_are_key_bound_and_recheck_expiry_revocation_generation_retention_and_copy_limits()
{
    let (_dir, mut state, owner) = setup().await;
    let created = key(&state, &owner).await;
    let secret = created["secret"].as_str().unwrap();
    let other = key(&state, &owner).await;
    save(
        &state.storage,
        &[
            event(1, 1743292800000, 180000),
            event(2, 1743292800001, 180000),
        ],
    )
    .await;
    let first = read(&state, secret, "/api/v1/listens?limit=1").await;
    let path = format!(
        "/api/v1/listens?cursor={}",
        first["next_cursor"].as_str().unwrap()
    );
    assert_eq!(
        request(
            &state,
            "GET",
            &path,
            None,
            other["secret"].as_str(),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE api_keys SET expires_at_ms=1 WHERE digest=?")
        .bind(auth::digest(secret))
        .execute(&state.private_db)
        .await
        .unwrap();
    assert_eq!(
        request(
            &state,
            "GET",
            &path,
            Some(&owner),
            Some(secret),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE api_keys SET expires_at_ms=NULL WHERE digest=?")
        .bind(auth::digest(secret))
        .execute(&state.private_db)
        .await
        .unwrap();
    state.config.api_snapshot_limits.max_copies = 1;
    let (status, headers, value) = request(
        &state,
        "GET",
        "/api/v1/listens",
        None,
        Some(secret),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(headers.contains_key("retry-after"));
    assert_eq!(value["error"]["code"], "snapshot_unavailable");
    sqlx::query("UPDATE api_cursors SET expires_at_ms=1")
        .execute(&state.private_db)
        .await
        .unwrap();
    assert_eq!(
        request(&state, "GET", &path, None, Some(secret), None, Value::Null)
            .await
            .0,
        StatusCode::CONFLICT
    );
    state.config.api_snapshot_limits.max_copies = 4;
    let full = read(&state, secret, "/api/v1/listens").await;
    sqlx::query("UPDATE archive_state SET revision=10001")
        .execute(&state.db)
        .await
        .unwrap();
    save(&state.storage, &[event(3, 1743292800002, 180000)]).await;
    let changes = format!(
        "/api/v1/changes?cursor={}",
        full["sync_cursor"].as_str().unwrap()
    );
    assert_eq!(
        request(
            &state,
            "GET",
            &changes,
            None,
            Some(secret),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let min: i64 = sqlx::query_scalar("SELECT MIN(revision) FROM changes")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert!(min > 2);
    let fresh = read(&state, secret, "/api/v1/listens?limit=1").await;
    state.storage.maintenance(true).await.unwrap();
    state.storage.rotate_generation().await.unwrap();
    state.storage.maintenance(false).await.unwrap();
    let path = format!(
        "/api/v1/listens?cursor={}",
        fresh["next_cursor"].as_str().unwrap()
    );
    assert_eq!(
        request(&state, "GET", &path, None, Some(secret), None, Value::Null)
            .await
            .0,
        StatusCode::CONFLICT
    );
    owner_post(
        &state,
        &owner,
        &format!(
            "/api/v1/api-keys/{}/revoke",
            created["id"].as_str().unwrap()
        ),
        json!({}),
    )
    .await;
    assert_eq!(
        request(
            &state,
            "GET",
            &path,
            Some(&owner),
            Some(secret),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    state.storage.close().await;
}

#[tokio::test]
async fn cors_preflight_errors_rate_limits_empty_archives_and_unknown_durations_are_explicit() {
    let (_dir, state, owner) = setup().await;
    let created = key(&state, &owner).await;
    let secret = created["secret"].as_str().unwrap();
    owner_post(
        &state,
        &owner,
        "/api/v1/api-keys/cors",
        json!({"origins":["https://itsmrma.github.io"]}),
    )
    .await;
    let response = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/v1/listens")
                .header("host", "127.0.0.1:3210")
                .header("origin", "https://itsmrma.github.io")
                .header("access-control-request-method", "GET")
                .header("access-control-request-headers", "Authorization")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "https://itsmrma.github.io"
    );
    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-credentials")
    );
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
    let (status, headers, _) = request(
        &state,
        "GET",
        "/api/v1/info",
        None,
        None,
        Some("https://itsmrma.github.io"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers.contains_key("access-control-allow-origin"));
    let (status, headers, _) = request(
        &state,
        "GET",
        "/api/v1/info",
        None,
        Some(secret),
        Some("https://evil.example"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!headers.contains_key("access-control-allow-origin"));
    let empty = read(&state, secret, "/api/v1/listens").await;
    assert_eq!(empty["items"], json!([]));
    assert_eq!(empty["complete"], true);
    let mut unknown = event(0, 1743292800000, 180000);
    unknown.source = "listenbrainz";
    unknown.duration_kind = db::DurationKind::Unknown;
    unknown.listened_ms = None;
    unknown.counted = true;
    save(&state.storage, &[unknown]).await;
    let full = read(&state, secret, "/api/v1/listens").await;
    assert_eq!(full["items"][0]["duration_kind"], "unknown");
    assert_eq!(full["items"][0]["listened_ms"], Value::Null);
    assert_eq!(full["items"][0]["counted"], true);
    assert_eq!(full["items"][0]["sources"], json!(["listenbrainz"]));
    // Earlier reads can belong to the previous minute on a slower runner.
    // Seed the current window explicitly, retrying only a real clock rollover.
    let mut same_window = None;
    for _ in 0..3 {
        let window = auth::now_ms() / 60000;
        sqlx::query("UPDATE api_rate_limits SET requests=120,window_ms=?")
            .bind(window)
            .execute(&state.private_db)
            .await
            .unwrap();
        let response = request(
            &state,
            "GET",
            "/api/v1/info",
            None,
            Some(secret),
            Some("https://itsmrma.github.io"),
            Value::Null,
        )
        .await;
        if auth::now_ms() / 60000 == window {
            same_window = Some(response);
            break;
        }
    }
    let (status, headers, body) =
        same_window.expect("clock crossed three rate-limit windows during one request");
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "rate_limited");
    assert!(headers.contains_key("retry-after"));
    assert!(headers.contains_key("access-control-allow-origin"));
    // A genuinely older window must reset, regardless of its request count.
    sqlx::query("UPDATE api_rate_limits SET requests=120,window_ms=-1")
        .execute(&state.private_db)
        .await
        .unwrap();
    assert_eq!(
        request(
            &state,
            "GET",
            "/api/v1/info",
            None,
            Some(secret),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    state.storage.close().await;
}

#[tokio::test]
#[ignore = "100,000 observations / 5,000 tracks API snapshot and paging workload"]
async fn read_api_load() {
    let (_dir, state, owner) = setup().await;
    let created = key(&state, &owner).await;
    let secret = created["secret"].as_str().unwrap();
    for batch in 0..200 {
        let events: Vec<_> = (batch * 500..(batch + 1) * 500)
            .map(|i| event(i, 1743292800000 + i as i64 * 180000, 180000))
            .collect();
        save(&state.storage, &events).await;
    }
    let start = std::time::Instant::now();
    let mut page = read(&state, secret, "/api/v1/listens?limit=1000").await;
    let preparation = start.elapsed();
    let mut total = page["items"].as_array().unwrap().len();
    while let Some(cursor) = page["next_cursor"].as_str() {
        page = read(
            &state,
            secret,
            &format!("/api/v1/listens?cursor={cursor}&limit=1000"),
        )
        .await;
        total += page["items"].as_array().unwrap().len();
    }
    assert_eq!(total, 100000);
    assert!(page["complete"].as_bool().unwrap());
    eprintln!(
        "100,000 listens / 5,000 tracks: copy + first 1,000 rows {preparation:?}, all 100 pages {:?}",
        start.elapsed()
    );
    state.storage.close().await;
}

#[tokio::test]
async fn reconciliation_preserves_id_provenance_and_reports_countability_changes() {
    let (_dir, state, owner) = setup().await;
    let created = key(&state, &owner).await;
    let secret = created["secret"].as_str().unwrap();
    let start = 1743292800000;
    let mut api = event(1, start, 180000);
    api.source = "spotify_api";
    api.source_key = "api-event-1".into();
    api.source_timestamp_ms = start;
    api.duration_kind = db::DurationKind::Estimated;
    api.track.structured = true;
    api.track.duration_ms = Some(180000);
    save(&state.storage, &[api]).await;
    let full = read(&state, secret, "/api/v1/listens?counted_only=true").await;
    assert_eq!(full["items"][0]["duration_kind"], "estimated");
    let id = full["items"][0]["id"].clone();
    save(&state.storage, &[event(1, start, 10000)]).await;
    let changed = read(
        &state,
        secret,
        &format!(
            "/api/v1/changes?cursor={}",
            full["sync_cursor"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(changed["items"], json!([]));
    assert_eq!(changed["removed"][0]["id"], id);
    let all = read(&state, secret, "/api/v1/listens").await;
    assert_eq!(all["items"].as_array().unwrap().len(), 1);
    assert_eq!(all["items"][0]["id"], id);
    assert_eq!(all["items"][0]["duration_kind"], "actual");
    assert_eq!(all["items"][0]["listened_ms"], 10000);
    assert_eq!(all["items"][0]["counted"], false);
    assert_eq!(all["items"][0]["track_duration_ms"], 180000);
    assert_eq!(
        all["items"][0]["sources"],
        json!(["spotify_api", "spotify_import"])
    );
    // Restoring a trashed listen appears as an upsert under the same identity.
    let latest = read(&state, secret, "/api/v1/listens").await;
    trash(&state, &[id.as_str().unwrap().to_owned()], Some(1)).await;
    let removed = read(
        &state,
        secret,
        &format!(
            "/api/v1/changes?cursor={}",
            latest["sync_cursor"].as_str().unwrap()
        ),
    )
    .await;
    trash(&state, &[id.as_str().unwrap().to_owned()], None).await;
    let restored = read(
        &state,
        secret,
        &format!(
            "/api/v1/changes?cursor={}",
            removed["sync_cursor"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(restored["items"][0]["id"], id);
    state.storage.close().await;
}

#[tokio::test]
async fn owner_management_rejects_invalid_origins_expiry_and_missing_csrf() {
    let (_dir, state, owner) = setup().await;
    for origin in [
        "*",
        "https://*.example.com",
        "https://example.com/repo",
        "http://example.com",
        "https://user:secret@example.com",
    ] {
        let (status, _, _) = request(
            &state,
            "POST",
            "/api/v1/api-keys/cors",
            Some(&owner),
            None,
            Some("http://127.0.0.1:3210"),
            json!({"origins":[origin]}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{origin}");
    }
    let response = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/api-keys")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("cookie", format!("scrobbl_session={owner}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"name":"test","application":"app","expires_at_ms":null}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    for body in [
        json!({"name":"","application":"app","expires_at_ms":null}),
        json!({"name":"test","application":"app","expires_at_ms":1}),
    ] {
        assert_eq!(
            request(
                &state,
                "POST",
                "/api/v1/api-keys",
                Some(&owner),
                None,
                Some("http://127.0.0.1:3210"),
                body
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let expires = auth::now_ms() + 60000;
    owner_post(
        &state,
        &owner,
        "/api/v1/api-keys",
        json!({"name":"expires","application":"other","expires_at_ms":expires}),
    )
    .await;
    let (_, _, settings) = request(
        &state,
        "GET",
        "/api/v1/api-keys",
        Some(&owner),
        None,
        None,
        Value::Null,
    )
    .await;
    assert_eq!(settings["keys"][0]["expires_at_ms"], expires);
    state.storage.close().await;
}

#[tokio::test]
async fn checkpoints_survive_restart_and_incompatible_versions_require_a_new_copy() {
    let (dir, state, owner) = setup().await;
    let created = key(&state, &owner).await;
    let secret = created["secret"].as_str().unwrap();
    save(&state.storage, &[event(0, 1743292800000, 180000)]).await;
    let first = read(&state, secret, "/api/v1/listens").await;
    let cursor = first["sync_cursor"].as_str().unwrap();
    let config = state.config.clone();
    state.storage.close().await;
    let state = AppState::new(db::open(dir.path()).await.unwrap(), config);
    save(&state.storage, &[event(1, 1743292800001, 180000)]).await;
    let result = read(&state, secret, &format!("/api/v1/changes?cursor={cursor}")).await;
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    sqlx::query("UPDATE api_cursors SET value=json_set(value,'$.version',2) WHERE token=?")
        .bind(result["sync_cursor"].as_str().unwrap())
        .execute(&state.private_db)
        .await
        .unwrap();
    let (status, _, error) = request(
        &state,
        "GET",
        &format!(
            "/api/v1/changes?cursor={}",
            result["sync_cursor"].as_str().unwrap()
        ),
        None,
        Some(secret),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"]["code"], "version_incompatible");
    state.storage.close().await;
}

#[tokio::test]
async fn private_migration_preserves_existing_key_digests_and_revocations() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(include_str!("../private-migrations/0001_access.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../private-migrations/0002_oauth_outcomes.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    let digest = auth::digest(&auth::random_secret());
    sqlx::query("INSERT INTO api_keys VALUES(?,'Previous key',123456789,42)")
        .bind(&digest)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../private-migrations/0003_read_api.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let (stored, id, expires, revoked): (String, String, i64, i64) =
        sqlx::query_as("SELECT digest,id,expires_at_ms,revoked_at_ms FROM api_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, digest);
    assert_eq!(id.len(), 64);
    assert_eq!(expires, 123456789);
    assert_eq!(revoked, 42);
    pool.close().await;
}
