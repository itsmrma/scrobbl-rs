use crate::{
    AppState, auth,
    dto::*,
    error::{AppError, Result},
    jobs::Job,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use sqlx::Row;

fn present_activity(
    job: Job,
    members: i64,
    known: i64,
    total: Option<i64>,
    calendar: Option<(String, i64)>,
) -> Activity {
    // A subset of the inventory (or counts left from an earlier attempt) is never a total.
    let preparing_spotify = job.kind == "spotify_import"
        && (!job.inventory_complete || matches!(job.phase.as_str(), "inventory" | "counting"));
    let expected_rows = if !preparing_spotify
        && members == known
        && (members > 0 || job.kind == "spotify_import")
    {
        Some(total.unwrap_or(0))
    } else {
        None
    };
    Activity {
        id: job.id,
        created_at_ms: job.created_at_ms,
        kind: job.kind,
        state: job.state,
        filename: job.filename,
        attempt: job.attempt,
        processed: job.processed,
        added: job.added,
        reconciled: job.reconciled,
        duplicates: job.duplicates,
        invalid: job.invalid,
        skipped: job.skipped,
        ambiguous: job.ambiguous,
        retry_at_ms: if job.phase == "receiving" {
            None
        } else {
            job.retry_at_ms
        },
        error_code: job.error_code,
        expected_rows,
        phase: job.phase,
        outcome: job.outcome,
        inventory_complete: job.inventory_complete,
        originals: job.originals,
        variants: job.variants,
        below_threshold: job.below_threshold,
        target_timezone: calendar.as_ref().map(|(tz, _)| tz.clone()),
        target_week_start: calendar.map(|(_, week_start)| week_start),
    }
}

async fn present(conn: &mut sqlx::SqliteConnection, job: Job) -> Result<Activity> {
    let totals = sqlx::query(
        "SELECT COUNT(*) AS members, COUNT(expected_rows) AS known, SUM(expected_rows) AS total FROM job_members WHERE job_id=? AND kind NOT IN ('directory','foreign')",
    )
    .bind(&job.id)
    .fetch_one(&mut *conn)
    .await?;
    let calendar = if job.kind == "calendar" {
        let params: serde_json::Value =
            serde_json::from_str(&job.parameters_json).map_err(|_| AppError::Internal)?;
        let c = crate::calendar::get(
            conn,
            params["calendar_id"].as_str().ok_or(AppError::Internal)?,
        )
        .await?;
        Some((c.timezone, c.week_start))
    } else {
        None
    };
    Ok(present_activity(
        job,
        totals.get("members"),
        totals.get("known"),
        totals.get("total"),
        calendar,
    ))
}
#[derive(Default, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub page: u32,
    pub kind: Option<String>,
}
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<Activity>>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    // Active work stays visible even when newer completed jobs fill the recent history.
    let kind = query.kind.filter(|k| !k.is_empty());
    let rows = sqlx::query(
        "WITH page AS (
             SELECT * FROM jobs
             WHERE (? IS NULL OR kind=?)
             ORDER BY CASE WHEN state IN ('queued','running','waiting') THEN 0 ELSE 1 END,
                      created_at_ms DESC,
                      id DESC
             LIMIT 100 OFFSET ?
         )
         SELECT p.*,
                COALESCE(m.members, 0) AS agg_members,
                COALESCE(m.known, 0) AS agg_known,
                m.total AS agg_total,
                c.timezone AS cal_timezone,
                c.week_start AS cal_week_start
         FROM page p
         LEFT JOIN (
             SELECT jm.job_id,
                    COUNT(*) AS members,
                    COUNT(jm.expected_rows) AS known,
                    SUM(jm.expected_rows) AS total
             FROM job_members jm
             JOIN page p2 ON p2.id = jm.job_id
             WHERE jm.kind NOT IN ('directory','foreign')
             GROUP BY jm.job_id
         ) m ON m.job_id = p.id
         LEFT JOIN calendars c
           ON p.kind = 'calendar'
          AND json_valid(p.parameters_json)
          AND c.id = json_extract(p.parameters_json, '$.calendar_id')
         ORDER BY CASE WHEN p.state IN ('queued','running','waiting') THEN 0 ELSE 1 END,
                  p.created_at_ms DESC,
                  p.id DESC",
    )
    .bind(&kind)
    .bind(&kind)
    .bind(i64::from(query.page) * 100)
    .fetch_all(&mut *view.transaction)
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        use sqlx::FromRow;
        let job = Job::from_row(&row)?;
        let calendar = if job.kind == "calendar" {
            let tz: Option<String> = row.get("cal_timezone");
            let ws: Option<i64> = row.get("cal_week_start");
            Some(tz.zip(ws).ok_or(AppError::Internal)?)
        } else {
            None
        };
        items.push(present_activity(
            job,
            row.get("agg_members"),
            row.get("agg_known"),
            row.get("agg_total"),
            calendar,
        ));
    }
    Ok(Json(items))
}
#[derive(Default, Deserialize)]
pub struct DetailQuery {
    #[serde(default)]
    pub issues_page: u32,
}
pub async fn detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<DetailQuery>,
) -> Result<Json<ActivityDetail>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    let job: Job = sqlx::query_as("SELECT * FROM jobs WHERE id=?")
        .bind(&id)
        .fetch_optional(&mut *view.transaction)
        .await?
        .ok_or_else(|| AppError::BadRequest("Unknown activity.".into()))?;
    let mut members: Vec<ActivityMember> = sqlx::query_as("SELECT member,kind,state,expected_rows,processed,error_code,originals,variants,duplicates,added,reconciled,invalid,skipped,below_threshold FROM job_members WHERE job_id=? ORDER BY member LIMIT ? OFFSET ?").bind(&id).bind(if job.kind=="artwork" {100i64} else {i64::MAX}).bind(if job.kind=="artwork" {i64::from(query.issues_page)*100} else {0}).fetch_all(&mut *view.transaction).await?;
    let source_parameters = if job.kind == "artwork" {
        let current_provider = if job.state == "running" {
            sqlx::query_scalar::<_, String>("SELECT current_provider FROM job_members WHERE job_id=? AND current_provider IS NOT NULL ORDER BY member LIMIT 1")
                .bind(&job.id)
                .fetch_optional(&mut *view.transaction)
                .await?
        } else {
            None
        };
        let next_at_ms: Option<i64> = sqlx::query_scalar::<_, String>(
            "SELECT value FROM settings WHERE key='artwork_background_next_at_ms'",
        )
        .fetch_optional(&mut *view.transaction)
        .await?
        .and_then(|v| v.parse().ok());
        Some(serde_json::json!({
            "interval_ms": crate::artwork::IMAGE_INTERVAL_MS,
            "next_at_ms": next_at_ms,
            "current_provider": current_provider,
        }))
    } else if job.kind.starts_with("listenbrainz_") {
        Some(serde_json::from_str(&job.parameters_json).map_err(|_| AppError::Internal)?)
    } else {
        None
    };
    if job.kind == "artwork" && !members.is_empty() {
        let groups: Vec<&str> = members
            .iter()
            .filter_map(|m| m.member.split_once(':').map(|(_, id)| id))
            .filter(|id| !id.is_empty())
            .collect();
        let mut names = std::collections::HashMap::with_capacity(groups.len());
        if !groups.is_empty() {
            let mut builder = sqlx::QueryBuilder::new(
                "SELECT g.id, COALESCE(ar.name, al.name) AS name FROM name_groups g LEFT JOIN artists ar ON g.kind='artists' AND ar.id=g.representative_id LEFT JOIN albums al ON g.kind='albums' AND al.id=g.representative_id WHERE g.id IN (",
            );
            let mut separated = builder.separated(",");
            for group in &groups {
                separated.push_bind(*group);
            }
            separated.push_unseparated(")");
            let rows: Vec<(String, Option<String>)> = builder
                .build_query_as()
                .fetch_all(&mut *view.transaction)
                .await?;
            for (group_id, name) in rows {
                if let Some(name) = name {
                    names.insert(group_id, name);
                }
            }
        }
        for member in &mut members {
            let group = member
                .member
                .split_once(':')
                .map(|(_, id)| id)
                .unwrap_or("");
            let name = names.get(group).map(String::as_str);
            member.member = format!(
                "{} · {}",
                if member.kind == "artists" {
                    "Artist"
                } else {
                    "Album"
                },
                name.unwrap_or("No longer in the library")
            );
        }
    }

    Ok(Json(ActivityDetail {
        source_parameters,
        issues: sqlx::query_as("SELECT member,row_number,reason FROM source_occurrences WHERE job_id=? AND (outcome='invalid' OR (? LIKE 'listenbrainz_%' AND reason IS NOT NULL)) ORDER BY member,row_number LIMIT 100 OFFSET ?").bind(&id).bind(&job.kind).bind(i64::from(query.issues_page)*100).fetch_all(&mut *view.transaction).await?,
        activity: present(&mut view.transaction, job).await?,
        members,
    }))
}
#[derive(Deserialize)]
pub struct Action {
    pub action: String,
}
pub async fn action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(form): Json<Action>,
) -> Result<Json<serde_json::Value>> {
    auth::require_write(&state, &headers).await?;
    if form.action == "cancel" {
        state.storage.job(&id).await?;
        state.storage.cancel(&id).await?;
        return Ok(Json(serde_json::json!({"ok":true})));
    }
    // Retry/other controls remain owned by their source workflow.
    let job = state.storage.job(&id).await?;
    if !matches!(
        job.kind.as_str(),
        "calendar" | "spotify_import" | "listenbrainz_file" | "listenbrainz_fetch" | "artwork"
    ) {
        return Err(AppError::BadRequest(
            "Controls for this activity belong to its source workflow.".into(),
        ));
    }
    if job.phase == "receiving" {
        return Err(AppError::BadRequest(
            "Upload receipt is incomplete. Send the complete selection again.".into(),
        ));
    }
    if form.action == "retry" && job.kind == "spotify_import" {
        sqlx::query("UPDATE jobs SET parameters_json=? WHERE id=? AND (state IN ('failed','cancelled') OR outcome='partial')").bind(serde_json::to_string(&state.config.import_limits).map_err(|_|AppError::Internal)?).bind(&id).execute(&state.db).await?;
    }
    if form.action == "retry" && job.kind == "listenbrainz_fetch" && job.state == "completed" {
        return Err(AppError::BadRequest("Use the official ListenBrainz ZIP to complete an uncertain interval, or start a new bounded fetch.".into()));
    }
    if form.action == "retry"
        && job.kind == "listenbrainz_file"
        && (matches!(job.state.as_str(), "failed" | "cancelled")
            || job.outcome.as_deref() == Some("partial"))
    {
        let mut parameters: serde_json::Value =
            serde_json::from_str(&job.parameters_json).map_err(|_| AppError::Internal)?;
        parameters["limits"] = serde_json::json!(
            crate::listenbrainz::Limits::from_env().map_err(|_| AppError::Internal)?
        );
        sqlx::query("UPDATE jobs SET parameters_json=? WHERE id=?")
            .bind(parameters.to_string())
            .bind(&id)
            .execute(&state.db)
            .await?;
    }
    match form.action.as_str() {
        "pause" if job.kind == "artwork" => state.storage.pause_artwork(&id).await?,
        "resume" if job.kind == "artwork" => state.storage.resume_artwork(&id).await?,
        "retry" => state.storage.retry(&id).await?,
        _ => return Err(AppError::BadRequest("Unknown activity action.".into())),
    }
    Ok(Json(serde_json::json!({"ok":true})))
}

/// Separate cancellation route stays available while restore drains other writes.
pub async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    auth::require_write(&state, &headers).await?;
    state.storage.job(&id).await?;
    state.storage.cancel(&id).await?;
    Ok(Json(serde_json::json!({"ok":true})))
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    #[tokio::test]
    async fn cancellation_and_status_remain_available_during_exclusive_restore() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::storage::open(dir.path()).await.unwrap();
        let state = AppState::new(
            store,
            crate::config::Config {
                bind_ip: "127.0.0.1".parse().unwrap(),
                port: 3210,
                data_dir: dir.path().into(),
                external_origin: None,
                sync_seconds: 120,
                now_playing_seconds: 30,
                import_limits: Default::default(),
                api_snapshot_limits: Default::default(),
                env_client_id: None,
                env_client_secret: None,
            },
        );
        let token = auth::random_secret();
        sqlx::query("INSERT INTO sessions VALUES(?,'csrf',9999999999999)")
            .bind(auth::digest(&token))
            .execute(&state.private_db)
            .await
            .unwrap();
        let id = state
            .storage
            .enqueue("recovery_restore", None, None, &serde_json::json!({}))
            .await
            .unwrap();
        let _exclusive = state.storage.requests.write().await;
        for (method, path) in [
            ("GET", format!("/api/v1/activities/{id}")),
            ("POST", format!("/api/v1/activities/{id}/cancel")),
        ] {
            let request = Request::builder()
                .method(method)
                .uri(path)
                .header("host", "127.0.0.1:3210")
                .header("origin", "http://127.0.0.1:3210")
                .header("cookie", format!("scrobbl_session={token}"))
                .header("x-csrf-token", "csrf")
                .body(Body::empty())
                .unwrap();
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                crate::web::router(state.clone()).oneshot(request),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::OK);
        }
        assert_eq!(state.storage.job(&id).await.unwrap().state, "cancelled");
    }
}
