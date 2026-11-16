//! Ask projection: a separate SQLite file built from the canonical
//! consultation selection, containing only the documented listening tables
//! and never any settings, secrets, sessions, or source payloads.
use crate::{
    auth,
    consultation::{Filters, selection_at},
    error::{AppError, Result},
    library::METADATA,
    storage::Storage,
};
use chrono::{DateTime, Datelike, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OpenFlags, params};
use sqlx::{FromRow, QueryBuilder};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock};

pub const PROJECTION_FILENAME: &str = "ask_projection.db";

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(PROJECTION_FILENAME)
}

/// Coordinates on-demand rebuilds and atomic swaps with sandbox readers.
#[derive(Clone, Default)]
pub struct ProjectionGate {
    build: Arc<Mutex<()>>,
    swap: Arc<RwLock<()>>,
}

impl ProjectionGate {
    pub async fn read_lock(&self) -> tokio::sync::RwLockReadGuard<'_, ()> {
        self.swap.read().await
    }
}

#[derive(FromRow)]
struct ArtistRow {
    id: String,
    name: String,
}

#[derive(FromRow)]
struct AlbumRow {
    id: String,
    name: String,
}

#[derive(FromRow)]
struct TrackRow {
    id: String,
    title: String,
    album_id: Option<String>,
    duration_ms: Option<i64>,
}

#[derive(FromRow)]
struct TrackArtistRow {
    track_id: String,
    artist_id: String,
    ordinal: i64,
}

#[derive(FromRow)]
struct RawListenRow {
    id: String,
    track_id: String,
    played_at_ms: i64,
    local_date: String,
    week_date: String,
    listened_ms: Option<i64>,
    duration_kind: String,
    counted: i64,
    needs_review: i64,
    timezone: String,
}

struct ProjectionListenRow {
    id: String,
    track_id: String,
    played_at_ms: i64,
    local_date: String,
    week_date: String,
    local_year: i32,
    local_month: u32,
    weekday: u32,
    local_hour: u32,
    listened_ms: Option<i64>,
    duration_kind: String,
    counted: i64,
    needs_review: i64,
}

struct ProjectionSnapshot {
    generation: String,
    revision: i64,
    artists: Vec<ArtistRow>,
    albums: Vec<AlbumRow>,
    tracks: Vec<TrackRow>,
    track_artists: Vec<TrackArtistRow>,
    listens: Vec<ProjectionListenRow>,
}

fn is_current(projection_path: &Path, generation: &str, revision: i64) -> bool {
    if !projection_path.is_file() {
        return false;
    }
    let Ok(conn) = Connection::open_with_flags(
        projection_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return false;
    };
    conn.query_row(
        "SELECT generation, revision FROM _projection_meta WHERE singleton = 1",
        [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    )
    .is_ok_and(|(gen_id, rev)| gen_id == generation && rev == revision)
}

fn write_projection(tmp_path: &Path, snapshot: &ProjectionSnapshot) -> Result<()> {
    let mut conn = Connection::open(tmp_path)?;
    conn.execute_batch(DDL)?;
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("INSERT INTO artists(id, name) VALUES (?1, ?2)")?;
        for row in &snapshot.artists {
            stmt.execute(params![row.id, row.name])?;
        }
    }
    {
        let mut stmt = tx.prepare("INSERT INTO albums(id, name) VALUES (?1, ?2)")?;
        for row in &snapshot.albums {
            stmt.execute(params![row.id, row.name])?;
        }
    }
    {
        let mut stmt = tx.prepare(
            "INSERT INTO tracks(id, title, album_id, duration_ms) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for row in &snapshot.tracks {
            stmt.execute(params![row.id, row.title, row.album_id, row.duration_ms])?;
        }
    }
    {
        let mut stmt = tx.prepare(
            "INSERT INTO track_artists(track_id, artist_id, ordinal) VALUES (?1, ?2, ?3)",
        )?;
        for row in &snapshot.track_artists {
            stmt.execute(params![row.track_id, row.artist_id, row.ordinal])?;
        }
    }
    {
        let mut stmt = tx.prepare(
            "INSERT INTO listens(id, track_id, played_at_ms, local_date, week_date, local_year, local_month, weekday, local_hour, listened_ms, duration_kind, counted, needs_review) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        )?;
        for row in &snapshot.listens {
            stmt.execute(params![
                row.id,
                row.track_id,
                row.played_at_ms,
                row.local_date,
                row.week_date,
                row.local_year,
                row.local_month,
                row.weekday,
                row.local_hour,
                row.listened_ms,
                row.duration_kind,
                row.counted,
                row.needs_review,
            ])?;
        }
    }
    tx.execute(
        "INSERT INTO _projection_meta(singleton, generation, revision) VALUES (1, ?1, ?2)",
        params![snapshot.generation, snapshot.revision],
    )?;
    tx.commit()?;
    Ok(())
}

/// Ensure the Ask projection exists and matches the current archive revision,
/// rebuilding it in full on demand from the canonical consultation selection.
pub async fn ensure(storage: &Storage, gate: &ProjectionGate) -> Result<PathBuf> {
    let target = path(&storage.directory);
    let view = storage.read_view().await?;
    let generation = view.identity.generation.clone();
    let revision = view.identity.revision;
    drop(view);

    let check_target = target.clone();
    let check_gen = generation.clone();
    let current = {
        let _read = gate.swap.read().await;
        tokio::task::spawn_blocking(move || is_current(&check_target, &check_gen, revision))
            .await
            .map_err(|_| AppError::Internal)?
    };
    if current {
        return Ok(target);
    }

    let _build = gate.build.lock().await;
    let mut view = storage.read_view().await?;
    let generation = view.identity.generation.clone();
    let revision = view.identity.revision;
    let check_target = target.clone();
    let check_gen = generation.clone();
    let current = {
        let _read = gate.swap.read().await;
        tokio::task::spawn_blocking(move || is_current(&check_target, &check_gen, revision))
            .await
            .map_err(|_| AppError::Internal)?
    };
    if current {
        return Ok(target);
    }

    let mut q = QueryBuilder::new(METADATA);
    q.push(" SELECT id, name FROM artists");
    let artists: Vec<ArtistRow> = q.build_query_as().fetch_all(&mut *view.transaction).await?;

    let mut q = QueryBuilder::new(METADATA);
    q.push(" SELECT id, name FROM albums");
    let albums: Vec<AlbumRow> = q.build_query_as().fetch_all(&mut *view.transaction).await?;

    let mut q = QueryBuilder::new(METADATA);
    q.push(" SELECT id, title, album_id, duration_ms FROM tracks");
    let tracks: Vec<TrackRow> = q.build_query_as().fetch_all(&mut *view.transaction).await?;

    let mut q = QueryBuilder::new(METADATA);
    q.push(" SELECT track_id, artist_id, ordinal FROM track_artists");
    let track_artists: Vec<TrackArtistRow> =
        q.build_query_as().fetch_all(&mut *view.transaction).await?;

    let mut listens_query = selection_at(
        &mut view.transaction,
        &Filters::default(),
        false,
        auth::now_ms(),
    )
    .await?;
    listens_query.push(
        "SELECT selected.id, selected.track_id, selected.played_at_ms, selected.local_date, selected.week_date, \
         selected.listened_ms, selected.duration_kind, selected.counted, selected.needs_review, c.timezone \
         FROM selected JOIN calendars c ON c.id = selected.calendar_id",
    );
    let raw_listens: Vec<RawListenRow> = listens_query
        .build_query_as()
        .fetch_all(&mut *view.transaction)
        .await?;
    drop(view);

    let mut listens = Vec::with_capacity(raw_listens.len());
    for row in raw_listens {
        let date = NaiveDate::parse_from_str(&row.local_date, "%Y-%m-%d")
            .map_err(|_| AppError::Internal)?;
        let tz: Tz = row.timezone.parse().map_err(|_| AppError::Internal)?;
        let local_hour = DateTime::<Utc>::from_timestamp_millis(row.played_at_ms)
            .ok_or(AppError::Internal)?
            .with_timezone(&tz)
            .hour();
        listens.push(ProjectionListenRow {
            id: row.id,
            track_id: row.track_id,
            played_at_ms: row.played_at_ms,
            local_date: row.local_date,
            week_date: row.week_date,
            local_year: date.year(),
            local_month: date.month(),
            weekday: date.weekday().number_from_monday(),
            local_hour,
            listened_ms: row.listened_ms,
            duration_kind: row.duration_kind,
            counted: row.counted,
            needs_review: row.needs_review,
        });
    }

    let snapshot = ProjectionSnapshot {
        generation,
        revision,
        artists,
        albums,
        tracks,
        track_artists,
        listens,
    };
    let tmp_path = storage
        .directory
        .join(format!("ask_projection.tmp-{}", auth::random_secret()));
    let write_path = tmp_path.clone();
    let write_result =
        tokio::task::spawn_blocking(move || write_projection(&write_path, &snapshot))
            .await
            .map_err(|_| AppError::Internal)?;
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    let _swap = gate.swap.write().await;
    if std::fs::rename(&tmp_path, &target).is_err() {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(AppError::Internal);
    }

    Ok(target)
}

pub struct TableSpec {
    pub name: &'static str,
    pub columns: &'static [&'static str],
}

pub const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "artists",
        columns: &["id", "name"],
    },
    TableSpec {
        name: "albums",
        columns: &["id", "name"],
    },
    TableSpec {
        name: "tracks",
        columns: &["id", "title", "album_id", "duration_ms"],
    },
    TableSpec {
        name: "track_artists",
        columns: &["track_id", "artist_id", "ordinal"],
    },
    TableSpec {
        name: "listens",
        columns: &[
            "id",
            "track_id",
            "played_at_ms",
            "local_date",
            "week_date",
            "local_year",
            "local_month",
            "weekday",
            "local_hour",
            "listened_ms",
            "duration_kind",
            "counted",
            "needs_review",
        ],
    },
];

pub fn is_allowed_column(table: &str, column: &str) -> bool {
    let Some(spec) = TABLES
        .iter()
        .find(|spec| spec.name.eq_ignore_ascii_case(table))
    else {
        return false;
    };
    column.is_empty()
        || spec
            .columns
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(column))
}

pub fn is_allowed_table(table: &str) -> bool {
    TABLES
        .iter()
        .any(|spec| spec.name.eq_ignore_ascii_case(table))
}

/// Canonical DDL used to create a fresh Ask projection database.
pub const DDL: &str = "
CREATE TABLE artists (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL
);
CREATE TABLE albums (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL
);
CREATE TABLE tracks (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    album_id TEXT,
    duration_ms INTEGER
);
CREATE TABLE track_artists (
    track_id TEXT NOT NULL,
    artist_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    PRIMARY KEY (track_id, artist_id)
);
CREATE TABLE listens (
    id TEXT PRIMARY KEY,
    track_id TEXT NOT NULL,
    played_at_ms INTEGER NOT NULL,
    local_date TEXT NOT NULL,
    week_date TEXT NOT NULL,
    local_year INTEGER NOT NULL,
    local_month INTEGER NOT NULL,
    weekday INTEGER NOT NULL,
    local_hour INTEGER NOT NULL,
    listened_ms INTEGER,
    duration_kind TEXT NOT NULL,
    counted INTEGER NOT NULL,
    needs_review INTEGER NOT NULL
);
CREATE INDEX idx_listens_track ON listens(track_id, played_at_ms);
CREATE INDEX idx_listens_date ON listens(local_date);
CREATE INDEX idx_listens_counted ON listens(counted);
CREATE INDEX idx_track_artists_artist ON track_artists(artist_id, track_id);
CREATE INDEX idx_tracks_album ON tracks(album_id);
CREATE TABLE _projection_meta (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    generation TEXT NOT NULL,
    revision INTEGER NOT NULL
);
";

/// Schema description fed directly into the model prompt so the documented
/// projection and the SQLite schema cannot drift apart.
pub const SCHEMA_DESCRIPTION: &str = "\
Tables in the SQLite Ask projection (read-only):
- artists(id TEXT, name TEXT) — canonical grouped artists with local corrections applied.
- albums(id TEXT, name TEXT) — canonical grouped albums with local corrections applied.
- tracks(id TEXT, title TEXT, album_id TEXT, duration_ms INTEGER) — tracks with local corrections applied; album_id may be NULL.
- track_artists(track_id TEXT, artist_id TEXT, ordinal INTEGER) — artist credits per track (ordinal = 0 is the primary artist; a track can have multiple artists).
- listens(id TEXT, track_id TEXT, played_at_ms INTEGER, local_date TEXT, week_date TEXT, local_year INTEGER, local_month INTEGER, weekday INTEGER, local_hour INTEGER, listened_ms INTEGER, duration_kind TEXT, counted INTEGER, needs_review INTEGER)
  * trashed listens are already excluded.
  * played_at_ms: UTC epoch milliseconds.
  * local_date: YYYY-MM-DD in the listen's assigned calendar.
  * week_date: YYYY-MM-DD start of the listen's assigned calendar week.
  * local_year: integer year (e.g. 2024).
  * local_month: integer month 1..12.
  * weekday: integer 1 (Monday) .. 7 (Sunday).
  * local_hour: integer 0..23 in the listen's assigned calendar timezone.
  * listened_ms: milliseconds played, or NULL when duration_kind = 'unknown'.
  * duration_kind: 'actual', 'estimated', or 'unknown'.
  * counted: 1 for counted plays, 0 for short plays (<30s actual).
  * needs_review: 1 if awaiting duplicate review, 0 otherwise.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ask::sandbox::{self, SandboxLimits, SqlValue},
        db::{self, DurationKind, Event},
        spotify::parse_track,
    };
    use serde_json::json;

    #[tokio::test]
    async fn projection_applies_corrections_excludes_trash_and_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let storage = db::open(dir.path()).await.unwrap();
        let t1 = chrono::DateTime::parse_from_rfc3339("2024-01-10T10:00:00Z")
            .unwrap()
            .timestamp_millis();
        let track = parse_track(&json!({
            "uri": "spotify:track:0000000000000000000001",
            "type": "track",
            "name": "Jóga",
            "duration_ms": 300_000,
            "album": {"id": "al1", "name": "Homogenic", "images": []},
            "artists": [{"id": "ar1", "name": "Bjork"}],
        }))
        .unwrap();
        let events = vec![
            Event {
                source: "spotify_api",
                source_key: "k1".into(),
                source_timestamp_ms: t1,
                played_at_ms: t1,
                listened_ms: Some(200_000),
                duration_kind: DurationKind::Actual,
                counted: true,
                track: track.clone(),
                raw: json!({}),
            },
            Event {
                source: "spotify_api",
                source_key: "k2".into(),
                source_timestamp_ms: t1 + 60_000,
                played_at_ms: t1 + 60_000,
                listened_ms: Some(10_000),
                duration_kind: DurationKind::Actual,
                counted: false,
                track: track.clone(),
                raw: json!({}),
            },
            Event {
                source: "spotify_api",
                source_key: "k3".into(),
                source_timestamp_ms: t1 + 120_000,
                played_at_ms: t1 + 120_000,
                listened_ms: None,
                duration_kind: DurationKind::Unknown,
                counted: true,
                track: track.clone(),
                raw: json!({}),
            },
            Event {
                source: "spotify_api",
                source_key: "k4".into(),
                source_timestamp_ms: t1 + 180_000,
                played_at_ms: t1 + 180_000,
                listened_ms: Some(200_000),
                duration_kind: DurationKind::Actual,
                counted: true,
                track,
                raw: json!({}),
            },
        ];
        let mut tx = storage.writer.begin().await.unwrap();
        db::ingest(&mut tx, &events).await.unwrap();
        // Trash k4 and store a secret in archive settings.
        sqlx::query("UPDATE listens SET trashed_at_ms = 1 WHERE id IN (SELECT listen_id FROM observations WHERE source_key = 'k4')")
            .execute(&mut *tx)
            .await
            .unwrap();
        db::set_setting(&mut tx, "ai_remote_key", "sk-secret-do-not-leak")
            .await
            .unwrap();
        let artist_group: String = sqlx::query_scalar(
            "SELECT group_id FROM name_group_members WHERE kind='artists' LIMIT 1",
        )
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        sqlx::query("INSERT INTO local_corrections(entity_kind, entity_id, field, value_json) VALUES ('artist', ?, 'name', '\"Björk\"')")
            .bind(&artist_group)
            .execute(&mut *tx)
            .await
            .unwrap();
        db::publish(&mut tx, &std::collections::BTreeSet::new())
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let gate = ProjectionGate::default();
        let proj_path = ensure(&storage, &gate).await.unwrap();
        assert_ne!(proj_path, dir.path().join("archive.db"));

        // Verify counted, short, unknown duration, and corrected artist name through the sandbox.
        let out = sandbox::execute(
            &proj_path,
            "SELECT a.name, SUM(CASE WHEN l.counted = 1 THEN 1 ELSE 0 END) AS counted_plays, COUNT(*) AS total_non_trashed, SUM(CASE WHEN l.duration_kind = 'unknown' AND l.listened_ms IS NULL THEN 1 ELSE 0 END) AS unknown_rows FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id GROUP BY a.id",
            SandboxLimits::default(),
        )
        .unwrap();
        assert_eq!(
            out.rows,
            vec![vec![
                SqlValue::Text("Björk".into()),
                SqlValue::Integer(2),
                SqlValue::Integer(3),
                SqlValue::Integer(1),
            ]]
        );

        // Verify the projection file physically contains no secrets.
        let raw_bytes = std::fs::read(&proj_path).unwrap();
        let raw_lossy = String::from_utf8_lossy(&raw_bytes);
        assert!(!raw_lossy.contains("sk-secret-do-not-leak"));
    }
}
