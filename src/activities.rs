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

async fn present(conn: &mut sqlx::SqliteConnection, job: Job) -> Result<Activity> {
    let totals = sqlx::query("SELECT COUNT(*) AS members, COUNT(expected_rows) AS known, SUM(expected_rows) AS total FROM job_members WHERE job_id=? AND kind NOT IN ('directory','foreign')").bind(&job.id).fetch_one(&mut *conn).await?;
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
        target_timezone: calendar.as_ref().map(|c| c.timezone.clone()),
        target_week_start: calendar.map(|c| c.week_start),
    })
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
    let jobs: Vec<Job> = sqlx::query_as("SELECT * FROM jobs WHERE (? IS NULL OR kind=?) ORDER BY CASE WHEN state IN ('queued','running','waiting') THEN 0 ELSE 1 END, created_at_ms DESC,id DESC LIMIT 100 OFFSET ?").bind(&kind).bind(&kind).bind(i64::from(query.page)*100).fetch_all(&mut *view.transaction).await?;
    let mut items = Vec::new();
    for job in jobs {
        items.push(present(&mut view.transaction, job).await?);
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
    let members = sqlx::query_as("SELECT member,kind,state,expected_rows,processed,error_code,originals,variants,duplicates,added,reconciled,invalid,skipped,below_threshold FROM job_members WHERE job_id=? ORDER BY member").bind(&id).fetch_all(&mut *view.transaction).await?;
    Ok(Json(ActivityDetail {
        source_parameters: if job.kind.starts_with("listenbrainz_") {Some(serde_json::from_str(&job.parameters_json).map_err(|_|AppError::Internal)?)}else{None},
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
    // Only expose controls for the worker available in this block. Source adapters own their controls.
    let job = state.storage.job(&id).await?;
    if !matches!(
        job.kind.as_str(),
        "calendar" | "spotify_import" | "listenbrainz_file" | "listenbrainz_fetch"
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
        "cancel" => state.storage.cancel(&id).await?,
        "retry" => state.storage.retry(&id).await?,
        _ => return Err(AppError::BadRequest("Unknown activity action.".into())),
    }
    Ok(Json(serde_json::json!({"ok":true})))
}
