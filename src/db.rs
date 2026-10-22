use crate::{auth::digest, error::Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Sqlite, SqliteConnection, SqlitePool, Transaction};
use std::collections::BTreeSet;

pub use crate::storage::open;
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
pub struct Artist {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub album_id: Option<String>,
    pub album: Option<String>,
    pub cover_url: Option<String>,
    pub duration_ms: Option<i64>,
    pub artists: Vec<Artist>,
    pub structured: bool,
}
#[derive(Debug, Clone)]
pub struct Event {
    pub source: &'static str,
    pub source_key: String,
    pub source_timestamp_ms: i64,
    pub played_at_ms: i64,
    pub listened_ms: Option<i64>,
    pub duration_kind: DurationKind,
    pub counted: bool,
    pub track: Track,
    pub raw: Value,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum DurationKind {
    Actual,
    Estimated,
    Unknown,
}
impl DurationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Actual => "actual",
            Self::Estimated => "estimated",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Debug, Default, Serialize, Deserialize, Clone, ts_rs::TS)]
#[serde(default)]
pub struct IngestReport {
    pub originals: u64,
    pub variants: u64,
    pub added: u64,
    pub reconciled: u64,
    pub duplicates: u64,
    pub skipped: u64,
    pub invalid: u64,
    pub ambiguous: u64,
    pub errors: Vec<String>,
}
impl IngestReport {
    pub fn add(&mut self, other: &Self) {
        self.originals += other.originals;
        self.variants += other.variants;
        self.added += other.added;
        self.reconciled += other.reconciled;
        self.duplicates += other.duplicates;
        self.skipped += other.skipped;
        self.invalid += other.invalid;
        self.ambiguous += other.ambiguous;
        self.errors.extend(
            other
                .errors
                .iter()
                .take(10usize.saturating_sub(self.errors.len()))
                .cloned(),
        );
    }
}
/// Internal IDs are independent of source keys, titles and timestamps.
pub fn new_id() -> String {
    crate::auth::random_secret()
}

pub fn payload_fingerprint(value: &Value) -> String {
    // serde_json's default Map is sorted, recursively, independently of input key order.
    digest(&value.to_string())
}

pub(crate) async fn publish(conn: &mut SqliteConnection, ids: &BTreeSet<String>) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let revision: i64=sqlx::query_scalar("UPDATE archive_state SET revision=revision+1 WHERE singleton=1 AND maintenance=0 RETURNING revision")
        .fetch_optional(&mut *conn).await?.ok_or_else(||crate::error::AppError::Conflict("Archive is in maintenance.".into()))?;
    for id in ids {
        sqlx::query("UPDATE listens SET revision=? WHERE id=?")
            .bind(revision)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        sqlx::query("INSERT INTO changes(revision,listen_id,operation) SELECT ?,id,CASE WHEN trashed_at_ms IS NULL THEN 'upsert' ELSE 'remove' END FROM listens WHERE id=?")
            .bind(revision)
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    prune_revision_log(conn, revision).await?;
    Ok(())
}

pub(crate) async fn prune_revision_log(conn: &mut SqliteConnection, revision: i64) -> Result<()> {
    let through = revision.saturating_sub(10000);
    sqlx::query("UPDATE archive_state SET retained_from=MAX(retained_from,?) WHERE singleton=1")
        .bind(through)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM changes WHERE revision<=?")
        .bind(through)
        .execute(conn)
        .await?;
    Ok(())
}

/// Recovery and later library tools use the same revision protocol for removals and restores.
pub async fn set_trashed(
    tx: &mut Transaction<'_, Sqlite>,
    ids: &[String],
    at: Option<i64>,
) -> Result<()> {
    let mut changed = BTreeSet::new();
    for id in ids {
        let row: Option<String> = sqlx::query_scalar(
            "UPDATE listens SET trashed_at_ms=? WHERE id=? AND trashed_at_ms IS NOT ? RETURNING id",
        )
        .bind(at)
        .bind(id)
        .bind(at)
        .fetch_optional(&mut **tx)
        .await?;
        changed.extend(row);
    }
    publish(tx, &changed).await
}

/// Source adapters own source keys; this interface owns all archive invariants and revisions.
/// A caller commits this transaction together with its job/checkpoint, never separately.
pub async fn ingest(tx: &mut Transaction<'_, Sqlite>, events: &[Event]) -> Result<IngestReport> {
    ingest_at(tx, events, crate::auth::now_ms()).await
}
pub async fn ingest_at(
    tx: &mut Transaction<'_, Sqlite>,
    events: &[Event],
    acquired_at_ms: i64,
) -> Result<IngestReport> {
    crate::ingest::with_connection(tx, |conn| {
        crate::ingest::ingest(conn, events, acquired_at_ms)
    })
    .await
}

pub fn metadata_id(prefix: &str, parts: &[&str]) -> String {
    format!(
        "{prefix}:{}",
        digest(&serde_json::to_string(parts).unwrap_or_default())
    )
}
pub async fn setting(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await?,
    )
}
pub async fn set_setting(conn: &mut SqliteConnection, key: &str, value: &str) -> Result<()> {
    sqlx::query("INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").bind(key).bind(value).execute(conn).await?;
    Ok(())
}
