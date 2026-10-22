//! Compact history retains provenance and hashes, and only the source files still
//! needed by unfinished work. File publication, backup and deletion share a gate.
use anyhow::{Context, ensure};
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
    let pinned = legacy_pins(store)?;
    let mut tx = store.writer.begin().await?;
    // A source may be shared by several uploads/jobs. Only discard bytes once ALL
    // references have completed successfully. Partial, failed and cancelled work
    // remains retryable. Unattached uploads are retained, too.
    let candidates: Vec<String> = sqlx::query_scalar(
        "SELECT f.id FROM source_files f WHERE
         EXISTS(SELECT 1 FROM jobs j WHERE j.file_id=f.id OR EXISTS(SELECT 1 FROM job_inputs i WHERE i.job_id=j.id AND i.file_id=f.id) OR EXISTS(SELECT 1 FROM job_members m WHERE m.job_id=j.id AND m.file_id=f.id))
         AND NOT EXISTS(SELECT 1 FROM jobs j WHERE (j.file_id=f.id OR EXISTS(SELECT 1 FROM job_inputs i WHERE i.job_id=j.id AND i.file_id=f.id) OR EXISTS(SELECT 1 FROM job_members m WHERE m.job_id=j.id AND m.file_id=f.id))
         AND (j.state!='completed' OR COALESCE(j.outcome,'complete') NOT IN ('complete','no_music')))",
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
    let removed: Vec<String> =
        sqlx::query_scalar("SELECT content_hash FROM source_files WHERE retained=0")
            .fetch_all(&store.readers)
            .await?;
    for hash in removed {
        ensure!(
            hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()),
            "Invalid source hash"
        );
        if pinned.contains(&hash) {
            continue;
        }
        match std::fs::remove_file(store.directory.join("spool").join(hash)) {
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
