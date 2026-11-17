//! Imported-file history and the deletion of imported listens.
//!
//! The archive keeps live collection and imports apart through the source of
//! each observation. These handlers let the owner remove the listens that came
//! from a given uploaded file, or from every Spotify import, without touching
//! the listens collected live from Spotify.
use crate::{
    AppState, auth,
    dto::ImportFile,
    error::{AppError, Result},
    ingest,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Row, Sqlite};
use std::collections::BTreeSet;

#[derive(Deserialize)]
pub struct Confirm {
    #[serde(default)]
    pub confirm: bool,
}

/// One uploaded file of a Spotify import, with the counters the import recorded
/// and the number of listens still attributed to it.
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ImportFile>>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    let rows = sqlx::query(
        "SELECT i.file_id AS file_id,i.filename AS filename,i.job_id AS job_id,j.created_at_ms AS created_at_ms,\
         j.state AS state,j.outcome AS outcome,j.error_code AS error_code,\
         COALESCE(f.size_bytes,0) AS size_bytes,COALESCE(f.received_at_ms,0) AS received_at_ms,\
         COALESCE((SELECT SUM(m.added) FROM job_members m WHERE m.job_id=j.id AND m.file_id=i.file_id),0) AS added,\
         COALESCE((SELECT SUM(m.duplicates) FROM job_members m WHERE m.job_id=j.id AND m.file_id=i.file_id),0) AS duplicates,\
         COALESCE((SELECT SUM(m.invalid) FROM job_members m WHERE m.job_id=j.id AND m.file_id=i.file_id),0) AS invalid,\
         COALESCE((SELECT SUM(m.skipped) FROM job_members m WHERE m.job_id=j.id AND m.file_id=i.file_id),0) AS skipped,\
         (SELECT COUNT(DISTINCT o.listen_id) FROM source_occurrences oc \
            JOIN observation_variants v ON v.id=oc.variant_id \
            JOIN observations o ON o.id=v.observation_id \
            WHERE oc.file_id=i.file_id AND o.source='spotify_import') AS listens \
         FROM job_inputs i JOIN jobs j ON j.id=i.job_id LEFT JOIN source_files f ON f.id=i.file_id \
         WHERE j.kind='spotify_import' ORDER BY j.created_at_ms DESC,i.ordinal",
    )
    .fetch_all(&mut *view.transaction)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|row| ImportFile {
                file_id: row.get("file_id"),
                filename: row.get("filename"),
                job_id: row.get("job_id"),
                created_at_ms: row.get("created_at_ms"),
                state: row.get("state"),
                outcome: row.get("outcome"),
                error_code: row.get("error_code"),
                size_bytes: row.get("size_bytes"),
                received_at_ms: row.get("received_at_ms"),
                listens: row.get("listens"),
                added: row.get("added"),
                duplicates: row.get("duplicates"),
                invalid: row.get("invalid"),
                skipped: row.get("skipped"),
            })
            .collect(),
    ))
}

pub async fn delete_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(file_id): Path<String>,
    Json(form): Json<Confirm>,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    if !form.confirm {
        return Err(AppError::BadRequest(
            "Confirm the deletion of the listens imported from this file.".into(),
        ));
    }
    let mut tx = state.storage.writer.begin().await?;
    let removed = delete_imports(&mut tx, Some(&file_id)).await?;
    tx.commit().await?;
    Ok(Json(json!({"removed": removed})))
}

pub async fn delete_all(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<Confirm>,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    if !form.confirm {
        return Err(AppError::BadRequest(
            "Confirm the deletion of every imported listen.".into(),
        ));
    }
    let mut tx = state.storage.writer.begin().await?;
    let removed = delete_imports(&mut tx, None).await?;
    tx.commit().await?;
    Ok(Json(json!({"removed": removed})))
}

/// Remove the import-sourced observations of one file (or of every import) and
/// keep the listens that still have another source, such as live collection.
pub async fn delete_imports(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    file: Option<&str>,
) -> Result<i64> {
    let file = file.map(str::to_owned);
    ingest::with_connection(&mut *tx, |conn| remove(conn, file.as_deref())).await
}

fn remove(conn: &Connection, file: Option<&str>) -> Result<i64> {
    let listens: Vec<String> = conn
        .prepare_cached(
            "SELECT DISTINCT o.listen_id FROM source_occurrences oc \
             JOIN observation_variants v ON v.id=oc.variant_id \
             JOIN observations o ON o.id=v.observation_id \
             WHERE o.source='spotify_import' AND (?1 IS NULL OR oc.file_id=?1)",
        )?
        .query_map(params![file], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if listens.is_empty() {
        forget_import(conn, file)?;
        return Ok(0);
    }
    let mut touched = BTreeSet::new();
    let mut removed = BTreeSet::new();
    for listen in &listens {
        conn.prepare_cached(
            "DELETE FROM source_occurrences WHERE rowid IN (\
               SELECT oc.rowid FROM source_occurrences oc \
               JOIN observation_variants v ON v.id=oc.variant_id \
               JOIN observations o ON o.id=v.observation_id \
               WHERE o.listen_id=?2 AND o.source='spotify_import' AND (?1 IS NULL OR oc.file_id=?1))",
        )?
        .execute(params![file, listen])?;
        conn.prepare_cached(
            "DELETE FROM observation_variants WHERE observation_id IN (\
               SELECT id FROM observations WHERE listen_id=?1 AND source='spotify_import') \
             AND NOT EXISTS (SELECT 1 FROM source_occurrences oc WHERE oc.variant_id=observation_variants.id)",
        )?
        .execute(params![listen])?;
        conn.prepare_cached(
            "DELETE FROM observations WHERE listen_id=?1 AND source='spotify_import' \
             AND NOT EXISTS (SELECT 1 FROM observation_variants v WHERE v.observation_id=observations.id)",
        )?
        .execute(params![listen])?;
        let left: i64 = conn
            .prepare_cached("SELECT COUNT(*) FROM observations WHERE listen_id=?1")?
            .query_row(params![listen], |row| row.get(0))?;
        if left == 0 {
            conn.prepare_cached("DELETE FROM calendar_staging WHERE listen_id=?1")?
                .execute(params![listen])?;
            conn.prepare_cached("DELETE FROM conflicts WHERE listen_id=?1")?
                .execute(params![listen])?;
            conn.prepare_cached("DELETE FROM listens WHERE id=?1")?
                .execute(params![listen])?;
            removed.insert(listen.clone());
        } else {
            // Another source still explains this listen: recompute its canonical
            // values from the variants that remain.
            crate::ingest::canonicalize(conn, listen)?;
            touched.insert(listen.clone());
        }
    }
    crate::ingest::publish_with_removals(conn, &touched, &removed)?;
    forget_import(conn, file)?;
    // Payloads only referenced by the rows that just disappeared are dropped;
    // the fingerprints that other rows need stay in place.
    conn.prepare_cached(
        "DELETE FROM original_payloads WHERE id NOT IN (\
           SELECT payload_id FROM observations WHERE payload_id IS NOT NULL) \
         AND id NOT IN (SELECT payload_id FROM observation_variants WHERE payload_id IS NOT NULL) \
         AND id NOT IN (SELECT payload_id FROM source_occurrences WHERE payload_id IS NOT NULL)",
    )?
    .execute([])?;
    Ok(removed.len() as i64)
}

/// Drop the bookkeeping of the deleted import so the report stops listing it.
fn forget_import(conn: &Connection, file: Option<&str>) -> Result<()> {
    match file {
        Some(file) => {
            conn.prepare_cached(
                "DELETE FROM job_inputs WHERE file_id=?1 AND job_id IN (SELECT id FROM jobs WHERE kind='spotify_import')",
            )?
            .execute(params![file])?;
            conn.prepare_cached(
                "DELETE FROM job_members WHERE file_id=?1 AND job_id IN (SELECT id FROM jobs WHERE kind='spotify_import')",
            )?
            .execute(params![file])?;
            conn.prepare_cached(
                "DELETE FROM jobs WHERE kind='spotify_import' AND id NOT IN (SELECT job_id FROM job_inputs)",
            )?
            .execute([])?;
        }
        None => {
            conn.prepare_cached(
                "DELETE FROM job_inputs WHERE job_id IN (SELECT id FROM jobs WHERE kind='spotify_import')",
            )?
            .execute([])?;
            conn.prepare_cached(
                "DELETE FROM job_members WHERE job_id IN (SELECT id FROM jobs WHERE kind='spotify_import')",
            )?
            .execute([])?;
            conn.prepare_cached("DELETE FROM jobs WHERE kind='spotify_import'")?
                .execute([])?;
        }
    }
    Ok(())
}
