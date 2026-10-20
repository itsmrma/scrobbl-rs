use crate::{auth::digest, error::Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{
    Row, Sqlite, SqliteConnection, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};

pub async fn open(directory: &Path) -> anyhow::Result<SqlitePool> {
    let existed = directory.exists();
    std::fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !existed {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let path = directory.join("scrobbl.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(30));
    // One connection serializes archive imports and collection. No read can see a partial import.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(120))
        .connect_with(options)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for suffix in ["", "-wal", "-shm"] {
            let file = directory.join(format!("scrobbl.db{suffix}"));
            if file.exists() {
                std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))?;
            }
        }
    }
    Ok(pool)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone)]
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
    pub track: Track,
    pub raw: Value,
}
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct IngestReport {
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
async fn upsert_track(conn: &mut SqliteConnection, track: &Track) -> Result<()> {
    let prior =
        sqlx::query_scalar::<_, bool>("SELECT structured_metadata FROM tracks WHERE id = ?")
            .bind(&track.id)
            .fetch_optional(&mut *conn)
            .await?;
    if prior == Some(true) && !track.structured {
        return Ok(());
    }
    if let (Some(id), Some(name)) = (&track.album_id, &track.album) {
        sqlx::query("INSERT INTO albums(id, name, cover_url) VALUES (?, ?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name, cover_url = COALESCE(excluded.cover_url, albums.cover_url)")
            .bind(id).bind(name).bind(&track.cover_url).execute(&mut *conn).await?;
    }
    let spotify_url = track
        .id
        .strip_prefix("spotify:track:")
        .map(|id| format!("https://open.spotify.com/track/{id}"));
    sqlx::query("INSERT INTO tracks(id, title, album_id, duration_ms, spotify_url, structured_metadata) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET title = excluded.title, album_id = COALESCE(excluded.album_id, tracks.album_id), duration_ms = COALESCE(excluded.duration_ms, tracks.duration_ms), spotify_url = excluded.spotify_url, structured_metadata = excluded.structured_metadata")
        .bind(&track.id).bind(&track.title).bind(&track.album_id).bind(track.duration_ms).bind(spotify_url).bind(track.structured).execute(&mut *conn).await?;
    sqlx::query("DELETE FROM track_artists WHERE track_id = ?")
        .bind(&track.id)
        .execute(&mut *conn)
        .await?;
    for (ordinal, artist) in track.artists.iter().enumerate() {
        sqlx::query("INSERT INTO artists(id, name) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name")
            .bind(&artist.id).bind(&artist.name).execute(&mut *conn).await?;
        sqlx::query(
            "INSERT OR IGNORE INTO track_artists(track_id, artist_id, ordinal) VALUES (?, ?, ?)",
        )
        .bind(&track.id)
        .bind(&artist.id)
        .bind(ordinal as i64)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}
/// Deduplicate only stable Spotify IDs and compatible inferred starts, one observation per source.
/// In-source exact replays are idempotent; distinct timestamps are never collapsed.
pub async fn ingest(tx: &mut Transaction<'_, Sqlite>, events: &[Event]) -> Result<IngestReport> {
    let mut report = IngestReport::default();
    // Check uniqueness in both directions within a batch, including repeated plays close together.
    let mut incoming: BTreeMap<(&str, &str), BTreeSet<(i64, &str)>> = BTreeMap::new();
    for event in events {
        incoming
            .entry((event.source, &event.track.id))
            .or_default()
            .insert((event.played_at_ms, &event.source_key));
    }
    for event in events {
        let duplicate: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM observations WHERE source = ? AND source_key = ?)",
        )
        .bind(event.source)
        .bind(&event.source_key)
        .fetch_one(&mut **tx)
        .await?;
        if duplicate {
            report.duplicates += 1;
            continue;
        }
        upsert_track(&mut *tx, &event.track).await?;
        let candidates = if event.track.id.starts_with("spotify:track:") {
            sqlx::query("SELECT l.id, l.played_at_ms FROM listens l WHERE l.track_id = ? AND l.played_at_ms BETWEEN ? AND ? AND NOT EXISTS(SELECT 1 FROM observations o WHERE o.listen_id = l.id AND o.source = ?) LIMIT 2")
                .bind(&event.track.id).bind(event.played_at_ms - 2000).bind(event.played_at_ms + 2000).bind(event.source).fetch_all(&mut **tx).await?
        } else {
            Vec::new()
        };
        let actual = event.source == "spotify_import";
        let kind = if actual {
            "actual"
        } else if event.listened_ms.is_some() {
            "estimated"
        } else {
            "unknown"
        };
        let counted = !actual || event.listened_ms.is_some_and(|ms| ms >= 30_000);
        let reverse_ambiguous = candidates.first().is_some_and(|candidate| {
            let time: i64 = candidate.get("played_at_ms");
            incoming
                .get(&(event.source, event.track.id.as_str()))
                .is_some_and(|times| {
                    times
                        .range((time - 2000, "")..=(time + 2000, "\u{10ffff}"))
                        .any(|(_, key)| *key != event.source_key)
                })
        });
        let ambiguous = candidates.len() > 1 || reverse_ambiguous;
        let listen_id = if candidates.len() == 1 && !ambiguous {
            let id: i64 = candidates[0].get("id");
            if actual {
                sqlx::query("UPDATE listens SET listened_ms = ?, duration_kind = 'actual', counted = ? WHERE id = ?")
                    .bind(event.listened_ms).bind(counted).bind(id).execute(&mut **tx).await?;
            }
            report.reconciled += 1;
            id
        } else {
            if ambiguous {
                report.ambiguous += 1;
                sqlx::query("UPDATE listens SET needs_review=1 WHERE track_id=? AND played_at_ms BETWEEN ? AND ?")
                    .bind(&event.track.id).bind(event.played_at_ms-2000).bind(event.played_at_ms+2000).execute(&mut **tx).await?;
            }
            report.added += 1;
            sqlx::query("INSERT INTO listens(track_id, played_at_ms, listened_ms, duration_kind, counted, needs_review) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(&event.track.id).bind(event.played_at_ms).bind(event.listened_ms).bind(kind).bind(counted).bind(ambiguous).execute(&mut **tx).await?.last_insert_rowid()
        };
        sqlx::query("INSERT INTO observations(source, source_key, listen_id, source_timestamp_ms, raw_json) VALUES (?, ?, ?, ?, ?)")
            .bind(event.source).bind(&event.source_key).bind(listen_id).bind(event.source_timestamp_ms).bind(event.raw.to_string()).execute(&mut **tx).await?;
    }
    Ok(report)
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
