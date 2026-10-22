use scrobbl_rs::{
    db,
    jobs::{Job, SourceRow},
    listenbrainz::{self as lb, FetchRequest, Limits},
    storage::Storage,
};
use serde_json::{Value, json};
use std::io::{Cursor, Write};

fn row(ts: i64) -> Value {
    json!({"listened_at":ts,"recording_msid":"recording-not-playback-id","track_metadata":{"artist_name":"Artist","track_name":"Song","additional_info":{}}})
}
fn event(row: Value) -> db::Event {
    match lb::parse(row, None) {
        SourceRow::Event(e) => *e,
        _ => panic!("Expected a listen"),
    }
}
async fn enqueue(storage: &Storage, name: &str, bytes: &[u8], account: Option<&str>) -> String {
    let id = storage
        .receive(&mut &*bytes, 512 * 1024 * 1024)
        .await
        .unwrap();
    storage
        .enqueue(
            "listenbrainz_file",
            Some(&id),
            Some(name),
            &json!({"account":account,"limits":Limits::default()}),
        )
        .await
        .unwrap()
}
async fn run(storage: &Storage, id: &str) -> Job {
    let job = storage.claim("listenbrainz_file").await.unwrap().unwrap();
    assert_eq!(job.id, id);
    lb::import_file(storage, job, Limits::default())
        .await
        .unwrap();
    storage.job(id).await.unwrap()
}
fn zip(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        archive
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        archive.write_all(bytes).unwrap();
    }
    archive.finish().unwrap().into_inner()
}
async fn count(storage: &Storage) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM listens")
        .fetch_one(&storage.readers)
        .await
        .unwrap()
}

#[tokio::test]
async fn compact_retention_releases_complete_listenbrainz_files_but_keeps_partial_sources() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let complete = enqueue(
        &storage,
        "complete.json",
        &serde_json::to_vec(&[row(1700000000)]).unwrap(),
        None,
    )
    .await;
    assert_eq!(
        run(&storage, &complete).await.outcome.as_deref(),
        Some("complete")
    );
    let partial = enqueue(&storage, "partial.jsonl", b"{broken row\n", None).await;
    assert_eq!(
        run(&storage, &partial).await.outcome.as_deref(),
        Some("partial")
    );
    scrobbl_rs::retention::cleanup(&storage).await.unwrap();
    let available = |id: String| {
        let reader = storage.readers.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT retained FROM source_files WHERE id=(SELECT file_id FROM jobs WHERE id=?)",
            )
            .bind(id)
            .fetch_one(&reader)
            .await
            .unwrap()
        }
    };
    assert!(!available(complete).await);
    assert!(available(partial).await);
    let raw: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM original_payloads WHERE raw_json!='null'")
            .fetch_one(&storage.readers)
            .await
            .unwrap();
    assert_eq!(raw, 0);
    storage.close().await;
}

#[test]
fn timestamps_durations_and_optional_metadata_are_not_invented() {
    for (value, kind, ms, counted) in [
        (Some(0), db::DurationKind::Actual, Some(0), false),
        (Some(29), db::DurationKind::Actual, Some(29000), false),
        (Some(30), db::DurationKind::Actual, Some(30000), true),
        (None, db::DurationKind::Unknown, None, true),
    ] {
        let mut input = row(1_700_000_000);
        if let Some(value) = value {
            input["track_metadata"]["additional_info"]["duration_played"] = json!(value);
        }
        let e = event(input);
        assert_eq!(
            (e.duration_kind, e.listened_ms, e.counted),
            (kind, ms, counted)
        );
        assert_eq!(e.played_at_ms, 1_700_000_000_000);
        assert!(e.track.album.is_none());
    }
    let mut input = row(1_700_000_000);
    input["listened_at"] = json!(1_700_000_000.0);
    input["track_metadata"]["additional_info"] =
        json!({"duration_ms":20000,"duration_played":"bad","duration":30});
    let e = event(input.clone());
    assert_eq!(e.duration_kind, db::DurationKind::Estimated);
    assert!(e.counted);
    assert_eq!(e.listened_ms, Some(20000));
    assert!(
        lb::duration_warning(&input)
            .unwrap()
            .contains("Conflicting")
    );
    for key in ["listened_at", "track_metadata"] {
        let mut input = row(1_700_000_000);
        input.as_object_mut().unwrap().remove(key);
        assert!(matches!(lb::parse(input, None), SourceRow::Invalid { .. }));
    }
    let mut fractional = row(1_700_000_000);
    fractional["listened_at"] = json!(1_700_000_000.5);
    assert!(matches!(
        lb::parse(fractional, None),
        SourceRow::Invalid { .. }
    ));
}

#[test]
fn calendar_dates_include_dst_days_and_freeze_the_upper_bound() {
    let form = |from: &str, to: &str| FetchRequest {
        username: " Listener ".into(),
        from: Some(from.into()),
        to: Some(to.into()),
    };
    let now = 1_800_000_000_000;
    let spring =
        lb::fetch_parameters(form("2026-03-29", "2026-03-29"), "Europe/Rome", now).unwrap();
    assert_eq!(spring.end - spring.start, 23 * 3600);
    assert_eq!(spring.username, "listener");
    let autumn =
        lb::fetch_parameters(form("2026-10-25", "2026-10-25"), "Europe/Rome", now).unwrap();
    assert_eq!(autumn.end - autumn.start, 25 * 3600);
    let defaults = lb::fetch_parameters(
        FetchRequest {
            username: "listener".into(),
            from: None,
            to: None,
        },
        "Europe/Rome",
        now,
    )
    .unwrap();
    assert_eq!(defaults.end, now / 1000 + 1);
    assert_eq!(defaults.end - defaults.start, 14 * 86400);
    assert!(lb::fetch_parameters(form("2020-01-01", "2026-10-25"), "Europe/Rome", now).is_err());
    assert!(lb::fetch_parameters(form("2026-10-25", "2026-10-20"), "Europe/Rome", now).is_err());
}

#[test]
fn pagination_keeps_equal_timestamp_boundaries_and_refuses_false_completion() {
    let params = lb::FetchParameters {
        username: "listener".into(),
        timezone: "UTC".into(),
        start: 100,
        end: 10000,
        cursor: 10000,
        pages: 0,
    };
    let mut rows: Vec<_> = (1000..1999).rev().map(row).collect();
    rows.push(row(1000));
    let page = lb::page(&json!({"payload":{"listens":rows}}), &params).unwrap();
    assert_eq!(page.next, 1001);
    assert_eq!(page.rows.len(), 998);
    assert!(!page.complete);
    assert!(!page.doubtful);
    let ties = lb::page(
        &json!({"payload":{"listens":vec![row(1000);1000]}}),
        &params,
    )
    .unwrap();
    assert!(ties.doubtful);
    assert!(!ties.complete);
    assert_eq!(ties.rows.len(), 1000);
    assert!(lb::page(&json!({"payload":{"listens":[row(10000)]}}), &params).is_err());
    assert!(
        lb::page(
            &json!({"payload":{"listens":[row(1000),row(1001)]}}),
            &params
        )
        .is_err()
    );
    assert!(
        lb::page(&json!({"payload":{"listens":[]}}), &params)
            .unwrap()
            .complete
    );
}

#[tokio::test]
async fn official_jsonl_zip_json_and_enriched_response_reimport_preserve_multiplicity() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let samples = include_bytes!("fixtures/listenbrainz/upstream.jsonl");
    let mut rows: Vec<Value> = String::from_utf8_lossy(samples)
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    rows.push(rows[0].clone()); // even indistinguishable repeated rows remain two observations
    let jsonl = rows
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();
    let archive = zip(&[
        (
            "user.json",
            br#"{"user_id":42,"username":"listener"}"#.to_vec(),
        ),
        ("listens/2025/6.jsonl", jsonl.clone()),
        ("feedback.jsonl", b"not music".to_vec()),
        ("pinned_recording.jsonl", b"also not music".to_vec()),
    ]);
    let id = enqueue(&storage, "official-layout.zip", &archive, None).await;
    let job = run(&storage, &id).await;
    assert_eq!(job.state, "completed");
    assert_eq!(job.added, 3);
    assert_eq!(count(&storage).await, 3);
    let inventory: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job_members WHERE job_id=?")
        .bind(&id)
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    assert_eq!(inventory, 4);
    let id = enqueue(&storage, "renamed.jsonl", &jsonl, Some("listener")).await;
    let job = run(&storage, &id).await;
    assert_eq!(job.added, 0);
    assert_eq!(job.duplicates, 3);
    for r in &mut rows {
        r["inserted_at"] = json!(0);
        r["track_metadata"]["mbid_mapping"] = json!({"new":"enrichment"});
        r["listened_at"] = json!(r["listened_at"].as_f64().unwrap());
    }
    let id = enqueue(
        &storage,
        "response.json",
        &serde_json::to_vec_pretty(&json!({"payload":{"listens":rows,"user_id":"listener"}}))
            .unwrap(),
        None,
    )
    .await;
    let job = run(&storage, &id).await;
    assert_eq!(job.added, 0);
    assert_eq!(job.variants, 3);
    assert_eq!(count(&storage).await, 3);
    let raw: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_occurrences")
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    assert_eq!(raw, 9);
    let id = enqueue(
        &storage,
        "array.json",
        &serde_json::to_vec(&rows).unwrap(),
        Some("listener"),
    )
    .await;
    assert_eq!(run(&storage, &id).await.added, 0);
    let listen_id: String = sqlx::query_scalar(
        "SELECT listen_id FROM observations WHERE source='listenbrainz' LIMIT 1",
    )
    .fetch_one(&storage.readers)
    .await
    .unwrap();
    let track_id: String = sqlx::query_scalar("SELECT track_id FROM listens WHERE id=?")
        .bind(&listen_id)
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    let mut tx = storage.writer.begin().await.unwrap();
    db::set_trashed(&mut tx, std::slice::from_ref(&listen_id), Some(1))
        .await
        .unwrap();
    sqlx::query("INSERT INTO local_corrections(entity_kind,entity_id,field,value_json) VALUES('track',?,'title','\"My correction\"')").bind(track_id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let id = enqueue(&storage, "after-trash.jsonl", &jsonl, Some("listener")).await;
    assert_eq!(run(&storage, &id).await.added, 0);
    let trashed: Option<i64> = sqlx::query_scalar("SELECT trashed_at_ms FROM listens WHERE id=?")
        .bind(listen_id)
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    assert_eq!(trashed, Some(1));
    let corrections: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_corrections")
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    assert_eq!(corrections, 1);
    storage.close().await;
}

#[tokio::test]
async fn streaming_json_crosses_bounded_batches_without_losing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let rows: Vec<_> = (0..1001).map(|i| row(1_700_000_000 + i)).collect();
    let bytes = serde_json::to_vec(&rows).unwrap();
    let id = enqueue(&storage, "many-listens.json", &bytes, Some("listener")).await;
    let job = run(&storage, &id).await;
    assert_eq!((job.processed, job.added, job.invalid), (1001, 1001, 0));
    assert_eq!(count(&storage).await, 1001);
    let id = enqueue(&storage, "same-listens.json", &bytes, Some("listener")).await;
    let job = run(&storage, &id).await;
    assert_eq!((job.added, job.duplicates), (0, 1001));
    storage.close().await;
}

#[tokio::test]
async fn corrupt_rows_and_other_files_keep_originals_and_persistent_partial_reports() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let mut good = row(1_700_000_000);
    good["track_metadata"]["additional_info"]["duration_played"] = json!(0);
    let bytes = format!("{}\n{{bad JSON\n{}\n", good, json!({"listened_at":1}));
    let archive = zip(&[
        ("listens/2023/11.jsonl", bytes.into_bytes()),
        (
            "listens/2023/12.jsonl",
            format!("{}\n", row(1_700_000_100)).into_bytes(),
        ),
        ("account.txt", vec![1]),
    ]);
    let id = enqueue(&storage, "history.zip", &archive, None).await;
    let job = run(&storage, &id).await;
    assert_eq!((job.added, job.invalid), (2, 2));
    assert_eq!(
        job.error_code.as_deref(),
        Some("partial_import_review_report")
    );
    let invalid:i64=sqlx::query_scalar("SELECT COUNT(*) FROM source_occurrences WHERE outcome='invalid' AND payload_id IS NOT NULL AND reason IS NOT NULL").fetch_one(&storage.readers).await.unwrap();
    assert_eq!(invalid, 2);
    let counted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM listens WHERE counted=1")
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    assert_eq!(counted, 1);
    let broken = enqueue(&storage, "broken.json", b"[{not json", None).await;
    assert_eq!(run(&storage, &broken).await.state, "failed");
    let valid = enqueue(
        &storage,
        "valid.json",
        &serde_json::to_vec(&vec![row(1_700_001_000)]).unwrap(),
        None,
    )
    .await;
    assert_eq!(run(&storage, &valid).await.added, 1);
    storage.close().await;
}

#[tokio::test]
async fn cancelled_job_is_fenced_and_restart_retry_resumes_without_new_identity_slots() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let rows = vec![row(1_700_000_000), row(1_700_000_000), row(1_700_000_001)];
    let id = enqueue(
        &storage,
        "listens.json",
        &serde_json::to_vec(&rows).unwrap(),
        None,
    )
    .await;
    let lease = storage.claim("listenbrainz_file").await.unwrap().unwrap();
    let parsed = lb::parse(rows[0].clone(), None);
    storage
        .commit_member_batch(&lease, "listens.json", 1, &[parsed], false)
        .await
        .unwrap();
    let current = storage.job(&id).await.unwrap();
    storage.cancel(&id).await.unwrap();
    assert!(
        storage
            .commit_member_batch(
                &current,
                "listens.json",
                2,
                &[lb::parse(rows[1].clone(), None)],
                false
            )
            .await
            .is_err()
    );
    storage.close().await;
    let storage = db::open(dir.path()).await.unwrap();
    assert!(storage.claim("listenbrainz_file").await.unwrap().is_none());
    storage.retry(&id).await.unwrap();
    let job = run(&storage, &id).await;
    assert_eq!(job.processed, 3);
    assert_eq!(job.added, 3);
    assert_eq!(count(&storage).await, 3);
    // A process interruption after a committed batch restarts only unconfirmed rows.
    let id = enqueue(
        &storage,
        "listens-again.json",
        &serde_json::to_vec(&rows).unwrap(),
        None,
    )
    .await;
    let lease = storage.claim("listenbrainz_file").await.unwrap().unwrap();
    storage
        .commit_member_batch(
            &lease,
            "listens-again.json",
            1,
            &[lb::parse(rows[0].clone(), None)],
            false,
        )
        .await
        .unwrap();
    storage.close().await;
    let storage = db::open(dir.path()).await.unwrap();
    let job = run(&storage, &id).await;
    assert_eq!(job.duplicates, 3);
    assert_eq!(count(&storage).await, 3);
    storage.close().await;
}

#[tokio::test]
async fn source_identity_changes_accounts_and_uncertain_spotify_overlaps_remain_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let mut input = row(1_700_000_000);
    input["track_metadata"]["additional_info"]["spotify_id"] =
        json!("https://open.spotify.com/track/0000000000000000000001");
    let mut spotify = event(input.clone());
    spotify.source = "spotify_api";
    spotify.source_key = "spotify-playback".into();
    let mut tx = storage.writer.begin().await.unwrap();
    db::ingest(&mut tx, &[spotify]).await.unwrap();
    tx.commit().await.unwrap();
    for account in [Some("one"), Some("two"), None] {
        let id = enqueue(
            &storage,
            "listens.json",
            &serde_json::to_vec(&[input.clone()]).unwrap(),
            account,
        )
        .await;
        let job = run(&storage, &id).await;
        assert_eq!(job.added, 1);
        assert_eq!(job.reconciled, 0);
        assert_eq!(job.ambiguous, 1);
    }
    input["track_metadata"]["track_name"] = json!("Changed original metadata");
    let id = enqueue(
        &storage,
        "changed.json",
        &serde_json::to_vec(&[input]).unwrap(),
        Some("one"),
    )
    .await;
    assert_eq!(run(&storage, &id).await.added, 1);
    let id: String = sqlx::query_scalar("SELECT id FROM listens LIMIT 1")
        .fetch_one(&storage.readers)
        .await
        .unwrap();
    let mut tx = storage.writer.begin().await.unwrap();
    db::set_trashed(&mut tx, &[id], Some(1)).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(count(&storage).await, 5);
    storage.close().await;
}

#[tokio::test]
async fn invalid_wrappers_unsafe_archives_and_configured_limits_are_explicit_failures() {
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    for input in [
        json!({"listen_type":"import","payload":[row(1_700_000_000)]}),
        json!({"payload":{"playing_now":true,"listens":[row(1_700_000_000)]}}),
    ] {
        let id = enqueue(
            &storage,
            "bad.json",
            &serde_json::to_vec(&input).unwrap(),
            None,
        )
        .await;
        assert_eq!(run(&storage, &id).await.state, "failed");
    }
    let bytes = zip(&[(
        "../listens/2023/11.jsonl",
        row(1_700_000_000).to_string().into_bytes(),
    )]);
    let id = enqueue(&storage, "unsafe.zip", &bytes, None).await;
    assert_eq!(run(&storage, &id).await.state, "failed");
    assert_eq!(count(&storage).await, 0);
    let bytes = zip(&[(
        "listens/2023/11.jsonl",
        row(1_700_000_000).to_string().into_bytes(),
    )]);
    let id = enqueue(&storage, "limited.zip", &bytes, None).await;
    let lease = storage.claim("listenbrainz_file").await.unwrap().unwrap();
    lb::import_file(
        &storage,
        lease,
        Limits {
            expanded_bytes: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(storage.job(&id).await.unwrap().state, "failed");
    storage.close().await;
}

#[tokio::test]
#[ignore = "Requires SCROBBL_LB_SAMPLE_PATH pointing at an actual public API response outside the repository"]
async fn actual_public_api_response_imports_into_a_temporary_archive() {
    let bytes = std::fs::read(std::env::var("SCROBBL_LB_SAMPLE_PATH").unwrap()).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    let expected = value["payload"]["listens"].as_array().unwrap().len() as i64;
    assert!(expected > 0);
    let dir = tempfile::tempdir().unwrap();
    let storage = db::open(dir.path()).await.unwrap();
    let id = enqueue(&storage, "public-api.json", &bytes, None).await;
    let job = run(&storage, &id).await;
    assert_eq!(job.invalid, 0);
    assert_eq!(job.added, expected);
    assert_eq!(count(&storage).await, expected);
    storage.close().await;
}
