use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get as route_get,
};
use http_body_util::BodyExt;
use scrobbl_rs::{AppState, ai, ai::catalog, ai::models, auth, config::Config, db, web};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc, LazyLock, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tower::ServiceExt;

static TEST_LOCK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

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

async fn send(
    state: &AppState,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:3210")
        .header("origin", "http://127.0.0.1:3210")
        .header("cookie", format!("scrobbl_session={token}"))
        .header("x-csrf-token", "test-csrf");
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = web::router(state.clone())
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get_json(state: &AppState, token: &str, path: &str) -> Value {
    let (status, value) = send(state, token, "GET", path, None).await;
    assert_eq!(status, StatusCode::OK, "{path}: {value}");
    value
}

async fn post(state: &AppState, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
    send(state, token, "POST", path, Some(body)).await
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn synthetic_model() -> Vec<u8> {
    (0..3 * 1024 * 1024)
        .map(|index| ((index * 31 + 7) % 251) as u8)
        .collect()
}

struct TestServer {
    bytes: Vec<u8>,
    requests: Mutex<Vec<Option<String>>>,
    truncate_first: AtomicBool,
}

impl TestServer {
    fn requested_ranges(&self) -> Vec<Option<String>> {
        self.requests.lock().unwrap().clone()
    }
}

fn parse_range(value: &str, total: usize) -> Option<usize> {
    value
        .strip_prefix("bytes=")?
        .strip_suffix('-')?
        .parse::<usize>()
        .ok()
        .filter(|start| *start <= total)
}

async fn serve_model(State(server): State<Arc<TestServer>>, headers: HeaderMap) -> Response {
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    server.requests.lock().unwrap().push(range.clone());
    let total = server.bytes.len();
    let start = range
        .as_deref()
        .and_then(|value| parse_range(value, total))
        .unwrap_or(0);
    let body = server.bytes[start..].to_vec();
    let status = if start > 0 {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let mut builder = Response::builder()
        .status(status)
        .header(header::ACCEPT_RANGES, "bytes");
    if start > 0 {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{}/{total}", total.saturating_sub(1)),
        );
    }
    let truncated = start == 0 && server.truncate_first.swap(false, Ordering::SeqCst);
    let body = if truncated {
        body[..body.len() / 2].to_vec()
    } else {
        body
    };
    builder.body(Body::from(body)).unwrap().into_response()
}

async fn fake_server(data: Vec<u8>) -> (String, Arc<TestServer>, tokio::task::JoinHandle<()>) {
    let server = Arc::new(TestServer {
        bytes: data,
        requests: Mutex::new(Vec::new()),
        truncate_first: AtomicBool::new(false),
    });
    let app = axum::Router::new()
        .route("/model.gguf", route_get(serve_model))
        .with_state(server.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{address}/model.gguf"), server, handle)
}

fn test_model(id: &str, url: String, sha256: String, size_bytes: u64) -> catalog::CatalogModel {
    catalog::CatalogModel {
        id: id.to_string(),
        name: format!("Test model {id}"),
        tier: "light".to_string(),
        status: "available".to_string(),
        description: "A synthetic model for tests.".to_string(),
        download: catalog::DownloadSource {
            repo: "test/local".to_string(),
            revision: "test".to_string(),
            file: "model.gguf".to_string(),
            sha256,
            size_bytes,
            source_url: Some(url),
        },
        license: catalog::License {
            name: "Apache-2.0".to_string(),
            url: "https://example.test/license".to_string(),
        },
        requirements: catalog::Requirements {
            disk_bytes: size_bytes,
            ram_min_bytes: 1024,
            ram_recommended_bytes: 2048,
            cpu_cores: 1,
            gpu_optional: false,
            context_tokens: 2048,
        },
        languages: vec![catalog::LanguageScore {
            language: "en".to_string(),
            exact_match: Some(1.0),
            cases: Some(1),
        }],
        parser: catalog::ParserScores {
            intent: None,
            slots: None,
            refusals: None,
            exact_match: None,
        },
        speed: None,
        backend: "test".to_string(),
        selectable: true,
    }
}

async fn enable(state: &AppState, token: &str) -> Value {
    let (status, value) = post(
        state,
        token,
        "/api/v1/ai/settings",
        json!({
            "enabled": true,
            "active_model": null,
            "threads": 4,
            "unload_minutes": 15,
            "model_dir": null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value
}

#[tokio::test]
async fn ai_is_off_by_default_and_download_requires_consent() {
    let _guard = TEST_LOCK.lock().await;
    catalog::clear_test_models();
    download_reset();
    let (dir, state, token) = setup().await;
    let data = synthetic_model();
    let (url, server, handle) = fake_server(data.clone()).await;
    let id = "test-off-by-default";
    catalog::register_test_model(test_model(id, url, sha256(&data), data.len() as u64));

    let settings = get_json(&state, &token, "/api/v1/ai/settings").await;
    assert_eq!(settings["enabled"], false);
    assert_eq!(settings["threads"], 4);
    assert_eq!(settings["unload_minutes"], 15);
    assert_eq!(settings["runtime_available"], cfg!(feature = "llama"));
    assert_eq!(settings["model_ready"], false);
    // The image sets SCROBBL_DATA_DIR=/data, so the default folder is /data/models
    // and the runtime never needs a writable path outside the data directory.
    let model_dir = std::path::Path::new(settings["effective_model_dir"].as_str().unwrap());
    assert_eq!(model_dir.file_name().unwrap(), "models");
    assert!(model_dir.starts_with(std::fs::canonicalize(dir.path()).unwrap()));

    let ask = get_json(&state, &token, "/api/v1/ask/status").await;
    assert_eq!(ask["enabled"], false);
    assert_eq!(ask["runtime_available"], cfg!(feature = "llama"));
    assert_eq!(ask["model_ready"], false);
    assert_eq!(ask["model_loading"], false);

    let (status, value) = post(
        &state,
        &token,
        &format!("/api/v1/ai/models/{id}/download"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    assert!(
        value["error"]
            .as_str()
            .unwrap()
            .contains("Enable AI before downloading")
    );
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE kind='model_download'")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(jobs, 0);
    assert!(server.requested_ranges().is_empty());
    handle.abort();
}

fn download_reset() {
    ai::download::set_available_bytes_for_tests(None);
}

#[tokio::test]
async fn catalog_exposes_requirements_license_size_and_gpu_future_entries() {
    let _guard = TEST_LOCK.lock().await;
    catalog::clear_test_models();
    let (_dir, state, token) = setup().await;
    let catalog = get_json(&state, &token, "/api/v1/ai/catalog").await;
    assert_eq!(catalog["runtime_available"], cfg!(feature = "llama"));
    let models = catalog["models"].as_array().unwrap();
    assert!(models.len() >= 5);
    let default = models
        .iter()
        .find(|model| model["id"] == "qwen3-1.7b-q4_k_m")
        .unwrap();
    assert_eq!(default["tier"], "default");
    assert_eq!(default["status"], "available");
    assert_eq!(default["selectable"], true);
    assert_eq!(default["installed"], false);
    assert_eq!(default["license"]["name"], "Apache-2.0");
    assert!(
        default["license"]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://")
    );
    assert_eq!(default["download"]["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(default["download"]["size_bytes"], 1_282_439_264_u64);
    assert_eq!(default["requirements"]["context_tokens"], 2048);
    assert!(default["requirements"]["ram_min_bytes"].as_u64().unwrap() > 0);
    assert!(
        default["requirements"]["ram_recommended_bytes"]
            .as_u64()
            .unwrap()
            >= default["requirements"]["ram_min_bytes"].as_u64().unwrap()
    );
    assert_eq!(default["speed"]["p50_ms"], 4138);
    let languages = default["languages"].as_array().unwrap();
    assert_eq!(languages.len(), 2);
    assert!(
        languages
            .iter()
            .all(|language| language["exact_match"].is_number() && language["cases"] == 104),
        "the measured languages carry 104 cases and an exact-match score"
    );
    for field in ["intent", "slots", "refusals", "exact_match"] {
        assert!(
            default["parser"][field].is_number(),
            "measured parser field {field}"
        );
    }

    let future = models
        .iter()
        .find(|model| model["id"] == "qwen3-8b-q4_k_m")
        .unwrap();
    assert_eq!(future["status"], "gpu_future");
    assert_eq!(future["selectable"], false);
    assert!(
        future["parser"]["exact_match"].is_null(),
        "unmeasured models keep null scores"
    );
    let large = models
        .iter()
        .find(|model| model["id"] == "qwen3-32b-q4_k_m")
        .unwrap();
    assert_eq!(large["status"], "gpu_future");
    assert_eq!(large["selectable"], false);
}

#[tokio::test]
async fn download_completes_with_verified_checksum_and_atomic_rename() {
    let _guard = TEST_LOCK.lock().await;
    catalog::clear_test_models();
    download_reset();
    let (_dir, state, token) = setup().await;
    let data = synthetic_model();
    let (url, _server, handle) = fake_server(data.clone()).await;
    let id = "test-complete";
    catalog::register_test_model(test_model(id, url, sha256(&data), data.len() as u64));

    let settings = enable(&state, &token).await;
    assert_eq!(settings["enabled"], true);
    assert_eq!(settings["model_ready"], false);
    assert!(
        settings["message"]
            .as_str()
            .unwrap()
            .contains("no model is selected")
    );

    let (status, value) = post(
        &state,
        &token,
        &format!("/api/v1/ai/models/{id}/download"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let job_id = value["job_id"].as_str().unwrap().to_string();

    let catalog_before = get_json(&state, &token, "/api/v1/ai/catalog").await;
    let entry = catalog_before["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == id)
        .unwrap();
    assert_eq!(entry["download_job"]["state"], "queued");
    assert_eq!(entry["download_job"]["expected_bytes"], data.len() as u64);

    assert!(ai::download::run_once(&state).await.unwrap());
    assert!(!ai::download::run_once(&state).await.unwrap());

    let job = state.storage.job(&job_id).await.unwrap();
    assert_eq!(job.state, "completed");
    assert_eq!(job.outcome.as_deref(), Some("complete"));
    assert_eq!(job.processed, data.len() as i64);

    let root = models::root(&state).await.unwrap();
    let dir = models::model_dir(&root, id).unwrap();
    let final_path = dir.join("model.gguf");
    assert!(final_path.exists());
    assert!(!dir.join("model.gguf.part").exists());
    let installed = std::fs::read(&final_path).unwrap();
    assert_eq!(sha256(&installed), sha256(&data));
    let metadata: models::ModelMetadata =
        serde_json::from_slice(&std::fs::read(dir.join(models::METADATA_FILE)).unwrap()).unwrap();
    assert_eq!(metadata.catalog_id, id);
    assert!(metadata.checksum_verified);
    assert_eq!(metadata.size_bytes, data.len() as u64);

    let catalog_after = get_json(&state, &token, "/api/v1/ai/catalog").await;
    let entry = catalog_after["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == id)
        .unwrap();
    assert_eq!(entry["installed"], true);
    assert_eq!(entry["checksum_verified"], true);
    assert_eq!(entry["installed_size_bytes"], data.len() as u64);
    assert_eq!(entry["download_job"]["state"], "completed");
    assert_eq!(entry["download_job"]["processed_bytes"], data.len() as u64);
    assert!(catalog_after["total_installed_bytes"].as_u64().unwrap() >= data.len() as u64);

    let activities = get_json(&state, &token, "/api/v1/activities").await;
    let activity = activities
        .as_array()
        .unwrap()
        .iter()
        .find(|activity| activity["id"] == job_id)
        .unwrap();
    assert_eq!(activity["kind"], "model_download");
    assert_eq!(activity["expected_rows"], data.len() as u64);
    assert_eq!(activity["processed"], data.len() as u64);
    assert_eq!(activity["state"], "completed");
    handle.abort();
}

#[tokio::test]
async fn truncated_response_is_resumed_with_range() {
    let _guard = TEST_LOCK.lock().await;
    catalog::clear_test_models();
    download_reset();
    let (_dir, state, token) = setup().await;
    let data = synthetic_model();
    let (url, server, handle) = fake_server(data.clone()).await;
    server.truncate_first.store(true, Ordering::SeqCst);
    let id = "test-resume";
    catalog::register_test_model(test_model(id, url, sha256(&data), data.len() as u64));
    enable(&state, &token).await;
    let (status, value) = post(
        &state,
        &token,
        &format!("/api/v1/ai/models/{id}/download"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let job_id = value["job_id"].as_str().unwrap().to_string();

    assert!(ai::download::run_once(&state).await.unwrap());
    let job = state.storage.job(&job_id).await.unwrap();
    assert_eq!(job.state, "waiting");
    assert_eq!(job.error_code.as_deref(), Some("model_download_retry"));
    let root = models::root(&state).await.unwrap();
    let part = models::model_dir(&root, id)
        .unwrap()
        .join("model.gguf.part");
    let partial_len = std::fs::metadata(&part).unwrap().len();
    assert!(partial_len > 0 && partial_len < data.len() as u64);

    sqlx::query("UPDATE jobs SET retry_at_ms=0 WHERE id=?")
        .bind(&job_id)
        .execute(&state.db)
        .await
        .unwrap();
    assert!(ai::download::run_once(&state).await.unwrap());

    let job = state.storage.job(&job_id).await.unwrap();
    assert_eq!(job.state, "completed");
    let ranges = server.requested_ranges();
    assert!(ranges.len() >= 2);
    assert_eq!(ranges[0], None);
    assert!(ranges[1].is_some());
    let installed =
        std::fs::read(models::model_dir(&root, id).unwrap().join("model.gguf")).unwrap();
    assert_eq!(sha256(&installed), sha256(&data));
    assert!(!part.exists());
    handle.abort();
}

#[tokio::test]
async fn checksum_mismatch_fails_and_removes_the_partial_file() {
    let _guard = TEST_LOCK.lock().await;
    catalog::clear_test_models();
    download_reset();
    let (_dir, state, token) = setup().await;
    let data = synthetic_model();
    let (url, _server, handle) = fake_server(data.clone()).await;
    let id = "test-bad-checksum";
    catalog::register_test_model(test_model(id, url, "0".repeat(64), data.len() as u64));
    enable(&state, &token).await;
    let (status, value) = post(
        &state,
        &token,
        &format!("/api/v1/ai/models/{id}/download"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let job_id = value["job_id"].as_str().unwrap().to_string();

    assert!(ai::download::run_once(&state).await.unwrap());
    let job = state.storage.job(&job_id).await.unwrap();
    assert_eq!(job.state, "failed");
    assert_eq!(job.error_code.as_deref(), Some("model_checksum_mismatch"));
    let root = models::root(&state).await.unwrap();
    let dir = models::model_dir(&root, id).unwrap();
    assert!(!dir.join("model.gguf").exists());
    assert!(!dir.join("model.gguf.part").exists());
    assert!(!dir.join(models::METADATA_FILE).exists());
    handle.abort();
}

#[tokio::test]
async fn insufficient_disk_is_an_explicit_error_before_network_access() {
    let _guard = TEST_LOCK.lock().await;
    catalog::clear_test_models();
    download_reset();
    let (_dir, state, token) = setup().await;
    let data = synthetic_model();
    let (url, server, handle) = fake_server(data.clone()).await;
    let id = "test-disk-full";
    catalog::register_test_model(test_model(id, url, sha256(&data), data.len() as u64));
    enable(&state, &token).await;
    ai::download::set_available_bytes_for_tests(Some(1024));
    let (status, value) = post(
        &state,
        &token,
        &format!("/api/v1/ai/models/{id}/download"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let job_id = value["job_id"].as_str().unwrap().to_string();

    assert!(ai::download::run_once(&state).await.unwrap());
    download_reset();
    let job = state.storage.job(&job_id).await.unwrap();
    assert_eq!(job.state, "failed");
    assert_eq!(job.error_code.as_deref(), Some("model_disk_full"));
    assert!(server.requested_ranges().is_empty());
    handle.abort();
}

#[tokio::test]
async fn cancellation_removes_the_partial_file() {
    let _guard = TEST_LOCK.lock().await;
    catalog::clear_test_models();
    download_reset();
    let (_dir, state, token) = setup().await;
    let data = synthetic_model();
    let (url, server, handle) = fake_server(data.clone()).await;
    let id = "test-cancel";
    catalog::register_test_model(test_model(id, url, sha256(&data), data.len() as u64));
    enable(&state, &token).await;
    let (status, value) = post(
        &state,
        &token,
        &format!("/api/v1/ai/models/{id}/download"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let job = state
        .storage
        .claim("model_download")
        .await
        .unwrap()
        .unwrap();
    let root = models::root(&state).await.unwrap();
    let dir = models::model_dir(&root, id).unwrap();
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let part = dir.join("model.gguf.part");
    tokio::fs::write(&part, b"partial").await.unwrap();
    sqlx::query("UPDATE jobs SET cancel_requested=1 WHERE id=?")
        .bind(&job.id)
        .execute(&state.db)
        .await
        .unwrap();

    ai::download::run_claimed(&state, job).await.unwrap();
    let job = state
        .storage
        .job(value["job_id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(job.state, "cancelled");
    assert_eq!(job.error_code.as_deref(), Some("model_cancelled"));
    assert!(!part.exists());
    assert!(server.requested_ranges().is_empty());
    handle.abort();
}

#[tokio::test]
async fn installed_model_is_selectable_deletable_and_accounts_for_space() {
    let _guard = TEST_LOCK.lock().await;
    catalog::clear_test_models();
    download_reset();
    let (_dir, state, token) = setup().await;
    let data = synthetic_model();
    let (url, _server, handle) = fake_server(data.clone()).await;
    let id = "test-installed";
    let size = data.len() as u64;
    catalog::register_test_model(test_model(id, url, sha256(&data), size));
    enable(&state, &token).await;

    let root = models::root(&state).await.unwrap();
    let dir = models::model_dir(&root, id).unwrap();
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("model.gguf"), &data)
        .await
        .unwrap();
    models::write_metadata(
        &dir,
        &models::ModelMetadata {
            catalog_id: id.to_string(),
            file: "model.gguf".to_string(),
            sha256: sha256(&data),
            size_bytes: size,
            installed_at_ms: 1,
            license: "Apache-2.0".to_string(),
            checksum_verified: true,
        },
    )
    .await
    .unwrap();

    let catalog = get_json(&state, &token, "/api/v1/ai/catalog").await;
    let entry = catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == id)
        .unwrap();
    assert_eq!(entry["installed"], true);
    assert_eq!(entry["checksum_verified"], true);
    assert_eq!(catalog["total_installed_bytes"], size);
    assert!(catalog["disk_used_bytes"].as_u64().unwrap() > size);

    let (status, value) = post(
        &state,
        &token,
        &format!("/api/v1/ai/models/{id}/select"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["active_model"], id);
    let settings = get_json(&state, &token, "/api/v1/ai/settings").await;
    assert_eq!(settings["active_model"], id);
    assert_eq!(settings["model_installed"], true);
    assert_eq!(settings["model_ready"], true);

    let (status, value) = send(
        &state,
        &token,
        "DELETE",
        &format!("/api/v1/ai/models/{id}"),
        Some(json!({"confirm": false})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    let (status, value) = send(
        &state,
        &token,
        "DELETE",
        &format!("/api/v1/ai/models/{id}"),
        Some(json!({"confirm": true, "force": false})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    assert!(value["error"].as_str().unwrap().contains("active model"));

    let (status, value) = send(
        &state,
        &token,
        "DELETE",
        &format!("/api/v1/ai/models/{id}"),
        Some(json!({"confirm": true, "force": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["id"], id);
    assert!(value["freed_bytes"].as_u64().unwrap() >= size);
    assert!(!dir.exists());
    let settings = get_json(&state, &token, "/api/v1/ai/settings").await;
    assert!(settings["active_model"].is_null());
    assert_eq!(settings["model_installed"], false);

    let (status, value) = post(
        &state,
        &token,
        "/api/v1/ai/models/qwen3-1.7b-q4_k_m/select",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    handle.abort();
}

#[tokio::test]
async fn startup_scan_tolerates_orphan_and_corrupt_metadata() {
    let _guard = TEST_LOCK.lock().await;
    catalog::clear_test_models();
    let (_dir, state, token) = setup().await;
    let root = models::root(&state).await.unwrap();
    tokio::fs::create_dir_all(root.join("orphan"))
        .await
        .unwrap();
    tokio::fs::write(root.join("orphan").join(models::METADATA_FILE), b"not json")
        .await
        .unwrap();
    tokio::fs::create_dir_all(root.join("missing-file"))
        .await
        .unwrap();
    tokio::fs::write(
        root.join("missing-file").join(models::METADATA_FILE),
        serde_json::to_vec(&models::ModelMetadata {
            catalog_id: "missing-file".to_string(),
            file: "model.gguf".to_string(),
            sha256: "a".repeat(64),
            size_bytes: 10,
            installed_at_ms: 1,
            license: "Apache-2.0".to_string(),
            checksum_verified: true,
        })
        .unwrap(),
    )
    .await
    .unwrap();

    let scanned = models::scan(&root);
    let orphan = scanned.iter().find(|model| model.id == "orphan").unwrap();
    assert!(!orphan.usable());
    assert!(orphan.error.as_deref().unwrap().contains("metadata"));
    let missing = scanned
        .iter()
        .find(|model| model.id == "missing-file")
        .unwrap();
    assert!(!missing.usable());
    assert!(missing.error.as_deref().unwrap().contains("missing"));

    let catalog = get_json(&state, &token, "/api/v1/ai/catalog").await;
    let installed = catalog["installed"].as_array().unwrap();
    assert!(
        installed.iter().any(|model| model["id"] == "orphan"
            && model["error"].as_str().unwrap().contains("metadata"))
    );
    assert!(installed.iter().any(|model| model["id"] == "missing-file"
        && model["error"].as_str().unwrap().contains("missing")));
}

#[tokio::test]
async fn settings_validate_threads_model_dir_and_unknown_model() {
    let _guard = TEST_LOCK.lock().await;
    catalog::clear_test_models();
    let (_dir, state, token) = setup().await;
    let (status, value) = post(
        &state,
        &token,
        "/api/v1/ai/settings",
        json!({
            "enabled": true,
            "active_model": null,
            "threads": 0,
            "unload_minutes": 15,
            "model_dir": null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    let (status, value) = post(
        &state,
        &token,
        "/api/v1/ai/settings",
        json!({
            "enabled": true,
            "active_model": null,
            "threads": 4,
            "unload_minutes": 15,
            "model_dir": "relative/models"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    assert!(value["error"].as_str().unwrap().contains("absolute"));
    let (status, value) = post(
        &state,
        &token,
        "/api/v1/ai/settings",
        json!({
            "enabled": true,
            "active_model": "qwen3-1.7b-q4_k_m",
            "threads": 4,
            "unload_minutes": 15,
            "model_dir": null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["model_installed"], false);
    assert!(value["message"].as_str().unwrap().contains("not installed"));

    let (status, value) = post(
        &state,
        &token,
        "/api/v1/ai/settings",
        json!({
            "enabled": true,
            "active_model": "unknown-model",
            "threads": 4,
            "unload_minutes": 15,
            "model_dir": null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
    assert!(value["error"].as_str().unwrap().contains("Unknown model"));
}
