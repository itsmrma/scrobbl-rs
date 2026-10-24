//! Conservative batch review: preview, revalidate, apply atomically, and undo.
use super::*;

struct Candidate {
    id: String,
    revision: i64,
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
    conflicts: bool,
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
        && !a.conflicts
        && !b.conflicts
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

struct Plan {
    groups: Vec<Decision>,
    token: String,
    flagged: i64,
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
        EXISTS(SELECT 1 FROM conflicts WHERE listen_id=l.id AND resolved_at_ms IS NULL) AS conflicts
      FROM flagged f JOIN listens l ON l.played_at_ms BETWEEN f.played_at_ms-999 AND f.played_at_ms+999
      JOIN tracks t ON t.id=l.track_id LEFT JOIN albums al ON al.id=t.album_id
      WHERE l.trashed_at_ms IS NULL ORDER BY l.played_at_ms,l.id LIMIT 20001
    "#).fetch_all(&mut *conn).await?;
    if rows.len() > 20_000 {
        return Err(AppError::Conflict("Too many nearby records for one automatic review. Review some comparisons manually first.".into()));
    }
    let mut candidates = Vec::new();
    for row in rows {
        candidates.push(Candidate {
            id: row.get("id"),
            revision: row.get("revision"),
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
            conflicts: row.get("conflicts"),
        });
    }
    let mut groups = Vec::new();
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
            if compatible(a, b) {
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
    for group in &groups {
        for id in &group.ids {
            signature.push_str(&evidence_fingerprint(conn, id).await?);
        }
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
    Ok(Json(
        json!({"token":plan.token,"groups":plan.groups.len(),"records":plan.groups.len()*2,"remaining":plan.flagged-plan.groups.len() as i64*2}),
    ))
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
    let mut keepers = Vec::new();
    for group in &plan.groups {
        before.extend(apply_decision(&mut tx, group, now).await?);
        ids.extend(group.ids.iter().cloned());
        keepers.push(group.keep_id.as_ref().ok_or(AppError::Internal)?.clone());
    }
    db::publish(&mut tx, &ids).await?;
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM archive_state WHERE singleton=1")
        .fetch_one(&mut *tx)
        .await?;
    let id = db::new_id();
    let evidence = json!({"before":before,"revision":revision,"removed":plan.groups.len(),"keep_ids":keepers,"automatic":true,"at_ms":now});
    sqlx::query("INSERT INTO identity_decisions VALUES(?,'listen_review',?,?,?,?)")
        .bind(&id)
        .bind(ids.first().ok_or(AppError::Internal)?)
        .bind(&keepers[0])
        .bind(evidence.to_string())
        .bind(now)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"decision_id":id,"removed":plan.groups.len(),"groups":plan.groups.len()}),
    ))
}
