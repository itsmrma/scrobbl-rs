use scrobbl_rs::{
    exports::{self, ExportSelection},
    imports::parse_extended,
    jobs::SourceRow,
    recovery,
    storage::{self, Storage},
};
use serde_json::{Value, json};
use sqlx::Connection;
use std::{io::Write, path::Path, time::Duration};

async fn seed(store: &Storage) {
    let source=json!([
        {"ts":"2026-01-01T00:00:00Z","ms_played":3000,"master_metadata_track_name":"=short","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:0000000000000000000001"},
        {"ts":"2026-01-02T00:00:00Z","ms_played":60000,"master_metadata_track_name":"Repeated","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:0000000000000000000002"},
        {"ts":"2026-01-03T00:00:00Z","ms_played":60000,"master_metadata_track_name":"Repeated","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:0000000000000000000002"}
    ]).to_string();
    let id = store.receive(&mut source.as_bytes(), 10240).await.unwrap();
    store
        .enqueue("test_import", Some(&id), Some("history.json"), &json!({}))
        .await
        .unwrap();
    let job = store.claim("test_import").await.unwrap().unwrap();
    let (events, _) = parse_extended(source.as_bytes()).unwrap();
    store
        .commit_batch(
            &job,
            "history.json",
            &events.into_iter().map(SourceRow::from).collect::<Vec<_>>(),
            false,
        )
        .await
        .unwrap();
    // Keep this import running in the backup, with a confirmed checkpoint and material to resume.
    sqlx::query("INSERT INTO settings VALUES('instance_name','Recovery room')")
        .execute(&store.writer)
        .await
        .unwrap();
    sqlx::query("INSERT INTO owner VALUES(1,'secret-password-verifier',0)")
        .execute(&store.private)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sessions VALUES('secret-session','secret-csrf',9999999999999)")
        .execute(&store.private)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO spotify_tokens VALUES(1,'secret-access','secret-refresh',9999999999999)",
    )
    .execute(&store.private)
    .await
    .unwrap();
    sqlx::query("INSERT INTO settings VALUES('spotify_client_secret','secret-client')")
        .execute(&store.private)
        .await
        .unwrap();
    sqlx::query("INSERT INTO api_keys VALUES('secret-api','test',NULL,NULL)")
        .execute(&store.private)
        .await
        .unwrap();
}
async fn backup(store: &Storage) -> (tempfile::TempDir, std::path::PathBuf, recovery::Manifest) {
    let work = tempfile::tempdir().unwrap();
    let output = work.path().join("backup.zip");
    let manifest = recovery::create_backup(store, work.path(), &output)
        .await
        .unwrap();
    (work, output, manifest)
}
async fn register(store: &Storage, path: &Path, manifest: &recovery::Manifest) -> String {
    let id = recovery::enqueue_asset(store, "backup", "zip", json!({}))
        .await
        .unwrap();
    let asset = recovery::asset(store, &id).await.unwrap();
    std::fs::copy(path, recovery::asset_path(store, &asset).unwrap()).unwrap();
    sqlx::query("UPDATE jobs SET state='completed' WHERE id=?")
        .bind(&asset.job_id)
        .execute(&store.writer)
        .await
        .unwrap();
    sqlx::query("UPDATE recovery_assets SET state='ready',manifest_json=?,size_bytes=? WHERE id=?")
        .bind(serde_json::to_string(manifest).unwrap())
        .bind(std::fs::metadata(path).unwrap().len() as i64)
        .bind(&id)
        .execute(&store.writer)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn portable_backup_restores_originals_pending_jobs_and_files_without_any_access_storage() {
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    seed(&store).await;
    let (_work, package, manifest) = backup(&store).await;
    assert_eq!(manifest.counts["listens"], 3);
    assert_eq!(manifest.files.len(), 2);
    let check = tempfile::tempdir().unwrap();
    recovery::check_backup(&package, check.path())
        .await
        .unwrap();
    assert!(check.path().join("archive.db").is_file());
    assert_eq!(
        std::fs::read_dir(check.path().join("spool"))
            .unwrap()
            .count(),
        0
    );
    let trial = tempfile::tempdir().unwrap();
    recovery::trial_restore(&package, trial.path())
        .await
        .unwrap();
    assert!(!trial.path().join("private").exists());
    let bytes = std::fs::read(trial.path().join("archive.db")).unwrap();
    for secret in [
        "secret-password-verifier",
        "secret-session",
        "secret-refresh",
        "secret-access",
        "secret-client",
        "secret-api",
    ] {
        assert!(
            !bytes
                .windows(secret.len())
                .any(|part| part == secret.as_bytes())
        );
    }
    let mut conn = recovery::connect(&trial.path().join("archive.db"), true)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT checkpoint FROM jobs WHERE kind='test_import'")
            .fetch_one(&mut conn)
            .await
            .unwrap(),
        3
    );
    let refs: Vec<(String, i64)> =
        sqlx::query_as("SELECT content_hash,size_bytes FROM source_files")
            .fetch_all(&mut conn)
            .await
            .unwrap();
    for (hash, size) in refs {
        assert_eq!(
            std::fs::metadata(trial.path().join("spool").join(hash))
                .unwrap()
                .len(),
            size as u64
        );
    }
    conn.close().await.unwrap();
    assert_eq!(
        store
            .job(
                &sqlx::query_scalar::<_, String>("SELECT id FROM jobs WHERE kind='test_import'")
                    .fetch_one(&store.readers)
                    .await
                    .unwrap()
            )
            .await
            .unwrap()
            .state,
        "running"
    );
    store.close().await;
}

#[tokio::test]
async fn exports_use_one_row_per_listen_preserve_unknown_time_and_share_assigned_calendar_filters()
{
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    seed(&store).await;
    // A second source for one event must enrich a JSON record rather than multiply CSV rows.
    sqlx::query("UPDATE tracks SET title='+44' WHERE title='Repeated'")
        .execute(&store.writer)
        .await
        .unwrap();
    sqlx::query("INSERT INTO original_payloads(id,raw_json) VALUES(-1,'{}'); INSERT INTO observations(id,source,source_key,listen_id,source_timestamp_ms,payload_id) SELECT 'extra','listenbrainz','extra',id,played_at_ms,-1 FROM listens ORDER BY played_at_ms LIMIT 1").execute(&store.writer).await.unwrap();
    sqlx::query("UPDATE listens SET listened_ms=NULL,duration_kind='unknown' WHERE played_at_ms=(SELECT MAX(played_at_ms) FROM listens)").execute(&store.writer).await.unwrap();
    let work = tempfile::tempdir().unwrap();
    let frozen = work.path().join("frozen.db");
    recovery::freeze(&store, &frozen).await.unwrap();
    let full = work.path().join("all.json");
    assert_eq!(
        exports::write(&frozen, &full, "json", &ExportSelection::default())
            .await
            .unwrap(),
        3
    );
    let data: Value = serde_json::from_slice(&std::fs::read(full).unwrap()).unwrap();
    assert_eq!(
        data["listens"][0]["observations"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        data["listens"][0]["observations"][0]["variants"][0]["occurrences"][0]["row_number"],
        1
    );
    assert_eq!(data["listens"][2]["listened_ms"], Value::Null);
    assert_eq!(data["listens"][1]["title"], "+44");
    assert_eq!(data["listens"][0]["calendar"]["timezone"], "Europe/Rome");
    let csv = work.path().join("all.csv");
    exports::write(&frozen, &csv, "csv", &ExportSelection::default())
        .await
        .unwrap();
    let csv = std::fs::read_to_string(csv).unwrap();
    assert_eq!(csv.lines().count(), 4);
    assert!(csv.contains("\"'=short\""));
    assert!(csv.contains("\"'+44\""));
    assert!(csv.contains(",,unknown,"));
    let filtered = ExportSelection {
        from: Some("2026-01-02".into()),
        to: Some("2026-01-02".into()),
        ..Default::default()
    };
    assert_eq!(
        exports::write(
            &frozen,
            &work.path().join("filtered.json"),
            "json",
            &filtered
        )
        .await
        .unwrap(),
        1
    );
    store.close().await;
}

#[tokio::test]
async fn replacement_requires_new_access_rotates_generation_and_pauses_jobs_until_resumed() {
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    seed(&store).await;
    let identity = store.read_view().await.unwrap().identity;
    let (_work, path, manifest) = backup(&store).await;
    let id = register(&store, &path, &manifest).await;
    assert!(
        recovery::prepare_restore(&store, &id, "short".into())
            .await
            .is_err()
    );
    assert!(!directory.path().join("recovery.pending").exists());
    recovery::prepare_restore(&store, &id, "a completely new password".into())
        .await
        .unwrap();
    assert!(store.claim("test_import").await.unwrap().is_none());
    store.close().await;
    let restored = storage::open(directory.path()).await.unwrap();
    assert!(recovery::paused(&restored).await);
    assert!(restored.claim("test_import").await.unwrap().is_none());
    assert_ne!(
        restored.read_view().await.unwrap().identity.generation,
        identity.generation
    );
    assert!(restored.changes(&identity, 100).await.is_err());
    for table in ["sessions", "spotify_tokens", "api_keys", "oauth_states"] {
        let mut query = sqlx::QueryBuilder::new("SELECT COUNT(*) FROM ");
        query.push(table);
        assert_eq!(
            query
                .build_query_scalar::<i64>()
                .fetch_one(&restored.private)
                .await
                .unwrap(),
            0
        );
    }
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM owner")
        .fetch_one(&restored.private)
        .await
        .unwrap();
    use argon2::{Argon2, PasswordHash, PasswordVerifier};
    assert!(
        Argon2::default()
            .verify_password(
                b"a completely new password",
                &PasswordHash::new(&hash).unwrap()
            )
            .is_ok()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key='instance_name'")
            .fetch_one(&restored.readers)
            .await
            .unwrap(),
        "Recovery room"
    );
    let previous = std::fs::read_dir(directory.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().starts_with("recovery-"))
        .unwrap()
        .path()
        .join("previous");
    assert!(previous.join("archive.db").exists());
    assert!(previous.join("private/auth.db").exists());
    restored.close().await;
    // A normal subsequent restart must not activate the same preparation a second time.
    let again = storage::open(directory.path()).await.unwrap();
    assert!(recovery::paused(&again).await);
    again.close().await;
}

#[tokio::test]
async fn creation_check_rejects_damaged_spool_without_extracting_it() {
    use std::io::Read;
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    seed(&store).await;
    let (_work, package, _) = backup(&store).await;
    let bad = tempfile::tempdir().unwrap();
    let output = bad.path().join("damaged-spool.zip");
    let mut source = zip::ZipArchive::new(std::fs::File::open(package).unwrap()).unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&output).unwrap());
    for i in 0..source.len() {
        let mut member = source.by_index(i).unwrap();
        let name = member.name().unwrap().into_owned();
        zip.start_file(&name, zip::write::SimpleFileOptions::default())
            .unwrap();
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).unwrap();
        if name.starts_with("spool/") {
            bytes[0] ^= 1;
        }
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
    let check = tempfile::tempdir().unwrap();
    let error = recovery::check_backup(&output, check.path())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("checksum mismatch"), "{error:#}");
    assert_eq!(
        std::fs::read_dir(check.path().join("spool"))
            .unwrap()
            .count(),
        0
    );
    store.close().await;
}

#[tokio::test]
async fn damaged_or_incompatible_backup_never_replaces_the_destination() {
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    seed(&store).await;
    let (_work, path, mut manifest) = backup(&store).await;
    let bad = tempfile::tempdir().unwrap();
    let output = bad.path().join("bad.zip");
    // Repackage a valid database with a mismatched manifest checksum.
    let mut source = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    manifest.files[0].sha256 = "0".repeat(64);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&output).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for i in 0..source.len() {
        let mut member = source.by_index(i).unwrap();
        let name = member.name().unwrap().into_owned();
        zip.start_file(&name, options).unwrap();
        if name == "manifest.json" {
            zip.write_all(&serde_json::to_vec(&manifest).unwrap())
                .unwrap();
        } else {
            std::io::copy(&mut member, &mut zip).unwrap();
        }
    }
    zip.finish().unwrap();
    let trial = tempfile::tempdir().unwrap();
    assert!(
        recovery::trial_restore(&output, trial.path())
            .await
            .is_err()
    );
    let id = register(&store, &output, &manifest).await;
    assert!(
        recovery::prepare_restore(&store, &id, "a completely new password".into())
            .await
            .is_err()
    );
    assert!(!directory.path().join("recovery.pending").exists());
    assert!(
        std::fs::read_dir(directory.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("recovery-")
        }),
        "failed preparation must release its staging space"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens")
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        3
    );
    store.close().await;
}

#[tokio::test]
async fn failed_restore_after_database_open_releases_staging_on_repeated_attempts() {
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    seed(&store).await;
    let (_work, package, mut manifest) = backup(&store).await;
    *manifest.counts.get_mut("listens").unwrap() += 1;
    let bad = tempfile::tempdir().unwrap();
    let output = bad.path().join("bad-counts.zip");
    let mut source = zip::ZipArchive::new(std::fs::File::open(package).unwrap()).unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&output).unwrap());
    for i in 0..source.len() {
        let mut member = source.by_index(i).unwrap();
        let name = member.name().unwrap().into_owned();
        zip.start_file(&name, zip::write::SimpleFileOptions::default())
            .unwrap();
        if name == "manifest.json" {
            zip.write_all(&serde_json::to_vec(&manifest).unwrap())
                .unwrap();
        } else {
            std::io::copy(&mut member, &mut zip).unwrap();
        }
    }
    zip.finish().unwrap();
    let id = register(&store, &output, &manifest).await;
    for _ in 0..3 {
        assert!(
            recovery::prepare_restore(&store, &id, "a completely new password".into())
                .await
                .is_err()
        );
        assert!(!directory.path().join("recovery.pending").exists());
        assert!(
            std::fs::read_dir(directory.path()).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("recovery-")
            }),
            "failed preparation must not accumulate extracted databases"
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens")
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        3
    );
    store.close().await;
}

#[tokio::test]
async fn queued_operations_survive_restart_and_verification_does_not_change_live_music() {
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    seed(&store).await;
    let id = recovery::enqueue_asset(&store, "backup", "zip", json!({}))
        .await
        .unwrap();
    store.close().await;
    let store = storage::open(directory.path()).await.unwrap();
    let identity = store.read_view().await.unwrap().identity;
    let worker = tokio::spawn(recovery::worker(store.clone()));
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if recovery::asset(&store, &id).await.unwrap().state == "ready" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let asset = recovery::asset(&store, &id).await.unwrap();
    assert!(asset.verified_at_ms.is_none());
    let verify = recovery::enqueue(&store, "recovery_verify", json!({"asset_id":id}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if store.job(&verify).await.unwrap().state == "completed" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        recovery::asset(&store, &id)
            .await
            .unwrap()
            .verified_at_ms
            .is_some()
    );
    assert_eq!(store.read_view().await.unwrap().identity, identity);
    worker.abort();
    let _ = worker.await;
    store.close().await;
}

#[tokio::test]
async fn startup_finishes_activation_interrupted_between_resource_renames() {
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    seed(&store).await;
    let (_work, path, manifest) = backup(&store).await;
    let id = register(&store, &path, &manifest).await;
    recovery::prepare_restore(&store, &id, "a completely new password".into())
        .await
        .unwrap();
    store.close().await;
    let stage_id = std::fs::read_to_string(directory.path().join("recovery.pending")).unwrap();
    let stage = directory.path().join(format!("recovery-{stage_id}"));
    // State after archive activation and a crash halfway through private-store activation.
    std::fs::rename(
        directory.path().join("archive.db"),
        stage.join("previous/archive.db"),
    )
    .unwrap();
    std::fs::rename(
        stage.join("new/archive.db"),
        directory.path().join("archive.db"),
    )
    .unwrap();
    std::fs::rename(
        directory.path().join("private"),
        stage.join("previous/private"),
    )
    .unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    assert!(recovery::paused(&store).await);
    assert!(!directory.path().join("recovery.pending").exists());
    assert!(stage.join("previous/private/auth.db").exists());
    assert_eq!(recovery::assets(&store).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
async fn failed_migration_keeps_a_complete_offline_recoverable_previous_copy() {
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    seed(&store).await;
    // A schema-4 installation with an object conflicting with the new table fails migration.
    sqlx::query("DROP INDEX listens_overview; DELETE FROM _sqlx_migrations WHERE version>=5")
        .execute(&store.writer)
        .await
        .unwrap();
    store.close().await;
    assert!(storage::open(directory.path()).await.is_err());
    let safety = std::fs::read_dir(directory.path().join("backups"))
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().starts_with("pre-upgrade-"))
        .unwrap();
    assert!(safety.path().join("complete.json").exists());
    let destination = directory.path().join("recovered-old");
    recovery::recover_upgrade(
        directory.path(),
        &safety.file_name().to_string_lossy(),
        &destination,
    )
    .await
    .unwrap();
    assert!(destination.join("recovery.complete").exists());
    assert!(!destination.join("recovery.incomplete").exists());
    let mut conn = recovery::connect(&destination.join("archive.db"), true)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&mut conn)
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens")
            .fetch_one(&mut conn)
            .await
            .unwrap(),
        3
    );
    conn.close().await.unwrap();
    assert!(destination.join("private/auth.db").exists());
    assert!(
        recovery::recover_upgrade(
            directory.path(),
            &safety.file_name().to_string_lossy(),
            &destination
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn failed_production_keeps_prior_copies_and_never_publishes_partial_output() {
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    seed(&store).await;
    let (_work, package, manifest) = backup(&store).await;
    let prior = register(&store, &package, &manifest).await;
    let file: String = sqlx::query_scalar("SELECT content_hash FROM source_files LIMIT 1")
        .fetch_one(&store.readers)
        .await
        .unwrap();
    std::fs::remove_file(directory.path().join("spool").join(file)).unwrap();
    let id = recovery::enqueue_asset(&store, "backup", "zip", json!({}))
        .await
        .unwrap();
    let worker = tokio::spawn(recovery::worker(store.clone()));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if recovery::asset(&store, &id).await.unwrap().state == "failed" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let failed = recovery::asset(&store, &id).await.unwrap();
    assert!(!recovery::asset_path(&store, &failed).unwrap().exists());
    let previous = recovery::asset(&store, &prior).await.unwrap();
    assert_eq!(
        std::fs::read(recovery::asset_path(&store, &previous).unwrap()).unwrap(),
        std::fs::read(package).unwrap()
    );
    worker.abort();
    let _ = worker.await;
    store.close().await;
}

#[tokio::test]
async fn recovery_http_requires_owner_csrf_and_explicit_confirmation() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    seed(&store).await;
    let config = scrobbl_rs::config::Config {
        port: 3210,
        bind_ip: "127.0.0.1".parse().unwrap(),
        external_origin: None,
        data_dir: directory.path().into(),
        sync_seconds: 120,
        now_playing_seconds: 30,
        import_limits: Default::default(),
        env_client_id: None,
        env_client_secret: None,
    };
    let app = scrobbl_rs::web::router(scrobbl_rs::AppState::new(store.clone(), config));
    let token = scrobbl_rs::auth::random_secret();
    sqlx::query("INSERT INTO sessions VALUES(?,'test-csrf',9999999999999)")
        .bind(scrobbl_rs::auth::digest(&token))
        .execute(&store.private)
        .await
        .unwrap();
    let cookie = format!("scrobbl_session={token}");
    let (_work, package, manifest) = backup(&store).await;
    let asset_id = register(&store, &package, &manifest).await;
    let verification = recovery::enqueue(&store, "recovery_verify", json!({"asset_id":asset_id}))
        .await
        .unwrap();
    for job_state in ["queued", "running", "waiting", "completed"] {
        sqlx::query("UPDATE jobs SET state=? WHERE id=?")
            .bind(job_state)
            .bind(&verification)
            .execute(&store.writer)
            .await
            .unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/recovery/{asset_id}"))
                    .header("host", "127.0.0.1:3210")
                    .header("origin", "http://127.0.0.1:3210")
                    .header("cookie", &cookie)
                    .header("x-csrf-token", "test-csrf")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"action":"delete"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if job_state == "completed" {
                StatusCode::OK
            } else {
                StatusCode::CONFLICT
            },
            "verification {job_state}"
        );
    }
    for (method, path, authenticated, csrf, body, expected) in [
        (
            "GET",
            "/api/v1/recovery",
            false,
            false,
            json!({}),
            StatusCode::UNAUTHORIZED,
        ),
        (
            "POST",
            "/api/v1/recovery",
            true,
            false,
            json!({"kind":"backup"}),
            StatusCode::FORBIDDEN,
        ),
        (
            "POST",
            "/api/v1/recovery/restore",
            true,
            true,
            json!({"asset_id":"0".repeat(64),"password":"a completely new password","confirm_replace":false}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "POST",
            "/api/v1/recovery",
            true,
            true,
            json!({"kind":"export","format":"csv","selection":{"from":"2026-12-01","to":"2026-01-01"}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "POST",
            "/api/v1/recovery",
            true,
            true,
            json!({"kind":"backup"}),
            StatusCode::OK,
        ),
    ] {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:3210")
            .header("origin", "http://127.0.0.1:3210")
            .header("content-type", "application/json");
        if authenticated {
            request = request.header("cookie", &cookie);
        }
        if csrf {
            request = request.header("x-csrf-token", "test-csrf");
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{path}");
    }
    assert!(!directory.path().join("recovery.pending").exists());
    store.close().await;
}
