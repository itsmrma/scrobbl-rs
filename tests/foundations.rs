use scrobbl_rs::{
    auth,
    calendar::Calendar,
    consultation::{Position, SnapshotLimits},
    db::{self, Event},
    imports::parse_extended,
    jobs::{Job, SourceRow},
    storage::{self, Storage},
};
use serde_json::json;
use sqlx::{Connection, Row, sqlite::SqliteConnectOptions};
use std::{process::Command, time::Duration};

fn event(index: i64) -> Event {
    let end = chrono::DateTime::from_timestamp_millis(1_748_800_000_000 + index * 180_000)
        .unwrap()
        .to_rfc3339();
    parse_extended(json!([{"ts":end,"ms_played":180000,"master_metadata_track_name":"Song","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:0000000000000000000001"}]).to_string().as_bytes()).unwrap().0.remove(0)
}
async fn save(store: &Storage, events: &[Event]) {
    let mut tx = store.writer.begin().await.unwrap();
    db::ingest(&mut tx, events).await.unwrap();
    tx.commit().await.unwrap();
}
async fn job(store: &Storage) -> Job {
    let mut bytes = &b"original upload"[..];
    let file = store.receive(&mut bytes, 1024).await.unwrap();
    store
        .enqueue(
            "test_import",
            Some(&file),
            Some("../../history.json"),
            &json!({}),
        )
        .await
        .unwrap();
    store.claim("test_import").await.unwrap().unwrap()
}

#[tokio::test]
async fn variants_occurrences_reimports_and_atomic_checkpoints() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    let lease = job(&store).await;
    let original = event(0);
    let mut variant = original.clone();
    variant.raw["new_metadata"] = json!("retained");
    let rows = vec![
        SourceRow::from(original.clone()),
        SourceRow::from(original),
        SourceRow::from(variant),
        SourceRow::Invalid {
            raw: json!({"broken":true}),
            reason: "missing timestamp".into(),
        },
        SourceRow::Skipped {
            raw: json!({"podcast":true}),
            reason: "not music".into(),
        },
    ];
    let result = store
        .commit_batch(&lease, "member.json", &rows, true)
        .await
        .unwrap();
    assert_eq!(
        (
            result.added,
            result.duplicates,
            result.invalid,
            result.skipped
        ),
        (1, 1, 1, 1)
    );
    assert_eq!((result.originals, result.variants), (1, 1));
    assert_eq!(store.job(&lease.id).await.unwrap().checkpoint, 5);
    let counts:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM listens),(SELECT COUNT(*) FROM observations),(SELECT COUNT(*) FROM observation_variants),(SELECT COUNT(*) FROM source_occurrences)").fetch_one(&store.readers).await.unwrap();
    assert_eq!(counts, (1, 1, 2, 5));
    let later = job(&store).await;
    store
        .commit_batch(&later, "history.json", &[SourceRow::from(event(0))], true)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM source_occurrences")
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        6
    );
    // A failure after ingesting the first row must roll back music, provenance and progress.
    let bad = job(&store).await;
    let before = store.read_view().await.unwrap().identity;
    let mut inconsistent = event(2);
    inconsistent.listened_ms = None;
    assert!(
        store
            .commit_batch(
                &bad,
                "bad.json",
                &[SourceRow::from(event(1)), SourceRow::from(inconsistent)],
                false
            )
            .await
            .is_err()
    );
    assert_eq!(store.job(&bad.id).await.unwrap().checkpoint, 0);
    assert_eq!(store.read_view().await.unwrap().identity, before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens")
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        1
    );
    store.close().await;
}

#[tokio::test]
async fn interrupted_spool_cancellation_waits_and_attempt_fencing() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    assert!(store.receive(&mut &b"too big"[..], 2).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM source_files")
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("spool/incoming"))
            .unwrap()
            .count(),
        0
    );
    let lease = job(&store).await;
    store
        .commit_batch(&lease, "m", &[SourceRow::from(event(0))], false)
        .await
        .unwrap();
    assert!(
        store
            .commit_batch(&lease, "m", &[SourceRow::from(event(1))], false)
            .await
            .is_err()
    );
    let progress = store.job(&lease.id).await.unwrap();
    store.cancel(&lease.id).await.unwrap();
    assert!(
        store
            .commit_batch(&progress, "m", &[SourceRow::from(event(1))], true)
            .await
            .is_err()
    );
    store.close().await;
    let store = storage::open(dir.path()).await.unwrap();
    assert!(store.claim("test_import").await.unwrap().is_none());
    store.retry(&lease.id).await.unwrap();
    let resumed = store.claim("test_import").await.unwrap().unwrap();
    assert_eq!(resumed.checkpoint, 1);
    assert!(resumed.attempt > lease.attempt);
    store
        .pause(&resumed, Some(auth::now_ms() + 60000), "rate_limit")
        .await
        .unwrap();
    assert!(store.claim("test_import").await.unwrap().is_none());
    store.close().await;
}

#[test]
fn process_crash_helper() {
    let Some(path) = std::env::var_os("SCROBBL_FOUNDATIONS_CRASH_DIR") else {
        return;
    };
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let store = storage::open(std::path::Path::new(&path)).await.unwrap();
        let lease = store.claim("test_import").await.unwrap().unwrap();
        store
            .commit_batch(&lease, "member", &[SourceRow::from(event(0))], false)
            .await
            .unwrap();
        let mut tx = store.writer.begin().await.unwrap();
        db::ingest(&mut tx, &[event(1)]).await.unwrap();
        // Bypass destructors: SQLite must recover the uncommitted second row from WAL.
        std::process::exit(23);
    });
}

#[tokio::test]
async fn process_crash_resumes_only_confirmed_progress() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    let mut bytes = &b"persistent file"[..];
    let file = store.receive(&mut bytes, 1024).await.unwrap();
    let id = store
        .enqueue("test_import", Some(&file), Some("history.json"), &json!({}))
        .await
        .unwrap();
    store.close().await;
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "process_crash_helper", "--nocapture"])
        .env("SCROBBL_FOUNDATIONS_CRASH_DIR", dir.path())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(23),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let store = storage::open(dir.path()).await.unwrap();
    let recovered = store.job(&id).await.unwrap();
    assert_eq!(recovered.state, "queued");
    assert_eq!(recovered.checkpoint, 1);
    assert!(dir.path().join("spool").join(file).is_file());
    let resumed = store.claim("test_import").await.unwrap().unwrap();
    assert_eq!(resumed.attempt, 2);
    store
        .commit_batch(&resumed, "member", &[SourceRow::from(event(1))], true)
        .await
        .unwrap();
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM listens),(SELECT COUNT(*) FROM source_occurrences)",
    )
    .fetch_one(&store.readers)
    .await
    .unwrap();
    assert_eq!(counts, (2, 2));
    store.close().await;
}

#[tokio::test]
async fn mixed_calendars_preserve_acquisition_and_global_recalculation_rebases() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    save(&store, &[event(0)]).await;
    let before: (String, i64) =
        sqlx::query_as("SELECT calendar_id,first_acquired_at_ms FROM listens")
            .fetch_one(&store.readers)
            .await
            .unwrap();
    store
        .change_calendar("America/Los_Angeles", 6, false)
        .await
        .unwrap();
    let mut variant = event(0);
    variant.raw["changed"] = json!(true);
    save(&store, &[variant, event(1)]).await;
    let same: (String, i64) = sqlx::query_as(
        "SELECT calendar_id,first_acquired_at_ms FROM listens ORDER BY played_at_ms LIMIT 1",
    )
    .fetch_one(&store.readers)
    .await
    .unwrap();
    assert_eq!(same, before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(DISTINCT calendar_id) FROM listens")
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        2
    );
    let events: Vec<_> = (2..1002).map(event).collect();
    save(&store, &events).await;
    let id = store
        .change_calendar("Asia/Tokyo", 2, true)
        .await
        .unwrap()
        .unwrap();
    let lease = store.claim("calendar").await.unwrap().unwrap();
    assert!(!store.calendar_step(&lease).await.unwrap());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(DISTINCT calendar_id) FROM listens")
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        2
    );
    save(&store, &[event(1002)]).await;
    let lease = store.job(&id).await.unwrap();
    assert!(!store.calendar_step(&lease).await.unwrap());
    assert_eq!(store.job(&id).await.unwrap().checkpoint, 0); // concurrent write caused rebase
    loop {
        let lease = store.job(&id).await.unwrap();
        if store.calendar_step(&lease).await.unwrap() {
            break;
        }
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(DISTINCT calendar_id) FROM listens")
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        1
    );
    let calendar: Calendar = sqlx::query_as(
        "SELECT c.* FROM calendars c JOIN settings s ON s.value=c.id WHERE s.key='calendar_id'",
    )
    .fetch_one(&store.readers)
    .await
    .unwrap();
    for row in sqlx::query("SELECT played_at_ms,local_date,week_date FROM listens")
        .fetch_all(&store.readers)
        .await
        .unwrap()
    {
        assert_eq!(
            calendar.dates(row.get("played_at_ms")).unwrap(),
            (
                row.get::<String, _>("local_date"),
                row.get::<String, _>("week_date")
            )
        );
    }
    let id = store
        .change_calendar("UTC", 0, true)
        .await
        .unwrap()
        .unwrap();
    let lease = store.claim("calendar").await.unwrap().unwrap();
    assert!(!store.calendar_step(&lease).await.unwrap());
    store.cancel(&id).await.unwrap();
    assert!(
        store
            .calendar_step(&store.job(&id).await.unwrap())
            .await
            .is_err()
    );
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT timezone FROM calendars c JOIN settings s ON s.value=c.id WHERE s.key='calendar_id'").fetch_one(&store.readers).await.unwrap(),"Asia/Tokyo");
    store.close().await;
}

#[test]
fn assigned_calendar_handles_dst_dates_and_week_starts() {
    let calendar = Calendar {
        id: "test".into(),
        timezone: "Europe/Rome".into(),
        week_start: 6,
        rules_version: 1,
    };
    for timestamp in ["2025-03-30T00:30:00Z", "2025-03-30T01:30:00Z"] {
        let ms = chrono::DateTime::parse_from_rfc3339(timestamp)
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            calendar.dates(ms).unwrap(),
            ("2025-03-30".into(), "2025-03-30".into())
        );
    }
}

#[tokio::test]
async fn concurrent_writers_coherent_views_snapshots_and_revision_retention() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    let initial = store.read_view().await.unwrap().identity;
    let mut writers = Vec::new();
    for n in 0..12 {
        let store = store.clone();
        writers.push(tokio::spawn(async move {
            save(&store, &[event(n)]).await;
        }));
    }
    for writer in writers {
        writer.await.unwrap();
    }
    let mut pinned = store.read_view().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM listens")
        .fetch_one(&mut *pinned.transaction)
        .await
        .unwrap();
    assert_eq!(count, 12);
    save(&store, &[event(12)]).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens")
            .fetch_one(&mut *pinned.transaction)
            .await
            .unwrap(),
        12
    );
    drop(pinned);
    let limits = SnapshotLimits {
        max_copies: 1,
        ..Default::default()
    };
    let frozen = store.materialize(&limits).await.unwrap();
    save(&store, &[event(13)]).await;
    let (_, first) = store.snapshot_page(&frozen.id, None, 5).await.unwrap();
    assert_eq!(first.len(), 5);
    let last = first.last().unwrap();
    let (_, rest) = store
        .snapshot_page(
            &frozen.id,
            Some(&Position {
                played_at_ms: last.played_at_ms,
                id: last.id.clone(),
            }),
            1000,
        )
        .await
        .unwrap();
    assert_eq!(rest.len(), 8);
    assert!(store.materialize(&limits).await.is_err());
    let mut cursor = initial;
    let mut seen = 0;
    loop {
        let (next, changes) = store.changes(&cursor, 1).await.unwrap();
        seen += changes.len();
        if next.revision == cursor.revision {
            break;
        }
        cursor = next;
    }
    assert!(seen >= 14);
    store.prune_changes(cursor.revision).await.unwrap();
    let mut old = cursor.clone();
    old.revision = 0;
    assert!(store.changes(&old, 10).await.is_err());
    let mut other = cursor.clone();
    other.generation = "different".into();
    assert!(store.changes(&other, 10).await.is_err());
    let expiry = SnapshotLimits {
        ttl: Duration::ZERO,
        ..Default::default()
    };
    let expired = store.materialize(&expiry).await.unwrap();
    assert!(store.snapshot_page(&expired.id, None, 1).await.is_err());
    assert!(store.snapshot_page("../../archive", None, 1).await.is_err());
    store.close().await;
}

#[tokio::test]
async fn lock_secrets_read_only_pool_and_access_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    assert!(storage::open(dir.path()).await.is_err());
    let output = Command::new(env!("CARGO_BIN_EXE_scrobbl-rs"))
        .env("SCROBBL_DATA_DIR", dir.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Another process"));
    assert!(
        sqlx::query("INSERT INTO settings VALUES('bad','writer')")
            .execute(&store.readers)
            .await
            .is_err()
    );
    sqlx::query("INSERT INTO settings VALUES('spotify_client_secret','private-marker')")
        .execute(&store.private)
        .await
        .unwrap();
    sqlx::query("INSERT INTO access_operations VALUES('interrupted','spotify_access_changed',0)")
        .execute(&store.private)
        .await
        .unwrap();
    sqlx::query("INSERT INTO settings VALUES('spotify_error','old')")
        .execute(&store.writer)
        .await
        .unwrap();
    assert!(
        sqlx::query("SELECT * FROM owner")
            .fetch_all(&store.writer)
            .await
            .is_err()
    );
    store.close().await;
    let store = storage::open(dir.path()).await.unwrap();
    assert!(
        db::setting(&store.readers, "spotify_error")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT completed FROM access_operations")
            .fetch_one(&store.private)
            .await
            .unwrap(),
        1
    );
    assert!(
        !String::from_utf8_lossy(&std::fs::read(dir.path().join("archive.db")).unwrap())
            .contains("private-marker")
    );
    let version: String =
        sqlx::query_scalar("SELECT sqlite_version() || ' ' || sqlite_source_id()")
            .fetch_one(&store.readers)
            .await
            .unwrap();
    println!("SQLite artifact: {version}");
    // Check the library used by SQLx against the native symbols in this binary,
    // rather than accepting the version of a separate system SQLite command.
    let native_version = unsafe {
        std::ffi::CStr::from_ptr(libsqlite3_sys::sqlite3_libversion())
            .to_str()
            .unwrap()
    };
    let native_source = unsafe {
        std::ffi::CStr::from_ptr(libsqlite3_sys::sqlite3_sourceid())
            .to_str()
            .unwrap()
    };
    assert_eq!(version, format!("{native_version} {native_source}"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("PRAGMA synchronous")
            .fetch_one(&store.writer)
            .await
            .unwrap(),
        2
    );
    store.close().await;
    let prototype = tempfile::tempdir().unwrap();
    std::fs::write(prototype.path().join("scrobbl.db"), b"untouched").unwrap();
    assert!(storage::open(prototype.path()).await.is_err());
    assert_eq!(
        std::fs::read(prototype.path().join("scrobbl.db")).unwrap(),
        b"untouched"
    );
}

#[tokio::test]
async fn ambiguous_repetitions_and_conflicting_variants_do_not_silently_replace_actuals() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    let first = event(0);
    let mut changed = first.clone();
    changed.listened_ms = Some(120000);
    changed.raw["ms_played"] = json!(120000);
    save(&store, &[first.clone(), changed.clone()]).await;
    let value: (Option<i64>, String, bool) =
        sqlx::query_as("SELECT listened_ms,duration_kind,needs_review FROM listens")
            .fetch_one(&store.readers)
            .await
            .unwrap();
    assert_eq!(value, (None, "unknown".into(), true));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM conflicts")
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        1
    );
    let initial = store.read_view().await.unwrap().identity;
    save(&store, &[changed, first]).await;
    assert_eq!(store.read_view().await.unwrap().identity, initial);
    let mut unknown_a = event(1);
    unknown_a.track.id = "local:unknown".into();
    let mut unknown_b = unknown_a.clone();
    unknown_b.source_key = "another distinct event".into();
    unknown_b.played_at_ms += 1;
    save(&store, &[unknown_a, unknown_b]).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM tracks")
            .fetch_one(&store.readers)
            .await
            .unwrap(),
        3
    );
    store.close().await;
}

#[tokio::test]
async fn migration_backup_is_verified_before_schema_changes() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    save(&store, &[event(0)]).await;
    store.close().await;
    // Simulate an older release schema. The migration transaction is idempotent for this fixture.
    let mut conn = sqlx::SqliteConnection::connect_with(
        &SqliteConnectOptions::new().filename(dir.path().join("archive.db")),
    )
    .await
    .unwrap();
    sqlx::query("DROP INDEX variants_observation; DROP INDEX occurrences_file; DROP INDEX listens_acquired; DELETE FROM _sqlx_migrations WHERE version=2").execute(&mut conn).await.unwrap();
    conn.close().await.unwrap();
    let upgraded = storage::open(dir.path()).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE version=2")
            .fetch_one(&upgraded.readers)
            .await
            .unwrap(),
        1
    );
    upgraded.close().await;
    let backups: Vec<_> = std::fs::read_dir(dir.path().join("backups"))
        .unwrap()
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = backups[0].as_ref().unwrap().path();
    assert!(backup.join("complete.json").exists());
    let mut conn = sqlx::SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(backup.join("archive.db"))
            .read_only(true),
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM listens")
            .fetch_one(&mut conn)
            .await
            .unwrap(),
        1
    );
    conn.close().await.unwrap();
}

#[tokio::test]
async fn backup_failure_blocks_migration_and_maintenance_blocks_workers() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    save(&store, &[event(0)]).await;
    let lease = job(&store).await;
    let frozen = store.materialize(&SnapshotLimits::default()).await.unwrap();
    let cursor = store.read_view().await.unwrap().identity;
    assert!(store.rotate_generation().await.is_err());
    store.maintenance(true).await.unwrap();
    assert!(
        store
            .commit_batch(&lease, "m", &[SourceRow::from(event(1))], true)
            .await
            .is_err()
    );
    assert!(store.enqueue("new", None, None, &json!({})).await.is_err());
    assert!(store.claim("test_import").await.unwrap().is_none());
    store.rotate_generation().await.unwrap();
    assert!(store.changes(&cursor, 10).await.is_err());
    assert!(store.snapshot_page(&frozen.id, None, 10).await.is_err());
    store.close().await;
    assert!(storage::open(dir.path()).await.is_err());
    let mut conn = sqlx::SqliteConnection::connect_with(
        &SqliteConnectOptions::new().filename(dir.path().join("archive.db")),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE archive_state SET maintenance=0; DROP INDEX variants_observation; DROP INDEX occurrences_file; DROP INDEX listens_acquired; DELETE FROM _sqlx_migrations WHERE version=2").execute(&mut conn).await.unwrap();
    conn.close().await.unwrap();
    // Make backup creation fail, without relying on filesystem permission behavior as administrator.
    std::fs::remove_dir(dir.path().join("backups")).unwrap();
    std::fs::write(dir.path().join("backups"), b"occupied").unwrap();
    assert!(storage::open(dir.path()).await.is_err());
    let mut conn = sqlx::SqliteConnection::connect_with(
        &SqliteConnectOptions::new().filename(dir.path().join("archive.db")),
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE version=2")
            .fetch_one(&mut conn)
            .await
            .unwrap(),
        0
    );
    conn.close().await.unwrap();
}

#[tokio::test]
async fn mixed_calendar_date_selection_uses_assigned_days() {
    use scrobbl_rs::calendar::{Period, filter};
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    let mut a = event(0);
    a.played_at_ms = chrono::DateTime::parse_from_rfc3339("2025-06-01T23:30:00Z")
        .unwrap()
        .timestamp_millis();
    save(&store, &[a.clone()]).await;
    store
        .change_calendar("America/Los_Angeles", 6, false)
        .await
        .unwrap();
    let mut b = a;
    b.source_key = "second-distinct".into();
    save(&store, &[b]).await;
    let mut view = store.read_view().await.unwrap();
    let mut query = sqlx::QueryBuilder::new("SELECT COUNT(*) FROM listens l WHERE 1=1");
    filter(
        &mut view.transaction,
        &mut query,
        &Period::Dates {
            from: Some("2025-06-02".parse().unwrap()),
            through: Some("2025-06-02".parse().unwrap()),
        },
        auth::now_ms(),
    )
    .await
    .unwrap();
    assert_eq!(
        query
            .build_query_scalar::<i64>()
            .fetch_one(&mut *view.transaction)
            .await
            .unwrap(),
        1
    );
    drop(view);
    store.close().await;
}

#[tokio::test]
async fn zip_member_rows_have_independent_positions_and_unknown_inventory_counts() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    let lease = job(&store).await;
    store
        .record_member(
            &lease.id,
            "unreadable.json",
            "invalid",
            "failed",
            None,
            Some("invalid_json"),
        )
        .await
        .unwrap();
    store
        .commit_member_batch(
            &lease,
            "nested/one.json",
            1,
            &[SourceRow::from(event(0))],
            false,
        )
        .await
        .unwrap();
    let lease = store.job(&lease.id).await.unwrap();
    store
        .commit_member_batch(
            &lease,
            "nested/two.json",
            1,
            &[SourceRow::from(event(1))],
            true,
        )
        .await
        .unwrap();
    let positions: Vec<i64> =
        sqlx::query_scalar("SELECT row_number FROM source_occurrences ORDER BY member")
            .fetch_all(&store.readers)
            .await
            .unwrap();
    assert_eq!(positions, vec![1, 1]);
    let count: Option<i64> =
        sqlx::query_scalar("SELECT expected_rows FROM job_members WHERE member='unreadable.json'")
            .fetch_one(&store.readers)
            .await
            .unwrap();
    assert_eq!(count, None);
    assert_eq!(store.job(&lease.id).await.unwrap().checkpoint, 2);
    store.close().await;
}

#[tokio::test]
async fn trash_restore_and_reimport_share_the_change_log() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    save(&store, &[event(0)]).await;
    let id: String = sqlx::query_scalar("SELECT id FROM listens")
        .fetch_one(&store.readers)
        .await
        .unwrap();
    let cursor = store.read_view().await.unwrap().identity;
    let mut tx = store.writer.begin().await.unwrap();
    db::set_trashed(&mut tx, std::slice::from_ref(&id), Some(auth::now_ms()))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let (removed, changes) = store.changes(&cursor, 100).await.unwrap();
    assert_eq!(changes[0].operation, "remove");
    save(&store, &[event(0)]).await;
    assert_eq!(store.read_view().await.unwrap().identity, removed);
    let mut tx = store.writer.begin().await.unwrap();
    db::set_trashed(&mut tx, std::slice::from_ref(&id), None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let (_, changes) = store.changes(&removed, 100).await.unwrap();
    assert_eq!(changes[0].operation, "upsert");
    store.close().await;
}

/// Resource check, not a source parser/import acceptance test. No personal data is accessed.
#[tokio::test]
#[ignore = "synthetic 124 MB spool and 100,000 observations; run explicitly for resource measurements"]
async fn foundation_load() {
    use std::time::Instant;
    use tokio::io::AsyncReadExt;
    let started = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let store = storage::open(dir.path()).await.unwrap();
    let total = 124_396_022u64;
    let mut files = Vec::new();
    for n in 0..18 {
        let bytes = total / 18 + u64::from(n < (total % 18));
        let prefix = format!("synthetic source file {n:02}\n");
        let mut stream = prefix
            .as_bytes()
            .chain(tokio::io::repeat(b'x').take(bytes - prefix.len() as u64));
        files.push(store.receive(&mut stream, total).await.unwrap());
    }
    let spool_seconds = started.elapsed().as_secs_f64();
    println!("LOAD spool=18 files / {total} bytes in {spool_seconds:.3}s");
    let ingest_start = Instant::now();
    let id = store
        .enqueue("load", Some(&files[0]), Some("synthetic"), &json!({}))
        .await
        .unwrap();
    let mut lease = store.claim("load").await.unwrap().unwrap();
    for block in 0..200 {
        let rows: Vec<_> = (block * 500..(block + 1) * 500)
            .map(|n| {
                let mut value = event(n);
                value.track.id = format!("spotify:track:{:022}", n % 5000);
                value.raw["spotify_track_uri"] = json!(value.track.id);
                value.raw["padding"] = json!("x".repeat(640));
                SourceRow::from(value)
            })
            .collect();
        store
            .commit_batch(&lease, "synthetic", &rows, block == 199)
            .await
            .unwrap();
        lease = store.job(&id).await.unwrap();
        if block % 50 == 49 {
            println!(
                "LOAD acquired={} elapsed={:.3}s",
                lease.processed,
                ingest_start.elapsed().as_secs_f64()
            );
        }
    }
    let ingest_seconds = ingest_start.elapsed().as_secs_f64();
    let read_start = Instant::now();
    let mut view = store.read_view().await.unwrap();
    let totals: (i64, i64) =
        sqlx::query_as("SELECT COUNT(*),SUM(listened_ms) FROM listens WHERE counted=1")
            .fetch_one(&mut *view.transaction)
            .await
            .unwrap();
    assert_eq!(totals.0, 100000);
    drop(view);
    let read_ms = read_start.elapsed().as_secs_f64() * 1000.;
    // Exercise the real owner HTTP queries, not just a cheap aggregate over listens.
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let config = scrobbl_rs::config::Config {
        port: 3210,
        bind_ip: "127.0.0.1".parse().unwrap(),
        external_origin: None,
        data_dir: dir.path().into(),
        sync_seconds: 120,
        now_playing_seconds: 30,
        import_limits: Default::default(),
        env_client_id: None,
        env_client_secret: None,
    };
    let app = scrobbl_rs::web::router(scrobbl_rs::AppState::new(store.clone(), config));
    let token = auth::random_secret();
    sqlx::query("INSERT INTO sessions VALUES(?,'load-csrf',9999999999999)")
        .bind(auth::digest(&token))
        .execute(&store.private)
        .await
        .unwrap();
    for path in ["/api/history", "/api/history?page=1000", "/api/dashboard"] {
        for attempt in 1..=3 {
            let query_start = Instant::now();
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header("host", "127.0.0.1:3210")
                        .header("cookie", format!("scrobbl_session={token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let body = http_body_util::BodyExt::collect(response.into_body())
                .await
                .unwrap()
                .to_bytes();
            let _: serde_json::Value = serde_json::from_slice(&body).unwrap();
            println!(
                "LOAD HTTP path={path} attempt={attempt} ms={:.3}",
                query_start.elapsed().as_secs_f64() * 1000.
            );
        }
    }
    // Use the complete production worker: compression, creation-check, publication and cleanup.
    fn work_bytes(root: &std::path::Path) -> u64 {
        std::fs::read_dir(root)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    work_bytes(&path)
                } else {
                    path.metadata().unwrap().len()
                }
            })
            .sum()
    }
    let backup_start = Instant::now();
    let asset_id = scrobbl_rs::recovery::enqueue_asset(&store, "backup", "zip", json!({}))
        .await
        .unwrap();
    let worker = tokio::spawn(scrobbl_rs::recovery::worker(store.clone()));
    let mut peak_work_bytes = 0;
    loop {
        peak_work_bytes = peak_work_bytes.max(work_bytes(&dir.path().join("backups")));
        let asset = scrobbl_rs::recovery::asset(&store, &asset_id)
            .await
            .unwrap();
        assert_ne!(asset.state, "failed", "{:?}", asset.error);
        if asset.state == "ready" {
            println!(
                "LOAD BACKUP seconds={:.3} output_bytes={} peak_backup_directory_bytes={peak_work_bytes}",
                backup_start.elapsed().as_secs_f64(),
                asset.size_bytes.unwrap()
            );
            break;
        }
        assert!(backup_start.elapsed() < Duration::from_secs(300));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Wait for cleanup/completion before stopping the worker.
    let asset = scrobbl_rs::recovery::asset(&store, &asset_id)
        .await
        .unwrap();
    while store.job(&asset.job_id).await.unwrap().state != "completed" {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    worker.abort();
    let _ = worker.await;
    let calendar_start = Instant::now();
    let id = store
        .change_calendar("America/New_York", 6, true)
        .await
        .unwrap()
        .unwrap();
    let _ = store.claim("calendar").await.unwrap().unwrap();
    loop {
        let lease = store.job(&id).await.unwrap();
        if store.calendar_step(&lease).await.unwrap() {
            break;
        }
    }
    let calendar_seconds = calendar_start.elapsed().as_secs_f64();
    let snapshot_start = Instant::now();
    let snapshot = store.materialize(&SnapshotLimits::default()).await.unwrap();
    let snapshot_seconds = snapshot_start.elapsed().as_secs_f64();
    let (_, page) = store.snapshot_page(&snapshot.id, None, 1000).await.unwrap();
    assert_eq!(page.len(), 1000);
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&store.writer)
        .await
        .unwrap();
    let archive_bytes = std::fs::metadata(dir.path().join("archive.db"))
        .unwrap()
        .len();
    println!(
        "LOAD RESULT spool_bytes={total} observations=100000 tracks=5000 ingest_seconds={ingest_seconds:.3} total_query_ms={read_ms:.3} calendar_seconds={calendar_seconds:.3} snapshot_seconds={snapshot_seconds:.3} archive_bytes={archive_bytes} elapsed_seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    store.close().await;
}
