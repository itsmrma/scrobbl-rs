//! Batch review on request: preview, revalidate, apply atomically, and undo.
//!
//! Two kinds of evidence are decided automatically; everything else stays for
//! manual comparison.
//!
//! - **One Spotify export playback logged more than once.** Spotify exports
//!   sometimes write several rows for the same track ending in the same second
//!   with different playback times (for example 55 ms and 780 ms, or a phone
//!   and the Connect speaker it controlled). Those plays overlap, so they are
//!   one listen: the row with the longest actual playback is kept and the
//!   others move to trash. Rows recorded offline are different: a whole
//!   offline session carries the sync time, so rows sharing it are separate
//!   plays and are all kept. A group mixing online and offline rows, or rows
//!   whose online/offline state is unknown, stays manual.
//! - **One listen reported by two sources.** An isolated pair starting less
//!   than one second apart, describing the same Spotify track or with exactly
//!   matching title, album and artist credits, keeps actual playback over an
//!   estimate.
use super::*;
use std::collections::BTreeMap;

struct Candidate {
    id: String,
    revision: i64,
    track_id: String,
    at: i64,
    acquired: i64,
    flagged: bool,
    title: String,
    album: String,
    artists: Vec<String>,
    spotify: Vec<String>,
    sources: BTreeSet<String>,
    kind: String,
    duration: Option<i64>,
    counted: bool,
    /// Kinds of the unresolved conflicts on this listen.
    conflicts: BTreeSet<String>,
    /// End time of the single Spotify export row behind this listen.
    export_end: Option<i64>,
    /// Online/offline state reported by every export variant, when known.
    offline: Option<bool>,
}

fn same_track(a: &Candidate, b: &Candidate) -> bool {
    if a.spotify.len() > 1 || b.spotify.len() > 1 {
        return false;
    }
    if !a.spotify.is_empty() && !b.spotify.is_empty() {
        return a.spotify.len() == 1 && a.spotify == b.spotify;
    }
    // No fuzzy matching or personal name overlays: incomplete metadata stays manual.
    !a.title.trim().is_empty()
        && !a.album.trim().is_empty()
        && !a.artists.is_empty()
        && a.title == b.title
        && a.album == b.album
        && a.artists == b.artists
}

fn compatible(a: &Candidate, b: &Candidate) -> bool {
    a.flagged
        && b.flagged
        && same_track(a, b)
        && a.conflicts.is_empty()
        && b.conflicts.is_empty()
        && a.counted == b.counted
        && !a.sources.is_empty()
        && !b.sources.is_empty()
        && a.sources.union(&b.sources).all(|source| {
            matches!(
                source.as_str(),
                "spotify_api" | "spotify_import" | "listenbrainz"
            )
        })
        && !(a.kind == "actual" && b.kind == "actual" && a.duration != b.duration)
}

fn quality(candidate: &Candidate) -> (u8, bool, std::cmp::Reverse<i64>, &str) {
    (
        match candidate.kind.as_str() {
            "actual" => 2,
            "estimated" => 1,
            _ => 0,
        },
        candidate.sources.contains("spotify_api"),
        std::cmp::Reverse(candidate.acquired),
        &candidate.id,
    )
}

/// The export row that best describes one overlapping playback: the longest
/// actual playback, then the earliest archived, then a stable id.
fn export_quality(candidate: &Candidate) -> (i64, std::cmp::Reverse<i64>, std::cmp::Reverse<&str>) {
    (
        if candidate.kind == "actual" {
            candidate.duration.unwrap_or(-1)
        } else {
            -1
        },
        std::cmp::Reverse(candidate.acquired),
        std::cmp::Reverse(&candidate.id),
    )
}

/// Only listens whose single reason for review is the export's own
/// same-second rows qualify for the export rule.
fn export_member(candidate: &Candidate) -> Option<(&str, i64)> {
    let only_export_conflict =
        candidate.conflicts.len() == 1 && candidate.conflicts.contains("spotify_export_duration");
    let only_import = candidate.sources.len() == 1 && candidate.sources.contains("spotify_import");
    (candidate.flagged && only_export_conflict && only_import)
        .then_some(())
        .and(candidate.export_end)
        .map(|end| (candidate.track_id.as_str(), end))
}

fn offline_state(contexts: &[Value]) -> Option<bool> {
    let states: BTreeSet<bool> = contexts
        .iter()
        .map(|context| context["offline"].as_bool())
        .collect::<Option<_>>()?;
    match states.len() {
        1 => states.into_iter().next(),
        _ => None,
    }
}

struct Plan {
    groups: Vec<Decision>,
    token: String,
    flagged: i64,
}

impl Plan {
    fn trashed(&self) -> usize {
        self.groups
            .iter()
            .filter(|group| group.keep_id.is_some())
            .map(|group| group.ids.len() - 1)
            .sum()
    }

    fn records(&self) -> usize {
        self.groups.iter().map(|group| group.ids.len()).sum()
    }
}

async fn plan(conn: &mut sqlx::SqliteConnection) -> Result<Plan> {
    let flagged: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM listens WHERE needs_review=1 AND trashed_at_ms IS NULL",
    )
    .fetch_one(&mut *conn)
    .await?;
    if flagged > 10_000 {
        return Err(AppError::Conflict("Review some records first: automatic review supports up to 10,000 flagged records at a time.".into()));
    }
    // Include unflagged neighbours, too: they may make a pair ambiguous. The
    // date index bounds each join; do not read the entire listening archive.
    let rows = sqlx::query(r#"
      WITH flagged AS (SELECT played_at_ms FROM listens WHERE needs_review=1 AND trashed_at_ms IS NULL)
      SELECT DISTINCT l.*, t.title, COALESCE(al.name,'') AS album,
        (SELECT json_group_array(name) FROM (SELECT ar.name FROM track_artists ta JOIN artists ar ON ar.id=ta.artist_id WHERE ta.track_id=t.id ORDER BY ta.ordinal)) AS artists,
        (SELECT json_group_array(external_id) FROM (SELECT external_id FROM entity_links INDEXED BY entity_links_entity WHERE kind='track' AND namespace='spotify' AND provisional=0 AND entity_id=t.id ORDER BY external_id)) AS spotify,
        (SELECT json_group_array(source) FROM observations WHERE listen_id=l.id) AS sources,
        (SELECT json_group_array(kind) FROM conflicts WHERE listen_id=l.id AND resolved_at_ms IS NULL) AS conflict_kinds,
        (SELECT CASE WHEN COUNT(*)=1 THEN MIN(source_timestamp_ms) END FROM observations WHERE listen_id=l.id AND source='spotify_import') AS export_end,
        (SELECT json_group_array(json(v.context_json)) FROM observations o JOIN observation_variants v ON v.observation_id=o.id WHERE o.listen_id=l.id AND o.source='spotify_import') AS contexts
      FROM flagged f JOIN listens l ON l.played_at_ms BETWEEN f.played_at_ms-999 AND f.played_at_ms+999
      JOIN tracks t ON t.id=l.track_id LEFT JOIN albums al ON al.id=t.album_id
      WHERE l.trashed_at_ms IS NULL ORDER BY l.played_at_ms,l.id LIMIT 20001
    "#).fetch_all(&mut *conn).await?;
    if rows.len() > 20_000 {
        return Err(AppError::Conflict("Too many nearby records for one automatic review. Review some comparisons manually first.".into()));
    }
    let json = |text: &str| -> Result<Value> {
        serde_json::from_str(text).map_err(|_| AppError::Internal)
    };
    let mut candidates = Vec::new();
    for row in rows {
        let contexts = json(row.get("contexts"))?;
        candidates.push(Candidate {
            id: row.get("id"),
            revision: row.get("revision"),
            track_id: row.get("track_id"),
            at: row.get("played_at_ms"),
            acquired: row.get("first_acquired_at_ms"),
            flagged: row.get("needs_review"),
            title: row.get("title"),
            album: row.get("album"),
            artists: serde_json::from_str(row.get("artists")).map_err(|_| AppError::Internal)?,
            spotify: serde_json::from_str(row.get("spotify")).map_err(|_| AppError::Internal)?,
            sources: serde_json::from_str(row.get("sources")).map_err(|_| AppError::Internal)?,
            kind: row.get("duration_kind"),
            duration: row.get("listened_ms"),
            counted: row.get("counted"),
            conflicts: serde_json::from_str(row.get("conflict_kinds"))
                .map_err(|_| AppError::Internal)?,
            export_end: row.get("export_end"),
            offline: contexts
                .as_array()
                .filter(|contexts| !contexts.is_empty())
                .and_then(|contexts| offline_state(contexts)),
        });
    }
    let mut groups = Vec::new();
    // Export rule. Clusters are keyed by track and export end time; only the
    // flagged members that the export itself put in review take part.
    let mut clusters: BTreeMap<(&str, i64), Vec<&Candidate>> = BTreeMap::new();
    for candidate in &candidates {
        if let Some(key) = export_member(candidate) {
            clusters.entry(key).or_default().push(candidate);
        }
    }
    let mut decided = BTreeSet::new();
    for members in clusters.values_mut() {
        // The neighbour join can return a listen once per nearby flagged row.
        members.sort_by(|a, b| a.id.cmp(&b.id));
        members.dedup_by(|a, b| a.id == b.id);
        if members.len() < 2 {
            continue;
        }
        let states: BTreeSet<Option<bool>> = members.iter().map(|m| m.offline).collect();
        let keep_id = match states.into_iter().collect::<Vec<_>>().as_slice() {
            // Overlapping online rows: one playback.
            [Some(false)] => Some(
                members
                    .iter()
                    .max_by(|a, b| export_quality(a).cmp(&export_quality(b)))
                    .ok_or(AppError::Internal)?
                    .id
                    .clone(),
            ),
            // One offline session synced at once: separate plays.
            [Some(true)] => None,
            _ => continue,
        };
        decided.extend(members.iter().map(|m| m.id.as_str()));
        groups.push(Decision {
            ids: members.iter().map(|m| m.id.clone()).collect(),
            revisions: members.iter().map(|m| m.revision).collect(),
            keep_id,
        });
    }
    // Cross-source rule over the remaining records, in time order.
    let mut start = 0;
    while start < candidates.len() {
        let mut end = start + 1;
        while end < candidates.len()
            && candidates[end].at.saturating_sub(candidates[end - 1].at) < 1000
        {
            end += 1;
        }
        // Repeats, chains and competing records remain for manual comparison.
        if end - start == 2 {
            let a = &candidates[start];
            let b = &candidates[start + 1];
            if a.id != b.id
                && !decided.contains(a.id.as_str())
                && !decided.contains(b.id.as_str())
                && compatible(a, b)
            {
                let keep = if quality(a) >= quality(b) { a } else { b };
                groups.push(Decision {
                    ids: vec![a.id.clone(), b.id.clone()],
                    revisions: vec![a.revision, b.revision],
                    keep_id: Some(keep.id.clone()),
                });
            }
        }
        start = end;
    }
    let mut signature = serde_json::to_string(&groups).map_err(|_| AppError::Internal)?;
    let matched_ids: Vec<&str> = groups
        .iter()
        .flat_map(|g| g.ids.iter().map(String::as_str))
        .collect();
    let mut evidence_map = std::collections::HashMap::with_capacity(matched_ids.len());
    for chunk in matched_ids.chunks(500) {
        let mut builder = sqlx::QueryBuilder::new(
            "SELECT listen_id, group_concat(id || ':' || fingerprint, '|') FROM (SELECT o.listen_id, v.id, v.fingerprint FROM observation_variants v JOIN observations o ON o.id=v.observation_id WHERE o.listen_id IN (",
        );
        let mut separated = builder.separated(",");
        for id in chunk {
            separated.push_bind(*id);
        }
        separated.push_unseparated(") ORDER BY o.listen_id, v.id) GROUP BY listen_id");
        let rows: Vec<(String, Option<String>)> =
            builder.build_query_as().fetch_all(&mut *conn).await?;
        for (listen_id, concat) in rows {
            if let Some(concat) = concat {
                evidence_map.insert(listen_id, concat);
            }
        }
    }
    for id in matched_ids {
        signature.push_str(&auth::digest(
            evidence_map.get(id).map(String::as_str).unwrap_or(""),
        ));
    }
    Ok(Plan {
        groups,
        token: auth::digest(&signature),
        flagged,
    })
}

pub async fn preview(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    let plan = plan(&mut view.transaction).await?;
    let records = plan.records();
    let trashed = plan.trashed();
    Ok(Json(json!({
        "token": plan.token,
        "groups": plan.groups.len(),
        "records": records,
        "trash": trashed,
        "separate": plan.groups.iter().filter(|g| g.keep_id.is_none()).count(),
        "remaining": plan.flagged - records as i64,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    token: String,
}

pub async fn execute(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    crate::library::writable(&state).await?;
    let mut tx = state.db.begin().await?;
    // Reserve the writer before planning; collection cannot alter this preview
    // between validation and the updates, or enter maintenance concurrently.
    let reserved = sqlx::query(
        "UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if reserved != 1 {
        return Err(AppError::Conflict("Archive is in maintenance.".into()));
    }
    let plan = plan(&mut tx).await?;
    if request.token != plan.token {
        return Err(AppError::Conflict(
            "The matching records changed. Refresh the automatic review preview before continuing."
                .into(),
        ));
    }
    if plan.groups.is_empty() {
        return Err(AppError::Conflict(
            "No records meet the automatic review criteria.".into(),
        ));
    }
    let now = auth::now_ms();
    let mut before = Vec::new();
    let mut ids = BTreeSet::new();
    // Undo restores trash state from this list: a record is trashed by the
    // batch exactly when it is not listed here.
    let mut keepers = Vec::new();
    for group in &plan.groups {
        before.extend(apply_decision(&mut tx, group, now).await?);
        ids.extend(group.ids.iter().cloned());
        match &group.keep_id {
            Some(keep) => keepers.push(keep.clone()),
            None => keepers.extend(group.ids.iter().cloned()),
        }
    }
    let removed = plan.trashed();
    db::publish(&mut tx, &ids).await?;
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM archive_state WHERE singleton=1")
        .fetch_one(&mut *tx)
        .await?;
    let id = db::new_id();
    let evidence = json!({"before":before,"revision":revision,"removed":removed,"keep_ids":keepers,"automatic":true,"at_ms":now});
    sqlx::query("INSERT INTO identity_decisions VALUES(?,'listen_review',?,?,?,?)")
        .bind(&id)
        .bind(ids.first().ok_or(AppError::Internal)?)
        .bind(keepers.first().ok_or(AppError::Internal)?)
        .bind(evidence.to_string())
        .bind(now)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"decision_id":id,"removed":removed,"groups":plan.groups.len()}),
    ))
}
