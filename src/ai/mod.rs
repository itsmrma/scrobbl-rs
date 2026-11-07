//! Optional local AI: consent, curated catalog, model management and downloads.
//!
//! The model never sees archive rows. This module owns only the settings and
//! model lifecycle surface consumed by the Ask page and inference worker.
pub mod catalog;
pub mod download;
pub mod models;

use crate::{
    AppState, auth,
    error::{AppError, Result},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use ts_rs::TS;

pub const DEFAULT_THREADS: u32 = 4;
pub const DEFAULT_UNLOAD_MINUTES: u32 = 15;

/// Whether this build contains the inference runtime. The runtime is part of
/// the official build (`llama` is a default feature); a `--no-default-features`
/// build reports it as unavailable and keeps settings/downloads usable.
pub fn runtime_available() -> bool {
    cfg!(feature = "llama")
}

#[derive(Clone, Debug)]
pub struct StoredSettings {
    pub enabled: bool,
    pub active_model: Option<String>,
    pub threads: u32,
    pub unload_minutes: u32,
    pub model_dir: Option<String>,
}

impl Default for StoredSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            active_model: None,
            threads: DEFAULT_THREADS,
            unload_minutes: DEFAULT_UNLOAD_MINUTES,
            model_dir: None,
        }
    }
}

/// Read the AI settings from any pool. The inference adapter uses this without
/// constructing a second `AppState`.
pub async fn read_settings_from(db: &sqlx::SqlitePool) -> Result<StoredSettings> {
    let setting = |key: &'static str| async move { crate::db::setting(db, key).await };
    let enabled = setting("ai_enabled")
        .await?
        .as_deref()
        .is_some_and(|value| value == "1");
    let active_model = setting("ai_active_model")
        .await?
        .filter(|value| !value.is_empty());
    let threads = setting("ai_threads")
        .await?
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(DEFAULT_THREADS);
    let unload_minutes = setting("ai_unload_minutes")
        .await?
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(DEFAULT_UNLOAD_MINUTES);
    let model_dir = setting("ai_model_dir")
        .await?
        .filter(|value| !value.trim().is_empty());
    Ok(StoredSettings {
        enabled,
        active_model,
        threads,
        unload_minutes,
        model_dir,
    })
}

pub async fn read_settings(state: &AppState) -> Result<StoredSettings> {
    read_settings_from(&state.db).await
}

/// Filesystem truth for the selected model, shared by the settings surface and
/// the Ask gate so both apply exactly the same rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelReadiness {
    pub installed: bool,
    pub selected_available: bool,
}

pub async fn selected_model_state(
    state: &AppState,
    stored: &StoredSettings,
) -> Result<(PathBuf, ModelReadiness)> {
    let (root, scanned) = active_scan(state, stored.active_model.as_deref()).await?;
    let catalog_model = stored.active_model.as_deref().and_then(catalog::find);
    let installed = scanned.as_ref().is_some_and(models::ScannedModel::usable)
        && catalog_model.as_ref().is_some_and(|model| {
            scanned.as_ref().and_then(|entry| entry.file.as_deref())
                == Some(model.download.file.as_str())
        });
    let selected_available = catalog_model
        .as_ref()
        .is_some_and(catalog::CatalogModel::is_available);
    Ok((
        root,
        ModelReadiness {
            installed,
            selected_available,
        },
    ))
}

pub async fn selected_model_readiness(
    state: &AppState,
    stored: &StoredSettings,
) -> Result<ModelReadiness> {
    Ok(selected_model_state(state, stored).await?.1)
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiSettings {
    pub enabled: bool,
    pub active_model: Option<String>,
    pub threads: u32,
    pub unload_minutes: u32,
    pub model_dir: Option<String>,
    pub effective_model_dir: String,
    pub runtime_available: bool,
    pub model_installed: bool,
    pub model_ready: bool,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct SaveAiSettings {
    pub enabled: bool,
    pub active_model: Option<String>,
    pub threads: u32,
    pub unload_minutes: u32,
    pub model_dir: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiAskStatus {
    pub enabled: bool,
    pub runtime_available: bool,
    pub model_ready: bool,
    pub model_loading: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiLicense {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiDownload {
    pub repo: String,
    pub revision: String,
    pub file: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiRequirements {
    pub disk_bytes: u64,
    pub ram_min_bytes: u64,
    pub ram_recommended_bytes: u64,
    pub cpu_cores: u32,
    pub gpu_optional: bool,
    pub context_tokens: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiLanguageScore {
    pub language: String,
    pub exact_match: Option<f64>,
    pub cases: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiParserScores {
    pub intent: Option<f64>,
    pub slots: Option<f64>,
    pub refusals: Option<f64>,
    pub exact_match: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiSpeed {
    pub p50_ms: u32,
    pub p95_ms: u32,
    pub ttft_ms: u32,
    pub decode_tokens_per_second: f64,
    pub prefill_tokens_per_second: f64,
    pub measured_on: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiDownloadJob {
    pub id: String,
    pub state: String,
    pub processed_bytes: u64,
    pub expected_bytes: u64,
    pub cancel_requested: bool,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiCatalogModel {
    pub id: String,
    pub name: String,
    pub tier: String,
    pub status: String,
    pub description: String,
    pub license: AiLicense,
    pub download: AiDownload,
    pub requirements: AiRequirements,
    pub languages: Vec<AiLanguageScore>,
    pub parser: AiParserScores,
    pub speed: Option<AiSpeed>,
    pub backend: String,
    pub selectable: bool,
    pub installed: bool,
    pub installed_size_bytes: Option<u64>,
    pub checksum_verified: bool,
    pub error: Option<String>,
    pub download_job: Option<AiDownloadJob>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiInstalledModel {
    pub id: String,
    pub name: Option<String>,
    pub file: Option<String>,
    pub size_bytes: u64,
    pub installed_at_ms: Option<i64>,
    pub checksum_verified: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiCatalog {
    pub runtime_available: bool,
    pub model_dir: String,
    pub models: Vec<AiCatalogModel>,
    pub installed: Vec<AiInstalledModel>,
    pub total_installed_bytes: u64,
    pub disk_used_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiDownloadStarted {
    pub job_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiDeleteRequest {
    pub confirm: bool,
    #[serde(default)]
    pub force: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiDeleteResult {
    pub id: String,
    pub freed_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiSelection {
    pub active_model: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AiTestResult {
    pub runtime_available: bool,
    pub model_loaded: bool,
    pub model_ready: bool,
    pub message: String,
}

async fn active_scan(
    state: &AppState,
    active: Option<&str>,
) -> Result<(PathBuf, Option<models::ScannedModel>)> {
    let root = models::root(state).await?;
    let scanned =
        active.and_then(|id| models::scan(&root).into_iter().find(|model| model.id == id));
    Ok((root, scanned))
}

fn settings_message(
    stored: &StoredSettings,
    installed: bool,
    selected_available: bool,
) -> Option<String> {
    if !stored.enabled {
        return None;
    }
    match (&stored.active_model, installed) {
        (None, _) => Some(
            "AI is enabled, but no model is selected. Download a model and select it before asking questions."
                .to_string(),
        ),
        (Some(_), false) => Some(
            "The selected model is not installed or did not pass verification. Download it again or choose another model."
                .to_string(),
        ),
        (Some(_), true) if !selected_available => Some(
            "The selected model is not available for questions in this release. Choose another model."
                .to_string(),
        ),
        (Some(_), true) if !runtime_available() => Some(
            "This build does not include the local AI runtime. Settings and downloads stay available, but questions remain disabled."
                .to_string(),
        ),
        _ => None,
    }
}

async fn present_settings(state: &AppState, stored: StoredSettings) -> Result<AiSettings> {
    let (root, readiness) = selected_model_state(state, &stored).await?;
    let installed = readiness.installed;
    let selected_available = readiness.selected_available;
    let ready = stored.enabled && installed && selected_available;
    Ok(AiSettings {
        enabled: stored.enabled,
        active_model: stored.active_model.clone(),
        threads: stored.threads,
        unload_minutes: stored.unload_minutes,
        model_dir: stored.model_dir.clone(),
        effective_model_dir: root.display().to_string(),
        runtime_available: runtime_available(),
        model_installed: installed,
        model_ready: ready,
        message: settings_message(&stored, installed, selected_available),
    })
}

pub async fn settings_get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AiSettings>> {
    auth::session(&state, &headers).await?;
    let stored = read_settings(&state).await?;
    Ok(Json(present_settings(&state, stored).await?))
}

fn validate_model_dir(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.contains('\0') || !PathBuf::from(value).is_absolute() {
        return Err(AppError::BadRequest(
            "Use an absolute folder for the model directory.".into(),
        ));
    }
    Ok(Some(value.to_string()))
}

pub async fn settings_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<SaveAiSettings>,
) -> Result<Json<AiSettings>> {
    auth::require_write(&state, &headers).await?;
    if !(1..=64).contains(&form.threads) {
        return Err(AppError::BadRequest(
            "Threads must be between 1 and 64.".into(),
        ));
    }
    if !(1..=1440).contains(&form.unload_minutes) {
        return Err(AppError::BadRequest(
            "Unload minutes must be between 1 and 1440.".into(),
        ));
    }
    let model_dir = validate_model_dir(form.model_dir.as_deref())?;
    let active_model = form
        .active_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(id) = active_model.as_deref() {
        let Some(model) = catalog::find(id) else {
            return Err(AppError::BadRequest("Unknown model.".into()));
        };
        if !model.is_available() {
            return Err(AppError::BadRequest(
                "This model is not selectable in this release.".into(),
            ));
        }
        // A known model id may stay selected even if its files were removed or
        // failed verification; the settings response then reports the missing
        // model explicitly. Selecting a model for use still requires an
        // installed file through the select endpoint.
    }

    let mut tx = state.db.begin().await?;
    crate::db::set_setting(&mut tx, "ai_enabled", if form.enabled { "1" } else { "0" }).await?;
    crate::db::set_setting(
        &mut tx,
        "ai_active_model",
        active_model.as_deref().unwrap_or(""),
    )
    .await?;
    crate::db::set_setting(&mut tx, "ai_threads", &form.threads.to_string()).await?;
    crate::db::set_setting(
        &mut tx,
        "ai_unload_minutes",
        &form.unload_minutes.to_string(),
    )
    .await?;
    crate::db::set_setting(&mut tx, "ai_model_dir", model_dir.as_deref().unwrap_or("")).await?;
    tx.commit().await?;

    if !form.enabled {
        // Turning AI off is an explicit consent withdrawal: stop active model
        // downloads too, so no further network transfer continues.
        let jobs: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM jobs WHERE kind=? AND state IN ('queued','running','waiting')",
        )
        .bind(download::JOB_KIND)
        .fetch_all(&state.db)
        .await?;
        for job in jobs {
            let _ = state.storage.cancel(&job).await;
        }
    }

    let stored = StoredSettings {
        enabled: form.enabled,
        active_model,
        threads: form.threads,
        unload_minutes: form.unload_minutes,
        model_dir,
    };
    Ok(Json(present_settings(&state, stored).await?))
}

pub async fn ask_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AiAskStatus>> {
    auth::session(&state, &headers).await?;
    let stored = read_settings(&state).await?;
    let readiness = selected_model_readiness(&state, &stored).await?;
    Ok(Json(AiAskStatus {
        enabled: stored.enabled,
        // The build feature and the installed parser must both be present: a
        // test or a future fallback may install a stub/unavailable parser.
        runtime_available: runtime_available() && state.ask.runtime_available,
        model_ready: stored.enabled && readiness.installed && readiness.selected_available,
        model_loading: state.ask.model_loading(),
    }))
}

#[derive(sqlx::FromRow)]
struct DownloadRow {
    id: String,
    state: String,
    processed: i64,
    parameters_json: String,
    cancel_requested: bool,
    error_code: Option<String>,
}

pub async fn catalog_get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AiCatalog>> {
    auth::session(&state, &headers).await?;
    let root = models::root(&state).await?;
    let scanned = models::scan(&root);
    let rows: Vec<DownloadRow> = sqlx::query_as(
        "SELECT id,state,processed,parameters_json,cancel_requested,error_code FROM jobs WHERE kind=? ORDER BY created_at_ms DESC,id DESC",
    )
    .bind(download::JOB_KIND)
    .fetch_all(&state.db)
    .await?;
    let mut latest_jobs: std::collections::HashMap<String, DownloadRow> =
        std::collections::HashMap::new();
    for row in rows {
        let Some(model_id) = serde_json::from_str::<serde_json::Value>(&row.parameters_json)
            .ok()
            .and_then(|value| value["model_id"].as_str().map(str::to_owned))
        else {
            continue;
        };
        latest_jobs.entry(model_id).or_insert(row);
    }

    let models = catalog::all()
        .into_iter()
        .map(|model| {
            let installed_entry = scanned.iter().find(|entry| entry.id == model.id);
            let installed = installed_entry.is_some_and(|entry| {
                entry.usable() && entry.file.as_deref() == Some(model.download.file.as_str())
            });
            let job = latest_jobs.get(&model.id).map(|row| AiDownloadJob {
                id: row.id.clone(),
                state: row.state.clone(),
                processed_bytes: u64::try_from(row.processed).unwrap_or(0),
                expected_bytes: model.download.size_bytes,
                cancel_requested: row.cancel_requested,
                error_code: row.error_code.clone(),
            });
            AiCatalogModel {
                id: model.id.clone(),
                name: model.name.clone(),
                tier: model.tier.clone(),
                status: model.status.clone(),
                description: model.description.clone(),
                license: AiLicense {
                    name: model.license.name.clone(),
                    url: model.license.url.clone(),
                },
                download: AiDownload {
                    repo: model.download.repo.clone(),
                    revision: model.download.revision.clone(),
                    file: model.download.file.clone(),
                    sha256: model.download.sha256.clone(),
                    size_bytes: model.download.size_bytes,
                },
                requirements: AiRequirements {
                    disk_bytes: model.requirements.disk_bytes,
                    ram_min_bytes: model.requirements.ram_min_bytes,
                    ram_recommended_bytes: model.requirements.ram_recommended_bytes,
                    cpu_cores: model.requirements.cpu_cores,
                    gpu_optional: model.requirements.gpu_optional,
                    context_tokens: model.requirements.context_tokens,
                },
                languages: model
                    .languages
                    .iter()
                    .map(|language| AiLanguageScore {
                        language: language.language.clone(),
                        exact_match: language.exact_match,
                        cases: language.cases,
                    })
                    .collect(),
                parser: AiParserScores {
                    intent: model.parser.intent,
                    slots: model.parser.slots,
                    refusals: model.parser.refusals,
                    exact_match: model.parser.exact_match,
                },
                speed: model.speed.as_ref().map(|speed| AiSpeed {
                    p50_ms: speed.p50_ms,
                    p95_ms: speed.p95_ms,
                    ttft_ms: speed.ttft_ms,
                    decode_tokens_per_second: speed.decode_tokens_per_second,
                    prefill_tokens_per_second: speed.prefill_tokens_per_second,
                    measured_on: speed.measured_on.clone(),
                }),
                backend: model.backend.clone(),
                selectable: model.selectable,
                installed,
                installed_size_bytes: installed_entry.map(|entry| entry.size_bytes),
                checksum_verified: installed_entry.is_some_and(|entry| entry.checksum_verified),
                error: if job
                    .as_ref()
                    .is_some_and(|j| matches!(j.state.as_str(), "queued" | "running" | "waiting"))
                {
                    None
                } else {
                    installed_entry.and_then(|entry| entry.error.clone())
                },
                download_job: job,
            }
        })
        .collect();
    let installed = scanned
        .iter()
        .filter(|entry| {
            !latest_jobs
                .get(&entry.id)
                .is_some_and(|j| matches!(j.state.as_str(), "queued" | "running" | "waiting"))
        })
        .map(|entry| AiInstalledModel {
            id: entry.id.clone(),
            name: catalog::find(&entry.id).map(|model| model.name),
            file: entry.file.clone(),
            size_bytes: entry.size_bytes,
            installed_at_ms: entry.installed_at_ms,
            checksum_verified: entry.checksum_verified,
            error: entry.error.clone(),
        })
        .collect();
    let total_installed_bytes = scanned
        .iter()
        .filter(|entry| entry.usable())
        .map(|entry| entry.size_bytes)
        .fold(0_u64, u64::saturating_add);
    Ok(Json(AiCatalog {
        runtime_available: runtime_available(),
        model_dir: root.display().to_string(),
        models,
        installed,
        total_installed_bytes,
        disk_used_bytes: models::total_size(&root),
    }))
}

fn model_id_from_path(id: String) -> Result<String> {
    Ok(models::safe_id(&id)?.to_string())
}

pub async fn download_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<AiDownloadStarted>> {
    auth::require_write(&state, &headers).await?;
    let id = model_id_from_path(id)?;
    let stored = read_settings(&state).await?;
    if !stored.enabled {
        return Err(AppError::BadRequest(
            "Enable AI before downloading a model.".into(),
        ));
    }
    let Some(model) = catalog::find(&id) else {
        return Err(AppError::NotFound);
    };
    if !model.is_available() {
        return Err(AppError::BadRequest(
            "This model is not available for download in this release.".into(),
        ));
    }
    let root = models::root(&state).await?;
    if models::scan(&root)
        .into_iter()
        .any(|entry| entry.id == id && entry.usable())
    {
        return Err(AppError::Conflict(
            "This model is already installed.".into(),
        ));
    }
    let active = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM jobs WHERE kind=? AND state IN ('queued','running','waiting') AND json_extract(parameters_json,'$.model_id')=?",
    )
    .bind(download::JOB_KIND)
    .bind(&id)
    .fetch_one(&state.db)
    .await?;
    if active > 0 {
        return Err(AppError::Conflict(
            "A download for this model is already in progress.".into(),
        ));
    }
    let job_id = download::enqueue(&state, &model).await?;
    Ok(Json(AiDownloadStarted { job_id }))
}

pub async fn download_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    auth::require_write(&state, &headers).await?;
    let id = model_id_from_path(id)?;
    let job: Option<String> = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE kind=? AND state IN ('queued','running','waiting') AND json_extract(parameters_json,'$.model_id')=? ORDER BY created_at_ms DESC,id DESC LIMIT 1",
    )
    .bind(download::JOB_KIND)
    .bind(&id)
    .fetch_optional(&state.db)
    .await?;
    let Some(job) = job else {
        return Err(AppError::BadRequest(
            "No download is running for this model.".into(),
        ));
    };
    state.storage.cancel(&job).await?;
    Ok(Json(serde_json::json!({"cancelled": true})))
}

pub async fn model_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(form): Json<AiDeleteRequest>,
) -> Result<Json<AiDeleteResult>> {
    auth::require_write(&state, &headers).await?;
    let id = model_id_from_path(id)?;
    if !form.confirm {
        return Err(AppError::BadRequest(
            "Confirm that you want to delete this model.".into(),
        ));
    }
    let root = models::root(&state).await?;
    let found = models::scan(&root).into_iter().any(|entry| entry.id == id);
    if !found {
        return Err(AppError::NotFound);
    }
    let active_job: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE kind=? AND state IN ('queued','running','waiting') AND json_extract(parameters_json,'$.model_id')=?",
    )
    .bind(download::JOB_KIND)
    .bind(&id)
    .fetch_one(&state.db)
    .await?;
    if active_job > 0 {
        return Err(AppError::Conflict(
            "Cancel the download before deleting this model.".into(),
        ));
    }
    let stored = read_settings(&state).await?;
    if stored.active_model.as_deref() == Some(id.as_str()) {
        if !form.force {
            return Err(AppError::BadRequest(
                "This is the active model. Select another model, or delete it with force.".into(),
            ));
        }
        crate::db::set_setting(&mut *state.db.acquire().await?, "ai_active_model", "").await?;
    }
    let freed_bytes = models::remove(&root, &id).await?;
    Ok(Json(AiDeleteResult { id, freed_bytes }))
}

pub async fn model_select(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<AiSelection>> {
    auth::require_write(&state, &headers).await?;
    let id = model_id_from_path(id)?;
    let Some(model) = catalog::find(&id) else {
        return Err(AppError::NotFound);
    };
    if !model.is_available() {
        return Err(AppError::BadRequest(
            "This model is not selectable in this release.".into(),
        ));
    }
    let root = models::root(&state).await?;
    let installed = models::scan(&root).into_iter().any(|entry| {
        entry.id == id
            && entry.usable()
            && entry.file.as_deref() == Some(model.download.file.as_str())
    });
    if !installed {
        return Err(AppError::BadRequest(
            "Install and verify this model before selecting it.".into(),
        ));
    }
    crate::db::set_setting(&mut *state.db.acquire().await?, "ai_active_model", &id).await?;
    Ok(Json(AiSelection { active_model: id }))
}

pub async fn model_test(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AiTestResult>> {
    auth::require_write(&state, &headers).await?;
    let stored = read_settings(&state).await?;
    let runtime = runtime_available();
    if !runtime {
        return Ok(Json(AiTestResult {
            runtime_available: false,
            model_loaded: false,
            model_ready: false,
            message:
                "This build does not include the local AI runtime. Model downloads and settings remain available, but questions stay disabled."
                    .to_string(),
        }));
    }
    let (_, scanned) = active_scan(&state, stored.active_model.as_deref()).await?;
    let usable = scanned.as_ref().is_some_and(models::ScannedModel::usable);
    let message = if stored.active_model.is_none() {
        "Select an installed model first."
    } else if usable {
        "The model file is ready and verified. It will load when you ask the first question."
    } else {
        "The selected model is missing or failed verification. Download it again or choose another model."
    };
    Ok(Json(AiTestResult {
        runtime_available: true,
        model_loaded: false,
        model_ready: usable,
        message: message.to_string(),
    }))
}

/// Startup scan: tolerate orphan/corrupt directories and leave an explicit
/// diagnostic in the log; the catalog endpoint repeats the scan per request.
pub async fn worker(state: AppState) {
    if let Ok(root) = models::root(&state).await {
        for model in models::scan(&root) {
            if let Some(error) = model.error {
                eprintln!("AI model directory '{}' is not usable: {error}", model.id);
            }
        }
    }
    download::worker(state).await;
}
