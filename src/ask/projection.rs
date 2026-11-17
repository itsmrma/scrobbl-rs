//! Ask projection: a separate SQLite file built from the canonical
//! consultation selection, containing only the documented listening tables
//! and never any settings, secrets, sessions, or source payloads.
use crate::{
    auth, calendar,
    consultation::{Filters, selection_at},
    error::{AppError, Result},
    library::METADATA,
    storage::Storage,
};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;
use sqlx::{FromRow, QueryBuilder};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration as StdDuration,
};
use tokio::sync::{Mutex, MutexGuard, Notify, RwLock};

pub const PROJECTION_FILENAME: &str = "ask_projection.db";
const TMP_PREFIX: &str = "ask_projection.tmp-";

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(PROJECTION_FILENAME)
}

pub fn size_bytes(data_dir: &Path) -> u64 {
    std::fs::metadata(path(data_dir))
        .map(|m| m.len())
        .unwrap_or(0)
}

pub fn read_built_at_ms(projection_path: &Path) -> Option<i64> {
    if !projection_path.is_file() {
        return None;
    }
    let conn = Connection::open_with_flags(
        projection_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    conn.query_row("SELECT built_at_ms FROM meta LIMIT 1", [], |row| {
        row.get::<_, i64>(0)
    })
    .ok()
}

/// Coordinates on-demand and background rebuilds and atomic swaps with sandbox readers.
#[derive(Clone, Default)]
pub struct ProjectionGate {
    build: Arc<Mutex<()>>,
    swap: Arc<RwLock<()>>,
    last_built_at_ms: Arc<AtomicI64>,
    notify: Arc<Notify>,
}

impl ProjectionGate {
    pub async fn read_lock(&self) -> tokio::sync::RwLockReadGuard<'_, ()> {
        self.swap.read().await
    }

    pub fn notify(&self) {
        self.notify.notify_one();
    }

    #[cfg(test)]
    async fn lock_build(&self) -> MutexGuard<'_, ()> {
        self.build.lock().await
    }

    fn next_built_at_ms(&self) -> i64 {
        let now = auth::now_ms();
        let mut prev = self.last_built_at_ms.load(Ordering::Relaxed);
        loop {
            let next = now.max(prev + 1);
            match self.last_built_at_ms.compare_exchange_weak(
                prev,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return next,
                Err(actual) => prev = actual,
            }
        }
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
struct RawTrackRow {
    id: String,
    title: String,
    album_id: Option<String>,
    duration_ms: Option<i64>,
}

struct TrackRow {
    id: String,
    title: String,
    album_id: Option<String>,
    duration_ms: Option<i64>,
    edition_key: String,
}

#[derive(FromRow)]
struct TrackArtistRow {
    track_id: String,
    artist_id: String,
    ordinal: i64,
}

#[derive(FromRow)]
struct RawAlbumArtistRow {
    album_id: String,
    artist_id: String,
}

struct AlbumArtistRow {
    album_id: String,
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
    week_start: i64,
    sources: String,
    contexts: String,
}

struct ProjectionListenRow {
    id: String,
    track_id: String,
    played_at_ms: i64,
    played_at_local: String,
    local_date: String,
    week_date: String,
    local_year: i32,
    local_month: u32,
    weekday: u32,
    week_start: i64,
    local_hour: u32,
    listened_ms: Option<i64>,
    duration_kind: String,
    counted: i64,
    needs_review: i64,
    sources: String,
    device: Option<String>,
    country: Option<String>,
    shuffle: Option<i64>,
    skipped: Option<i64>,
    offline: Option<i64>,
    reason_start: Option<String>,
    reason_end: Option<String>,
}

struct CalendarDayRow {
    local_date: String,
    week_date: String,
    year: i32,
    month: u32,
    day: u32,
    weekday: u32,
}

struct ProjectionSnapshot {
    generation: String,
    revision: i64,
    built_at_ms: i64,
    calendar_id: String,
    timezone: String,
    week_start: i64,
    artists: Vec<ArtistRow>,
    albums: Vec<AlbumRow>,
    tracks: Vec<TrackRow>,
    track_artists: Vec<TrackArtistRow>,
    album_artists: Vec<AlbumArtistRow>,
    listens: Vec<ProjectionListenRow>,
    calendar: Vec<CalendarDayRow>,
}

#[derive(Default)]
struct SpotifyContext {
    device: Option<String>,
    country: Option<String>,
    shuffle: Option<i64>,
    skipped: Option<i64>,
    offline: Option<i64>,
    reason_start: Option<String>,
    reason_end: Option<String>,
}

fn agreed_text(contexts: &[Value], key: &str) -> Option<String> {
    let mut values = BTreeSet::new();
    for ctx in contexts {
        if let Some(s) = ctx
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            values.insert(s.to_owned());
            if values.len() > 1 {
                return None;
            }
        }
    }
    values.into_iter().next()
}

fn agreed_bool(contexts: &[Value], key: &str) -> Option<i64> {
    let mut values = BTreeSet::new();
    for ctx in contexts {
        if let Some(b) = ctx.get(key).and_then(Value::as_bool) {
            values.insert(i64::from(b));
            if values.len() > 1 {
                return None;
            }
        }
    }
    values.into_iter().next()
}

fn extract_spotify_context(contexts: &[Value]) -> SpotifyContext {
    SpotifyContext {
        device: agreed_text(contexts, "platform"),
        country: agreed_text(contexts, "conn_country"),
        shuffle: agreed_bool(contexts, "shuffle"),
        skipped: agreed_bool(contexts, "skipped"),
        offline: agreed_bool(contexts, "offline"),
        reason_start: agreed_text(contexts, "reason_start"),
        reason_end: agreed_text(contexts, "reason_end"),
    }
}

fn is_current(projection_path: &Path, generation: &str, revision: i64, calendar_id: &str) -> bool {
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
        "SELECT generation, revision, calendar_id FROM _projection_meta WHERE singleton = 1",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )
    .is_ok_and(|(gen_id, rev, cal_id)| {
        gen_id == generation && rev == revision && cal_id == calendar_id
    })
}

fn write_projection(tmp_path: &Path, snapshot: &ProjectionSnapshot) -> Result<()> {
    let mut conn = Connection::open(tmp_path)?;
    conn.execute_batch("PRAGMA journal_mode = DELETE; PRAGMA synchronous = NORMAL;")?;
    conn.execute_batch(&ddl())?;
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
            "INSERT INTO tracks(id, title, album_id, duration_ms, edition_key) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for row in &snapshot.tracks {
            stmt.execute(params![
                row.id,
                row.title,
                row.album_id,
                row.duration_ms,
                row.edition_key
            ])?;
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
            "INSERT INTO album_artists(album_id, artist_id, ordinal) VALUES (?1, ?2, ?3)",
        )?;
        for row in &snapshot.album_artists {
            stmt.execute(params![row.album_id, row.artist_id, row.ordinal])?;
        }
    }
    {
        let mut stmt = tx.prepare(
            "INSERT INTO listens(id, track_id, played_at_ms, played_at_local, local_date, week_date, local_year, local_month, weekday, week_start, local_hour, listened_ms, duration_kind, counted, needs_review, sources, device, country, shuffle, skipped, offline, reason_start, reason_end) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
        )?;
        for row in &snapshot.listens {
            stmt.execute(params![
                row.id,
                row.track_id,
                row.played_at_ms,
                row.played_at_local,
                row.local_date,
                row.week_date,
                row.local_year,
                row.local_month,
                row.weekday,
                row.week_start,
                row.local_hour,
                row.listened_ms,
                row.duration_kind,
                row.counted,
                row.needs_review,
                row.sources,
                row.device,
                row.country,
                row.shuffle,
                row.skipped,
                row.offline,
                row.reason_start,
                row.reason_end,
            ])?;
        }
    }
    {
        let mut stmt = tx.prepare(
            "INSERT INTO calendar(local_date, week_date, year, month, day, weekday) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for row in &snapshot.calendar {
            stmt.execute(params![
                row.local_date,
                row.week_date,
                row.year,
                row.month,
                row.day,
                row.weekday
            ])?;
        }
    }
    tx.execute(
        "INSERT INTO meta(revision, built_at_ms, timezone, week_start) VALUES (?1, ?2, ?3, ?4)",
        params![
            snapshot.revision,
            snapshot.built_at_ms,
            snapshot.timezone,
            snapshot.week_start
        ],
    )?;
    tx.execute(
        "INSERT INTO _projection_meta(singleton, generation, revision, built_at_ms, calendar_id) VALUES (1, ?1, ?2, ?3, ?4)",
        params![
            snapshot.generation,
            snapshot.revision,
            snapshot.built_at_ms,
            snapshot.calendar_id
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// Remove the Ask projection file (e.g. when AI is disabled or after restore).
pub async fn remove(data_dir: &Path, gate: &ProjectionGate) -> Result<()> {
    let target = path(data_dir);
    let _swap = gate.swap.write().await;
    if target.exists() {
        std::fs::remove_file(&target).map_err(|_| AppError::Internal)?;
    }
    cleanup_temp_files(data_dir);
    Ok(())
}

pub fn cleanup_temp_files(data_dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(data_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str()
                && name.starts_with(TMP_PREFIX)
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

async fn check_current(
    storage: &Storage,
    gate: &ProjectionGate,
    target: &Path,
) -> Result<(bool, bool)> {
    let mut view = storage.read_view().await?;
    let generation = view.identity.generation.clone();
    let revision = view.identity.revision;
    let current_cal = calendar::current(&mut view.transaction).await?;
    drop(view);

    let check_target = target.to_path_buf();
    let check_gen = generation;
    let check_cal = current_cal.id;
    let _read = gate.swap.read().await;
    tokio::task::spawn_blocking(move || {
        let exists = check_target.is_file();
        let current = exists && is_current(&check_target, &check_gen, revision, &check_cal);
        (exists, current)
    })
    .await
    .map_err(|_| AppError::Internal)
}

/// Return the path to a complete projection for answering a query.
/// If a background rebuild is currently running and a previous complete
/// projection exists on disk, returns the previous projection immediately so
/// Ask never stalls behind a rebuild.
pub async fn for_query(storage: &Storage, gate: &ProjectionGate) -> Result<PathBuf> {
    let target = path(&storage.directory);
    let (exists, current) = check_current(storage, gate, &target).await?;
    if current {
        return Ok(target);
    }

    let _build = if exists {
        match gate.build.try_lock() {
            Ok(guard) => guard,
            Err(_) => return Ok(target),
        }
    } else {
        gate.build.lock().await
    };

    rebuild_under_lock(storage, gate, &_build, false).await
}

/// Ensure the Ask projection exists and matches the current archive revision,
/// rebuilding it in full on demand from the canonical consultation selection.
pub async fn ensure(storage: &Storage, gate: &ProjectionGate) -> Result<PathBuf> {
    let target = path(&storage.directory);
    let (_, current) = check_current(storage, gate, &target).await?;
    if current {
        return Ok(target);
    }

    let _build = gate.build.lock().await;
    rebuild_under_lock(storage, gate, &_build, false).await
}

async fn rebuild_under_lock(
    storage: &Storage,
    gate: &ProjectionGate,
    _build: &MutexGuard<'_, ()>,
    require_ai_enabled: bool,
) -> Result<PathBuf> {
    let target = path(&storage.directory);
    let mut view = storage.read_view().await?;
    let generation = view.identity.generation.clone();
    let revision = view.identity.revision;
    let current_cal = calendar::current(&mut view.transaction).await?;
    let check_target = target.clone();
    let check_gen = generation.clone();
    let check_cal = current_cal.id.clone();
    let current = {
        let _read = gate.swap.read().await;
        tokio::task::spawn_blocking(move || {
            is_current(&check_target, &check_gen, revision, &check_cal)
        })
        .await
        .map_err(|_| AppError::Internal)?
    };
    if current {
        return Ok(target);
    }

    let now_ms = auth::now_ms();
    let mut listens_query =
        selection_at(&mut view.transaction, &Filters::default(), false, now_ms).await?;
    listens_query.push(
        "SELECT selected.id, selected.track_id, selected.played_at_ms, selected.local_date, selected.week_date, \
         selected.listened_ms, selected.duration_kind, selected.counted, selected.needs_review, \
         c.timezone, c.week_start, \
         (SELECT COALESCE(json_group_array(source), '[]') FROM (SELECT DISTINCT o.source FROM observations o WHERE o.listen_id = selected.id ORDER BY o.source)) AS sources, \
         (SELECT COALESCE(json_group_array(json(v.context_json)), '[]') FROM observations o JOIN observation_variants v ON v.observation_id = o.id WHERE o.listen_id = selected.id AND o.source = 'spotify_import') AS contexts \
         FROM selected JOIN calendars c ON c.id = selected.calendar_id",
    );
    let raw_listens: Vec<RawListenRow> = listens_query
        .build_query_as()
        .fetch_all(&mut *view.transaction)
        .await?;

    let mut q = QueryBuilder::new(METADATA);
    q.push(" SELECT id, name FROM artists");
    let all_artists: Vec<ArtistRow> = q.build_query_as().fetch_all(&mut *view.transaction).await?;

    let mut q = QueryBuilder::new(METADATA);
    q.push(" SELECT id, name FROM albums");
    let all_albums: Vec<AlbumRow> = q.build_query_as().fetch_all(&mut *view.transaction).await?;

    let mut q = QueryBuilder::new(METADATA);
    q.push(" SELECT id, title, album_id, duration_ms FROM tracks");
    let all_tracks: Vec<RawTrackRow> = q.build_query_as().fetch_all(&mut *view.transaction).await?;

    let mut q = QueryBuilder::new(METADATA);
    q.push(" SELECT track_id, artist_id, ordinal FROM track_artists");
    let all_track_artists: Vec<TrackArtistRow> =
        q.build_query_as().fetch_all(&mut *view.transaction).await?;

    let mut q = QueryBuilder::new(METADATA);
    q.push(
        " SELECT t.album_id, a.id AS artist_id \
         FROM track_artists ta \
         JOIN tracks t ON t.id = ta.track_id \
         JOIN artists a ON a.id = ta.artist_id \
         WHERE t.album_id IS NOT NULL \
         GROUP BY t.album_id, a.id \
         HAVING MIN(ta.ordinal) = 0 \
             OR (COUNT(DISTINCT ta.track_id) = (SELECT COUNT(*) FROM tracks t2 WHERE t2.album_id = t.album_id) \
                 AND (SELECT COUNT(*) FROM tracks t2 WHERE t2.album_id = t.album_id) > 1) \
         ORDER BY t.album_id, MIN(ta.ordinal), COUNT(DISTINCT ta.track_id) DESC, a.name COLLATE NOCASE, a.id",
    );
    let raw_album_artists: Vec<RawAlbumArtistRow> =
        q.build_query_as().fetch_all(&mut *view.transaction).await?;
    drop(view);

    let mut listens = Vec::with_capacity(raw_listens.len());
    let mut referenced_tracks = HashSet::new();
    let mut min_listen_date: Option<NaiveDate> = None;
    let mut max_listen_date: Option<NaiveDate> = None;

    for row in raw_listens {
        let date = NaiveDate::parse_from_str(&row.local_date, "%Y-%m-%d")
            .map_err(|_| AppError::Internal)?;
        min_listen_date = Some(min_listen_date.map_or(date, |d| d.min(date)));
        max_listen_date = Some(max_listen_date.map_or(date, |d| d.max(date)));

        let tz: Tz = row.timezone.parse().map_err(|_| AppError::Internal)?;
        let local_dt = DateTime::<Utc>::from_timestamp_millis(row.played_at_ms)
            .ok_or(AppError::Internal)?
            .with_timezone(&tz);
        let local_hour = local_dt.hour();
        let played_at_local = local_dt.format("%Y-%m-%d %H:%M:%S").to_string();
        let contexts: Vec<Value> =
            serde_json::from_str(&row.contexts).map_err(|_| AppError::Internal)?;
        let ctx = extract_spotify_context(&contexts);

        referenced_tracks.insert(row.track_id.clone());
        listens.push(ProjectionListenRow {
            id: row.id,
            track_id: row.track_id,
            played_at_ms: row.played_at_ms,
            played_at_local,
            local_date: row.local_date,
            week_date: row.week_date,
            local_year: date.year(),
            local_month: date.month(),
            weekday: date.weekday().number_from_monday(),
            week_start: row.week_start,
            local_hour,
            listened_ms: row.listened_ms,
            duration_kind: row.duration_kind,
            counted: row.counted,
            needs_review: row.needs_review,
            sources: row.sources,
            device: ctx.device,
            country: ctx.country,
            shuffle: ctx.shuffle,
            skipped: ctx.skipped,
            offline: ctx.offline,
            reason_start: ctx.reason_start,
            reason_end: ctx.reason_end,
        });
    }

    let track_artists: Vec<TrackArtistRow> = all_track_artists
        .into_iter()
        .filter(|ta| referenced_tracks.contains(&ta.track_id))
        .collect();
    let mut track_artist_ids: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut referenced_artists = HashSet::new();
    for ta in &track_artists {
        referenced_artists.insert(ta.artist_id.clone());
        track_artist_ids
            .entry(ta.track_id.clone())
            .or_default()
            .push(ta.artist_id.clone());
    }
    for ids in track_artist_ids.values_mut() {
        ids.sort();
        ids.dedup();
    }

    let mut referenced_albums = HashSet::new();
    let tracks: Vec<TrackRow> = all_tracks
        .into_iter()
        .filter(|t| referenced_tracks.contains(&t.id))
        .map(|t| {
            if let Some(album_id) = &t.album_id {
                referenced_albums.insert(album_id.clone());
            }
            let edition_key = match track_artist_ids.get(&t.id) {
                Some(artists) if !artists.is_empty() => {
                    serde_json::to_string(&(crate::ask::names::fold(&t.title), artists))
                        .unwrap_or_else(|_| format!("track:{}", t.id))
                }
                _ => format!("track:{}", t.id),
            };
            TrackRow {
                id: t.id,
                title: t.title,
                album_id: t.album_id,
                duration_ms: t.duration_ms,
                edition_key,
            }
        })
        .collect();

    let albums: Vec<AlbumRow> = all_albums
        .into_iter()
        .filter(|a| referenced_albums.contains(&a.id))
        .collect();
    let artists: Vec<ArtistRow> = all_artists
        .into_iter()
        .filter(|a| referenced_artists.contains(&a.id))
        .collect();

    let mut album_ordinals: BTreeMap<String, i64> = BTreeMap::new();
    let album_artists: Vec<AlbumArtistRow> = raw_album_artists
        .into_iter()
        .filter(|aa| referenced_albums.contains(&aa.album_id))
        .map(|aa| {
            let entry = album_ordinals.entry(aa.album_id.clone()).or_insert(0);
            let ordinal = *entry;
            *entry += 1;
            AlbumArtistRow {
                album_id: aa.album_id,
                artist_id: aa.artist_id,
                ordinal,
            }
        })
        .collect();

    let current_tz: Tz = current_cal
        .timezone
        .parse()
        .map_err(|_| AppError::Internal)?;
    let today = DateTime::<Utc>::from_timestamp_millis(now_ms)
        .ok_or(AppError::Internal)?
        .with_timezone(&current_tz)
        .date_naive();
    let cal_start = min_listen_date.unwrap_or(today);
    let cal_end = max_listen_date.map_or(today, |d| d.max(today));
    let mut calendar_rows = Vec::new();
    let mut day = cal_start;
    while day <= cal_end {
        let offset = (i64::from(day.weekday().num_days_from_monday()) - current_cal.week_start)
            .rem_euclid(7);
        let week_date = day
            .checked_sub_signed(Duration::days(offset))
            .ok_or(AppError::Internal)?
            .to_string();
        calendar_rows.push(CalendarDayRow {
            local_date: day.to_string(),
            week_date,
            year: day.year(),
            month: day.month(),
            day: day.day(),
            weekday: day.weekday().number_from_monday(),
        });
        let Some(next) = day.succ_opt() else {
            break;
        };
        day = next;
    }

    let built_at_ms = gate.next_built_at_ms();
    let snapshot = ProjectionSnapshot {
        generation,
        revision,
        built_at_ms,
        calendar_id: current_cal.id,
        timezone: current_cal.timezone,
        week_start: current_cal.week_start,
        artists,
        albums,
        tracks,
        track_artists,
        album_artists,
        listens,
        calendar: calendar_rows,
    };
    let tmp_path = storage
        .directory
        .join(format!("{TMP_PREFIX}{}", auth::random_secret()));
    let write_path = tmp_path.clone();
    let write_result =
        tokio::task::spawn_blocking(move || write_projection(&write_path, &snapshot))
            .await
            .map_err(|_| AppError::Internal)?;
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    if require_ai_enabled
        && crate::db::setting(&storage.readers, "ai_enabled")
            .await
            .ok()
            .flatten()
            .as_deref()
            != Some("1")
    {
        let _ = std::fs::remove_file(&tmp_path);
        return Ok(target);
    }

    let _swap = gate.swap.write().await;
    if std::fs::rename(&tmp_path, &target).is_err() {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(AppError::Internal);
    }

    Ok(target)
}

async fn ai_enabled(storage: &Storage) -> bool {
    crate::db::setting(&storage.readers, "ai_enabled")
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("1")
}

/// Background worker that keeps the Ask projection in sync with archive
/// revisions (debounced) while AI is enabled, and deletes it when AI is off.
pub async fn worker(state: crate::AppState) {
    worker_loop(
        state.storage,
        state.ask.projection,
        StdDuration::from_millis(500),
        StdDuration::from_millis(1000),
    )
    .await;
}

pub async fn worker_loop(
    storage: Storage,
    gate: ProjectionGate,
    poll_interval: StdDuration,
    debounce: StdDuration,
) {
    cleanup_temp_files(&storage.directory);
    loop {
        if !ai_enabled(&storage).await {
            if path(&storage.directory).exists() {
                let _ = remove(&storage.directory, &gate).await;
            }
            tokio::select! {
                _ = gate.notify.notified() => {}
                _ = tokio::time::sleep(poll_interval) => {}
            }
            continue;
        }

        let target = path(&storage.directory);
        if let Ok((exists, current)) = check_current(&storage, &gate, &target).await
            && !current
        {
            if exists && !debounce.is_zero() {
                // Debounce rapid revision bursts (e.g., batched imports), bounded
                // so a long import still refreshes periodically.
                let mut last_rev = storage.read_view().await.ok().map(|v| v.identity.revision);
                for _ in 0..4 {
                    tokio::select! {
                        _ = gate.notify.notified() => {
                            last_rev = storage.read_view().await.ok().map(|v| v.identity.revision);
                        }
                        _ = tokio::time::sleep(debounce) => {
                            let now_rev = storage.read_view().await.ok().map(|v| v.identity.revision);
                            if now_rev == last_rev {
                                break;
                            }
                            last_rev = now_rev;
                        }
                    }
                }
            }
            if ai_enabled(&storage).await {
                let _build = gate.build.lock().await;
                if let Err(err) = rebuild_under_lock(&storage, &gate, &_build, true).await {
                    eprintln!("Ask projection background rebuild failed: {err}");
                }
            }
            continue;
        }

        tokio::select! {
            _ = gate.notify.notified() => {}
            _ = tokio::time::sleep(poll_interval) => {}
        }
    }
}

pub struct ColumnSpec {
    pub name: &'static str,
    pub sql_type: &'static str,
    pub constraints: &'static str,
    pub description: &'static str,
}

pub struct TableSpec {
    pub name: &'static str,
    pub summary: &'static str,
    pub columns: &'static [ColumnSpec],
    pub table_constraint: &'static str,
}

pub const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "artists",
        summary: "canonical grouped artists with local corrections applied",
        columns: &[
            ColumnSpec {
                name: "id",
                sql_type: "TEXT",
                constraints: "PRIMARY KEY",
                description: "canonical grouped artist ID and `/music/artists/<id>` link key",
            },
            ColumnSpec {
                name: "name",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "display name after local corrections",
            },
        ],
        table_constraint: "",
    },
    TableSpec {
        name: "albums",
        summary: "canonical grouped albums with local corrections applied",
        columns: &[
            ColumnSpec {
                name: "id",
                sql_type: "TEXT",
                constraints: "PRIMARY KEY",
                description: "canonical grouped album ID and `/music/albums/<id>` link key",
            },
            ColumnSpec {
                name: "name",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "display name after local corrections",
            },
        ],
        table_constraint: "",
    },
    TableSpec {
        name: "tracks",
        summary: "tracks with local corrections applied; album_id may be NULL",
        columns: &[
            ColumnSpec {
                name: "id",
                sql_type: "TEXT",
                constraints: "PRIMARY KEY",
                description: "track ID and `/music/tracks/<id>` link key",
            },
            ColumnSpec {
                name: "title",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "display title after local corrections",
            },
            ColumnSpec {
                name: "album_id",
                sql_type: "TEXT",
                constraints: "",
                description: "canonical grouped `albums.id`, or NULL",
            },
            ColumnSpec {
                name: "duration_ms",
                sql_type: "INTEGER",
                constraints: "",
                description: "catalog track duration in milliseconds, or NULL",
            },
            ColumnSpec {
                name: "edition_key",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "shared key across editions of the same song (folded title + credited artists)",
            },
        ],
        table_constraint: "",
    },
    TableSpec {
        name: "track_artists",
        summary: "artist credits per track in credit order",
        columns: &[
            ColumnSpec {
                name: "track_id",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "referenced `tracks.id`",
            },
            ColumnSpec {
                name: "artist_id",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "canonical grouped `artists.id`",
            },
            ColumnSpec {
                name: "ordinal",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "zero-based credit order (`0` is the primary artist)",
            },
        ],
        table_constraint: "PRIMARY KEY (track_id, artist_id)",
    },
    TableSpec {
        name: "album_artists",
        summary: "primary album artists (first-credited on at least one track, or credited on every track of a multi-track album; single-track guests excluded)",
        columns: &[
            ColumnSpec {
                name: "album_id",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "referenced `albums.id`",
            },
            ColumnSpec {
                name: "artist_id",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "canonical grouped `artists.id`",
            },
            ColumnSpec {
                name: "ordinal",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "zero-based album artist order",
            },
        ],
        table_constraint: "PRIMARY KEY (album_id, artist_id)",
    },
    TableSpec {
        name: "listens",
        summary: "non-trashed listens (trashed listens are already excluded)",
        columns: &[
            ColumnSpec {
                name: "id",
                sql_type: "TEXT",
                constraints: "PRIMARY KEY",
                description: "listen ID",
            },
            ColumnSpec {
                name: "track_id",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "referenced `tracks.id`",
            },
            ColumnSpec {
                name: "played_at_ms",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "UTC epoch milliseconds when the listen started",
            },
            ColumnSpec {
                name: "played_at_local",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "local datetime (`YYYY-MM-DD HH:MM:SS`) in the listen's assigned calendar timezone",
            },
            ColumnSpec {
                name: "local_date",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "`YYYY-MM-DD` in the listen's assigned calendar",
            },
            ColumnSpec {
                name: "week_date",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "`YYYY-MM-DD` start of the listen's assigned calendar week",
            },
            ColumnSpec {
                name: "local_year",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "integer year (e.g. `2024`)",
            },
            ColumnSpec {
                name: "local_month",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "integer month `1..12`",
            },
            ColumnSpec {
                name: "weekday",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "integer `1` (Monday) .. `7` (Sunday)",
            },
            ColumnSpec {
                name: "week_start",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "assigned calendar week start (`0` = Monday .. `6` = Sunday)",
            },
            ColumnSpec {
                name: "local_hour",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "integer `0..23` in the listen's assigned calendar timezone",
            },
            ColumnSpec {
                name: "listened_ms",
                sql_type: "INTEGER",
                constraints: "",
                description: "milliseconds played, or `NULL` when `duration_kind = 'unknown'`",
            },
            ColumnSpec {
                name: "duration_kind",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "`'actual'`, `'estimated'`, or `'unknown'`",
            },
            ColumnSpec {
                name: "counted",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "`1` for counted plays, `0` for short actual plays (<30s)",
            },
            ColumnSpec {
                name: "needs_review",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "`1` if awaiting duplicate review, `0` otherwise",
            },
            ColumnSpec {
                name: "sources",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "JSON array of observation sources (e.g. `[\"spotify_api\",\"spotify_import\"]`); query with `json_each(sources)`",
            },
            ColumnSpec {
                name: "device",
                sql_type: "TEXT",
                constraints: "",
                description: "Spotify export device/platform when known without conflict, else `NULL`",
            },
            ColumnSpec {
                name: "country",
                sql_type: "TEXT",
                constraints: "",
                description: "Spotify export connection country code when known without conflict, else `NULL`",
            },
            ColumnSpec {
                name: "shuffle",
                sql_type: "INTEGER",
                constraints: "",
                description: "Spotify export shuffle (`0`/`1`) when known without conflict, else `NULL`",
            },
            ColumnSpec {
                name: "skipped",
                sql_type: "INTEGER",
                constraints: "",
                description: "Spotify export skipped (`0`/`1`) when known without conflict, else `NULL`",
            },
            ColumnSpec {
                name: "offline",
                sql_type: "INTEGER",
                constraints: "",
                description: "Spotify export offline (`0`/`1`) when known without conflict, else `NULL`",
            },
            ColumnSpec {
                name: "reason_start",
                sql_type: "TEXT",
                constraints: "",
                description: "Spotify export reason playback started when known without conflict, else `NULL`",
            },
            ColumnSpec {
                name: "reason_end",
                sql_type: "TEXT",
                constraints: "",
                description: "Spotify export reason playback ended when known without conflict, else `NULL`",
            },
        ],
        table_constraint: "",
    },
    TableSpec {
        name: "calendar",
        summary: "one row per day across the archive's date range for gap/streak queries without recursive CTEs",
        columns: &[
            ColumnSpec {
                name: "local_date",
                sql_type: "TEXT",
                constraints: "PRIMARY KEY",
                description: "date (`YYYY-MM-DD`)",
            },
            ColumnSpec {
                name: "week_date",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "week start date (`YYYY-MM-DD`) under the current configured week start",
            },
            ColumnSpec {
                name: "year",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "calendar year",
            },
            ColumnSpec {
                name: "month",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "calendar month `1..12`",
            },
            ColumnSpec {
                name: "day",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "day of month `1..31`",
            },
            ColumnSpec {
                name: "weekday",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "day of week (`1` = Monday .. `7` = Sunday)",
            },
        ],
        table_constraint: "",
    },
    TableSpec {
        name: "meta",
        summary: "projection metadata (as-of revision, build time, timezone, week start)",
        columns: &[
            ColumnSpec {
                name: "revision",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "archive revision reflected by this projection",
            },
            ColumnSpec {
                name: "built_at_ms",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "UTC epoch milliseconds when this projection was built",
            },
            ColumnSpec {
                name: "timezone",
                sql_type: "TEXT",
                constraints: "NOT NULL",
                description: "current configured IANA timezone",
            },
            ColumnSpec {
                name: "week_start",
                sql_type: "INTEGER",
                constraints: "NOT NULL",
                description: "current configured week start (`0` = Monday .. `6` = Sunday)",
            },
        ],
        table_constraint: "",
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
            .any(|allowed| allowed.name.eq_ignore_ascii_case(column))
}

pub fn is_allowed_table(table: &str) -> bool {
    TABLES
        .iter()
        .any(|spec| spec.name.eq_ignore_ascii_case(table))
}

/// Canonical DDL generated directly from [`TABLES`] so the SQLite schema,
/// sandbox allow-list, and prompt documentation never drift apart.
pub fn ddl() -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for table in TABLES {
        let _ = writeln!(out, "CREATE TABLE {} (", table.name);
        let mut items: Vec<String> = table
            .columns
            .iter()
            .map(|col| {
                if col.constraints.is_empty() {
                    format!("    {} {}", col.name, col.sql_type)
                } else {
                    format!("    {} {} {}", col.name, col.sql_type, col.constraints)
                }
            })
            .collect();
        if !table.table_constraint.is_empty() {
            items.push(format!("    {}", table.table_constraint));
        }
        let _ = writeln!(out, "{}", items.join(",\n"));
        let _ = writeln!(out, ");");
    }
    out.push_str(
        "CREATE INDEX idx_listens_track ON listens(track_id, played_at_ms);\n\
         CREATE INDEX idx_listens_date ON listens(local_date);\n\
         CREATE INDEX idx_listens_counted ON listens(counted);\n\
         CREATE INDEX idx_track_artists_artist ON track_artists(artist_id, track_id);\n\
         CREATE INDEX idx_album_artists_artist ON album_artists(artist_id, album_id);\n\
         CREATE INDEX idx_tracks_album ON tracks(album_id);\n\
         CREATE INDEX idx_tracks_edition ON tracks(edition_key);\n\
         CREATE TABLE _projection_meta (\n\
             singleton INTEGER PRIMARY KEY CHECK(singleton = 1),\n\
             generation TEXT NOT NULL,\n\
             revision INTEGER NOT NULL,\n\
             built_at_ms INTEGER NOT NULL,\n\
             calendar_id TEXT NOT NULL\n\
         );\n",
    );
    out
}

/// Schema description generated from [`TABLES`] and fed directly into the
/// model prompt so the documented projection and SQLite schema stay identical.
pub fn schema_description() -> String {
    use std::fmt::Write;
    let mut out = String::from("Tables in the SQLite Ask projection (read-only):\n");
    for table in TABLES {
        let cols: Vec<String> = table
            .columns
            .iter()
            .map(|col| format!("{} {}", col.name, col.sql_type))
            .collect();
        let _ = writeln!(
            out,
            "- {}({}) — {}",
            table.name,
            cols.join(", "),
            table.summary
        );
        for col in table.columns {
            let _ = writeln!(out, "  * {}: {}", col.name, col.description);
        }
    }
    out
}

/// Documentation section for `docs/ask-projection.md`, generated from the same
/// [`TABLES`] the prompt and the sandbox allow-list use. A test fails when the
/// checked-in document drifts.
pub fn schema_markdown() -> String {
    use std::fmt::Write;
    let mut out = String::from(
        "The tables are generated from `src/ask/projection.rs::TABLES`, the same source as the\n\
         model prompt and the sandbox allow-list. Do not edit this section by hand; regenerate it\n\
         with `UPDATE_ASK_DOC` when `TABLES` changes.\n\n",
    );
    for table in TABLES {
        let _ = writeln!(out, "### `{}`\n", table.name);
        let _ = writeln!(out, "{}\n", table.summary);
        out.push_str("| column | type | constraints | meaning |\n");
        out.push_str("| --- | --- | --- | --- |\n");
        for col in table.columns {
            let constraints = if col.constraints.is_empty() {
                "—"
            } else {
                col.constraints
            };
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} |",
                col.name, col.sql_type, constraints, col.description
            );
        }
        out.push('\n');
    }
    out.push_str(
        "### `_projection_meta` (internal)\n\n\
         One row per build, holding the archive generation, revision, build time and calendar id\n\
         used to decide whether the file is current. It is deliberately absent from `TABLES` and\n\
         from every allow-list: no query from the model may read it.\n\n\
         ### Indexes\n\n\
         The projection carries `listens(track_id, played_at_ms)`, `listens(local_date)`,\n\
         `listens(counted)`, `track_artists(artist_id, track_id)`,\n\
         `album_artists(artist_id, album_id)`, `tracks(album_id)` and `tracks(edition_key)`, so\n\
         ordinary rankings, streaks and gap scans do not become full scans.\n\n\
         ### Exact DDL\n\n```sql\n",
    );
    out.push_str(&ddl());
    out.push_str("```");
    out
}

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

    #[test]
    fn schema_is_single_source_of_truth_for_ddl_sandbox_and_prompt() {
        let ddl = ddl();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&ddl).unwrap();

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != '_projection_meta' ORDER BY name")
            .unwrap();
        let db_tables: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        let mut spec_tables: Vec<String> = TABLES.iter().map(|t| t.name.to_owned()).collect();
        spec_tables.sort();
        assert_eq!(db_tables, spec_tables);

        for table in TABLES {
            assert!(is_allowed_table(table.name));
            let mut pragma = conn
                .prepare(&format!("PRAGMA table_info({})", table.name))
                .unwrap();
            let db_cols: Vec<(String, String)> = pragma
                .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap();
            let spec_cols: Vec<(String, String)> = table
                .columns
                .iter()
                .map(|c| (c.name.to_owned(), c.sql_type.to_owned()))
                .collect();
            assert_eq!(db_cols, spec_cols, "column mismatch on {}", table.name);
            for col in table.columns {
                assert!(is_allowed_column(table.name, col.name));
            }
            assert!(!is_allowed_column(table.name, "nonexistent_col"));
        }

        for required in [
            "album_artists",
            "calendar",
            "meta",
            "listens",
            "tracks",
            "albums",
            "artists",
            "track_artists",
        ] {
            assert!(is_allowed_table(required), "missing table {required}");
        }
        for col in [
            "played_at_local",
            "week_start",
            "sources",
            "device",
            "country",
            "shuffle",
            "skipped",
            "offline",
            "reason_start",
            "reason_end",
        ] {
            assert!(is_allowed_column("listens", col), "missing listens.{col}");
        }
        assert!(is_allowed_column("tracks", "edition_key"));

        let prompt = crate::ask::prompt::remote_sql_system_prompt("2025-01-01");
        let desc = schema_description();
        assert!(prompt.contains(&desc));
        for table in TABLES {
            assert!(
                desc.contains(table.name),
                "schema_description missing {}",
                table.name
            );
            for col in table.columns {
                assert!(
                    desc.contains(col.name),
                    "schema_description missing {}.{}",
                    table.name,
                    col.name
                );
            }
        }
    }

    #[tokio::test]
    async fn projection_populates_complete_columns_album_artists_calendar_editions_and_meta() {
        let dir = tempfile::tempdir().unwrap();
        let storage = db::open(dir.path()).await.unwrap();

        // Multi-track album with a primary artist ("BTS") and a single-track guest ("Halsey").
        // Also a second edition of "Boy With Luv" on a compilation album, and a listen across midnight DST.
        let t_base = chrono::DateTime::parse_from_rfc3339("2024-03-01T23:30:00Z")
            .unwrap()
            .timestamp_millis(); // 2024-03-02 00:30:00 in Europe/Rome (UTC+1)
        let t_next = chrono::DateTime::parse_from_rfc3339("2024-03-04T10:00:00Z")
            .unwrap()
            .timestamp_millis();

        let track_collab = parse_track(&json!({
            "uri": "spotify:track:0000000000000000000011",
            "type": "track",
            "name": "Boy With Luv",
            "duration_ms": 230_000,
            "album": {"id": "al-persona", "name": "Map of the Soul: Persona", "images": []},
            "artists": [
                {"id": "ar-bts", "name": "BTS"},
                {"id": "ar-halsey", "name": "Halsey"}
            ],
        }))
        .unwrap();
        let track_solo = parse_track(&json!({
            "uri": "spotify:track:0000000000000000000012",
            "type": "track",
            "name": "Mikrokosmos",
            "duration_ms": 224_000,
            "album": {"id": "al-persona", "name": "Map of the Soul: Persona", "images": []},
            "artists": [{"id": "ar-bts", "name": "BTS"}],
        }))
        .unwrap();
        let track_edition = parse_track(&json!({
            "uri": "spotify:track:0000000000000000000013",
            "type": "track",
            "name": "Boy With Luv",
            "duration_ms": 230_000,
            "album": {"id": "al-proof", "name": "Proof", "images": []},
            "artists": [
                {"id": "ar-bts", "name": "BTS"},
                {"id": "ar-halsey", "name": "Halsey"}
            ],
        }))
        .unwrap();
        let track_trashed_only = parse_track(&json!({
            "uri": "spotify:track:0000000000000000000099",
            "type": "track",
            "name": "Deleted Song",
            "duration_ms": 100_000,
            "album": {"id": "al-deleted", "name": "Deleted Album", "images": []},
            "artists": [{"id": "ar-deleted", "name": "Deleted Artist"}],
        }))
        .unwrap();

        let events = vec![
            // Spotify import event with full playback context.
            Event {
                source: "spotify_import",
                source_key: "imp-1".into(),
                source_timestamp_ms: t_base + 200_000,
                played_at_ms: t_base,
                listened_ms: Some(200_000),
                duration_kind: DurationKind::Actual,
                counted: true,
                track: track_collab.clone(),
                raw: json!({
                    "platform": "android",
                    "conn_country": "IT",
                    "shuffle": true,
                    "skipped": false,
                    "offline": false,
                    "reason_start": "trackdone",
                    "reason_end": "endplay"
                }),
            },
            // Reconciled spotify_api event for the same listen -> sources has both.
            Event {
                source: "spotify_api",
                source_key: "api-1".into(),
                source_timestamp_ms: t_base,
                played_at_ms: t_base,
                listened_ms: Some(230_000),
                duration_kind: DurationKind::Estimated,
                counted: true,
                track: track_collab,
                raw: json!({}),
            },
            Event {
                source: "listenbrainz",
                source_key: "lb-2".into(),
                source_timestamp_ms: t_next,
                played_at_ms: t_next,
                listened_ms: None,
                duration_kind: DurationKind::Unknown,
                counted: true,
                track: track_solo,
                raw: json!({}),
            },
            Event {
                source: "spotify_api",
                source_key: "api-3".into(),
                source_timestamp_ms: t_next + 60_000,
                played_at_ms: t_next + 60_000,
                listened_ms: Some(15_000),
                duration_kind: DurationKind::Actual,
                counted: false,
                track: track_edition,
                raw: json!({}),
            },
            Event {
                source: "spotify_api",
                source_key: "api-trashed".into(),
                source_timestamp_ms: t_next + 120_000,
                played_at_ms: t_next + 120_000,
                listened_ms: Some(100_000),
                duration_kind: DurationKind::Actual,
                counted: true,
                track: track_trashed_only,
                raw: json!({}),
            },
        ];

        let mut tx = storage.writer.begin().await.unwrap();
        db::ingest(&mut tx, &events).await.unwrap();
        sqlx::query("UPDATE listens SET trashed_at_ms = 1 WHERE id IN (SELECT listen_id FROM observations WHERE source_key = 'api-trashed')")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("UPDATE listens SET needs_review = 1 WHERE id IN (SELECT listen_id FROM observations WHERE source_key = 'lb-2')")
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let gate = ProjectionGate::default();
        let proj_path = ensure(&storage, &gate).await.unwrap();

        // 1. Trashed-only artist/album/track must not appear in the projection.
        let out = sandbox::execute(
            &proj_path,
            "SELECT COUNT(*) FROM artists WHERE name = 'Deleted Artist'",
            SandboxLimits::default(),
        )
        .unwrap();
        assert_eq!(out.rows, vec![vec![SqlValue::Integer(0)]]);

        // 2. Listen local datetime, week_start, sources, and Spotify export context.
        let out = sandbox::execute(
            &proj_path,
            "SELECT played_at_local, local_date, local_hour, weekday, week_start, sources, device, country, shuffle, skipped, offline, reason_start, reason_end FROM listens WHERE local_date = '2024-03-02'",
            SandboxLimits::default(),
        )
        .unwrap();
        assert_eq!(
            out.rows,
            vec![vec![
                SqlValue::Text("2024-03-02 00:30:00".into()),
                SqlValue::Text("2024-03-02".into()),
                SqlValue::Integer(0),
                SqlValue::Integer(6), // Saturday
                SqlValue::Integer(0), // Monday start
                SqlValue::Text("[\"spotify_api\",\"spotify_import\"]".into()),
                SqlValue::Text("android".into()),
                SqlValue::Text("IT".into()),
                SqlValue::Integer(1),
                SqlValue::Integer(0),
                SqlValue::Integer(0),
                SqlValue::Text("trackdone".into()),
                SqlValue::Text("endplay".into()),
            ]]
        );

        // 3. album_artists excludes single-track guest Halsey on Map of the Soul: Persona.
        let out = sandbox::execute(
            &proj_path,
            "SELECT a.name FROM album_artists aa JOIN albums al ON al.id = aa.album_id JOIN artists a ON a.id = aa.artist_id WHERE al.name = 'Map of the Soul: Persona' ORDER BY aa.ordinal",
            SandboxLimits::default(),
        )
        .unwrap();
        assert_eq!(out.rows, vec![vec![SqlValue::Text("BTS".into())]]);

        // 4. edition_key groups both editions of "Boy With Luv" while keeping "Mikrokosmos" separate.
        let out = sandbox::execute(
            &proj_path,
            "SELECT COUNT(DISTINCT edition_key) FROM tracks",
            SandboxLimits::default(),
        )
        .unwrap();
        assert_eq!(out.rows, vec![vec![SqlValue::Integer(2)]]);

        // 5. calendar table covers every day from 2024-03-02 through at least 2024-03-04 without gaps.
        let out = sandbox::execute(
            &proj_path,
            "SELECT local_date, week_date, year, month, day, weekday FROM calendar WHERE local_date BETWEEN '2024-03-02' AND '2024-03-04' ORDER BY local_date",
            SandboxLimits::default(),
        )
        .unwrap();
        assert_eq!(
            out.rows,
            vec![
                vec![
                    SqlValue::Text("2024-03-02".into()),
                    SqlValue::Text("2024-02-26".into()),
                    SqlValue::Integer(2024),
                    SqlValue::Integer(3),
                    SqlValue::Integer(2),
                    SqlValue::Integer(6),
                ],
                vec![
                    SqlValue::Text("2024-03-03".into()),
                    SqlValue::Text("2024-02-26".into()),
                    SqlValue::Integer(2024),
                    SqlValue::Integer(3),
                    SqlValue::Integer(3),
                    SqlValue::Integer(7),
                ],
                vec![
                    SqlValue::Text("2024-03-04".into()),
                    SqlValue::Text("2024-03-04".into()),
                    SqlValue::Integer(2024),
                    SqlValue::Integer(3),
                    SqlValue::Integer(4),
                    SqlValue::Integer(1),
                ],
            ]
        );

        // 6. meta table reports revision, built_at_ms > 0, timezone, week_start.
        let out = sandbox::execute(
            &proj_path,
            "SELECT revision, built_at_ms, timezone, week_start FROM meta",
            SandboxLimits::default(),
        )
        .unwrap();
        assert_eq!(out.rows.len(), 1);
        assert_eq!(out.rows[0][0], SqlValue::Integer(1));
        assert!(matches!(out.rows[0][1], SqlValue::Integer(ms) if ms > 0));
        assert_eq!(out.rows[0][2], SqlValue::Text("Europe/Rome".into()));
        assert_eq!(out.rows[0][3], SqlValue::Integer(0));
    }

    #[tokio::test]
    async fn for_query_serves_previous_projection_while_rebuild_runs_and_remove_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let storage = db::open(dir.path()).await.unwrap();
        let t1 = chrono::DateTime::parse_from_rfc3339("2024-04-01T10:00:00Z")
            .unwrap()
            .timestamp_millis();
        let track1 = parse_track(&json!({
            "uri": "spotify:track:0000000000000000000021",
            "type": "track",
            "name": "Track One",
            "duration_ms": 200_000,
            "album": {"id": "al-1", "name": "Album One", "images": []},
            "artists": [{"id": "ar-1", "name": "Artist One"}],
        }))
        .unwrap();
        let mut tx = storage.writer.begin().await.unwrap();
        db::set_setting(&mut tx, "ai_enabled", "1").await.unwrap();
        db::ingest(
            &mut tx,
            &[Event {
                source: "spotify_api",
                source_key: "ev-1".into(),
                source_timestamp_ms: t1,
                played_at_ms: t1,
                listened_ms: Some(200_000),
                duration_kind: DurationKind::Actual,
                counted: true,
                track: track1.clone(),
                raw: json!({}),
            }],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let gate = ProjectionGate::default();
        let proj_path = for_query(&storage, &gate).await.unwrap();
        let as_of_1 = read_built_at_ms(&proj_path).unwrap();
        assert!(size_bytes(dir.path()) > 0);

        // Ingest a second play so the archive revision advances, then simulate an active
        // background rebuild by holding the build lock.
        let mut tx = storage.writer.begin().await.unwrap();
        db::ingest(
            &mut tx,
            &[Event {
                source: "spotify_api",
                source_key: "ev-2".into(),
                source_timestamp_ms: t1 + 60_000,
                played_at_ms: t1 + 60_000,
                listened_ms: Some(200_000),
                duration_kind: DurationKind::Actual,
                counted: true,
                track: track1,
                raw: json!({}),
            }],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let build_guard = gate.lock_build().await;
        // for_query must not block on the rebuild and must return the previous projection.
        let served = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            for_query(&storage, &gate),
        )
        .await
        .expect("for_query should not block while rebuild runs")
        .unwrap();
        assert_eq!(read_built_at_ms(&served), Some(as_of_1));
        let out = sandbox::execute(
            &served,
            "SELECT COUNT(*) FROM listens",
            SandboxLimits::default(),
        )
        .unwrap();
        assert_eq!(out.rows, vec![vec![SqlValue::Integer(1)]]);
        drop(build_guard);

        // Once the build lock is free, the next for_query updates to the new revision and as-of time.
        let updated = for_query(&storage, &gate).await.unwrap();
        let as_of_2 = read_built_at_ms(&updated).unwrap();
        assert!(as_of_2 > as_of_1);
        let out = sandbox::execute(
            &updated,
            "SELECT COUNT(*) FROM listens",
            SandboxLimits::default(),
        )
        .unwrap();
        assert_eq!(out.rows, vec![vec![SqlValue::Integer(2)]]);

        // Disabling / removing deletes the projection and reports 0 bytes.
        remove(&storage.directory, &gate).await.unwrap();
        assert!(!path(dir.path()).exists());
        assert_eq!(size_bytes(dir.path()), 0);
    }

    #[tokio::test]
    async fn background_worker_builds_only_when_ai_enabled_and_debounces_revision_changes() {
        let dir = tempfile::tempdir().unwrap();
        let storage = db::open(dir.path()).await.unwrap();
        let gate = ProjectionGate::default();
        let handle = tokio::spawn(worker_loop(
            storage.clone(),
            gate.clone(),
            std::time::Duration::from_millis(25),
            std::time::Duration::from_millis(60),
        ));

        // While AI is disabled, nothing is built.
        tokio::time::sleep(std::time::Duration::from_millis(90)).await;
        assert!(!path(dir.path()).exists());

        // Enable AI -> worker builds the projection automatically.
        let mut tx = storage.writer.begin().await.unwrap();
        db::set_setting(&mut tx, "ai_enabled", "1").await.unwrap();
        tx.commit().await.unwrap();
        gate.notify();

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if path(dir.path()).is_file() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            }
        })
        .await
        .expect("worker should build projection on enabling AI");

        let rev_before = sandbox::execute(
            &path(dir.path()),
            "SELECT revision FROM meta",
            SandboxLimits::default(),
        )
        .unwrap()
        .rows[0][0]
            .clone();
        assert_eq!(rev_before, SqlValue::Integer(0));

        // Ingest a listen -> debounced background rebuild updates meta.revision.
        let t1 = chrono::DateTime::parse_from_rfc3339("2024-05-01T10:00:00Z")
            .unwrap()
            .timestamp_millis();
        let track1 = parse_track(&json!({
            "uri": "spotify:track:0000000000000000000031",
            "type": "track",
            "name": "Debounce Song",
            "duration_ms": 180_000,
            "album": {"id": "al-deb", "name": "Debounce Album", "images": []},
            "artists": [{"id": "ar-deb", "name": "Debounce Artist"}],
        }))
        .unwrap();
        let mut tx = storage.writer.begin().await.unwrap();
        db::ingest(
            &mut tx,
            &[Event {
                source: "spotify_api",
                source_key: "deb-1".into(),
                source_timestamp_ms: t1,
                played_at_ms: t1,
                listened_ms: Some(180_000),
                duration_kind: DurationKind::Actual,
                counted: true,
                track: track1,
                raw: json!({}),
            }],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        gate.notify();

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let guard = gate.read_lock().await;
                let rev = sandbox::execute(
                    &path(dir.path()),
                    "SELECT revision FROM meta",
                    SandboxLimits::default(),
                );
                drop(guard);
                if let Ok(out) = rev
                    && out.rows == vec![vec![SqlValue::Integer(1)]]
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            }
        })
        .await
        .expect("worker should rebuild projection after revision change");

        // Disable AI -> worker deletes the projection.
        let mut tx = storage.writer.begin().await.unwrap();
        db::set_setting(&mut tx, "ai_enabled", "0").await.unwrap();
        tx.commit().await.unwrap();
        gate.notify();

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if !path(dir.path()).exists() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            }
        })
        .await
        .expect("worker should delete projection when AI is disabled");

        handle.abort();
        let _ = handle.await;
    }

    const DOC: &str = include_str!("../../docs/ask-projection.md");
    const DOC_BEGIN: &str = "<!-- BEGIN GENERATED: ask projection schema -->";
    const DOC_END: &str = "<!-- END GENERATED: ask projection schema -->";

    fn regenerate_document(document: &str, schema: &str) -> Option<String> {
        let start = document.find(DOC_BEGIN)? + DOC_BEGIN.len();
        let end = start + document[start..].find(DOC_END)?;
        Some(format!(
            "{}\n{schema}\n{}",
            &document[..start],
            &document[end..]
        ))
    }

    #[test]
    fn the_checked_in_projection_document_matches_the_generated_schema() {
        let schema = schema_markdown();
        let expected = regenerate_document(DOC, &schema)
            .expect("docs/ask-projection.md carries the generated-schema markers");
        if std::env::var_os("UPDATE_ASK_DOC").is_some() {
            std::fs::write(
                concat!(env!("CARGO_MANIFEST_DIR"), "/docs/ask-projection.md"),
                &expected,
            )
            .expect("write docs/ask-projection.md");
        }
        assert_eq!(
            DOC, expected,
            "docs/ask-projection.md is stale; run \
             $env:UPDATE_ASK_DOC='1'; cargo test the_checked_in_projection_document_matches_the_generated_schema"
        );
    }

    #[test]
    fn the_projection_document_mentions_every_table_and_rule_layer() {
        assert!(DOC.contains("Ask projection"));
        for table in TABLES {
            assert!(DOC.contains(table.name), "missing table {}", table.name);
        }
        for rule in [
            "query_only",
            "trusted_schema",
            "SQLITE_LIMIT_ATTACHED",
            "authorizer",
            "sql_too_long",
            "unauthorized_table",
            "timeout",
            "instruction_limit",
            "output_too_large",
            "cancelled",
        ] {
            assert!(DOC.contains(rule), "missing sandbox detail {rule}");
        }
    }
}
