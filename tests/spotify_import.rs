use scrobbl_rs::{db, import_worker, imports::ImportLimits, storage::Storage};
use serde_json::{Value, json};
use std::io::{Cursor, Write};

fn row(second: i64, ms: i64) -> Value {
    json!({"ts":chrono::DateTime::from_timestamp(1_749_000_000+second,0).unwrap().to_rfc3339(),"ms_played":ms,"master_metadata_track_name":"Song","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:0000000000000000000001"})
}
fn zip(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        writer
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
async fn queue(storage: &Storage, files: &[(&str, Vec<u8>)], limits: ImportLimits) -> String {
    let id = storage
        .enqueue("spotify_import", None, Some("Test history"), &json!(limits))
        .await
        .unwrap();
    for (index, (name, bytes)) in files.iter().enumerate() {
        let file = storage
            .receive(&mut bytes.as_slice(), limits.upload_bytes)
            .await
            .unwrap();
        sqlx::query("INSERT INTO job_inputs(job_id,ordinal,file_id,filename) VALUES(?,?,?,?)")
            .bind(&id)
            .bind(index as i64 + 1)
            .bind(file)
            .bind(name)
            .execute(&storage.writer)
            .await
            .unwrap();
    }
    id
}
async fn run(storage: &Storage, id: &str) {
    let lease = storage.claim("spotify_import").await.unwrap().unwrap();
    assert_eq!(lease.id, id);
    import_worker::run(storage, lease).await.unwrap();
}
async fn count(storage: &Storage, table: &str) -> i64 {
    sqlx::query_scalar(match table {
        "job_members" => "SELECT COUNT(*) FROM job_members",
        "source_occurrences" => "SELECT COUNT(*) FROM source_occurrences",
        "observation_variants" => "SELECT COUNT(*) FROM observation_variants",
        "listens" => "SELECT COUNT(*) FROM listens",
        _ => panic!("unknown test table"),
    })
    .fetch_one(&storage.readers)
    .await
    .unwrap()
}
#[tokio::test]
async fn zip_inventory_rows_variants_provenance_thresholds_and_semantic_reimport() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let mut variant = row(0, 12000);
    variant["master_metadata_track_name"] = json!("Renamed");
    let rows = json!([row(0,12000),row(0,12000),variant,row(60,29000),row(120,30000),{"ts":"broken","ms_played":40000,"spotify_track_uri":"bad"},{"spotify_episode_uri":"spotify:episode:episode"}]);
    let archive = zip(&[
        ("deep/music.dat", rows.to_string().into_bytes()),
        ("profile.json", br#"{"account":"synthetic"}"#.to_vec()),
        ("bad.json", b"[{broken]".to_vec()),
        ("readme.txt", b"Read me".to_vec()),
        (
            "podcasts.json",
            json!([{"spotify_episode_uri":"episode"}])
                .to_string()
                .into_bytes(),
        ),
    ]);
    let id = queue(
        &storage,
        &[("history.zip", archive)],
        ImportLimits::default(),
    )
    .await;
    run(&storage, &id).await;
    let result = storage.job(&id).await.unwrap();
    assert_eq!(result.outcome.as_deref(), Some("partial"));
    assert!(result.inventory_complete);
    assert_eq!(result.processed, 8);
    assert_eq!(result.added, 3);
    assert_eq!(result.variants, 1);
    assert_eq!(result.invalid, 1);
    assert_eq!(result.skipped, 2);
    assert_eq!(result.below_threshold, 4);
    assert_eq!(count(&storage, "job_members").await, 5);
    assert_eq!(count(&storage, "source_occurrences").await, 8);
    assert_eq!(count(&storage, "observation_variants").await, 4);
    let null_files: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM source_occurrences WHERE file_id IS NULL")
            .fetch_one(&storage.readers)
            .await
            .unwrap();
    assert_eq!(null_files, 0);
    let counted: i64 = sqlx::query_scalar("SELECT SUM(counted) FROM listens")
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    assert_eq!(counted, 1);
    let unknown: Option<i64> =
        sqlx::query_scalar("SELECT expected_rows FROM job_members WHERE member LIKE '%bad.json'")
            .fetch_one(&storage.readers)
            .await
            .unwrap();
    assert_eq!(unknown, None);
    let first_calendar: String = sqlx::query_scalar("SELECT calendar_id FROM listens LIMIT 1")
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    storage.change_calendar("UTC", 6, false).await.unwrap();
    sqlx::query("UPDATE listens SET trashed_at_ms=1 WHERE counted=1")
        .execute(&storage.writer)
        .await
        .unwrap();
    let again = queue(
        &storage,
        &[("renamed.json", serde_json::to_vec_pretty(&rows).unwrap())],
        ImportLimits::default(),
    )
    .await;
    run(&storage, &again).await;
    assert_eq!(storage.job(&again).await.unwrap().added, 0);
    assert_eq!(count(&storage, "listens").await, 3);
    assert_eq!(count(&storage, "observation_variants").await, 4);
    let same: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM listens WHERE calendar_id=?")
        .bind(first_calendar)
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    assert_eq!(same, 3);
    let trash: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM listens WHERE trashed_at_ms IS NOT NULL")
            .fetch_one(&storage.readers)
            .await
            .unwrap();
    assert_eq!(trash, 1);
    storage.close().await;
}
#[tokio::test]
async fn failures_exclusions_and_limits_are_explicit_and_retry_does_not_recount() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let mut limits = ImportLimits {
        member_bytes: 1000,
        row_bytes: 600,
        ..Default::default()
    };
    let oversized = json!([{"extra":"x".repeat(2000)}]).to_string().into_bytes();
    let archive = zip(&[
        ("good.json", json!([row(0, 30000)]).to_string().into_bytes()),
        ("too-big.json", oversized),
        ("../unsafe.json", b"[]".to_vec()),
        ("foreign.json", b"{}".to_vec()),
    ]);
    let id = queue(&storage, &[("history.zip", archive)], limits.clone()).await;
    run(&storage, &id).await;
    let job = storage.job(&id).await.unwrap();
    assert_eq!(job.outcome.as_deref(), Some("partial"));
    assert_eq!(job.added, 1);
    storage.retry(&id).await.unwrap();
    run(&storage, &id).await;
    let retried = storage.job(&id).await.unwrap();
    assert_eq!(retried.added, 1);
    assert_eq!(retried.processed, 1);
    assert_eq!(retried.duplicates, 0);
    let unsafe_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM job_members WHERE error_code='unsafe_zip_path'")
            .fetch_one(&storage.readers)
            .await
            .unwrap();
    assert_eq!(unsafe_count, 1);
    let empty = queue(
        &storage,
        &[
            ("unrelated.json", b"{}".to_vec()),
            (
                "podcasts.json",
                json!([{"spotify_episode_uri":"episode"}])
                    .to_string()
                    .into_bytes(),
            ),
        ],
        limits.clone(),
    )
    .await;
    run(&storage, &empty).await;
    assert_eq!(
        storage.job(&empty).await.unwrap().outcome.as_deref(),
        Some("no_music")
    );
    limits.files = 1;
    let incomplete = queue(
        &storage,
        &[(
            "huge.zip",
            zip(&[("a.json", b"[]".to_vec()), ("b.json", b"[]".to_vec())]),
        )],
        limits,
    )
    .await;
    run(&storage, &incomplete).await;
    let result = storage.job(&incomplete).await.unwrap();
    assert!(!result.inventory_complete);
    assert_eq!(result.outcome.as_deref(), Some("error"));
    storage.close().await;
}
#[tokio::test]
async fn restart_resumes_confirmed_rows_and_cancelled_jobs_require_explicit_retry() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let rows: Vec<_> = (0..1001).map(|i| row(i * 60, 30000)).collect();
    let id = queue(
        &storage,
        &[("rows.json", serde_json::to_vec(&rows).unwrap())],
        ImportLimits::default(),
    )
    .await;
    let job = storage.claim("spotify_import").await.unwrap().unwrap();
    let clone = storage.clone();
    let processing = tokio::spawn(async move { import_worker::run(&clone, job).await });
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if storage.job(&id).await.unwrap().processed >= 250 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    processing.abort();
    let _ = processing.await;
    let partial = storage.job(&id).await.unwrap().processed;
    assert!((250..1001).contains(&partial));
    storage.close().await;
    let restarted = db::open(dir.path()).await.unwrap();
    run(&restarted, &id).await;
    let job = restarted.job(&id).await.unwrap();
    assert_eq!(job.processed, 1001);
    assert_eq!(job.added, 1001);
    assert_eq!(job.outcome.as_deref(), Some("complete"));
    assert_eq!(count(&restarted, "source_occurrences").await, 1001);
    let cancelled = queue(
        &restarted,
        &[("same.json", serde_json::to_vec(&rows).unwrap())],
        ImportLimits::default(),
    )
    .await;
    restarted.cancel(&cancelled).await.unwrap();
    assert!(restarted.claim("spotify_import").await.unwrap().is_none());
    restarted.retry(&cancelled).await.unwrap();
    run(&restarted, &cancelled).await;
    assert_eq!(restarted.job(&cancelled).await.unwrap().duplicates, 1001);
    restarted.close().await;
}
#[tokio::test]
async fn changed_times_durations_and_uncertain_identity_stay_separate() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let mut unknown = row(0, 30000);
    unknown["spotify_track_uri"] = Value::Null;
    let mut changed = unknown.clone();
    changed["platform"] = json!("different evidence");
    let id = queue(
        &storage,
        &[(
            "data.json",
            json!([
                row(0, 30000),
                row(1, 30000),
                row(0, 31000),
                unknown.clone(),
                unknown,
                changed
            ])
            .to_string()
            .into_bytes(),
        )],
        ImportLimits::default(),
    )
    .await;
    run(&storage, &id).await;
    assert_eq!(count(&storage, "listens").await, 5);
    assert_eq!(storage.job(&id).await.unwrap().duplicates, 1);
    let review: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM listens WHERE needs_review=1")
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    assert_eq!(review, 4);
    storage.close().await;
}
#[tokio::test]
#[ignore = "124 MB ZIP/parser acceptance workload"]
async fn full_size_spotify_zip() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let bytes = 124_396_022usize;
    let mut files = Vec::new();
    for i in 0..18 {
        let target = bytes / 18 + usize::from(i < bytes % 18);
        let mut rows: Vec<_> = (0..1000)
            .map(|n| row((i * 1000 + n) as i64 * 60, 30000))
            .collect();
        // Padding is spread across rows, so the test exercises both full decoding and bounded batches.
        let base = serde_json::to_vec(&rows).unwrap().len() + 1000 * 11;
        let padding = target - base;
        for (n, row) in rows.iter_mut().enumerate() {
            row["extra"] = json!("x".repeat(padding / 1000 + usize::from(n < padding % 1000)));
        }
        let mut data = serde_json::to_vec(&rows).unwrap();
        // Whitespace closes any encoding-size difference while preserving valid JSON.
        assert!(data.len() <= target);
        data.resize(target, b' ');
        files.push((format!("nested/history-{i}.json"), data));
    }
    assert_eq!(files.iter().map(|(_, b)| b.len()).sum::<usize>(), bytes);
    let refs: Vec<_> = files.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    let archive = zip(&refs);
    drop(refs);
    drop(files);
    let start = std::time::Instant::now();
    let id = queue(&storage, &[("full.zip", archive)], ImportLimits::default()).await;
    run(&storage, &id).await;
    let result = storage.job(&id).await.unwrap();
    assert_eq!(result.outcome.as_deref(), Some("complete"));
    assert_eq!(result.processed, 18000);
    assert_eq!(count(&storage, "job_members").await, 18);
    println!(
        "18 JSON / {bytes} bytes / 18,000 synthetic rows: {:?}",
        start.elapsed()
    );
    storage.close().await;
}
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use scrobbl_rs::{AppState, auth, config::Config, web};
use tower::ServiceExt;
async fn http_state() -> (tempfile::TempDir, AppState, String) {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let config = Config {
        bind_ip: "127.0.0.1".parse().unwrap(),
        external_origin: None,
        port: 3210,
        data_dir: dir.path().to_owned(),
        sync_seconds: 120,
        now_playing_seconds: 30,
        env_client_id: None,
        env_client_secret: None,
        import_limits: ImportLimits {
            upload_bytes: 4096,
            ..Default::default()
        },
    };
    let state = AppState::new(storage, config);
    let cookie = auth::random_secret();
    sqlx::query("INSERT INTO sessions(digest,csrf,expires_at_ms) VALUES(?,'csrf',?)")
        .bind(auth::digest(&cookie))
        .bind(auth::now_ms() + 60000)
        .execute(&state.private_db)
        .await
        .unwrap();
    (dir, state, format!("scrobbl_session={cookie}"))
}
async fn upload_http(
    state: &AppState,
    cookie: &str,
    csrf: &str,
    body: String,
) -> (StatusCode, Value) {
    let response = web::router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/import/spotify")
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("cookie", cookie)
                .header("x-csrf-token", csrf)
                .header(
                    "content-type",
                    "multipart/form-data; boundary=test-boundary",
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let data =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, data)
}
#[tokio::test]
async fn multipart_receipt_is_guarded_and_incomplete_uploads_never_process() {
    let (_dir, state, cookie) = http_state().await;
    let part = format!(
        "--test-boundary\r\nContent-Disposition: form-data; name=\"files\"; filename=\"first.json\"\r\nContent-Type: application/json\r\n\r\n{}\r\n",
        json!([row(0, 30000)])
    );
    let valid = format!("{part}--test-boundary--\r\n");
    assert_eq!(
        upload_http(&state, &cookie, "wrong", valid.clone()).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(count(&state.storage, "job_members").await, 0);
    // First file reached its boundary; the second body was cut off before receipt completed.
    let truncated = format!(
        "{part}--test-boundary\r\nContent-Disposition: form-data; name=\"files\"; filename=\"second.json\"\r\n\r\n[{{"
    );
    assert_eq!(
        upload_http(&state, &cookie, "csrf", truncated).await.0,
        StatusCode::BAD_REQUEST
    );
    let failed: scrobbl_rs::jobs::Job =
        sqlx::query_as("SELECT * FROM jobs WHERE phase='receiving'")
            .fetch_one(&state.db)
            .await
            .unwrap();
    assert_eq!(failed.state, "failed");
    assert_eq!(failed.processed, 0);
    assert!(
        state
            .storage
            .claim("spotify_import")
            .await
            .unwrap()
            .is_none()
    );
    let received: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job_inputs WHERE job_id=?")
        .bind(failed.id)
        .fetch_one(&state.read_db)
        .await
        .unwrap();
    assert_eq!(received, 1);
    let (status, response) = upload_http(&state, &cookie, "csrf", valid).await;
    assert_eq!(status, StatusCode::OK);
    let id = response["job_id"].as_str().unwrap();
    assert_eq!(state.storage.job(id).await.unwrap().state, "queued");
    run(&state.storage, id).await;
    assert_eq!(state.storage.job(id).await.unwrap().added, 1);
    let too_big = format!(
        "--test-boundary\r\nContent-Disposition: form-data; name=\"files\"; filename=\"big.json\"\r\n\r\n{}\r\n--test-boundary--\r\n",
        "x".repeat(5000)
    );
    assert_eq!(
        upload_http(&state, &cookie, "csrf", too_big).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(count(&state.storage, "listens").await, 1);
    state.storage.close().await;
}
#[tokio::test]
async fn detailed_reports_page_every_invalid_row_and_history_can_be_filtered() {
    let (_dir, state, cookie) = http_state().await;
    let mut rows = vec![row(0, 30000)];
    rows.extend((0..205).map(|_| json!({"ts":"bad","ms_played":1})));
    let id = queue(
        &state.storage,
        &[("data.json", serde_json::to_vec(&rows).unwrap())],
        ImportLimits::default(),
    )
    .await;
    run(&state.storage, &id).await;
    async fn get(state: &AppState, cookie: &str, path: String) -> Value {
        let response = web::router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("host", "127.0.0.1:3210")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }
    let first = get(&state, &cookie, format!("/api/v1/activities/{id}")).await;
    let last = get(
        &state,
        &cookie,
        format!("/api/v1/activities/{id}?issues_page=2"),
    )
    .await;
    assert_eq!(first["issues"].as_array().unwrap().len(), 100);
    assert_eq!(last["issues"].as_array().unwrap().len(), 5);
    assert_eq!(last["issues"][4]["row_number"], 206);
    let filtered = get(&state, &cookie, "/api/v1/activities?kind=calendar".into()).await;
    assert_eq!(filtered, json!([]));
    let imports = get(
        &state,
        &cookie,
        "/api/v1/activities?kind=spotify_import".into(),
    )
    .await;
    assert_eq!(imports[0]["id"], id);
    assert!(!first.to_string().contains("raw_json"));
    assert!(!first.to_string().contains("parameters_json"));
    state.storage.close().await;
}
#[tokio::test]
async fn crc_encryption_and_row_expansion_limits_cannot_report_complete_imports() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "bad-crc.json",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer
        .write_all(json!([row(0, 30000)]).to_string().as_bytes())
        .unwrap();
    let mut bytes = writer.finish().unwrap().into_inner();
    let position = bytes.windows(4).position(|v| v == b"Song").unwrap();
    bytes[position] = b'T';
    let id = queue(&storage, &[("bad-crc.zip", bytes)], ImportLimits::default()).await;
    run(&storage, &id).await;
    assert_eq!(
        storage.job(&id).await.unwrap().outcome.as_deref(),
        Some("error")
    );
    assert_eq!(count(&storage, "listens").await, 0);
    let mut archive = zip(&[("encrypted.json", b"[]".to_vec())]);
    archive[6] |= 1;
    let central = archive.windows(4).position(|v| v == b"PK\x01\x02").unwrap();
    archive[central + 8] |= 1;
    let id = queue(
        &storage,
        &[("encrypted.zip", archive)],
        ImportLimits::default(),
    )
    .await;
    run(&storage, &id).await;
    let error: String = sqlx::query_scalar("SELECT error_code FROM job_members WHERE job_id=?")
        .bind(&id)
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    assert_eq!(error, "encrypted_zip_member");
    let mut large_row = row(0, 30000);
    large_row["extra"] = json!("x".repeat(700));
    let row_limits = ImportLimits {
        row_bytes: 600,
        ..Default::default()
    };
    let id = queue(
        &storage,
        &[(
            "large-row.json",
            json!([large_row]).to_string().into_bytes(),
        )],
        row_limits,
    )
    .await;
    run(&storage, &id).await;
    let error: String = sqlx::query_scalar("SELECT error_code FROM job_members WHERE job_id=?")
        .bind(&id)
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    assert_eq!(error, "json_row_byte_limit");
    let expanded = ImportLimits {
        expanded_bytes: 1,
        ..Default::default()
    };
    let id = queue(&storage, &[("small.json", b"[]".to_vec())], expanded).await;
    run(&storage, &id).await;
    let error: String = sqlx::query_scalar("SELECT error_code FROM job_members WHERE job_id=?")
        .bind(&id)
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    assert_eq!(error, "expanded_byte_limit");
    storage.close().await;
}
#[tokio::test]
async fn maintenance_fences_and_requeues_a_running_import_without_orphaning_its_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let rows: Vec<_> = (0..2001).map(|i| row(i * 60, 30000)).collect();
    let id = queue(
        &storage,
        &[("history.json", serde_json::to_vec(&rows).unwrap())],
        ImportLimits::default(),
    )
    .await;
    let work = storage.clone();
    let worker = tokio::spawn(async move { import_worker::worker(work).await });
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if storage.job(&id).await.unwrap().processed >= 250 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        storage.maintenance(true).await.unwrap();
        loop {
            if storage.job(&id).await.unwrap().state == "queued" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let paused = storage.job(&id).await.unwrap();
    assert!(paused.processed < 2001);
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    assert_eq!(storage.job(&id).await.unwrap().processed, paused.processed);
    storage.maintenance(false).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if storage.job(&id).await.unwrap().state == "completed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let complete = storage.job(&id).await.unwrap();
    assert_eq!(complete.processed, 2001);
    assert_eq!(complete.added, 2001);
    assert_eq!(complete.duplicates, 0);
    assert_eq!(complete.attempt, 2);
    worker.abort();
    let _ = worker.await;
    storage.close().await;
}
