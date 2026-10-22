use scrobbl_rs::{imports::parse_extended, jobs::SourceRow, recovery, storage};
use serde_json::json;
use sqlx::{Connection, Executor};

/// Exercise real import/provenance writes. Large originals must not be stored
/// again for the observation and for every occurrence of the same variant.
#[tokio::test]
async fn compact_import_retains_fingerprints_and_provenance_without_raw_payloads() {
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    let rows: Vec<_> = (0..256)
        .map(|n| {
            let timestamp = chrono::DateTime::from_timestamp(1_700_000_000 + n * 180, 0)
                .unwrap()
                .to_rfc3339();
            SourceRow::from(
                parse_extended(
                    json!([{
                        "ts":timestamp,"ms_played":180000,
                        "master_metadata_track_name":"Song",
                        "master_metadata_album_artist_name":"Artist",
                        "master_metadata_album_album_name":"Album",
                        "spotify_track_uri":"spotify:track:0000000000000000000001",
                        "extra_original_metadata":"x".repeat(8192)
                    }])
                    .to_string()
                    .as_bytes(),
                )
                .unwrap()
                .0
                .remove(0),
            )
        })
        .collect();
    for _ in 0..3 {
        let id = store
            .enqueue("space_test", None, Some("synthetic.json"), &json!({}))
            .await
            .unwrap();
        let job = store.claim("space_test").await.unwrap().unwrap();
        store
            .commit_batch(&job, "synthetic.json", &rows, true)
            .await
            .unwrap();
        assert_eq!(store.job(&id).await.unwrap().checkpoint, 256);
    }
    let frozen = directory.path().join("compact.db");
    recovery::freeze(&store, &frozen).await.unwrap();
    let bytes = std::fs::metadata(&frozen).unwrap().len();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM original_payloads")
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        256
    );
    let retained: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM original_payloads WHERE raw_json!='null' OR length(fingerprint)!=64 OR fingerprint IS NULL").fetch_one(&store.readers).await.unwrap();
    assert_eq!(retained, 0);
    println!("SPACE observations=256 occurrences=768 compact_archive_bytes={bytes}");
    assert!(
        bytes < 1536 * 1024,
        "Repeated originals consume {bytes} bytes; budget excludes raw payloads and allows normalized data, hashes and provenance"
    );
    store.close().await;
}

async fn legacy_archive(path: &std::path::Path) -> sqlx::SqliteConnection {
    let mut conn = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    let migrations = sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(
            storage::ARCHIVE_MIGRATOR
                .iter()
                .filter(|m| m.version < 7)
                .cloned()
                .collect(),
        ),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    migrations.run_direct(None, &mut conn, false).await.unwrap();
    conn.execute("INSERT INTO albums VALUES('album','Album',NULL,'rank');
        INSERT INTO tracks VALUES('track','Song','album',NULL,NULL,0,'rank');
        INSERT INTO listens(id,track_id,played_at_ms,first_acquired_at_ms,calendar_id,local_date,week_date,anchor_source,listened_ms,duration_kind,counted) VALUES('listen','track',1,1,'initial','2026-01-01','2025-12-29','spotify_import',1000,'actual',1);
        INSERT INTO observations(id,source,source_key,listen_id,source_timestamp_ms,raw_json) VALUES('obs','spotify_import','key','listen',1,'{\"original\":true}');
        INSERT INTO observation_variants VALUES('variant','obs','fingerprint','{\"original\":true}',1,1000,'actual',1,1);
        INSERT INTO observation_variants VALUES('variant2','obs','different','{\"variant\":true}',1,2000,'actual',1,1);
        INSERT INTO jobs(id,kind,state,created_at_ms,updated_at_ms) VALUES('job','test','completed',1,1);
        INSERT INTO source_occurrences VALUES('job',NULL,'file.json',1,'variant','{\"original\":true}','accepted',NULL);
        INSERT INTO source_occurrences VALUES('job',NULL,'file.json',2,'variant','{\"original\":true}','accepted',NULL);
        INSERT INTO source_occurrences VALUES('job',NULL,'file.json',3,NULL,'{\"invalid\":true}','invalid','reason');
        INSERT INTO source_occurrences VALUES('job',NULL,'file.json',4,NULL,'{\"skipped\":true}','skipped','reason');
        INSERT INTO source_occurrences VALUES('job',NULL,'file.json',5,'variant2','{\"legacy_mismatch\":true}','accepted',NULL);").await.unwrap();
    conn
}

#[tokio::test]
async fn shared_payload_migration_preserves_legacy_originals_and_foreign_keys() {
    let directory = tempfile::tempdir().unwrap();
    let mut conn = legacy_archive(&directory.path().join("legacy.db")).await;
    // Also preserve an original without a matching variant, rather than silently
    // replacing it with a different variant's payload during migration.
    conn.execute("UPDATE observations SET raw_json='{\"first_original\":true}'")
        .await
        .unwrap();
    storage::ARCHIVE_MIGRATOR
        .run_direct(None, &mut conn, false)
        .await
        .unwrap();
    recovery::validate_database(&mut conn).await.unwrap();
    let raw: String = sqlx::query_scalar(
        "SELECT p.raw_json FROM observations o JOIN original_payloads p ON p.id=o.payload_id",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(raw, "{\"first_original\":true}");
    let raws: Vec<String> = sqlx::query_scalar("SELECT p.raw_json FROM source_occurrences o JOIN original_payloads p ON p.id=o.payload_id ORDER BY o.row_number").fetch_all(&mut conn).await.unwrap();
    assert_eq!(
        raws,
        [
            "{\"original\":true}",
            "{\"original\":true}",
            "{\"invalid\":true}",
            "{\"skipped\":true}",
            "{\"legacy_mismatch\":true}"
        ]
    );
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT payload_id FROM source_occurrences WHERE row_number<=2 ORDER BY row_number",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    assert_eq!(ids[0], ids[1]);
    assert!(
        conn.execute("UPDATE observations SET payload_id=NULL")
            .await
            .is_err()
    );
    assert!(
        conn.execute("UPDATE source_occurrences SET payload_id=-1")
            .await
            .is_err()
    );
    conn.close().await.unwrap();
}

#[tokio::test]
async fn schema_six_portable_backup_restores_normalized_history_and_original_fingerprints() {
    use std::io::Write;
    let source = tempfile::tempdir().unwrap();
    let database = source.path().join("archive.db");
    let mut conn = legacy_archive(&database).await;
    let counts = recovery::validate_database(&mut conn).await.unwrap();
    let identity: (String, String, i64) =
        sqlx::query_as("SELECT instance_id,generation,revision FROM archive_state")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    conn.close().await.unwrap();
    let (sha256, size_bytes) = recovery::hash_file(&database).unwrap();
    let manifest = recovery::Manifest {
        format: "scrobbl-backup".into(),
        format_version: 1,
        app_version: "0.1.0".into(),
        schema_version: 6,
        created_at_ms: 1,
        instance_id: identity.0,
        generation: identity.1,
        revision: identity.2,
        counts,
        files: vec![recovery::InventoryFile {
            name: "archive.db".into(),
            sha256,
            size_bytes,
        }],
    };
    let package = source.path().join("legacy.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&package).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("archive.db", options).unwrap();
    std::io::copy(&mut std::fs::File::open(&database).unwrap(), &mut zip).unwrap();
    zip.start_file("manifest.json", options).unwrap();
    zip.write_all(&serde_json::to_vec(&manifest).unwrap())
        .unwrap();
    zip.finish().unwrap();
    let trial = tempfile::tempdir().unwrap();
    recovery::trial_restore(&package, trial.path())
        .await
        .unwrap();
    let mut restored = recovery::connect(&trial.path().join("archive.db"), true)
        .await
        .unwrap();
    let counts = recovery::validate_database(&mut restored).await.unwrap();
    assert_eq!(counts["listens"], 1);
    assert_eq!(counts["source_occurrences"], 5);
    let (raw, fingerprint): (String, String) = sqlx::query_as(
        "SELECT p.raw_json,p.fingerprint FROM observations o JOIN original_payloads p ON p.id=o.payload_id",
    ).fetch_one(&mut restored).await.unwrap();
    assert_eq!(raw, "null");
    assert_eq!(
        fingerprint,
        scrobbl_rs::db::payload_fingerprint(&json!({"original":true}))
    );
    restored.close().await.unwrap();
}

#[tokio::test]
async fn completed_checkpoint_releases_oversized_reusable_wal() {
    let directory = tempfile::tempdir().unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    // Lower only the checkpoint threshold to keep this retention regression small.
    sqlx::query("PRAGMA wal_autocheckpoint=16")
        .execute(&store.writer)
        .await
        .unwrap();
    sqlx::query("INSERT INTO original_payloads(raw_json) VALUES(?)")
        .bind(json!({"padding":"x".repeat(10*1024*1024)}).to_string())
        .execute(&store.writer)
        .await
        .unwrap();
    // The following write resets/reuses the checkpointed WAL with the configured
    // retention policy. This does not assert a limit while readers pin old pages.
    sqlx::query("INSERT INTO settings VALUES('wal_test','1')")
        .execute(&store.writer)
        .await
        .unwrap();
    let bytes = std::fs::metadata(directory.path().join("archive.db-wal"))
        .unwrap()
        .len();
    println!("SPACE reusable_wal_bytes={bytes}");
    assert!(
        bytes <= 8 * 1024 * 1024,
        "WAL retains an oversized checkpointed allocation: {bytes}"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        "ok"
    );
    store.close().await;
}

#[tokio::test]
async fn startup_compacts_existing_archives_and_keeps_the_pre_upgrade_copy() {
    let directory = tempfile::tempdir().unwrap();
    // Create the private store/directory structure, then replace ONLY this test's
    // empty archive with a genuine schema-6 database.
    storage::open(directory.path()).await.unwrap().close().await;
    std::fs::remove_file(directory.path().join("archive.db")).unwrap();
    let mut conn = legacy_archive(&directory.path().join("archive.db")).await;
    let wide = json!({"padding":"x".repeat(8192)}).to_string();
    sqlx::query("UPDATE observations SET raw_json=?")
        .bind(&wide)
        .execute(&mut conn)
        .await
        .unwrap();
    sqlx::query("UPDATE observation_variants SET raw_json=?")
        .bind(&wide)
        .execute(&mut conn)
        .await
        .unwrap();
    sqlx::query("UPDATE source_occurrences SET raw_json=?")
        .bind(&wide)
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();
    let before = std::fs::metadata(directory.path().join("archive.db"))
        .unwrap()
        .len();
    let store = storage::open(directory.path()).await.unwrap();
    let after = std::fs::metadata(directory.path().join("archive.db"))
        .unwrap()
        .len();
    println!("SPACE upgraded_archive_before={before} after={after}");
    assert!(after < before);
    let safety = std::fs::read_dir(directory.path().join("backups"))
        .unwrap()
        .find_map(|e| {
            let path = e.unwrap().path();
            path.join("complete.json").is_file().then_some(path)
        })
        .unwrap();
    let mut previous = recovery::connect(&safety.join("archive.db"), true)
        .await
        .unwrap();
    let old: String = sqlx::query_scalar("SELECT raw_json FROM observations")
        .fetch_one(&mut previous)
        .await
        .unwrap();
    assert_eq!(old, wide);
    previous.close().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM settings WHERE key='original_payload_compaction_pending'"
        )
        .fetch_one(&store.readers)
        .await
        .unwrap(),
        0
    );
    store.close().await;
}

#[tokio::test]
async fn compact_upgrade_keeps_independent_sources_for_offline_rollback_after_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    storage::open(directory.path()).await.unwrap().close().await;
    std::fs::remove_file(directory.path().join("archive.db")).unwrap();
    let mut conn = legacy_archive(&directory.path().join("archive.db")).await;
    let seven = sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(
            storage::ARCHIVE_MIGRATOR
                .iter()
                .filter(|m| m.version < 8)
                .cloned()
                .collect(),
        ),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    seven.run_direct(None, &mut conn, false).await.unwrap();
    let bytes = b"complete immutable history";
    let hash = scrobbl_rs::auth::digest(std::str::from_utf8(bytes).unwrap());
    std::fs::write(directory.path().join("spool").join(&hash), bytes).unwrap();
    sqlx::query("INSERT INTO source_files VALUES(?,?,?,1)")
        .bind(&hash)
        .bind(&hash)
        .bind(bytes.len() as i64)
        .execute(&mut conn)
        .await
        .unwrap();
    sqlx::query("UPDATE jobs SET file_id=?")
        .bind(&hash)
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();

    let store = storage::open(directory.path()).await.unwrap();
    scrobbl_rs::retention::cleanup(&store).await.unwrap();
    assert!(!directory.path().join("spool").join(&hash).exists());
    let mut compact = store.writer.acquire().await.unwrap();
    recovery::validate_database(&mut compact).await.unwrap();
    let original: (String,String) = sqlx::query_as("SELECT raw_json,fingerprint FROM original_payloads WHERE id=(SELECT payload_id FROM observations)")
        .fetch_one(&mut *compact).await.unwrap();
    assert_eq!(original.0, "null");
    assert_eq!(
        original.1,
        scrobbl_rs::db::payload_fingerprint(&json!({"original":true}))
    );
    drop(compact);
    store.close().await;

    let safety = std::fs::read_dir(directory.path().join("backups"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.join("complete.json").is_file())
        .unwrap();
    assert_eq!(
        std::fs::read(safety.join("spool").join(&hash)).unwrap(),
        bytes
    );
    let target = directory.path().join("offline-rollback");
    recovery::recover_upgrade(
        directory.path(),
        safety.file_name().unwrap().to_str().unwrap(),
        &target,
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(target.join("spool").join(hash)).unwrap(),
        bytes
    );
    let mut previous = recovery::connect(&target.join("archive.db"), true)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&mut previous)
            .await
            .unwrap(),
        7
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT raw_json FROM original_payloads WHERE id=(SELECT payload_id FROM observations)"
        )
        .fetch_one(&mut previous)
        .await
        .unwrap(),
        "{\"original\":true}"
    );
    previous.close().await.unwrap();
}

#[tokio::test]
async fn compact_upgrade_resumes_after_a_committed_payload_batch() {
    let directory = tempfile::tempdir().unwrap();
    storage::open(directory.path()).await.unwrap().close().await;
    std::fs::remove_file(directory.path().join("archive.db")).unwrap();
    let mut conn = legacy_archive(&directory.path().join("archive.db")).await;
    storage::ARCHIVE_MIGRATOR
        .run_direct(None, &mut conn, false)
        .await
        .unwrap();
    let (id, raw): (i64, String) =
        sqlx::query_as("SELECT id,raw_json FROM original_payloads ORDER BY id LIMIT 1")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    let fingerprint = scrobbl_rs::db::payload_fingerprint(
        &serde_json::from_str::<serde_json::Value>(&raw).unwrap(),
    );
    // Simulate a crash after one bounded transaction; the pending marker remains.
    sqlx::query("UPDATE original_payloads SET raw_json='null',fingerprint=? WHERE id=?")
        .bind(&fingerprint)
        .bind(id)
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();
    let store = storage::open(directory.path()).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT fingerprint FROM original_payloads WHERE id=?")
            .bind(id)
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        fingerprint
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM original_payloads WHERE raw_json!='null' OR fingerprint IS NULL"
        )
        .fetch_one(&store.readers)
        .await
        .unwrap(),
        0
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM settings WHERE key IN ('compact_retention_pending','original_payload_compaction_pending')").fetch_one(&store.readers).await.unwrap(),0);
    let mut conn = store.writer.acquire().await.unwrap();
    recovery::validate_database(&mut conn).await.unwrap();
    drop(conn);
    store.close().await;
}
