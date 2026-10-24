//! Bounded ingestion batches use one exclusive SQLite handle, inside the caller's
//! existing transaction. Avoid a worker-channel round trip for every tiny statement.
use crate::{
    calendar::Calendar,
    db::{self, DurationKind, Event, IngestReport},
    error::{AppError, Result},
};
use rusqlite::{Connection, OptionalExtension, Params, Row, params, types::FromSql};
use std::collections::{BTreeMap, BTreeSet};

fn archive_id() -> String {
    use std::sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    };
    static PREFIX: OnceLock<[u8; 8]> = OnceLock::new();
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let prefix = PREFIX.get_or_init(|| {
        let mut bytes = [0u8; 8];
        rand::fill(&mut bytes);
        bytes
    });
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(prefix);
    bytes[8..16].copy_from_slice(&SEQUENCE.fetch_add(1, Ordering::Relaxed).to_be_bytes());
    rand::fill(&mut bytes[16..]);
    // Opaque IDs retain 128 random suffix bits and the existing wire shape, while
    // adjacent archive inserts share B-tree pages. No source/listen time is encoded.
    crate::auth::encode_hex(&bytes)
}

pub(crate) async fn with_connection<T>(
    conn: &mut sqlx::SqliteConnection,
    operation: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let mut locked = conn.lock_handle().await?;
    // SAFETY: SQLx's exclusive handle lock outlives the borrowed connection and all
    // its statements. from_handle does not own/close it; no transaction is started
    // or committed here. Both crates link the same pinned libsqlite3-sys 0.37.
    let borrowed = unsafe { Connection::from_handle(locked.as_raw_handle().as_ptr()) }?;
    borrowed.set_prepared_statement_cache_capacity(64);
    if tokio::runtime::Handle::current().runtime_flavor()
        == tokio::runtime::RuntimeFlavor::MultiThread
    {
        // Let Tokio service other requests while this bounded SQLite batch runs.
        tokio::task::block_in_place(|| operation(&borrowed))
    } else {
        operation(&borrowed)
    }
}
fn execute(conn: &Connection, sql: &str, values: impl Params) -> Result<usize> {
    Ok(conn.prepare_cached(sql)?.execute(values)?)
}
fn one<T: FromSql>(conn: &Connection, sql: &str, values: impl Params) -> Result<T> {
    Ok(conn.prepare_cached(sql)?.query_row(values, |r| r.get(0))?)
}
fn optional<T: FromSql>(conn: &Connection, sql: &str, values: impl Params) -> Result<Option<T>> {
    Ok(conn
        .prepare_cached(sql)?
        .query_row(values, |r| r.get(0))
        .optional()?)
}
fn rows<T>(
    conn: &Connection,
    sql: &str,
    values: impl Params,
    map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    Ok(conn
        .prepare_cached(sql)?
        .query_map(values, map)?
        .collect::<rusqlite::Result<_>>()?)
}
fn ids(conn: &Connection, sql: &str, values: impl Params) -> Result<Vec<String>> {
    rows(conn, sql, values, |r| r.get(0))
}
fn calendar(conn: &Connection, id: Option<&str>) -> Result<Calendar> {
    let (sql, values) = match id {
        Some(id) => ("SELECT c.* FROM calendars c WHERE c.id=?", vec![id]),
        None => (
            "SELECT c.* FROM calendars c JOIN settings s ON s.value=c.id WHERE s.key='calendar_id'",
            vec![],
        ),
    };
    Ok(conn
        .prepare_cached(sql)?
        .query_row(rusqlite::params_from_iter(values), |r| {
            Ok(Calendar {
                id: r.get(0)?,
                timezone: r.get(1)?,
                week_start: r.get(2)?,
                rules_version: r.get(3)?,
            })
        })?)
}
fn entity(conn: &Connection, kind: &str, key: &str, scope: &str) -> Result<String> {
    let certain = key.starts_with(&format!("spotify:{kind}:"));
    let namespace = if certain { "spotify" } else { scope };
    if let Some(id) = optional(
        conn,
        "SELECT entity_id FROM entity_links WHERE kind=? AND namespace=? AND external_id=?",
        params![kind, namespace, key],
    )? {
        return Ok(id);
    }
    let id = archive_id();
    execute(
        conn,
        "INSERT INTO entity_links(kind,namespace,external_id,entity_id,provisional,evidence_json) VALUES(?,?,?,?,?,?)",
        params![
            kind,
            namespace,
            key,
            id,
            !certain,
            serde_json::json!({"source_key":key,"scope":scope}).to_string()
        ],
    )?;
    Ok(id)
}
fn upsert_track(conn: &Connection, event: &Event) -> Result<(String, bool)> {
    let track = &event.track;
    let scope = format!("{}:{}", event.source, event.source_key);
    let id = entity(conn, "track", &track.id, &scope)?;
    let rank = format!(
        "{}:{}",
        u8::from(track.structured),
        crate::auth::digest(&serde_json::to_string(track).map_err(|_| AppError::Internal)?)
    );
    let prior: Option<String> =
        optional(conn, "SELECT metadata_rank FROM tracks WHERE id=?", [&id])?;
    if prior.as_ref().is_some_and(|prior| prior >= &rank) {
        return Ok((id, false));
    }
    let mut album_id = None;
    if let (Some(key), Some(name)) = (&track.album_id, &track.album) {
        let album = entity(conn, "album", key, &format!("track:{id}"))?;
        execute(
            conn,
            "INSERT INTO albums(id,name,cover_url,metadata_rank) VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,cover_url=COALESCE(excluded.cover_url,albums.cover_url),metadata_rank=excluded.metadata_rank WHERE excluded.metadata_rank>albums.metadata_rank",
            params![album, name, track.cover_url, rank],
        )?;
        album_id = Some(album);
    }
    let url = track
        .id
        .strip_prefix("spotify:track:")
        .map(|key| format!("https://open.spotify.com/track/{key}"));
    execute(
        conn,
        "INSERT INTO tracks(id,title,album_id,duration_ms,spotify_url,structured_metadata,metadata_rank) VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET title=excluded.title,album_id=COALESCE(excluded.album_id,tracks.album_id),duration_ms=COALESCE(excluded.duration_ms,tracks.duration_ms),spotify_url=excluded.spotify_url,structured_metadata=excluded.structured_metadata,metadata_rank=excluded.metadata_rank",
        params![
            id,
            track.title,
            album_id,
            track.duration_ms,
            url,
            track.structured,
            rank
        ],
    )?;
    execute(conn, "DELETE FROM track_artists WHERE track_id=?", [&id])?;
    for (ordinal, artist) in track.artists.iter().enumerate() {
        let artist_id = entity(conn, "artist", &artist.id, &format!("track:{id}"))?;
        execute(
            conn,
            "INSERT INTO artists(id,name,metadata_rank) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,metadata_rank=excluded.metadata_rank WHERE excluded.metadata_rank>artists.metadata_rank",
            params![artist_id, artist.name, rank],
        )?;
        execute(
            conn,
            "INSERT OR IGNORE INTO track_artists VALUES(?,?,?)",
            params![id, artist_id, ordinal as i64],
        )?;
    }
    for artist in &track.artists {
        let source_id: String=optional(conn,"SELECT entity_id FROM entity_links WHERE kind='artist' AND external_id=? AND (namespace='spotify' OR namespace=?)",params![artist.id,format!("track:{id}")])?.ok_or(AppError::Internal)?;
        crate::name_groups::attach(conn, "artists", &source_id)?;
    }
    if let Some(album) =
        optional::<Option<String>>(conn, "SELECT album_id FROM tracks WHERE id=?", [&id])?.flatten()
    {
        crate::name_groups::attach(conn, "albums", &album)?;
    }

    Ok((id, true))
}
pub(crate) fn publish(conn: &Connection, affected: &BTreeSet<String>) -> Result<()> {
    if affected.is_empty() {
        return Ok(());
    }
    let revision: Option<i64> = optional(
        conn,
        "UPDATE archive_state SET revision=revision+1 WHERE singleton=1 AND maintenance=0 RETURNING revision",
        [],
    )?;
    let revision =
        revision.ok_or_else(|| AppError::Conflict("Archive is in maintenance.".into()))?;
    for id in affected {
        execute(
            conn,
            "UPDATE listens SET revision=? WHERE id=?",
            params![revision, id],
        )?;
        execute(
            conn,
            "INSERT INTO changes(revision,listen_id,operation) SELECT ?,id,CASE WHEN trashed_at_ms IS NULL THEN 'upsert' ELSE 'remove' END FROM listens WHERE id=?",
            params![revision, id],
        )?;
    }
    let through = revision.saturating_sub(10000);
    execute(
        conn,
        "UPDATE archive_state SET retained_from=MAX(retained_from,?) WHERE singleton=1",
        params![through],
    )?;
    execute(
        conn,
        "DELETE FROM changes WHERE revision<=?",
        params![through],
    )?;
    Ok(())
}
struct Variant {
    time: i64,
    duration: Option<i64>,
    kind: String,
    counted: bool,
}
fn canonicalize(conn: &Connection, id: &str) -> Result<()> {
    let variants = rows(
        conn,
        "SELECT v.played_at_ms,v.listened_ms,v.duration_kind,v.counted FROM observation_variants v JOIN observations o ON o.id=v.observation_id WHERE o.listen_id=? ORDER BY CASE o.source WHEN 'spotify_api' THEN 0 WHEN 'spotify_import' THEN 1 ELSE 2 END,v.played_at_ms,v.fingerprint",
        [id],
        |r| {
            Ok(Variant {
                time: r.get(0)?,
                duration: r.get(1)?,
                kind: r.get(2)?,
                counted: r.get(3)?,
            })
        },
    )?;
    let time = variants[0].time;
    let actual: BTreeSet<i64> = variants
        .iter()
        .filter(|v| v.kind == "actual")
        .filter_map(|v| v.duration)
        .collect();
    let conflict = actual.len() > 1;
    let (duration, kind) = if conflict {
        (None, "unknown")
    } else if let Some(ms) = actual.first() {
        (Some(*ms), "actual")
    } else {
        (variants.iter().find_map(|v| v.duration), "estimated")
    };
    let kind = if duration.is_none() { "unknown" } else { kind };
    let counted = if actual.is_empty() {
        variants.iter().all(|v| v.counted)
    } else {
        variants
            .iter()
            .filter(|v| v.kind == "actual")
            .all(|v| v.counted)
    };
    let calendar_id: String = one(conn, "SELECT calendar_id FROM listens WHERE id=?", [id])?;
    let (date, week) = calendar(conn, Some(&calendar_id))?.dates(time)?;
    execute(
        conn,
        "UPDATE listens SET played_at_ms=?,listened_ms=?,duration_kind=?,counted=?,local_date=?,week_date=?,needs_review=CASE WHEN ? THEN 1 ELSE needs_review END WHERE id=?",
        params![time, duration, kind, counted, date, week, conflict, id],
    )?;
    if conflict {
        execute(
            conn,
            "INSERT INTO conflicts(id,listen_id,kind,evidence_json) VALUES(?,?,'actual_duration',?) ON CONFLICT(listen_id,kind) DO UPDATE SET evidence_json=excluded.evidence_json,resolved_at_ms=NULL",
            params![
                archive_id(),
                id,
                serde_json::json!({"actual_ms":actual,"policy_version":1}).to_string()
            ],
        )?;
    } else {
        execute(
            conn,
            "UPDATE conflicts SET resolved_at_ms=? WHERE listen_id=? AND kind='actual_duration' AND resolved_at_ms IS NULL",
            params![crate::auth::now_ms(), id],
        )?;
    }
    Ok(())
}
fn split_pair(conn: &Connection, id: &str, affected: &mut BTreeSet<String>) -> Result<u64> {
    let detached = ids(
        conn,
        "SELECT o.id FROM observations o JOIN listens l ON l.id=o.listen_id WHERE l.id=? AND o.source!=l.anchor_source",
        [id],
    )?;
    let count = detached.len() as u64;
    for observation in detached {
        let new = archive_id();
        execute(
            conn,
            "INSERT INTO listens(id,track_id,played_at_ms,first_acquired_at_ms,calendar_id,local_date,week_date,policy_version,anchor_source,listened_ms,duration_kind,counted,needs_review,trashed_at_ms) SELECT ?,track_id,played_at_ms,first_acquired_at_ms,calendar_id,local_date,week_date,policy_version,(SELECT source FROM observations WHERE id=?),listened_ms,duration_kind,counted,1,trashed_at_ms FROM listens WHERE id=?",
            params![new, observation, id],
        )?;
        execute(
            conn,
            "UPDATE observations SET listen_id=? WHERE id=?",
            params![new, observation],
        )?;
        canonicalize(conn, &new)?;
        execute(conn,"INSERT INTO identity_decisions VALUES(?,'reconciliation_split',?,?,?,?)",params![archive_id(),id,new,serde_json::json!({"observation_id":observation,"reason":"competing_source_event","policy_version":1,"calendar":"inherited"}).to_string(),crate::auth::now_ms()])?;
        affected.insert(new);
    }
    execute(conn, "UPDATE listens SET needs_review=1 WHERE id=?", [id])?;
    canonicalize(conn, id)?;
    affected.insert(id.to_owned());
    Ok(count)
}
pub(crate) fn ingest(
    conn: &Connection,
    events: &[Event],
    acquired_at_ms: i64,
) -> Result<IngestReport> {
    let reservation: Option<i64> = optional(
        conn,
        "UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision",
        [],
    )?;
    if reservation.is_none() {
        return Err(AppError::Conflict("Archive is in maintenance.".into()));
    }
    let mut report = IngestReport::default();
    let mut incoming: BTreeMap<(&str, &str), BTreeSet<(i64, &str)>> = BTreeMap::new();
    for event in events {
        incoming
            .entry((event.source, &event.track.id))
            .or_default()
            .insert((event.played_at_ms, &event.source_key));
    }
    let mut affected = BTreeSet::new();
    let current_calendar = calendar(conn, None)?;
    let mut has_api: bool = one(
        conn,
        "SELECT EXISTS(SELECT 1 FROM observations WHERE source='spotify_api')",
        [],
    )?;
    let mut has_import: bool = one(
        conn,
        "SELECT EXISTS(SELECT 1 FROM observations WHERE source='spotify_import')",
        [],
    )?;
    for event in events {
        let fingerprint = db::payload_fingerprint(&event.raw);
        let context = crate::listening_patterns::context(event.source, &event.raw);
        let existing=rows(conn,"SELECT o.id,o.listen_id,EXISTS(SELECT 1 FROM observation_variants v WHERE v.observation_id=o.id AND fingerprint=?) AS same FROM observations o WHERE source=? AND source_key=?",params![fingerprint,event.source,event.source_key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?)))?.pop();
        if existing.as_ref().is_some_and(|r| r.2) {
            let (observation, listen, _) = existing.as_ref().ok_or(AppError::Internal)?;
            if execute(
                conn,
                "UPDATE observation_variants SET context_json=? WHERE observation_id=? AND fingerprint=? AND context_json!=?",
                params![context, observation, fingerprint, context],
            )? > 0
            {
                affected.insert(listen.clone());
            }
            report.duplicates += 1;
            continue;
        }
        let (track_id, metadata_changed) = upsert_track(conn, event)?;
        let kind = event.duration_kind.as_str();
        if (event.duration_kind == DurationKind::Unknown) != event.listened_ms.is_none()
            || event.listened_ms.is_some_and(|ms| ms < 0)
        {
            return Err(AppError::BadRequest(
                "Duration category and value disagree.".into(),
            ));
        }
        let payload_id = payload(conn, &event.raw)?;
        let mut new_listen = false;
        let (observation_id, listen_id) = if let Some((observation, listen, _)) = existing {
            report.variants += 1;
            (observation, listen)
        } else {
            report.originals += 1;
            let counterpart = match event.source {
                "spotify_import" if has_api => Some("spotify_api"),
                "spotify_api" if has_import => Some("spotify_import"),
                _ => None,
            };
            if counterpart.is_some() && event.track.id.starts_with("spotify:track:") {
                let paired = ids(
                    conn,
                    "SELECT l.id FROM listens l WHERE track_id=? AND trashed_at_ms IS NULL AND EXISTS(SELECT 1 FROM observations o WHERE o.listen_id=l.id AND source=?) AND EXISTS(SELECT 1 FROM observations o JOIN observation_variants v ON v.observation_id=o.id WHERE o.listen_id=l.id AND o.source=? AND v.played_at_ms BETWEEN ? AND ?)",
                    params![
                        track_id,
                        event.source,
                        counterpart,
                        event.played_at_ms.saturating_sub(2000),
                        event.played_at_ms.saturating_add(2000)
                    ],
                )?;
                for id in paired {
                    report.added += split_pair(conn, &id, &mut affected)?;
                }
            }
            let candidates = if event.track.id.starts_with("spotify:track:")
                && counterpart.is_some()
            {
                rows(
                    conn,
                    "SELECT l.id,l.played_at_ms FROM listens l WHERE l.track_id=? AND l.trashed_at_ms IS NULL AND l.played_at_ms BETWEEN ? AND ? AND EXISTS(SELECT 1 FROM observations o WHERE o.listen_id=l.id AND o.source=?) AND NOT EXISTS(SELECT 1 FROM observations o WHERE o.listen_id=l.id AND o.source=?) LIMIT 2",
                    params![
                        track_id,
                        event.played_at_ms.saturating_sub(2000),
                        event.played_at_ms.saturating_add(2000),
                        counterpart,
                        event.source
                    ],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                )?
            } else {
                vec![]
            };
            let mut reverse_ambiguous = candidates.first().is_some_and(|(_, time)| {
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
            if let Some((_, time)) = candidates.first() {
                reverse_ambiguous |= one::<bool>(
                    conn,
                    "SELECT EXISTS(SELECT 1 FROM listens l JOIN observations o ON o.listen_id=l.id WHERE l.track_id=? AND l.trashed_at_ms IS NULL AND o.source=? AND o.source_key!=? AND l.played_at_ms BETWEEN ? AND ?)",
                    params![
                        track_id,
                        event.source,
                        event.source_key,
                        time.saturating_sub(2000),
                        time.saturating_add(2000)
                    ],
                )?;
            }
            let identity_base = event
                .source_key
                .rsplit_once(':')
                .filter(|(_, slot)| slot.parse::<u64>().is_ok())
                .map(|(key, _)| key)
                .unwrap_or(&event.source_key);
            let repeated_identity = format!("{identity_base}:%");
            let uncertain = if event.source == "listenbrainz"
                || event.source.starts_with("spotify_")
            {
                ids(
                    conn,
                    "SELECT DISTINCT l.id FROM listens l JOIN tracks t ON t.id=l.track_id WHERE l.played_at_ms>=? AND l.played_at_ms<? AND l.trashed_at_ms IS NULL AND EXISTS(SELECT 1 FROM observations o WHERE o.listen_id=l.id AND (o.source='listenbrainz' OR ?='listenbrainz')) AND NOT EXISTS(SELECT 1 FROM observations o WHERE o.listen_id=l.id AND o.source='listenbrainz' AND o.source_key LIKE ?) AND (l.track_id=? OR (t.title=? AND EXISTS(SELECT 1 FROM track_artists ta JOIN artists a ON a.id=ta.artist_id WHERE ta.track_id=t.id AND a.name=?)))",
                    params![
                        event.played_at_ms.div_euclid(1000) * 1000,
                        event.played_at_ms.div_euclid(1000) * 1000 + 1000,
                        event.source,
                        repeated_identity,
                        track_id,
                        event.track.title,
                        event
                            .track
                            .artists
                            .first()
                            .map(|a| a.name.as_str())
                            .unwrap_or("")
                    ],
                )?
            } else {
                vec![]
            };
            for id in &uncertain {
                execute(conn, "UPDATE listens SET needs_review=1 WHERE id=?", [id])?;
            }
            affected.extend(uncertain.iter().cloned());
            let pairing_ambiguous = candidates.len() > 1 || reverse_ambiguous;
            let ambiguous = pairing_ambiguous || !uncertain.is_empty();
            let listen_id = if candidates.len() == 1 && !pairing_ambiguous {
                report.reconciled += 1;
                candidates[0].0.clone()
            } else {
                if ambiguous {
                    report.ambiguous += 1;
                    affected.extend(ids(conn,"UPDATE listens SET needs_review=1 WHERE track_id=? AND played_at_ms BETWEEN ? AND ? RETURNING id",params![track_id,event.played_at_ms.saturating_sub(2000),event.played_at_ms.saturating_add(2000)])?);
                }
                report.added += 1;
                let id = archive_id();
                new_listen = true;
                let calendar = &current_calendar;
                let (date, week) = calendar.dates(event.played_at_ms)?;
                execute(
                    conn,
                    "INSERT INTO listens(id,track_id,played_at_ms,first_acquired_at_ms,calendar_id,local_date,week_date,listened_ms,duration_kind,counted,needs_review,anchor_source) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                    params![
                        id,
                        track_id,
                        event.played_at_ms,
                        acquired_at_ms,
                        calendar.id,
                        date,
                        week,
                        event.listened_ms,
                        kind,
                        event.counted,
                        ambiguous,
                        event.source
                    ],
                )?;
                id
            };
            let observation = archive_id();
            execute(
                conn,
                "INSERT INTO observations(id,source,source_key,listen_id,source_timestamp_ms,payload_id) VALUES(?,?,?,?,?,?)",
                params![
                    observation,
                    event.source,
                    event.source_key,
                    listen_id,
                    event.source_timestamp_ms,
                    payload_id
                ],
            )?;
            (observation, listen_id)
        };
        execute(
            conn,
            "INSERT INTO observation_variants(id,observation_id,fingerprint,payload_id,played_at_ms,listened_ms,duration_kind,counted,policy_version,context_json) VALUES(?,?,?,?,?,?,?,?,1,?)",
            params![
                archive_id(),
                observation_id,
                fingerprint,
                payload_id,
                event.played_at_ms,
                event.listened_ms,
                kind,
                event.counted,
                context
            ],
        )?;
        // A fresh event's only variant is exactly the state just inserted. Re-run
        // canonicalization only when reconciling or adding a variant to an event.
        if !new_listen {
            canonicalize(conn, &listen_id)?;
        }
        has_api |= event.source == "spotify_api";
        has_import |= event.source == "spotify_import";
        affected.insert(listen_id);
        if metadata_changed {
            affected.extend(ids(conn,"SELECT l.id FROM listens l WHERE track_id=? UNION SELECT l.id FROM listens l JOIN tracks t ON t.id=l.track_id WHERE t.album_id=(SELECT album_id FROM tracks WHERE id=?) UNION SELECT l.id FROM listens l WHERE l.track_id IN (SELECT a.track_id FROM track_artists a JOIN track_artists b ON a.artist_id=b.artist_id WHERE b.track_id=?)",params![track_id,track_id,track_id])?);
        }
    }
    publish(conn, &affected)?;
    Ok(report)
}
pub(crate) fn review_spotify_imports(conn: &Connection, events: &[Event]) -> Result<()> {
    let mut changed = BTreeSet::new();
    for event in events.iter().filter(|e| e.source == "spotify_import") {
        let existing=rows(conn,"SELECT o.listen_id,l.track_id FROM observations o JOIN listens l ON l.id=o.listen_id WHERE o.source='spotify_import' AND o.source_key=?",[&event.source_key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.pop();
        let Some((listen, track)) = existing else {
            continue;
        };
        let uncertain = !event.track.id.starts_with("spotify:track:");
        let peers = if uncertain {
            vec![]
        } else {
            ids(
                conn,
                "SELECT o.listen_id FROM observations o JOIN listens l ON l.id=o.listen_id WHERE o.source='spotify_import' AND l.track_id=? AND o.source_timestamp_ms=? AND o.source_key!=?",
                params![track, event.source_timestamp_ms, event.source_key],
            )?
        };
        if !uncertain && peers.is_empty() {
            continue;
        }
        for id in std::iter::once(listen).chain(peers) {
            let kind = if uncertain {
                "spotify_identity_uncertain"
            } else {
                "spotify_export_duration"
            };
            let reviewed: bool = one(
                conn,
                "SELECT EXISTS(SELECT 1 FROM conflicts WHERE listen_id=? AND kind=? AND resolved_at_ms IS NOT NULL)",
                params![id, kind],
            )?;
            if reviewed {
                continue;
            }
            let flagged = execute(
                conn,
                "UPDATE listens SET needs_review=1 WHERE id=? AND needs_review=0",
                [&id],
            )?;
            let inserted=execute(conn,"INSERT INTO conflicts(id,listen_id,kind,evidence_json) VALUES(?,?,?,?) ON CONFLICT(listen_id,kind) DO NOTHING",params![archive_id(),id,if uncertain{"spotify_identity_uncertain"}else{"spotify_export_duration"},serde_json::json!({"source":"spotify_import","timestamp_ms":event.source_timestamp_ms}).to_string()])?;
            if flagged > 0 || inserted > 0 {
                changed.insert(id);
            }
        }
    }
    publish(conn, &changed)
}

pub(crate) fn import_rows(
    conn: &Connection,
    events: &[Event],
    job_id: &str,
    file_id: Option<&str>,
    member: &str,
    first_row: i64,
    source_rows: &[crate::jobs::SourceRow],
) -> Result<IngestReport> {
    use crate::jobs::SourceRow;
    let mut report = ingest(conn, events, crate::auth::now_ms())?;
    review_spotify_imports(conn, events)?;
    let mut accepted = events.iter();
    for (index, row) in source_rows.iter().enumerate() {
        let (payload_id, variant, outcome, reason) = match row {
            SourceRow::Event(_) => {
                let event = accepted.next().ok_or(AppError::Internal)?;
                let (variant, payload_id): (String, i64) = conn.prepare_cached(
                    "SELECT v.id,v.payload_id FROM observation_variants v JOIN observations o ON o.id=v.observation_id WHERE o.source=? AND o.source_key=? AND v.fingerprint=?",
                )?.query_row(params![
                        event.source,
                        event.source_key,
                        db::payload_fingerprint(&event.raw)
                    ], |r| Ok((r.get(0)?, r.get(1)?)))?;
                (
                    payload_id,
                    Some(variant),
                    "accepted",
                    if event.source == "listenbrainz" {
                        crate::listenbrainz::duration_warning(&event.raw)
                    } else {
                        None
                    },
                )
            }
            SourceRow::Invalid { raw, reason } => {
                report.invalid += 1;
                (payload(conn, raw)?, None, "invalid", Some(reason.clone()))
            }
            SourceRow::Skipped { raw, reason } => {
                report.skipped += 1;
                (payload(conn, raw)?, None, "skipped", Some(reason.clone()))
            }
        };
        execute(
            conn,
            "INSERT INTO source_occurrences(job_id,file_id,member,row_number,variant_id,payload_id,outcome,reason) VALUES(?,?,?,?,?,?,?,?)",
            params![
                job_id,
                file_id,
                member,
                first_row + index as i64,
                variant,
                payload_id,
                outcome,
                reason
            ],
        )?;
    }
    Ok(report)
}

fn payload(conn: &Connection, raw: &serde_json::Value) -> Result<i64> {
    one(
        conn,
        "INSERT INTO original_payloads(raw_json,fingerprint) VALUES('null',?) RETURNING id",
        [db::payload_fingerprint(raw)],
    )
}
