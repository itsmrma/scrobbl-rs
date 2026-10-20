use crate::{
    AppState, auth,
    dto::*,
    error::{AppError, Result},
    jobs::Job,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Deserialize;
use sqlx::Row;

async fn present(conn: &mut sqlx::SqliteConnection, job: Job) -> Result<Activity> {
    let totals = sqlx::query("SELECT COUNT(*) AS members, COUNT(expected_rows) AS known, SUM(expected_rows) AS total FROM job_members WHERE job_id=?").bind(&job.id).fetch_one(&mut *conn).await?;
    let expected_rows = if totals.get::<i64, _>("members") > 0
        && totals.get::<i64, _>("members") == totals.get::<i64, _>("known")
    {
        totals.get("total")
    } else {
        None
    };
    let calendar = if job.kind == "calendar" {
        let params: serde_json::Value =
            serde_json::from_str(&job.parameters_json).map_err(|_| AppError::Internal)?;
        Some(
            crate::calendar::get(
                conn,
                params["calendar_id"].as_str().ok_or(AppError::Internal)?,
            )
            .await?,
        )
    } else {
        None
    };
    Ok(Activity {
        id: job.id,
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
        retry_at_ms: job.retry_at_ms,
        error_code: job.error_code,
        expected_rows,
        target_timezone: calendar.as_ref().map(|c| c.timezone.clone()),
        target_week_start: calendar.map(|c| c.week_start),
    })
}
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Activity>>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    // Active work stays visible even when newer completed jobs fill the recent history.
    let jobs: Vec<Job> = sqlx::query_as("SELECT * FROM jobs ORDER BY CASE WHEN state IN ('queued','running','waiting') THEN 0 ELSE 1 END, created_at_ms DESC,id DESC LIMIT 100").fetch_all(&mut *view.transaction).await?;
    let mut items = Vec::new();
    for job in jobs {
        items.push(present(&mut view.transaction, job).await?);
    }
    Ok(Json(items))
}
pub async fn detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ActivityDetail>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    let job = sqlx::query_as("SELECT * FROM jobs WHERE id=?")
        .bind(&id)
        .fetch_optional(&mut *view.transaction)
        .await?
        .ok_or_else(|| AppError::BadRequest("Unknown activity.".into()))?;
    let members = sqlx::query_as("SELECT member,kind,state,expected_rows,processed,error_code FROM job_members WHERE job_id=? ORDER BY member").bind(&id).fetch_all(&mut *view.transaction).await?;
    Ok(Json(ActivityDetail {
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
    // Only expose controls for the worker available in this block. Source adapters own their controls.
    let job = state.storage.job(&id).await?;
    if job.kind != "calendar" {
        return Err(AppError::BadRequest(
            "Controls for this activity belong to its source workflow.".into(),
        ));
    }
    match form.action.as_str() {
        "cancel" => state.storage.cancel(&id).await?,
        "retry" => state.storage.retry(&id).await?,
        _ => return Err(AppError::BadRequest("Unknown activity action.".into())),
    }
    Ok(Json(serde_json::json!({"ok":true})))
}
