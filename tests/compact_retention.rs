use scrobbl_rs::{
    db, exports, imports::parse_extended, jobs::SourceRow, recovery, retention, storage,
};
use serde_json::json;
use sqlx::Connection;

fn source() -> String {
    json!([{"ts":"2026-01-01T00:00:00Z","ms_played":60000,
        "master_metadata_track_name":"Song","master_metadata_album_artist_name":"Artist",
        "master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:0000000000000000000001",
        "unmodeled":"original bytes must not survive"}]).to_string()
}

async fn imported(store: &storage::Storage, file: &str) -> String {
    let id = store
        .enqueue("test", Some(file), Some("history.json"), &json!({}))
        .await
        .unwrap();
    let lease = store.claim("test").await.unwrap().unwrap();
    let rows: Vec<_> = parse_extended(source().as_bytes())
        .unwrap()
        .0
        .into_iter()
        .map(SourceRow::from)
        .collect();
    store
        .commit_batch(&lease, "history.json", &rows, true)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn completed_import_releases_sources_and_reupload_preserves_identity_and_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    let bytes = source();
    let file = store.receive(&mut bytes.as_bytes(), 10000).await.unwrap();
    imported(&store, &file).await;
    retention::cleanup(&store).await.unwrap();
    assert!(!dir.path().join("spool").join(&file).exists());
    assert!(
        store
            .enqueue("test", Some(&file), None, &json!({}))
            .await
            .is_err()
    );

    // A backup preserves metadata for a released source without demanding its bytes.
    let work = tempfile::tempdir().unwrap();
    let package = work.path().join("backup.zip");
    let manifest = recovery::create_backup(&store, work.path(), &package)
        .await
        .unwrap();
    assert_eq!(manifest.files.len(), 1);
    let trial = tempfile::tempdir().unwrap();
    recovery::trial_restore(&package, trial.path())
        .await
        .unwrap();
    let mut conn = recovery::connect(&trial.path().join("archive.db"), true)
        .await
        .unwrap();
    let counts = recovery::validate_database(&mut conn).await.unwrap();
    assert_eq!(counts["source_files"], 1);
    assert_eq!(counts["source_occurrences"], 1);
    conn.close().await.unwrap();

    let again = store.receive(&mut bytes.as_bytes(), 10000).await.unwrap();
    assert_eq!(again, file);
    imported(&store, &again).await;
    retention::cleanup(&store).await.unwrap();
    let totals: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM listens),(SELECT COUNT(*) FROM observations),(SELECT COUNT(*) FROM observation_variants),(SELECT COUNT(*) FROM source_occurrences)")
        .fetch_one(&store.readers).await.unwrap();
    assert_eq!(totals, (1, 1, 1, 2));
    let frozen = dir.path().join("export.db");
    recovery::freeze(&store, &frozen).await.unwrap();
    let output = dir.path().join("export.json");
    exports::write(&frozen, &output, "json", &Default::default())
        .await
        .unwrap();
    let text = std::fs::read_to_string(output).unwrap();
    assert!(!text.contains("unmodeled"));
    let data: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(data["format_version"], 2);
    assert_eq!(data["original_retention"], "compact");
    let observation = &data["listens"][0]["observations"][0];
    assert_eq!(observation["raw"], serde_json::Value::Null);
    assert_eq!(observation["raw_available"], false);
    assert_eq!(
        observation["raw_fingerprint"],
        db::payload_fingerprint(&parse_extended(bytes.as_bytes()).unwrap().0[0].raw)
    );
    assert_eq!(
        observation["variants"][0]["occurrences"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        observation["variants"][0]["occurrences"][0]["source_retained"],
        false
    );
    store.close().await;
}

#[tokio::test]
async fn shared_source_waits_for_pending_failed_partial_and_cancelled_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    let file = store
        .receive(&mut source().as_bytes(), 10000)
        .await
        .unwrap();
    imported(&store, &file).await;
    let second = store
        .enqueue("pending", Some(&file), None, &json!({}))
        .await
        .unwrap();
    for (state, outcome) in [
        ("queued", None),
        ("running", None),
        ("waiting", None),
        ("failed", Some("error")),
        ("completed", Some("partial")),
        ("cancelled", None),
    ] {
        sqlx::query("UPDATE jobs SET state=?,outcome=? WHERE id=?")
            .bind(state)
            .bind(outcome)
            .bind(&second)
            .execute(&store.writer)
            .await
            .unwrap();
        retention::cleanup(&store).await.unwrap();
        assert!(
            dir.path().join("spool").join(&file).exists(),
            "{state}/{outcome:?}"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT retained FROM source_files WHERE id=?")
                .bind(&file)
                .fetch_one(&store.readers)
                .await
                .unwrap(),
            1
        );
    }
    // Model a crash after committing unavailability but before unlinking.
    sqlx::query("UPDATE jobs SET state='completed',outcome='complete' WHERE id=?")
        .bind(&second)
        .execute(&store.writer)
        .await
        .unwrap();
    sqlx::query("UPDATE source_files SET retained=0")
        .execute(&store.writer)
        .await
        .unwrap();
    store.close().await;
    let store = storage::open(dir.path()).await.unwrap();
    retention::cleanup(&store).await.unwrap();
    assert!(!dir.path().join("spool").join(file).exists());
    store.close().await;
}

#[tokio::test]
async fn legacy_upgrade_copies_keep_their_live_sources_pinned() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    let file = store
        .receive(&mut source().as_bytes(), 10000)
        .await
        .unwrap();
    imported(&store, &file).await;
    let old = dir.path().join("backups/pre-upgrade-legacy");
    std::fs::create_dir(&old).unwrap();
    std::fs::write(
        old.join("complete.json"),
        json!({"spool":"unchanged immutable files in data directory","files":[{"sha256":file}]})
            .to_string(),
    )
    .unwrap();
    retention::cleanup(&store).await.unwrap();
    assert!(dir.path().join("spool").join(file).is_file());
    store.close().await;
}
