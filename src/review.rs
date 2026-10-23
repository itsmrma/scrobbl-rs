//! Explicit, reversible review of listens. Original observations are never deleted.
use crate::{
    AppState, auth, db,
    error::{AppError, Result},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::BTreeSet;

#[derive(Default, Deserialize)]
pub struct Page {
    #[serde(default)]
    pub page: u32,
    #[serde(default)]
    pub decisions_page: u32,
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<Value>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    let conn = &mut *view.transaction;
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM listens WHERE needs_review=1 AND trashed_at_ms IS NULL",
    )
    .fetch_one(&mut *conn)
    .await?;
    let seeds: Vec<String> = sqlx::query_scalar("SELECT id FROM listens WHERE needs_review=1 AND trashed_at_ms IS NULL ORDER BY played_at_ms DESC,id DESC LIMIT 50 OFFSET ?").bind(i64::from(page.page)*50).fetch_all(&mut *conn).await?;
    let mut seen = BTreeSet::new();
    let mut groups = Vec::new();
    for seed in seeds {
        if seen.contains(&seed) {
            continue;
        }
        // Suggestions are evidence for comparison, never an automatic deletion.
        let ids: Vec<String> = sqlx::query_scalar("SELECT l.id FROM listens l JOIN tracks t ON t.id=l.track_id JOIN listens anchor ON anchor.id=? JOIN tracks at ON at.id=anchor.track_id WHERE l.needs_review=1 AND l.trashed_at_ms IS NULL AND l.played_at_ms BETWEEN anchor.played_at_ms-2000 AND anchor.played_at_ms+2000 AND (l.track_id=anchor.track_id OR (t.title=at.title AND EXISTS(SELECT 1 FROM track_artists x JOIN artists a ON a.id=x.artist_id JOIN track_artists y JOIN artists b ON b.id=y.artist_id WHERE x.track_id=t.id AND y.track_id=at.id AND a.name=b.name))) ORDER BY l.played_at_ms DESC,l.id DESC LIMIT 100").bind(&seed).fetch_all(&mut *conn).await?;
        let mut query = crate::consultation::selection_at(
            conn,
            &crate::web::Filters::default(),
            true,
            auth::now_ms(),
        )
        .await?;
        query
            .push(crate::web::LISTEN_COLUMNS)
            .push(" WHERE s.id IN (");
        let mut separated = query.separated(",");
        for id in &ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(") ORDER BY s.played_at_ms DESC,s.id DESC");
        let rows = query.build().fetch_all(&mut *conn).await?;
        let mut items = Vec::new();
        for row in rows {
            let revision: i64 = row.get("revision");
            let mut item = crate::web::listen_json(row);
            item["revision"] = json!(revision);
            seen.insert(item["id"].as_str().ok_or(AppError::Internal)?.to_owned());
            items.push(item);
        }
        groups.push(json!({"id":seed,"items":items}));
    }
    let decisions_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM identity_decisions d WHERE d.kind='listen_review' AND NOT EXISTS(SELECT 1 FROM identity_decisions u WHERE u.kind='listen_review_undo' AND u.to_id=d.id)").fetch_one(&mut *conn).await?;
    let recent: Vec<Value> = sqlx::query("SELECT d.id,d.created_at_ms,d.evidence_json FROM identity_decisions d WHERE d.kind='listen_review' AND NOT EXISTS(SELECT 1 FROM identity_decisions u WHERE u.kind='listen_review_undo' AND u.to_id=d.id) ORDER BY d.created_at_ms DESC,d.id DESC LIMIT 20 OFFSET ?").bind(i64::from(page.decisions_page)*20).fetch_all(&mut *conn).await?.into_iter().map(|r| {
        let evidence: Value = serde_json::from_str(r.get::<&str,_>("evidence_json")).unwrap_or(Value::Null);
        json!({"id":r.get::<String,_>("id"),"at_ms":r.get::<i64,_>("created_at_ms"),"count":evidence["before"].as_array().map_or(0,Vec::len),"removed":evidence["removed"]})
    }).collect();
    Ok(Json(
        json!({"total":total,"page":page.page,"groups":groups,"recent":recent,"decisions_total":decisions_total}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub ids: Vec<String>,
    pub revisions: Vec<i64>,
    pub keep_id: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Before {
    id: String,
    needs_review: bool,
    trashed_at_ms: Option<i64>,
    conflicts: Vec<String>,
    #[serde(default)]
    evidence_fingerprint: String,
}

async fn evidence_fingerprint(conn: &mut sqlx::SqliteConnection, id: &str) -> Result<String> {
    let evidence: Option<String> = sqlx::query_scalar("SELECT group_concat(id || ':' || fingerprint,'|') FROM (SELECT v.id,v.fingerprint FROM observation_variants v JOIN observations o ON o.id=v.observation_id WHERE o.listen_id=? ORDER BY v.id)").bind(id).fetch_one(conn).await?;
    Ok(auth::digest(evidence.as_deref().unwrap_or("")))
}

pub async fn decide(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<Decision>,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    crate::library::writable(&state).await?;
    let ids: BTreeSet<_> = form.ids.iter().cloned().collect();
    if ids.is_empty()
        || ids.len() > 100
        || ids.len() != form.ids.len()
        || form.revisions.len() != ids.len()
        || form.keep_id.as_ref().is_some_and(|id| !ids.contains(id))
    {
        return Err(AppError::BadRequest(
            "Select up to 100 distinct records and a record to keep, or keep them all separate."
                .into(),
        ));
    }
    let now = auth::now_ms();
    let mut tx = state.db.begin().await?;
    let mut before = Vec::new();
    for (id, revision) in form.ids.iter().zip(&form.revisions) {
        let prior =
            sqlx::query("SELECT needs_review,trashed_at_ms,revision FROM listens WHERE id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(AppError::NotFound)?;
        if !prior.get::<bool, _>("needs_review")
            || prior.get::<Option<i64>, _>("trashed_at_ms").is_some()
            || prior.get::<i64, _>("revision") != *revision
        {
            return Err(AppError::Conflict(
                "These records changed. Refresh the comparison before deciding.".into(),
            ));
        }
        before.push(Before {
            id: id.clone(),
            needs_review: true,
            trashed_at_ms: None,
            conflicts: sqlx::query_scalar(
                "SELECT id FROM conflicts WHERE listen_id=? AND resolved_at_ms IS NULL",
            )
            .bind(id)
            .fetch_all(&mut *tx)
            .await?,
            evidence_fingerprint: evidence_fingerprint(&mut tx, id).await?,
        });
        let trash = form
            .keep_id
            .as_ref()
            .filter(|keep| *keep != id)
            .map(|_| now);
        sqlx::query("UPDATE listens SET needs_review=0,trashed_at_ms=? WHERE id=?")
            .bind(trash)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE conflicts SET resolved_at_ms=? WHERE listen_id=? AND resolved_at_ms IS NULL",
        )
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    db::publish(&mut tx, &ids).await?;
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM archive_state WHERE singleton=1")
        .fetch_one(&mut *tx)
        .await?;
    let id = db::new_id();
    let removed = if form.keep_id.is_some() {
        ids.len() - 1
    } else {
        0
    };
    let evidence = json!({"before":before,"revision":revision,"removed":removed,"keep_id":form.keep_id,"at_ms":now});
    sqlx::query("INSERT INTO identity_decisions VALUES(?,'listen_review',?,?,?,?)")
        .bind(&id)
        .bind(&form.ids[0])
        .bind(form.keep_id.as_deref().unwrap_or(&form.ids[0]))
        .bind(evidence.to_string())
        .bind(now)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"decision_id":id,"removed":removed})))
}

pub async fn undo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    crate::library::writable(&state).await?;
    let mut tx = state.db.begin().await?;
    let evidence: Option<String> = sqlx::query_scalar("SELECT json_set(evidence_json,'$.at_ms',COALESCE(json_extract(evidence_json,'$.at_ms'),created_at_ms)) FROM identity_decisions d WHERE id=? AND kind='listen_review' AND NOT EXISTS(SELECT 1 FROM identity_decisions u WHERE u.kind='listen_review_undo' AND u.to_id=d.id)").bind(&id).fetch_optional(&mut *tx).await?;
    let evidence: Value = serde_json::from_str(&evidence.ok_or_else(|| {
        AppError::Conflict("This decision has already been undone or is unavailable.".into())
    })?)
    .map_err(|_| AppError::Internal)?;
    let before: Vec<Before> =
        serde_json::from_value(evidence["before"].clone()).map_err(|_| AppError::Internal)?;
    let at = evidence["at_ms"].as_i64().ok_or(AppError::Internal)?;
    let mut ids = BTreeSet::new();
    for prior in before {
        let unchanged = if prior.evidence_fingerprint.is_empty() {
            // Compatibility with decisions made in the initial local preview.
            sqlx::query_scalar::<_, i64>("SELECT revision FROM listens WHERE id=?")
                .bind(&prior.id)
                .fetch_optional(&mut *tx)
                .await?
                == evidence["revision"].as_i64()
        } else {
            evidence_fingerprint(&mut tx, &prior.id).await? == prior.evidence_fingerprint
        };
        if !unchanged {
            return Err(AppError::Conflict("New source evidence arrived after this decision. Refresh and compare the current records before changing them.".into()));
        }
        // A name correction, rating or calendar revision is independent of the
        // decision. Preserve those changes; only restore review/trash state.
        let expected_trash = evidence["keep_id"]
            .as_str()
            .filter(|keep| *keep != prior.id)
            .map(|_| at);
        let updated = sqlx::query(
            "UPDATE listens SET needs_review=?,trashed_at_ms=? WHERE id=? AND needs_review=0 AND trashed_at_ms IS ?",
        )
        .bind(prior.needs_review)
        .bind(prior.trashed_at_ms)
        .bind(&prior.id)
        .bind(expected_trash)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated != 1 {
            return Err(AppError::Conflict("A record has changed since this decision. Refresh and review its current evidence.".into()));
        }
        for conflict in prior.conflicts {
            sqlx::query("UPDATE conflicts SET resolved_at_ms=NULL WHERE id=?")
                .bind(conflict)
                .execute(&mut *tx)
                .await?;
        }
        ids.insert(prior.id);
    }
    db::publish(&mut tx, &ids).await?;
    sqlx::query("INSERT INTO identity_decisions VALUES(?,'listen_review_undo',?,?,?,?)")
        .bind(db::new_id())
        .bind(&id)
        .bind(&id)
        .bind("{}")
        .bind(auth::now_ms())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
