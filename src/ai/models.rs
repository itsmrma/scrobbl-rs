//! Installed model files. The filesystem is the source of truth: every model
//! directory carries its own metadata beside the GGUF file (or a valid GGUF
//! file directly), and the database stores consent, selection, and optional
//! custom local GGUF path.
use crate::{
    AppState,
    error::{AppError, Result},
};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub const METADATA_FILE: &str = "metadata.json";
const GGUF_MAGIC: &[u8; 4] = b"GGUF";
const MIN_GGUF_BYTES: u64 = 64;

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
    pub custom: bool,
    pub error: Option<String>,
}

impl ScannedModel {
    pub fn usable(&self) -> bool {
        self.error.is_none() && (self.checksum_verified || self.custom)
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

/// Verify that a path points to a readable, non-symlink regular `.gguf` file
/// starting with the `GGUF` magic header.
pub fn validate_gguf_file(path: &Path) -> std::result::Result<u64, String> {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_none_or(|ext| !ext.eq_ignore_ascii_case("gguf"))
    {
        return Err("Custom model file must end with .gguf.".to_string());
    }
    let meta = std::fs::symlink_metadata(path)
        .map_err(|_| "The specified model file was not found.".to_string())?;
    if meta.file_type().is_symlink() {
        return Err("Refusing to load a model through a symbolic link.".to_string());
    }
    if !meta.is_file() {
        return Err("The specified model path is not a regular file.".to_string());
    }
    if meta.len() < MIN_GGUF_BYTES {
        return Err("The specified .gguf file is too small to be a valid GGUF model.".to_string());
    }
    let mut file = std::fs::File::open(path)
        .map_err(|_| "Could not open the specified model file for reading.".to_string())?;
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic)
        .map_err(|_| "The specified file is not a valid GGUF model.".to_string())?;
    if &magic != GGUF_MAGIC {
        return Err("The specified file does not start with a valid GGUF header.".to_string());
    }
    Ok(meta.len())
}

/// Validate and resolve a custom local `.gguf` path entered by the owner.
/// Relative paths are resolved under `root` (`ai_model_dir`); `..` components,
/// control characters, NUL bytes, and non-`.gguf` extensions are rejected.
pub fn resolve_custom_model_path(root: &Path, raw: &str) -> std::result::Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Custom model path cannot be empty.".to_string());
    }
    if trimmed.chars().count() > 1024 || trimmed.chars().any(|c| c == '\0' || c.is_control()) {
        return Err("Use a valid local .gguf file path.".to_string());
    }
    let candidate = Path::new(trimmed);
    if candidate
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("Custom model path cannot contain parent directory (..) segments.".to_string());
    }
    if candidate
        .extension()
        .and_then(|ext| ext.to_str())
        .is_none_or(|ext| !ext.eq_ignore_ascii_case("gguf"))
    {
        return Err("Custom model file must end with .gguf.".to_string());
    }
    Ok(if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    })
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

fn is_partial_or_temporary(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.ends_with(".part") || name.ends_with(".tmp")
}

fn is_gguf_filename(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
}

fn is_allowed_model_filename(name: &str) -> bool {
    name == METADATA_FILE
        || is_gguf_filename(name)
        || name.ends_with(".part")
        || name.ends_with(".tmp")
}

/// Inspect a subdirectory without `metadata.json`.
/// Returns:
/// - `None` when the directory contains no `.gguf` files (arbitrary host folder or partial download only),
/// - `Some(Ok((file_name, bytes)))` when it holds a single valid `.gguf` model,
/// - `Some(Err((size_bytes, msg)))` when `.gguf` files are present but invalid/corrupt.
fn scan_unmanaged_gguf_dir(
    dir: &Path,
) -> Option<std::result::Result<(String, u64), (u64, String)>> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut gguf_files = Vec::new();
    let mut has_other_files = false;
    for entry in entries.flatten() {
        let entry_path = entry.path();
        let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_partial_or_temporary(&entry_path) {
            continue;
        }
        if is_gguf_filename(name) {
            gguf_files.push((name.to_string(), entry_path));
        } else {
            has_other_files = true;
        }
    }
    if gguf_files.is_empty() {
        // Neither metadata.json nor any .gguf file: ignore unrelated host directories.
        return None;
    }
    if gguf_files.len() > 1 || has_other_files {
        return Some(Err((
            directory_size(dir),
            "Model directory has missing metadata or unexpected files.".to_string(),
        )));
    }
    let (file_name, gguf_path) = gguf_files.into_iter().next()?;
    match validate_gguf_file(&gguf_path) {
        Ok(bytes) => Some(Ok((file_name, bytes))),
        Err(reason) => Some(Err((directory_size(dir), reason))),
    }
}

/// Scan every model directory. Directories containing neither `metadata.json`
/// nor a `.gguf` file are skipped so unrelated host directories under
/// `ai_model_dir` are never enumerated or exposed.
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
        let meta_path = path.join(METADATA_FILE);
        if !meta_path.exists() {
            match scan_unmanaged_gguf_dir(&path) {
                None => continue,
                Some(Ok((file, size_bytes))) => {
                    models.push(ScannedModel {
                        id: id.to_string(),
                        file: Some(file),
                        size_bytes,
                        installed_at_ms: None,
                        sha256: None,
                        license: None,
                        checksum_verified: false,
                        custom: true,
                        error: None,
                    });
                }
                Some(Err((size_bytes, reason))) => {
                    models.push(ScannedModel {
                        id: id.to_string(),
                        file: None,
                        size_bytes,
                        installed_at_ms: None,
                        sha256: None,
                        license: None,
                        checksum_verified: false,
                        custom: false,
                        error: Some(reason),
                    });
                }
            }
            continue;
        }
        let metadata = match read_metadata_sync(&meta_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                let size_bytes = directory_size(&path);
                models.push(ScannedModel {
                    id: id.to_string(),
                    file: None,
                    size_bytes,
                    installed_at_ms: None,
                    sha256: None,
                    license: None,
                    checksum_verified: false,
                    custom: false,
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
            custom: false,
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

/// Remove a model directory and report the bytes reclaimed. Refuses to delete
/// symlinks, nested subdirectories, or any file not belonging to a model
/// (`metadata.json`, `*.gguf`, `*.part`, `*.tmp`).
pub async fn remove(root: &Path, id: &str) -> Result<u64> {
    let path = model_dir(root, id)?;
    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|_| AppError::NotFound)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::BadRequest("Unknown model.".into()));
    }
    let mut entries = tokio::fs::read_dir(&path)
        .await
        .map_err(|_| AppError::Internal)?;
    let mut files_to_remove = Vec::new();
    let mut freed = 0_u64;
    while let Some(entry) = entries.next_entry().await.map_err(|_| AppError::Internal)? {
        let entry_path = entry.path();
        let entry_meta = tokio::fs::symlink_metadata(&entry_path)
            .await
            .map_err(|_| AppError::Internal)?;
        if entry_meta.file_type().is_symlink() || entry_meta.is_dir() {
            return Err(AppError::Conflict(
                "Refusing to delete a model directory containing subdirectories or links.".into(),
            ));
        }
        let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) else {
            return Err(AppError::Conflict(
                "Refusing to delete directory containing non-model files.".into(),
            ));
        };
        if !is_allowed_model_filename(name) {
            return Err(AppError::Conflict(
                "Refusing to delete directory containing non-model files.".into(),
            ));
        }
        freed = freed.saturating_add(entry_meta.len());
        files_to_remove.push(entry_path);
    }
    for file_path in files_to_remove {
        tokio::fs::remove_file(&file_path)
            .await
            .map_err(|_| AppError::Internal)?;
    }
    tokio::fs::remove_dir(&path)
        .await
        .map_err(|_| AppError::Internal)?;
    crate::storage::sync_directory(root).map_err(|_| AppError::Internal)?;
    Ok(freed)
}

pub fn total_size(root: &Path) -> u64 {
    directory_size(root)
}
