//! Compact history retains provenance and hashes, and only the source files still
//! needed by unfinished work. File publication, backup and deletion share a gate.
use anyhow::Context;
use sqlx::{Connection, Row, SqliteConnection};
use std::collections::BTreeSet;

use crate::storage::{self, Storage};

/// Also accepts older backup/upgrade schemas before migrating their isolated copy.
pub(crate) async fn files(conn: &mut SqliteConnection) -> anyhow::Result<Vec<(String, i64)>> {
    let compact = sqlx::query("PRAGMA table_info(source_files)")
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .any(|row| row.get::<String, _>("name") == "retained");
    Ok(sqlx::query_as(if compact {
        "SELECT content_hash,size_bytes FROM source_files WHERE retained=1 ORDER BY content_hash"
    } else {
        "SELECT content_hash,size_bytes FROM source_files ORDER BY content_hash"
    })
    .fetch_all(conn)
    .await?)
}

/// Called only before readers/workers start, including isolated legacy restores.
pub(crate) async fn compact_payloads(conn: &mut SqliteConnection) -> anyhow::Result<()> {
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM settings WHERE key='compact_retention_pending')",
    )
    .fetch_one(&mut *conn)
    .await?;
    if !pending {
        return Ok(());
    }
    let mut last_id = i64::MIN;
    loop {
        // A single row may be large; cap by both rows and original byte count.
        let mut tx = conn.begin().await?;
        let mut changed = 0;
        let mut bytes = 0;
        while changed < 256 && bytes < 1024 * 1024 {
            let row: Option<(i64, String)> = sqlx::query_as(
                "SELECT id,raw_json FROM original_payloads WHERE id>? AND fingerprint IS NULL ORDER BY id LIMIT 1",
            )
            .bind(last_id)
            .fetch_optional(&mut *tx)
            .await?;
            let Some((id, raw)) = row else { break };
            let value: serde_json::Value = serde_json::from_str(&raw)?;
            sqlx::query("UPDATE original_payloads SET raw_json='null',fingerprint=? WHERE id=?")
                .bind(crate::db::payload_fingerprint(&value))
                .bind(id)
                .execute(&mut *tx)
                .await?;
            bytes += raw.len();
            changed += 1;
            last_id = id;
        }
        tx.commit().await?;
        if changed == 0 {
            break;
        }
    }
    sqlx::query("DELETE FROM settings WHERE key='compact_retention_pending'")
        .execute(conn)
        .await?;
    Ok(())
}

/// Old technical copies depended on the live spool. Never invalidate those copies.
fn legacy_pins(store: &Storage) -> anyhow::Result<BTreeSet<String>> {
    let mut pinned = BTreeSet::new();
    for entry in std::fs::read_dir(store.directory.join("backups"))? {
        let entry = entry?;
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("pre-upgrade-")
        {
            continue;
        }
        let path = entry.path().join("complete.json");
        if !path.is_file() {
            continue;
        }
        let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        if manifest["spool"] == "snapshot" {
            continue;
        }
        for file in manifest["files"]
            .as_array()
            .context("Missing upgrade file inventory")?
        {
            pinned.insert(
                file["sha256"]
                    .as_str()
                    .context("Missing upgrade file hash")?
                    .to_owned(),
            );
        }
    }
    Ok(pinned)
}

pub async fn cleanup(store: &Storage) -> anyhow::Result<()> {
    let _files = store.spool_operations.write().await;
    if store.directory.join("recovery.pending").exists() {
        return Ok(());
    }
    // Receivers hold a shared spool gate until publication/attachment. With the
    // exclusive gate, incoming files can only belong to interrupted uploads.
    for entry in std::fs::read_dir(store.directory.join("spool/incoming"))? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            std::fs::remove_file(entry.path())?;
        }
    }
    let pinned = legacy_pins(store)?;
    let mut tx = store.writer.begin().await?;
    sqlx::query("DELETE FROM artwork WHERE source='spotify' AND expires_at_ms<=? AND (SELECT maintenance FROM archive_state WHERE singleton=1)=0")
        .bind(crate::auth::now_ms()).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM artwork_misses WHERE expires_at_ms<=? AND (SELECT maintenance FROM archive_state WHERE singleton=1)=0")
        .bind(crate::auth::now_ms()).execute(&mut *tx).await?;
    // A source may be shared by several uploads/jobs. Only discard bytes once ALL
    // references have completed successfully. Partial, failed and cancelled work
    // remains retryable. Unattached uploads are retained, too.
    let candidates: Vec<String> = sqlx::query_scalar(
        "WITH references_to_files AS (
           SELECT file_id,id AS job_id FROM jobs WHERE file_id IS NOT NULL
           UNION ALL SELECT file_id,job_id FROM job_inputs
           UNION ALL SELECT file_id,job_id FROM job_members WHERE file_id IS NOT NULL
           UNION ALL SELECT file_id,job_id FROM listenbrainz_pages
         )
         SELECT f.id FROM references_to_files r
         JOIN source_files f ON f.id=r.file_id AND f.retained=1
         JOIN jobs j ON j.id=r.job_id
         GROUP BY f.id
         HAVING MIN(j.state='completed' AND COALESCE(j.outcome,'complete') IN ('complete','no_music'))=1",
    ).fetch_all(&mut *tx).await?;
    for id in candidates {
        if pinned.contains(&id) {
            continue;
        }
        sqlx::query("UPDATE source_files SET retained=0 WHERE id=? AND retained=1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    // Commit unavailability before unlinking; retry after a crash at either step.
    // Inspect only files still on disk, including publication interrupted before
    // source_files committed. Never walk historical released rows on every tick.
    for entry in std::fs::read_dir(store.directory.join("spool"))? {
        let entry = entry?;
        let name = entry.file_name();
        let hash = name.to_string_lossy();
        if !entry.file_type()?.is_file()
            || hash.len() != 64
            || !hash.bytes().all(|c| c.is_ascii_hexdigit())
            || pinned.contains(hash.as_ref())
        {
            continue;
        }
        let retained: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM source_files WHERE content_hash=? AND retained=1)",
        )
        .bind(hash.as_ref())
        .fetch_one(&store.readers)
        .await?;
        if retained {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    storage::sync_directory(&store.directory.join("spool"))?;
    Ok(())
}

pub async fn worker(store: Storage) {
    loop {
        if cleanup(&store).await.is_err() {
            eprintln!(
                "Import source cleanup could not finish; check storage permissions and free space. It will retry."
            );
        }
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
}
