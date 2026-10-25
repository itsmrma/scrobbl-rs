//! Reversible name-based consultation groups; source identities are never merged.
use crate::{
    AppState, auth, db,
    error::{AppError, Result},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::BTreeSet;

pub(crate) fn attach(conn: &Connection, kind: &str, id: &str) -> Result<()> {
    let present: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM name_group_members WHERE kind=? AND entity_id=?)",
        params![kind, id],
        |r| r.get(0),
    )?;
    if present {
        return Ok(());
    }
    let key: String = if kind == "artists" {
        conn.query_row(
            "SELECT lower(trim(name)) FROM artists WHERE id=?",
            [id],
            |r| r.get(0),
        )?
    } else {
        conn.query_row("SELECT json_array(lower(trim(a.name)),COALESCE((SELECT json_group_array(name) FROM (SELECT DISTINCT lower(trim(ar.name)) AS name FROM tracks t JOIN track_artists ta ON ta.track_id=t.id JOIN artists ar ON ar.id=ta.artist_id WHERE t.album_id=a.id ORDER BY name)),'')) FROM albums a WHERE a.id=?",[id],|r|r.get(0))?
    };
    let group = conn
        .query_row(
            "SELECT id FROM name_groups WHERE kind=? AND name_key=?",
            params![kind, key],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    let group = if let Some(group) = group {
        group
    } else {
        let group = format!("name-{}", db::new_id());
        conn.execute(
            "INSERT INTO name_groups VALUES(?,?,?,?)",
            params![group, kind, key, id],
        )?;
        group
    };
    conn.execute(
        "INSERT INTO name_group_members VALUES(?,?,?,?)",
        params![kind, id, group, group],
    )?;
    Ok(())
}
pub async fn canonical(conn: &mut sqlx::SqliteConnection, kind: &str, id: &str) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT group_id FROM name_group_members WHERE kind=? AND entity_id=?")
            .bind(kind)
            .bind(id)
            .fetch_optional(conn)
            .await?
            .unwrap_or_else(|| id.to_owned()),
    )
}
fn validate_kind(kind: &str) -> Result<()> {
    if !["artists", "albums"].contains(&kind) {
        return Err(AppError::BadRequest(
            "Name groups are available for artists and albums.".into(),
        ));
    }
    Ok(())
}
#[derive(Serialize, ts_rs::TS)]
pub struct GroupMember {
    pub id: String,
    pub name: String,
    pub sample: String,
    pub tracks: i64,
    pub listens: i64,
}
#[derive(Serialize, ts_rs::TS)]
pub struct GroupPage {
    pub members: Vec<GroupMember>,
    pub total: i64,
    pub page: i64,
    pub can_rejoin: bool,
}
#[derive(Default, Deserialize)]
pub struct GroupQuery {
    pub q: Option<String>,
    pub page: Option<i64>,
}
pub async fn members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
    Query(query): Query<GroupQuery>,
) -> Result<Json<GroupPage>> {
    auth::session(&state, &headers).await?;
    validate_kind(&kind)?;
    let mut view = state.storage.read_view().await?;
    let conn = &mut *view.transaction;
    let id = canonical(conn, &kind, &id).await?;
    let page = query.page.unwrap_or(1).clamp(1, 1000000);
    let q = query.q.unwrap_or_default();
    if q.len() > 500 {
        return Err(AppError::BadRequest(
            "Search is limited to 500 bytes.".into(),
        ));
    }
    let pattern = format!(
        "%{}%",
        q.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let source = if kind == "artists" {
        "main.artists e JOIN main.track_artists ta ON ta.artist_id=e.id JOIN main.tracks t ON t.id=ta.track_id"
    } else {
        "main.albums e JOIN main.tracks t ON t.album_id=e.id"
    };
    let sql = format!(
        "SELECT e.id,e.name,MIN(t.title) AS sample,COUNT(DISTINCT t.id) AS tracks,COUNT(DISTINCT l.id) AS listens FROM {source} JOIN name_group_members m ON m.entity_id=e.id AND m.kind=? LEFT JOIN listens l ON l.track_id=t.id AND l.trashed_at_ms IS NULL AND l.counted=1 WHERE m.group_id=? AND (e.name LIKE ? ESCAPE '\\' OR t.title LIKE ? ESCAPE '\\') GROUP BY e.id ORDER BY e.name COLLATE NOCASE,e.id LIMIT 24 OFFSET ?"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(&kind)
        .bind(&id)
        .bind(&pattern)
        .bind(&pattern)
        .bind((page - 1) * 24)
        .fetch_all(&mut *conn)
        .await?;
    let total:i64=sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(DISTINCT e.id) FROM {source} JOIN name_group_members m ON m.entity_id=e.id AND m.kind=? WHERE m.group_id=? AND (e.name LIKE ? ESCAPE '\\' OR t.title LIKE ? ESCAPE '\\')"))).bind(&kind).bind(&id).bind(&pattern).bind(&pattern).fetch_one(&mut *conn).await?;
    let members = rows
        .into_iter()
        .map(|r| GroupMember {
            id: r.get("id"),
            name: r.get("name"),
            sample: r.get("sample"),
            tracks: r.get("tracks"),
            listens: r.get("listens"),
        })
        .collect();
    let can_rejoin:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM name_group_members WHERE group_id=? AND group_id!=default_group_id)").bind(&id).fetch_one(conn).await?;
    Ok(Json(GroupPage {
        members,
        total,
        page,
        can_rejoin,
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeGroup {
    pub action: String,
    pub member_ids: Vec<String>,
}
pub async fn change(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
    Json(form): Json<ChangeGroup>,
) -> Result<Json<serde_json::Value>> {
    auth::require_write(&state, &headers).await?;
    validate_kind(&kind)?;
    crate::library::writable(&state).await?;
    if form.member_ids.is_empty() || form.member_ids.len() > 1000 {
        return Err(AppError::BadRequest(
            "Choose between 1 and 1000 source entries.".into(),
        ));
    }
    if !["separate", "rejoin"].contains(&form.action.as_str()) {
        return Err(AppError::BadRequest("Choose separate or rejoin.".into()));
    }
    let mut tx = state.db.begin().await?;
    let id = canonical(&mut tx, &kind, &id).await?;
    let mut groups = BTreeSet::from([id.clone()]);
    let mut targets = Vec::new();
    for member in &form.member_ids {
        let default:Option<String>=sqlx::query_scalar("SELECT default_group_id FROM name_group_members WHERE kind=? AND entity_id=? AND group_id=?").bind(&kind).bind(member).bind(&id).fetch_optional(&mut *tx).await?;
        targets.push(default.ok_or_else(|| {
            AppError::Conflict(
                "A selected entry is no longer in this group. Reload the page.".into(),
            )
        })?);
    }
    let destination = targets[0].clone();
    let new_id = if form.action == "separate" {
        let new_id = format!("name-{}", db::new_id());
        sqlx::query("INSERT INTO name_groups VALUES(?,?,?,?)")
            .bind(&new_id)
            .bind(&kind)
            .bind(format!("manual:{new_id}"))
            .bind(&form.member_ids[0])
            .execute(&mut *tx)
            .await?;
        // Copy the current display choices, retaining all source-specific overrides.
        sqlx::query("INSERT OR IGNORE INTO local_corrections SELECT entity_kind,?,field,value_json FROM local_corrections WHERE entity_id=?").bind(&new_id).bind(&id).execute(&mut *tx).await?;
        sqlx::query("INSERT OR IGNORE INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms,provenance_json) SELECT kind,?,source,bytes,mime,checked_at_ms,expires_at_ms,provenance_json FROM artwork WHERE entity_id=? AND source='custom'").bind(&new_id).bind(&id).execute(&mut *tx).await?;
        // A separated source entry recovers its own older personal choices.
        // Group display choices remain the fallback when no source override exists.
        if form.member_ids.len() == 1 {
            sqlx::query("INSERT OR REPLACE INTO local_corrections SELECT entity_kind,?,field,value_json FROM local_corrections WHERE entity_id=?").bind(&new_id).bind(&form.member_ids[0]).execute(&mut *tx).await?;
            sqlx::query("INSERT OR REPLACE INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms,provenance_json) SELECT kind,?,source,bytes,mime,checked_at_ms,expires_at_ms,provenance_json FROM artwork WHERE entity_id=? AND source='custom'").bind(&new_id).bind(&form.member_ids[0]).execute(&mut *tx).await?;
        }
        Some(new_id)
    } else {
        None
    };
    for (member, default) in form.member_ids.iter().zip(targets) {
        let target = new_id.as_ref().unwrap_or(&default);
        sqlx::query("UPDATE name_group_members SET group_id=? WHERE kind=? AND entity_id=?")
            .bind(target)
            .bind(&kind)
            .bind(member)
            .execute(&mut *tx)
            .await?;
        groups.insert(target.clone());
    }
    let mut affected = BTreeSet::new();
    let source = if kind == "artists" {
        "SELECT l.id FROM listens l JOIN main.track_artists ta ON ta.track_id=l.track_id JOIN name_group_members m ON m.kind='artists' AND m.entity_id=ta.artist_id WHERE m.group_id=?"
    } else {
        "SELECT l.id FROM listens l JOIN main.tracks t ON t.id=l.track_id JOIN name_group_members m ON m.kind='albums' AND m.entity_id=t.album_id WHERE m.group_id=?"
    };
    for group in &groups {
        affected.extend(
            sqlx::query_scalar::<_, String>(source)
                .bind(group)
                .fetch_all(&mut *tx)
                .await?,
        );
    }
    db::publish(&mut tx, &affected).await?;
    sqlx::query("INSERT INTO identity_decisions VALUES(?,?,?,?,?,?)")
        .bind(db::new_id())
        .bind(format!("name_group_{}", form.action))
        .bind(&id)
        .bind(new_id.as_deref().unwrap_or("default_name_group"))
        .bind(serde_json::json!({"members":form.member_ids}).to_string())
        .bind(auth::now_ms())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    state.dashboard_cache.lock().await.clear();
    Ok(Json(
        serde_json::json!({"saved":true,"group_id":new_id.unwrap_or(destination)}),
    ))
}
