use crate::{auth::digest, error::Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, Sqlite, SqliteConnection, SqlitePool, Transaction};
use std::collections::{BTreeMap, BTreeSet};

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
/// Source review markers are separate from reconciliation. Equal export end times with
/// different durations remain distinct; missing external identity never authorizes a merge.
pub(crate) async fn review_spotify_imports(
    tx: &mut Transaction<'_, Sqlite>,
    events: &[Event],
) -> Result<()> {
    let mut changed = BTreeSet::new();
    for event in events.iter().filter(|e| e.source == "spotify_import") {
        let rows:Vec<(String,String)>=sqlx::query_as("SELECT o.listen_id,l.track_id FROM observations o JOIN listens l ON l.id=o.listen_id WHERE o.source='spotify_import' AND o.source_key=?").bind(&event.source_key).fetch_all(&mut **tx).await?;
        let Some((listen, track)) = rows.first() else {
            continue;
        };
        let uncertain = !event.track.id.starts_with("spotify:track:");
        let peers: Vec<String> = if uncertain {
            Vec::new()
        } else {
            sqlx::query_scalar("SELECT o.listen_id FROM observations o JOIN listens l ON l.id=o.listen_id WHERE o.source='spotify_import' AND l.track_id=? AND o.source_timestamp_ms=? AND o.source_key!=?").bind(track).bind(event.source_timestamp_ms).bind(&event.source_key).fetch_all(&mut **tx).await?
        };
        if !uncertain && peers.is_empty() {
            continue;
        }
        for id in std::iter::once(listen.clone()).chain(peers) {
            let flagged =
                sqlx::query("UPDATE listens SET needs_review=1 WHERE id=? AND needs_review=0")
                    .bind(&id)
                    .execute(&mut **tx)
                    .await?
                    .rows_affected();
            let inserted=sqlx::query("INSERT INTO conflicts(id,listen_id,kind,evidence_json) VALUES(?,?,?,?) ON CONFLICT(listen_id,kind) DO NOTHING").bind(new_id()).bind(&id).bind(if uncertain{"spotify_identity_uncertain"}else{"spotify_export_duration"}).bind(serde_json::json!({"source":"spotify_import","timestamp_ms":event.source_timestamp_ms}).to_string()).execute(&mut **tx).await?.rows_affected();
            if flagged > 0 || inserted > 0 {
                changed.insert(id);
            }
        }
    }
    if !changed.is_empty() {
        publish(tx, &changed).await?;
    }
    Ok(())
}
/// Internal IDs are independent of source keys, titles and timestamps.
pub fn new_id() -> String {
    crate::auth::random_secret()
}

async fn entity(conn: &mut SqliteConnection, kind: &str, key: &str, scope: &str) -> Result<String> {
    let certain = key.starts_with(&format!("spotify:{kind}:"));
    let namespace = if certain { "spotify" } else { scope };
    if let Some(id) = sqlx::query_scalar(
        "SELECT entity_id FROM entity_links WHERE kind=? AND namespace=? AND external_id=?",
    )
    .bind(kind)
    .bind(namespace)
    .bind(key)
    .fetch_optional(&mut *conn)
    .await?
    {
        return Ok(id);
    }
    let id = new_id();
    sqlx::query("INSERT INTO entity_links(kind,namespace,external_id,entity_id,provisional,evidence_json) VALUES(?,?,?,?,?,?)")
        .bind(kind).bind(namespace).bind(key).bind(&id).bind(!certain)
        .bind(serde_json::json!({"source_key":key,"scope":scope}).to_string()).execute(conn).await?;
    Ok(id)
}

async fn upsert_track(conn: &mut SqliteConnection, event: &Event) -> Result<(String, bool)> {
    let track = &event.track;
    // Provisional names are scoped to an observation, never global proof of identity.
    let scope = format!("{}:{}", event.source, event.source_key);
    let id = entity(conn, "track", &track.id, &scope).await?;
    let rank = format!(
        "{}:{}",
        u8::from(track.structured),
        digest(&serde_json::to_string(track).map_err(|_| crate::error::AppError::Internal)?)
    );
    let prior = sqlx::query_scalar::<_, String>("SELECT metadata_rank FROM tracks WHERE id=?")
        .bind(&id)
        .fetch_optional(&mut *conn)
        .await?;
    if prior.as_ref().is_some_and(|prior| prior >= &rank) {
        return Ok((id, false));
    }
    let mut album_id = None;
    if let (Some(key), Some(name)) = (&track.album_id, &track.album) {
        let album = entity(conn, "album", key, &format!("track:{id}")).await?;
        sqlx::query("INSERT INTO albums(id,name,cover_url,metadata_rank) VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,cover_url=COALESCE(excluded.cover_url,albums.cover_url),metadata_rank=excluded.metadata_rank WHERE excluded.metadata_rank>albums.metadata_rank")
            .bind(&album).bind(name).bind(&track.cover_url).bind(&rank).execute(&mut *conn).await?;
        album_id = Some(album);
    }
    let spotify_url = track
        .id
        .strip_prefix("spotify:track:")
        .map(|key| format!("https://open.spotify.com/track/{key}"));
    sqlx::query("INSERT INTO tracks(id,title,album_id,duration_ms,spotify_url,structured_metadata,metadata_rank) VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET title=excluded.title,album_id=COALESCE(excluded.album_id,tracks.album_id),duration_ms=COALESCE(excluded.duration_ms,tracks.duration_ms),spotify_url=excluded.spotify_url,structured_metadata=excluded.structured_metadata,metadata_rank=excluded.metadata_rank")
        .bind(&id).bind(&track.title).bind(album_id).bind(track.duration_ms).bind(spotify_url).bind(track.structured).bind(&rank).execute(&mut *conn).await?;
    sqlx::query("DELETE FROM track_artists WHERE track_id=?")
        .bind(&id)
        .execute(&mut *conn)
        .await?;
    for (ordinal, artist) in track.artists.iter().enumerate() {
        let artist_id = entity(conn, "artist", &artist.id, &format!("track:{id}")).await?;
        sqlx::query("INSERT INTO artists(id,name,metadata_rank) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,metadata_rank=excluded.metadata_rank WHERE excluded.metadata_rank>artists.metadata_rank")
            .bind(&artist_id).bind(&artist.name).bind(&rank).execute(&mut *conn).await?;
        sqlx::query("INSERT OR IGNORE INTO track_artists VALUES(?,?,?)")
            .bind(&id)
            .bind(artist_id)
            .bind(ordinal as i64)
            .execute(&mut *conn)
            .await?;
    }
    Ok((id, true))
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
    sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision")
        .fetch_optional(&mut **tx).await?.ok_or_else(||crate::error::AppError::Conflict("Archive is in maintenance.".into()))?;
    let mut report = IngestReport::default();
    let mut incoming: BTreeMap<(&str, &str), BTreeSet<(i64, &str)>> = BTreeMap::new();
    for event in events {
        incoming
            .entry((event.source, &event.track.id))
            .or_default()
            .insert((event.played_at_ms, &event.source_key));
    }
    let mut affected = BTreeSet::new();
    for event in events {
        let fingerprint = payload_fingerprint(&event.raw);
        let existing=sqlx::query("SELECT o.id,o.listen_id,EXISTS(SELECT 1 FROM observation_variants v WHERE v.observation_id=o.id AND fingerprint=?) AS same FROM observations o WHERE source=? AND source_key=?")
            .bind(&fingerprint).bind(event.source).bind(&event.source_key).fetch_optional(&mut **tx).await?;
        if existing.as_ref().is_some_and(|r| r.get::<bool, _>("same")) {
            report.duplicates += 1;
            continue;
        }
        let (track_id, metadata_changed) = upsert_track(tx, event).await?;
        let kind = event.duration_kind.as_str();
        if (event.duration_kind == DurationKind::Unknown) != event.listened_ms.is_none()
            || event.listened_ms.is_some_and(|ms| ms < 0)
        {
            return Err(crate::error::AppError::BadRequest(
                "Duration category and value disagree.".into(),
            ));
        }
        let counted = event.counted;
        let (observation_id, listen_id) = if let Some(row) = existing {
            report.variants += 1;
            (
                row.get::<String, _>("id"),
                row.get::<String, _>("listen_id"),
            )
        } else {
            report.originals += 1;
            // Only the approved provisional Spotify API/export policy, not generic cross-source matching.
            let counterpart = match event.source {
                "spotify_import" => Some("spotify_api"),
                "spotify_api" => Some("spotify_import"),
                _ => None,
            };
            if counterpart.is_some() && event.track.id.starts_with("spotify:track:") {
                // A later batch can reveal that a provisional one-to-one pairing was ambiguous.
                // Split it before matching, preserving IDs for the original anchor observation.
                let paired:Vec<String>=sqlx::query_scalar("SELECT l.id FROM listens l WHERE track_id=? AND trashed_at_ms IS NULL AND EXISTS(SELECT 1 FROM observations o WHERE o.listen_id=l.id AND source=?) AND EXISTS(SELECT 1 FROM observations o JOIN observation_variants v ON v.observation_id=o.id WHERE o.listen_id=l.id AND o.source=? AND v.played_at_ms BETWEEN ? AND ?)")
                    .bind(&track_id).bind(event.source).bind(counterpart).bind(event.played_at_ms.saturating_sub(2000)).bind(event.played_at_ms.saturating_add(2000)).fetch_all(&mut **tx).await?;
                for id in paired {
                    report.added += split_pair(tx, &id, &mut affected).await?;
                }
            }
            let candidates = if event.track.id.starts_with("spotify:track:")
                && counterpart.is_some()
            {
                sqlx::query("SELECT l.id,l.played_at_ms FROM listens l WHERE l.track_id=? AND l.trashed_at_ms IS NULL AND l.played_at_ms BETWEEN ? AND ? AND EXISTS(SELECT 1 FROM observations o WHERE o.listen_id=l.id AND o.source=?) AND NOT EXISTS(SELECT 1 FROM observations o WHERE o.listen_id=l.id AND o.source=?) LIMIT 2")
                    .bind(&track_id).bind(event.played_at_ms.saturating_sub(2000)).bind(event.played_at_ms.saturating_add(2000)).bind(counterpart).bind(event.source).fetch_all(&mut **tx).await?
            } else {
                Vec::new()
            };
            let mut reverse_ambiguous = candidates.first().is_some_and(|r| {
                let time = r.get::<i64, _>("played_at_ms");
                incoming
                    .get(&(event.source, event.track.id.as_str()))
                    .is_some_and(|times| {
                        times
                            .range(
                                (time.saturating_sub(2000), "")
                                    ..=(time.saturating_add(2000), "\u{10ffff}"),
                            )
                            .any(|(_, key)| *key != event.source_key)
                    })
            });
            if let Some(candidate) = candidates.first() {
                let time = candidate.get::<i64, _>("played_at_ms");
                let known_competitor:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM listens l JOIN observations o ON o.listen_id=l.id WHERE l.track_id=? AND l.trashed_at_ms IS NULL AND o.source=? AND o.source_key!=? AND l.played_at_ms BETWEEN ? AND ?)")
                    .bind(&track_id).bind(event.source).bind(&event.source_key).bind(time.saturating_sub(2000)).bind(time.saturating_add(2000)).fetch_one(&mut **tx).await?;
                reverse_ambiguous |= known_competitor;
            }
            let ambiguous = candidates.len() > 1 || reverse_ambiguous;
            let listen_id = if candidates.len() == 1 && !ambiguous {
                report.reconciled += 1;
                candidates[0].get::<String, _>("id")
            } else {
                if ambiguous {
                    report.ambiguous += 1;
                    let ids:Vec<String>=sqlx::query_scalar("UPDATE listens SET needs_review=1 WHERE track_id=? AND played_at_ms BETWEEN ? AND ? RETURNING id")
                        .bind(&track_id).bind(event.played_at_ms.saturating_sub(2000)).bind(event.played_at_ms.saturating_add(2000)).fetch_all(&mut **tx).await?;
                    affected.extend(ids);
                }
                report.added += 1;
                let id = new_id();
                let calendar = crate::calendar::current(tx).await?;
                let (date, week) = calendar.dates(event.played_at_ms)?;
                sqlx::query("INSERT INTO listens(id,track_id,played_at_ms,first_acquired_at_ms,calendar_id,local_date,week_date,listened_ms,duration_kind,counted,needs_review,anchor_source) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)")
                    .bind(&id).bind(&track_id).bind(event.played_at_ms).bind(acquired_at_ms).bind(&calendar.id).bind(date).bind(week)
                    .bind(event.listened_ms).bind(kind).bind(counted).bind(ambiguous).bind(event.source).execute(&mut **tx).await?;
                id
            };
            let observation_id = new_id();
            sqlx::query("INSERT INTO observations(id,source,source_key,listen_id,source_timestamp_ms,raw_json) VALUES(?,?,?,?,?,?)")
                .bind(&observation_id).bind(event.source).bind(&event.source_key).bind(&listen_id).bind(event.source_timestamp_ms).bind(event.raw.to_string()).execute(&mut **tx).await?;
            (observation_id, listen_id)
        };
        sqlx::query("INSERT INTO observation_variants(id,observation_id,fingerprint,raw_json,played_at_ms,listened_ms,duration_kind,counted,policy_version) VALUES(?,?,?,?,?,?,?,?,1)")
            .bind(new_id()).bind(&observation_id).bind(&fingerprint).bind(event.raw.to_string()).bind(event.played_at_ms).bind(event.listened_ms).bind(kind).bind(counted).execute(&mut **tx).await?;
        canonicalize(tx, &listen_id).await?;
        affected.insert(listen_id);
        // Changes to shared Spotify artist/album metadata are visible to every linked listen.
        if metadata_changed {
            let ids:Vec<String>=sqlx::query_scalar("SELECT l.id FROM listens l WHERE track_id=? UNION SELECT l.id FROM listens l JOIN tracks t ON t.id=l.track_id WHERE t.album_id=(SELECT album_id FROM tracks WHERE id=?) UNION SELECT l.id FROM listens l WHERE l.track_id IN (SELECT a.track_id FROM track_artists a JOIN track_artists b ON a.artist_id=b.artist_id WHERE b.track_id=?)")
            .bind(&track_id).bind(&track_id).bind(&track_id).fetch_all(&mut **tx).await?;
            affected.extend(ids);
        }
    }
    publish(tx, &affected).await?;
    Ok(report)
}

async fn split_pair(
    conn: &mut SqliteConnection,
    id: &str,
    affected: &mut BTreeSet<String>,
) -> Result<u64> {
    let detached:Vec<String>=sqlx::query_scalar("SELECT o.id FROM observations o JOIN listens l ON l.id=o.listen_id WHERE l.id=? AND o.source!=l.anchor_source")
        .bind(id).fetch_all(&mut *conn).await?;
    let count = detached.len() as u64;
    for observation in detached {
        let new = new_id();
        sqlx::query("INSERT INTO listens(id,track_id,played_at_ms,first_acquired_at_ms,calendar_id,local_date,week_date,policy_version,anchor_source,listened_ms,duration_kind,counted,needs_review,trashed_at_ms) SELECT ?,track_id,played_at_ms,first_acquired_at_ms,calendar_id,local_date,week_date,policy_version,(SELECT source FROM observations WHERE id=?),listened_ms,duration_kind,counted,1,trashed_at_ms FROM listens WHERE id=?")
            .bind(&new).bind(&observation).bind(id).execute(&mut *conn).await?;
        sqlx::query("UPDATE observations SET listen_id=? WHERE id=?")
            .bind(&new)
            .bind(&observation)
            .execute(&mut *conn)
            .await?;
        canonicalize(conn, &new).await?;
        sqlx::query("INSERT INTO identity_decisions VALUES(?,'reconciliation_split',?,?,?,?)").bind(new_id()).bind(id).bind(&new)
            .bind(serde_json::json!({"observation_id":observation,"reason":"competing_source_event","policy_version":1,"calendar":"inherited"}).to_string()).bind(crate::auth::now_ms()).execute(&mut *conn).await?;
        affected.insert(new);
    }
    sqlx::query("UPDATE listens SET needs_review=1 WHERE id=?")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    canonicalize(conn, id).await?;
    affected.insert(id.to_owned());
    Ok(count)
}

async fn canonicalize(conn: &mut SqliteConnection, id: &str) -> Result<()> {
    let variants=sqlx::query("SELECT v.*,o.source FROM observation_variants v JOIN observations o ON o.id=v.observation_id WHERE o.listen_id=? ORDER BY CASE o.source WHEN 'spotify_api' THEN 0 WHEN 'spotify_import' THEN 1 ELSE 2 END,v.played_at_ms,v.fingerprint")
        .bind(id).fetch_all(&mut *conn).await?;
    let time = variants[0].get::<i64, _>("played_at_ms");
    let actual: BTreeSet<i64> = variants
        .iter()
        .filter(|v| v.get::<String, _>("duration_kind") == "actual")
        .filter_map(|v| v.get::<Option<i64>, _>("listened_ms"))
        .collect();
    let conflict = actual.len() > 1;
    let (duration, kind) = if conflict {
        (None, "unknown")
    } else if let Some(ms) = actual.first() {
        (Some(*ms), "actual")
    } else {
        (
            variants
                .iter()
                .find_map(|v| v.get::<Option<i64>, _>("listened_ms")),
            "estimated",
        )
    };
    let kind = if duration.is_none() { "unknown" } else { kind };
    let counted = if actual.is_empty() {
        variants.iter().all(|v| v.get::<bool, _>("counted"))
    } else {
        variants
            .iter()
            .filter(|v| v.get::<String, _>("duration_kind") == "actual")
            .all(|v| v.get::<bool, _>("counted"))
    };
    let calendar_id: String = sqlx::query_scalar("SELECT calendar_id FROM listens WHERE id=?")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    let calendar = crate::calendar::get(conn, &calendar_id).await?;
    let (date, week) = calendar.dates(time)?;
    sqlx::query("UPDATE listens SET played_at_ms=?,listened_ms=?,duration_kind=?,counted=?,local_date=?,week_date=?,needs_review=CASE WHEN ? THEN 1 ELSE needs_review END WHERE id=?")
        .bind(time).bind(duration).bind(kind).bind(counted).bind(date).bind(week).bind(conflict).bind(id).execute(&mut *conn).await?;
    if conflict {
        sqlx::query("INSERT INTO conflicts(id,listen_id,kind,evidence_json) VALUES(?,?,'actual_duration',?) ON CONFLICT(listen_id,kind) DO UPDATE SET evidence_json=excluded.evidence_json,resolved_at_ms=NULL")
            .bind(new_id()).bind(id).bind(serde_json::json!({"actual_ms":actual,"policy_version":1}).to_string()).execute(conn).await?;
    } else {
        sqlx::query("UPDATE conflicts SET resolved_at_ms=? WHERE listen_id=? AND kind='actual_duration' AND resolved_at_ms IS NULL")
            .bind(crate::auth::now_ms()).bind(id).execute(conn).await?;
    }
    Ok(())
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
