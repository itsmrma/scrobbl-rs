//! Durable model download worker.
//!
//! A job stores only the catalog id. The URL, size and SHA-256 are always read
//! from the curated catalog, so a client can never supply a download location.
//! Downloads resume with HTTP `Range`, verify the exact byte size and SHA-256,
//! then publish the file with an atomic rename before writing metadata.
use crate::{
    AppState,
    ai::catalog,
    ai::models,
    auth,
    error::{AppError, Result},
    jobs::Job,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const JOB_KIND: &str = "model_download";
const REPORT_BYTES: u64 = 512 * 1024;
const MAX_ATTEMPTS: i64 = 5;

static AVAILABLE_OVERRIDE_SET: AtomicBool = AtomicBool::new(false);
static AVAILABLE_OVERRIDE: AtomicU64 = AtomicU64::new(0);

/// Integration tests replace the filesystem free-space probe so the disk-full
/// path can run on a machine with plenty of space. `None` restores the probe.
#[doc(hidden)]
pub fn set_available_bytes_for_tests(value: Option<u64>) {
    match value {
        Some(value) => {
            AVAILABLE_OVERRIDE.store(value, Ordering::Relaxed);
            AVAILABLE_OVERRIDE_SET.store(true, Ordering::Relaxed);
        }
        None => AVAILABLE_OVERRIDE_SET.store(false, Ordering::Relaxed),
    }
}

fn available_bytes(path: &std::path::Path) -> u64 {
    if AVAILABLE_OVERRIDE_SET.load(Ordering::Relaxed) {
        return AVAILABLE_OVERRIDE.load(Ordering::Relaxed);
    }
    fs2::available_space(path).unwrap_or(u64::MAX)
}

fn retry_delay_ms(attempt: i64) -> i64 {
    let exponent = u32::try_from(attempt.clamp(0, 6)).unwrap_or(6);
    1_000_i64.saturating_mul(1_i64 << exponent).min(60_000)
}

fn is_transient_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

async fn cancel_requested(state: &AppState, job_id: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT cancel_requested FROM jobs WHERE id=?")
        .bind(job_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .unwrap_or(true)
}

async fn update_progress(
    state: &AppState,
    job: &Job,
    member: &str,
    processed: u64,
    phase: &str,
) -> bool {
    let processed = i64::try_from(processed).unwrap_or(i64::MAX);
    let changed = sqlx::query(
        "UPDATE jobs SET processed=?,phase=?,updated_at_ms=? WHERE id=? AND state='running' AND attempt=? AND cancel_requested=0",
    )
    .bind(processed)
    .bind(phase)
    .bind(auth::now_ms())
    .bind(&job.id)
    .bind(job.attempt)
    .execute(&state.db)
    .await
    .map(|result| result.rows_affected() == 1)
    .unwrap_or(false);
    if changed {
        let _ = sqlx::query("UPDATE job_members SET processed=? WHERE job_id=? AND member=?")
            .bind(processed)
            .bind(&job.id)
            .bind(member)
            .execute(&state.db)
            .await;
    }
    changed
}

struct Finish<'a> {
    state: &'a str,
    outcome: &'a str,
    error_code: Option<&'a str>,
    phase: &'a str,
    processed: u64,
}

async fn finish(state: &AppState, job: &Job, member: &str, finish: Finish<'_>) {
    let processed = i64::try_from(finish.processed).unwrap_or(i64::MAX);
    let _ = sqlx::query(
        "UPDATE jobs SET state=?,outcome=?,error_code=?,phase=?,processed=?,retry_at_ms=NULL,updated_at_ms=? WHERE id=?",
    )
    .bind(finish.state)
    .bind(finish.outcome)
    .bind(finish.error_code)
    .bind(finish.phase)
    .bind(processed)
    .bind(auth::now_ms())
    .bind(&job.id)
    .execute(&state.db)
    .await;
    let _ = sqlx::query(
        "UPDATE job_members SET state=?,processed=?,error_code=? WHERE job_id=? AND member=?",
    )
    .bind(finish.state)
    .bind(processed)
    .bind(finish.error_code)
    .bind(&job.id)
    .bind(member)
    .execute(&state.db)
    .await;
}

async fn wait_for_retry(state: &AppState, job: &Job, code: &str, attempt: i64) {
    if attempt >= MAX_ATTEMPTS {
        let processed = sqlx::query_scalar::<_, i64>("SELECT processed FROM jobs WHERE id=?")
            .bind(&job.id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
            .unwrap_or(0)
            .max(0) as u64;
        finish(
            state,
            job,
            job_member(job).as_str(),
            Finish {
                state: "failed",
                outcome: "error",
                error_code: Some("model_download_failed"),
                phase: "finished",
                processed,
            },
        )
        .await;
        return;
    }
    let retry_at = auth::now_ms() + retry_delay_ms(attempt);
    if state.storage.pause(job, Some(retry_at), code).await.is_ok() {
        let _ = sqlx::query("UPDATE jobs SET phase='waiting' WHERE id=?")
            .bind(&job.id)
            .execute(&state.db)
            .await;
    }
}

fn job_member(job: &Job) -> String {
    serde_json::from_str::<serde_json::Value>(&job.parameters_json)
        .ok()
        .and_then(|value| value["model_id"].as_str().map(str::to_owned))
        .unwrap_or_else(|| job.id.clone())
}

async fn fail(state: &AppState, job: &Job, member: &str, code: &str, processed: u64) {
    finish(
        state,
        job,
        member,
        Finish {
            state: "failed",
            outcome: "error",
            error_code: Some(code),
            phase: "finished",
            processed,
        },
    )
    .await;
}

async fn cancelled(state: &AppState, job: &Job, member: &str, processed: u64) {
    finish(
        state,
        job,
        member,
        Finish {
            state: "cancelled",
            outcome: "cancelled",
            error_code: Some("model_cancelled"),
            phase: "finished",
            processed,
        },
    )
    .await;
}

fn remove_partial(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

/// Claim and process exactly one download job. Tests drive this directly.
pub async fn run_once(state: &AppState) -> Result<bool> {
    let Some(job) = state.storage.claim(JOB_KIND).await? else {
        return Ok(false);
    };
    run_claimed(state, job).await?;
    Ok(true)
}

/// Process an already claimed job. Exposed so a test can claim, request a
/// cancellation, then prove the worker honours the durable flag.
#[doc(hidden)]
pub async fn run_claimed(state: &AppState, job: Job) -> Result<()> {
    let model_id = serde_json::from_str::<serde_json::Value>(&job.parameters_json)
        .ok()
        .and_then(|value| value["model_id"].as_str().map(str::to_owned))
        .ok_or_else(|| AppError::Internal)?;
    let member = model_id.clone();
    let Some(model) = catalog::find(&model_id) else {
        fail(state, &job, &member, "model_unavailable", 0).await;
        return Ok(());
    };
    let root = models::root(state).await?;
    let dir = models::model_dir(&root, &model.id)?;
    let final_path = models::model_file(&dir, &model.download.file)?;
    let part_path = dir.join(format!("{}.part", model.download.file));
    let expected = model.download.size_bytes;

    if cancel_requested(state, &job.id).await {
        remove_partial(&part_path);
        cancelled(state, &job, &member, 0).await;
        return Ok(());
    }

    let _ = sqlx::query(
        "UPDATE job_members SET state='running',error_code=NULL WHERE job_id=? AND member=?",
    )
    .bind(&job.id)
    .bind(&member)
    .execute(&state.db)
    .await;

    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|_| AppError::Internal)?;

    // An already published file makes the job idempotent.
    if let Ok(metadata) = tokio::fs::metadata(&final_path).await
        && metadata.is_file()
        && metadata.len() == expected
        && let Ok(bytes) = tokio::fs::read(dir.join(models::METADATA_FILE)).await
        && let Ok(stored) = serde_json::from_slice::<models::ModelMetadata>(&bytes)
        && stored.catalog_id == model.id
        && stored.size_bytes == expected
        && stored.sha256.eq_ignore_ascii_case(&model.download.sha256)
    {
        update_progress(state, &job, &member, expected, "verifying").await;
        finish(
            state,
            &job,
            &member,
            Finish {
                state: "completed",
                outcome: "complete",
                error_code: None,
                phase: "finished",
                processed: expected,
            },
        )
        .await;
        return Ok(());
    }

    let mut downloaded = match tokio::fs::metadata(&part_path).await {
        Ok(metadata) if metadata.is_file() && metadata.len() <= expected => metadata.len(),
        Ok(_) => {
            remove_partial(&part_path);
            0
        }
        Err(_) => 0,
    };
    if available_bytes(&dir) < expected.saturating_sub(downloaded) {
        fail(state, &job, &member, "model_disk_full", downloaded).await;
        return Ok(());
    }

    let client = reqwest::Client::builder()
        .user_agent(concat!("scrobbl.rs/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| AppError::Internal)?;
    let mut request = client.get(model.download.url());
    if downloaded > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={downloaded}-"));
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => {
            wait_for_retry(state, &job, "model_download_retry", job.attempt).await;
            return Ok(());
        }
    };
    let status = response.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        remove_partial(&part_path);
        wait_for_retry(state, &job, "model_download_retry", job.attempt).await;
        return Ok(());
    }
    if is_transient_status(status) {
        wait_for_retry(state, &job, "model_download_retry", job.attempt).await;
        return Ok(());
    }
    if !status.is_success() {
        fail(state, &job, &member, "model_source_rejected", downloaded).await;
        return Ok(());
    }
    if downloaded > 0 && status != reqwest::StatusCode::PARTIAL_CONTENT {
        // The source ignored Range. Restart cleanly rather than appending twice.
        remove_partial(&part_path);
        downloaded = 0;
    }

    let mut file = match tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(downloaded > 0)
        .truncate(downloaded == 0)
        .open(&part_path)
        .await
    {
        Ok(file) => file,
        Err(_) => {
            wait_for_retry(state, &job, "model_download_retry", job.attempt).await;
            return Ok(());
        }
    };
    let mut last_report = downloaded;
    let mut response = response;
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(_) => {
                let _ = file.sync_all().await;
                drop(file);
                wait_for_retry(state, &job, "model_download_retry", job.attempt).await;
                return Ok(());
            }
        };
        if downloaded.saturating_add(chunk.len() as u64) > expected {
            drop(file);
            remove_partial(&part_path);
            fail(state, &job, &member, "model_checksum_mismatch", 0).await;
            return Ok(());
        }
        if file.write_all(&chunk).await.is_err() {
            drop(file);
            wait_for_retry(state, &job, "model_download_retry", job.attempt).await;
            return Ok(());
        }
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded.saturating_sub(last_report) >= REPORT_BYTES {
            let _ = file.flush().await;
            if !update_progress(state, &job, &member, downloaded, "downloading").await {
                drop(file);
                remove_partial(&part_path);
                cancelled(state, &job, &member, downloaded).await;
                return Ok(());
            }
            if available_bytes(&dir) < expected.saturating_sub(downloaded) {
                let _ = file.sync_all().await;
                drop(file);
                fail(state, &job, &member, "model_disk_full", downloaded).await;
                return Ok(());
            }
            last_report = downloaded;
            if cancel_requested(state, &job.id).await {
                let _ = file.sync_all().await;
                drop(file);
                remove_partial(&part_path);
                cancelled(state, &job, &member, downloaded).await;
                return Ok(());
            }
        }
    }
    let _ = file.sync_all().await;
    drop(file);

    if downloaded != expected {
        wait_for_retry(state, &job, "model_download_retry", job.attempt).await;
        return Ok(());
    }
    if cancel_requested(state, &job.id).await {
        remove_partial(&part_path);
        cancelled(state, &job, &member, downloaded).await;
        return Ok(());
    }
    update_progress(state, &job, &member, downloaded, "verifying").await;

    let actual = match hash_file(&part_path).await {
        Ok(hash) => hash,
        Err(_) => {
            wait_for_retry(state, &job, "model_download_retry", job.attempt).await;
            return Ok(());
        }
    };
    if !actual.eq_ignore_ascii_case(&model.download.sha256) {
        remove_partial(&part_path);
        fail(state, &job, &member, "model_checksum_mismatch", 0).await;
        return Ok(());
    }

    if final_path.exists() {
        let _ = tokio::fs::remove_file(&final_path).await;
    }
    if tokio::fs::rename(&part_path, &final_path).await.is_err() {
        fail(state, &job, &member, "model_download_failed", downloaded).await;
        return Ok(());
    }
    crate::storage::sync_directory(&dir).map_err(|_| AppError::Internal)?;
    let metadata = models::ModelMetadata {
        catalog_id: model.id.clone(),
        file: model.download.file.clone(),
        sha256: model.download.sha256.clone(),
        size_bytes: expected,
        installed_at_ms: auth::now_ms(),
        license: model.license.name.clone(),
        checksum_verified: true,
    };
    if models::write_metadata(&dir, &metadata).await.is_err() {
        fail(state, &job, &member, "model_download_failed", downloaded).await;
        return Ok(());
    }
    finish(
        state,
        &job,
        &member,
        Finish {
            state: "completed",
            outcome: "complete",
            error_code: None,
            phase: "finished",
            processed: expected,
        },
    )
    .await;
    Ok(())
}

async fn hash_file(path: &std::path::Path) -> std::io::Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(crate::auth::encode_hex(&hasher.finalize()))
}

/// Enqueue a download from the curated catalog. The job carries only the model
/// id; the pinned URL is resolved by the worker.
pub async fn enqueue(state: &AppState, model: &catalog::CatalogModel) -> Result<String> {
    if !model.is_available() {
        return Err(AppError::BadRequest(
            "This model is not available for download in this release.".into(),
        ));
    }
    let job_id = state
        .storage
        .enqueue(
            JOB_KIND,
            None,
            Some(&model.name),
            &json!({"model_id": model.id}),
        )
        .await?;
    sqlx::query(
        "INSERT INTO job_members(job_id,member,kind,state,expected_rows,processed) VALUES(?,?,'model','queued',?,0) ON CONFLICT(job_id,member) DO UPDATE SET kind=excluded.kind,state=excluded.state,expected_rows=excluded.expected_rows",
    )
    .bind(&job_id)
    .bind(&model.id)
    .bind(i64::try_from(model.download.size_bytes).unwrap_or(i64::MAX))
    .execute(&state.db)
    .await?;
    sqlx::query("UPDATE jobs SET phase='queued' WHERE id=?")
        .bind(&job_id)
        .execute(&state.db)
        .await?;
    Ok(job_id)
}

/// Long-running worker used by `main`.
pub async fn worker(state: AppState) {
    loop {
        match run_once(&state).await {
            Ok(true) => {}
            Ok(false) => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(_) => {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}
