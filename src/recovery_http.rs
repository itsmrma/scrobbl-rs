use crate::{
    AppState, auth,
    error::{AppError, Result},
    recovery,
};
use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

fn failure(error: anyhow::Error) -> AppError {
    AppError::BadRequest(recovery::public_error(&error).into())
}
#[derive(Deserialize)]
pub struct Start {
    pub kind: String,
    pub format: Option<String>,
    #[serde(default)]
    pub selection: crate::exports::ExportSelection,
}
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::session(&state, &headers).await?;
    Ok(Json(
        json!({"assets":recovery::assets(&state.storage).await.map_err(failure)?,"paused":recovery::paused(&state.storage).await,"restart_required":state.storage.directory.join("recovery.pending").exists()}),
    ))
}
pub async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<Start>,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    let (kind, format, parameters) = match form.kind.as_str() {
        "backup" => ("backup", "zip", json!({})),
        "export" => {
            let format = form.format.as_deref().unwrap_or("json");
            if !matches!(format, "json" | "csv") {
                return Err(AppError::BadRequest("Choose JSON or CSV.".into()));
            }
            // Validate the shared selection before accepting a persistent job.
            let mut conn = state.read_db.acquire().await?;
            crate::web::selection(&mut conn, &form.selection.filters()).await?;
            (
                "export",
                format,
                serde_json::to_value(form.selection).map_err(|_| AppError::Internal)?,
            )
        }
        _ => return Err(AppError::BadRequest("Choose export or backup.".into())),
    };
    let id = recovery::enqueue_asset(&state.storage, kind, format, parameters)
        .await
        .map_err(failure)?;
    Ok(Json(json!({"id":id})))
}
pub async fn download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response> {
    auth::session(&state, &headers).await?;
    let asset = recovery::asset(&state.storage, &id)
        .await
        .map_err(failure)?;
    if asset.state != "ready" {
        return Err(AppError::Conflict("This file is not complete.".into()));
    }
    let path = recovery::asset_path(&state.storage, &asset).map_err(failure)?;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| AppError::BadRequest("The file is no longer available.".into()))?;
    let content = match asset.format.as_str() {
        "zip" => "application/zip",
        "csv" => "text/csv; charset=utf-8",
        _ => "application/json; charset=utf-8",
    };
    Ok((
        [
            (header::CONTENT_TYPE, content.to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"scrobbl-{}.{}\"",
                    asset.id, asset.format
                ),
            ),
        ],
        Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
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
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    let _operation = state.storage.recovery_operations.lock().await;
    let asset = recovery::asset(&state.storage, &id)
        .await
        .map_err(failure)?;
    match form.action.as_str() {
        "verify" if asset.state == "ready" && asset.format == "zip" => {
            let job = recovery::enqueue(&state.storage, "recovery_verify", json!({"asset_id":id}))
                .await
                .map_err(failure)?;
            Ok(Json(json!({"job_id":job})))
        }
        "delete" => {
            let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=? AND state IN ('queued','running','waiting'))").bind(&asset.job_id).fetch_one(&state.read_db).await?;
            if active {
                return Err(AppError::Conflict(
                    "Wait for this operation to finish before deleting its file.".into(),
                ));
            }
            let path = recovery::asset_path(&state.storage, &asset).map_err(failure)?;
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(AppError::Internal),
            }
            sqlx::query("DELETE FROM recovery_assets WHERE id=?")
                .bind(id)
                .execute(&state.db)
                .await?;
            Ok(Json(json!({"ok":true})))
        }
        _ => Err(AppError::BadRequest(
            "This action is unavailable for this file.".into(),
        )),
    }
}
pub async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    let id = crate::db::new_id();
    let path = state
        .storage
        .directory
        .join("backups")
        .join(format!("{id}.partial"));
    let result=async {
        let mut field=multipart.next_field().await.map_err(|_|AppError::BadRequest("Upload a complete backup ZIP.".into()))?.ok_or_else(||AppError::BadRequest("Choose a backup ZIP.".into()))?;
        let mut file=tokio::fs::OpenOptions::new().write(true).create_new(true).open(&path).await.map_err(|_|AppError::Internal)?;
        let mut size=0u64;
        while let Some(chunk)=field.chunk().await.map_err(|_|AppError::BadRequest("Backup upload interrupted; send the complete file again.".into()))? {
            size+=chunk.len() as u64;
            if size>recovery::PACKED_LIMIT {return Err(AppError::BadRequest("Backup uploads are limited to 2 GiB.".into()));}
            file.write_all(&chunk).await.map_err(|_|AppError::BadRequest("Could not save the upload. Check free disk space.".into()))?;
        }
        file.sync_all().await.map_err(|_|AppError::Internal)?;drop(file);
        drop(field);
        if multipart.next_field().await.map_err(|_|AppError::BadRequest("Upload a single complete ZIP.".into()))?.is_some() {return Err(AppError::BadRequest("Choose one backup ZIP at a time.".into()));}
        let destination=state.storage.directory.join("backups").join(format!("{id}.zip"));
        std::fs::rename(&path,&destination).map_err(|_|AppError::Internal)?;
        crate::storage::sync_directory(&state.storage.directory.join("backups")).map_err(|_|AppError::Internal)?;
        let mut tx=state.db.begin().await?;let job=crate::db::new_id();
        sqlx::query("INSERT INTO jobs(id,kind,state,phase,created_at_ms,updated_at_ms) VALUES(?,'recovery_inspect','queued','preparing',?,?)").bind(&job).bind(auth::now_ms()).bind(auth::now_ms()).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO recovery_assets(id,job_id,kind,format,created_at_ms,size_bytes) VALUES(?,?,'uploaded_backup','zip',?,?)").bind(&id).bind(job).bind(auth::now_ms()).bind(size as i64).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Json(json!({"id":id})))
    }.await;
    let _ = tokio::fs::remove_file(&path).await;
    result
}
#[derive(Deserialize)]
pub struct Restore {
    pub asset_id: String,
    pub password: String,
    pub confirm_replace: bool,
}
pub async fn restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<Restore>,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    if !form.confirm_replace {
        return Err(AppError::BadRequest(
            "Explicit confirmation is required to replace the archive.".into(),
        ));
    }
    // Drain an existing background collection, including token refresh and checkpoint
    // publication. HTTP requests were already drained by the exclusive request barrier.
    let _collection = state.spotify.collection_gate.lock().await;
    recovery::prepare_restore(&state.storage, &form.asset_id, form.password)
        .await
        .map_err(failure)?;
    Ok(Json(json!({"restart_required":true})))
}
pub async fn resume(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    sqlx::query("DELETE FROM settings WHERE key='recovery_paused'")
        .execute(&state.db)
        .await?;
    state.spotify.wake_collection();
    Ok(Json(json!({"ok":true})))
}
