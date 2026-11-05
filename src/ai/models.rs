//! Installed model files. The filesystem is the source of truth: every model
//! directory carries its own metadata beside the GGUF file, and the database
//! stores only consent and the selected id.
use crate::{
    AppState,
    error::{AppError, Result},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const METADATA_FILE: &str = "metadata.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelMetadata {
    pub catalog_id: String,
    pub file: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub installed_at_ms: i64,
    pub license: String,
    pub checksum_verified: bool,
}

#[derive(Clone, Debug)]
pub struct ScannedModel {
    pub id: String,
    pub file: Option<String>,
    pub size_bytes: u64,
    pub installed_at_ms: Option<i64>,
    pub sha256: Option<String>,
    pub license: Option<String>,
    pub checksum_verified: bool,
    pub error: Option<String>,
}

impl ScannedModel {
    pub fn usable(&self) -> bool {
        self.error.is_none() && self.checksum_verified
    }
}

pub async fn root(state: &AppState) -> Result<PathBuf> {
    match crate::db::setting(&state.db, "ai_model_dir").await? {
        Some(value) if !value.trim().is_empty() => Ok(PathBuf::from(value.trim())),
        _ => Ok(state.storage.directory.join("models")),
    }
}

/// Reject ids that could escape the models directory. Catalog ids are internal
/// constants, but the scan/delete paths also see directory names from disk.
pub fn safe_id(id: &str) -> Result<&str> {
    if id.is_empty() || id == "." || id == ".." || id.contains(['/', '\\']) || id.contains('\0') {
        return Err(AppError::BadRequest("Unknown model.".into()));
    }
    Ok(id)
}

pub fn model_dir(root: &Path, id: &str) -> Result<PathBuf> {
    Ok(root.join(safe_id(id)?))
}

pub fn model_file(dir: &Path, file: &str) -> Result<PathBuf> {
    if file.is_empty() || file.contains(['/', '\\']) || file == "." || file == ".." {
        return Err(AppError::BadRequest("Invalid model file name.".into()));
    }
    Ok(dir.join(file))
}

fn read_metadata_sync(path: &Path) -> std::result::Result<ModelMetadata, String> {
    let bytes = std::fs::read(path).map_err(|_| "Model metadata is missing.".to_string())?;
    let metadata: ModelMetadata =
        serde_json::from_slice(&bytes).map_err(|_| "Model metadata is unreadable.".to_string())?;
    if metadata.catalog_id.is_empty()
        || metadata.file.is_empty()
        || metadata.sha256.len() != 64
        || metadata.size_bytes == 0
    {
        return Err("Model metadata is incomplete.".to_string());
    }
    Ok(metadata)
}

pub fn directory_size(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.file_type().is_symlink() {
        return 0;
    }
    if metadata.is_file() {
        return metadata.len();
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| directory_size(&entry.path()))
        .fold(0, u64::saturating_add)
}

/// Scan every model directory. A corrupt or orphaned directory stays visible
/// with an explicit error instead of disappearing or crashing startup.
pub fn scan(root: &Path) -> Vec<ScannedModel> {
    let mut models = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return models;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if safe_id(id).is_err() {
            continue;
        }
        let metadata = match read_metadata_sync(&path.join(METADATA_FILE)) {
            Ok(metadata) => metadata,
            Err(error) => {
                let size_bytes = directory_size(&path);
                // A failed download or a deleted file can leave an empty
                // directory; it is not an installed model.
                if !path.join(METADATA_FILE).exists() && size_bytes == 0 {
                    continue;
                }
                models.push(ScannedModel {
                    id: id.to_string(),
                    file: None,
                    size_bytes,
                    installed_at_ms: None,
                    sha256: None,
                    license: None,
                    checksum_verified: false,
                    error: Some(error),
                });
                continue;
            }
        };
        let file_path = path.join(&metadata.file);
        let file_metadata = std::fs::symlink_metadata(&file_path).ok();
        let (size_bytes, error) = match file_metadata {
            Some(file) if file.is_file() && !file.file_type().is_symlink() => {
                if file.len() == metadata.size_bytes {
                    (metadata.size_bytes, None)
                } else {
                    (
                        file.len(),
                        Some("The model file size does not match its metadata.".to_string()),
                    )
                }
            }
            _ => (
                directory_size(&path),
                Some("The model file is missing.".to_string()),
            ),
        };
        let checksum_verified = metadata.checksum_verified && error.is_none();
        let error = if metadata.checksum_verified {
            error
        } else {
            Some("The model checksum was not verified at install.".to_string())
        };
        models.push(ScannedModel {
            id: id.to_string(),
            file: Some(metadata.file),
            size_bytes,
            installed_at_ms: Some(metadata.installed_at_ms),
            sha256: Some(metadata.sha256),
            license: Some(metadata.license),
            checksum_verified,
            error,
        });
    }
    models.sort_by(|left, right| left.id.cmp(&right.id));
    models
}

pub async fn write_metadata(dir: &Path, metadata: &ModelMetadata) -> Result<()> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|_| AppError::Internal)?;
    let bytes = serde_json::to_vec_pretty(metadata).map_err(|_| AppError::Internal)?;
    let temporary = dir.join(format!("{METADATA_FILE}.tmp"));
    {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::File::create(&temporary)
            .await
            .map_err(|_| AppError::Internal)?;
        file.write_all(&bytes)
            .await
            .map_err(|_| AppError::Internal)?;
        file.sync_all().await.map_err(|_| AppError::Internal)?;
    }
    let final_path = dir.join(METADATA_FILE);
    if final_path.exists() {
        let _ = tokio::fs::remove_file(&final_path).await;
    }
    tokio::fs::rename(&temporary, &final_path)
        .await
        .map_err(|_| AppError::Internal)?;
    crate::storage::sync_directory(dir).map_err(|_| AppError::Internal)?;
    Ok(())
}

/// Remove a model directory and report the bytes reclaimed. Callers must have
/// validated selection/confirmation.
pub async fn remove(root: &Path, id: &str) -> Result<u64> {
    let path = model_dir(root, id)?;
    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|_| AppError::NotFound)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::BadRequest("Unknown model.".into()));
    }
    let freed = directory_size(&path);
    tokio::fs::remove_dir_all(&path)
        .await
        .map_err(|_| AppError::Internal)?;
    crate::storage::sync_directory(root).map_err(|_| AppError::Internal)?;
    Ok(freed)
}

pub fn total_size(root: &Path) -> u64 {
    directory_size(root)
}
