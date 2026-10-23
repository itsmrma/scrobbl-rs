//! Portable backups contain only the archive and immutable received files.
//! Activation runs under the instance lock, before opening pools or starting workers.
use crate::{
    auth, db,
    storage::{self, Storage},
};
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const PACKED_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
const EXPANDED_LIMIT: u64 = 8 * 1024 * 1024 * 1024;
pub fn public_error(error: &anyhow::Error) -> &'static str {
    let full = error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|e| matches!(e.raw_os_error(), Some(28 | 112 | 39)))
            || cause.downcast_ref::<sqlx::Error>().is_some_and(
                |e| matches!(e,sqlx::Error::Database(db) if db.code().as_deref()==Some("13")),
            )
    });
    if full {
        "Insufficient disk space. Free space on the server and start a new operation; previous completed copies are retained."
    } else {
        "Recovery operation failed. Check backup integrity and compatibility, received files and available disk space; then start a new operation. Previous completed copies are retained."
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
pub struct InventoryFile {
    pub name: String,
    pub sha256: String,
    pub size_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub app_version: String,
    pub schema_version: i64,
    pub created_at_ms: i64,
    pub instance_id: String,
    pub generation: String,
    pub revision: i64,
    pub counts: BTreeMap<String, i64>,
    pub files: Vec<InventoryFile>,
}
#[derive(Debug, Clone, Serialize, sqlx::FromRow, ts_rs::TS)]
pub struct Asset {
    pub id: String,
    pub job_id: String,
    pub kind: String,
    pub format: String,
    pub state: String,
    pub created_at_ms: i64,
    pub size_bytes: Option<i64>,
    pub manifest_json: Option<String>,
    pub verified_at_ms: Option<i64>,
    pub verification_error: Option<String>,
    pub error: Option<String>,
}
pub fn identifier(id: &str) -> anyhow::Result<()> {
    ensure!(
        id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid recovery identifier"
    );
    Ok(())
}
pub fn asset_path(store: &Storage, asset: &Asset) -> anyhow::Result<PathBuf> {
    identifier(&asset.id)?;
    ensure!(
        matches!(asset.format.as_str(), "zip" | "json" | "csv"),
        "Invalid artifact format"
    );
    Ok(store
        .directory
        .join("backups")
        .join(format!("{}.{}", asset.id, asset.format)))
}
pub async fn connect(path: &Path, readonly: bool) -> anyhow::Result<SqliteConnection> {
    Ok(SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .read_only(readonly)
            .pragma("trusted_schema", "OFF")
            .foreign_keys(true),
    )
    .await?)
}
pub fn hash_file(path: &Path) -> anyhow::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((crate::auth::encode_hex(&hash.finalize()), size))
}
fn valid_member(name: &str) -> bool {
    name == "archive.db"
        || name
            .strip_prefix("spool/")
            .is_some_and(|hash| identifier(hash).is_ok())
}
pub async fn validate_database(
    conn: &mut SqliteConnection,
) -> anyhow::Result<BTreeMap<String, i64>> {
    let integrity: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_all(&mut *conn)
        .await?;
    ensure!(integrity == ["ok"], "Archive integrity check failed");
    ensure!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *conn)
            .await?
            .is_empty(),
        "Broken archive relationships"
    );
    let unsafe_schema:i64=sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type IN ('trigger','view') OR sql LIKE '%VIRTUAL TABLE%'").fetch_one(&mut *conn).await?;
    ensure!(unsafe_schema == 0, "Unsupported executable database schema");
    let mut counts = BTreeMap::new();
    for name in [
        "listens",
        "observations",
        "observation_variants",
        "source_occurrences",
        "source_files",
        "jobs",
        "job_members",
        "job_inputs",
        "calendars",
        "settings",
        "tracks",
        "artists",
        "albums",
        "track_artists",
        "entity_links",
        "local_corrections",
        "entity_groups",
        "group_members",
        "identity_decisions",
        "conflicts",
        "calendar_staging",
        "import_jobs",
    ] {
        let mut query = sqlx::QueryBuilder::new("SELECT COUNT(*) FROM ");
        query.push(name);
        let n: i64 = query.build_query_scalar().fetch_one(&mut *conn).await?;
        counts.insert(name.into(), n);
    }
    // Older portable packages are validated before their supported migrations.
    let shared: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='original_payloads')")
        .fetch_one(&mut *conn).await?;
    if shared {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM original_payloads")
            .fetch_one(&mut *conn)
            .await?;
        counts.insert("original_payloads".into(), n);
        let compact = sqlx::query("PRAGMA table_info(original_payloads)")
            .fetch_all(&mut *conn)
            .await?
            .iter()
            .any(|row| row.get::<String, _>("name") == "fingerprint");
        let pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM settings WHERE key='compact_retention_pending')",
        )
        .fetch_one(&mut *conn)
        .await?;
        if compact && !pending {
            let invalid: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM original_payloads WHERE raw_json!='null' OR fingerprint IS NULL OR length(fingerprint)!=64 OR fingerprint GLOB '*[^0-9a-f]*'")
                .fetch_one(&mut *conn).await?;
            ensure!(invalid == 0, "Invalid compact source fingerprints");
            let unavailable: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM source_files f WHERE retained=0 AND EXISTS(SELECT 1 FROM jobs j WHERE (j.file_id=f.id OR EXISTS(SELECT 1 FROM job_inputs i WHERE i.job_id=j.id AND i.file_id=f.id) OR EXISTS(SELECT 1 FROM job_members m WHERE m.job_id=j.id AND m.file_id=f.id)) AND (j.state!='completed' OR COALESCE(j.outcome,'complete') NOT IN ('complete','no_music'))))")
                .fetch_one(&mut *conn).await?;
            ensure!(!unavailable, "Unfinished import source was released");
        }
    }
    let invalid:i64=sqlx::query_scalar("SELECT COUNT(*) FROM listens l LEFT JOIN observations o ON o.listen_id=l.id WHERE o.id IS NULL OR (l.duration_kind='unknown')!=(l.listened_ms IS NULL)").fetch_one(&mut *conn).await?;
    ensure!(invalid == 0, "Invalid original observations or durations");
    let _: (String, String, i64) = sqlx::query_as(
        "SELECT instance_id,generation,revision FROM archive_state WHERE singleton=1",
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(counts)
}

/// Holding the sole writer connection fixes the database point. Files are immutable and
/// are copied from the inventory in that point, rather than a directory enumeration.
pub async fn freeze(store: &Storage, path: &Path) -> anyhow::Result<()> {
    let mut writer = store.writer.acquire().await?;
    sqlx::query("VACUUM INTO ?")
        .bind(path.to_str().context("Non-UTF8 recovery path")?)
        .execute(&mut *writer)
        .await?;
    drop(writer);
    storage::sync_file(path)?;
    Ok(())
}
async fn sanitized_manifest(snapshot: &Path) -> anyhow::Result<Manifest> {
    let mut conn = connect(snapshot, false).await?;
    // Recovery output files are not included recursively. Their jobs have no music provenance.
    sqlx::query("DELETE FROM recovery_assets")
        .execute(&mut conn)
        .await?;
    sqlx::query("DELETE FROM jobs WHERE kind LIKE 'recovery_%'")
        .execute(&mut conn)
        .await?;
    let identity: (String, String, i64) =
        sqlx::query_as("SELECT instance_id,generation,revision FROM archive_state")
            .fetch_one(&mut conn)
            .await?;
    let schema_version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success=1")
            .fetch_one(&mut conn)
            .await?;
    let counts = validate_database(&mut conn).await?;
    conn.close().await?;
    storage::sync_file(snapshot)?;
    Ok(Manifest {
        format: "scrobbl-backup".into(),
        format_version: 1,
        app_version: env!("CARGO_PKG_VERSION").into(),
        schema_version,
        created_at_ms: auth::now_ms(),
        instance_id: identity.0,
        generation: identity.1,
        revision: identity.2,
        counts,
        files: vec![],
    })
}
pub async fn create_backup(
    store: &Storage,
    work: &Path,
    output: &Path,
) -> anyhow::Result<Manifest> {
    // Pin the frozen database's file inventory through ZIP publication. Imports
    // can still commit while their completed source deletion waits for this copy.
    let _files = store.spool_operations.read().await;
    let snapshot = work.join("archive.db");
    freeze(store, &snapshot).await?;
    let manifest = sanitized_manifest(&snapshot).await?;
    let mut conn = connect(&snapshot, true).await?;
    let files = crate::retention::files(&mut conn).await?;
    conn.close().await?;
    let directory = store.directory.clone();
    let work = work.to_owned();
    let output = output.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut manifest = manifest;
        let mut inputs = vec![("archive.db".to_owned(), work.join("archive.db"), None)];
        for (hash, size) in files {
            identifier(&hash)?;
            ensure!(size >= 0, "Invalid source size");
            inputs.push((
                format!("spool/{hash}"),
                directory.join("spool").join(&hash),
                Some((hash, size as u64)),
            ));
        }
        let mut zip = zip::ZipWriter::new(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?,
        );
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o600);
        for (name, path, expected) in inputs {
            let (sha256, size_bytes) = hash_file(&path)?;
            if let Some((hash, size)) = expected {
                ensure!(
                    hash == sha256 && size == size_bytes,
                    "Received file missing or damaged"
                );
            }
            zip.start_file(&name, options)?;
            std::io::copy(&mut File::open(path)?, &mut zip)?;
            manifest.files.push(InventoryFile {
                name,
                sha256,
                size_bytes,
            });
        }
        zip.start_file("manifest.json", options)?;
        zip.write_all(&serde_json::to_vec(&manifest)?)?;
        let file = zip.finish()?;
        file.sync_all()?;
        Ok::<_, anyhow::Error>(manifest)
    })
    .await?
}

/// Extract only inventoried flat content-addressed files. Never trust ZIP paths or sizes.
fn unpack(package: &Path, work: &Path, materialize_spool: bool) -> anyhow::Result<Manifest> {
    ensure!(
        std::fs::metadata(package)?.len() <= PACKED_LIMIT,
        "Backup exceeds 2 GiB upload limit"
    );
    let mut zip = zip::ZipArchive::new(File::open(package)?)?;
    ensure!(zip.len() <= 100_002, "Too many backup members");
    let manifest: Manifest = {
        let file = zip.by_name("manifest.json")?;
        ensure!(file.size() <= 16 * 1024 * 1024, "Manifest too large");
        serde_json::from_reader(file.take(16 * 1024 * 1024 + 1))?
    };
    ensure!(
        manifest.format == "scrobbl-backup" && manifest.format_version == 1,
        "Unsupported backup format"
    );
    ensure!(
        manifest.files.len() + 1 == zip.len(),
        "Unexpected or duplicate backup entries"
    );
    let latest = storage::ARCHIVE_MIGRATOR
        .iter()
        .map(|m| m.version)
        .max()
        .unwrap_or(0);
    ensure!(
        manifest.schema_version <= latest,
        "Backup requires a newer application"
    );
    let mut inventory = BTreeMap::new();
    let mut total = 0u64;
    for file in &manifest.files {
        ensure!(
            valid_member(&file.name) && identifier(&file.sha256).is_ok(),
            "Invalid backup member"
        );
        total = total
            .checked_add(file.size_bytes)
            .context("Backup too large")?;
        ensure!(
            total <= EXPANDED_LIMIT,
            "Expanded backup exceeds 8 GiB limit"
        );
        ensure!(
            inventory.insert(file.name.clone(), file).is_none(),
            "Duplicate inventory entry"
        );
    }
    ensure!(
        inventory.contains_key("archive.db"),
        "Archive database missing"
    );
    std::fs::create_dir_all(work.join("spool"))?;
    let mut seen = std::collections::BTreeSet::new();
    for index in 0..zip.len() {
        let mut member = zip.by_index(index)?;
        let name = member.name()?.into_owned();
        ensure!(seen.insert(name.clone()), "Duplicate ZIP member");
        if name == "manifest.json" {
            continue;
        }
        let expected = inventory.get(&name).context("Uninventoried member")?;
        ensure!(
            member.size() == expected.size_bytes && !member.is_dir() && !member.is_symlink(),
            "Invalid backup member type or size"
        );
        let mut file = if materialize_spool || name == "archive.db" {
            Some(
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(work.join(&expected.name))?,
            )
        } else {
            None
        };
        // Read every uncompressed byte (including ZIP CRC verification), but
        // creation does not need a second disk copy of immutable source files.
        let mut reader = (&mut member).take(expected.size_bytes + 1);
        let mut hash = Sha256::new();
        let mut size = 0;
        let mut buffer = [0u8; 65536];
        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            size += n as u64;
            hash.update(&buffer[..n]);
            if let Some(file) = &mut file {
                file.write_all(&buffer[..n])?;
            }
        }
        ensure!(size == expected.size_bytes, "Backup member length mismatch");
        if let Some(file) = file {
            file.sync_all()?;
        }
        ensure!(
            crate::auth::encode_hex(&hash.finalize()) == expected.sha256,
            "Backup checksum mismatch"
        );
        if let Some(hash) = expected.name.strip_prefix("spool/") {
            ensure!(hash == expected.sha256, "Invalid content address");
        }
    }
    storage::sync_directory(&work.join("spool"))?;
    storage::sync_directory(work)?;
    Ok(manifest)
}

/// This is a real isolated restore: extraction, SQLite checks, migrations and file relationships.
/// No AppState, authentication store, workers or network clients are created here.
pub async fn trial_restore(package: &Path, work: &Path) -> anyhow::Result<Manifest> {
    check_package(package, work, true).await
}

/// Creation validates the entire package while only materializing its database.
/// Explicit Verify backup and preparation still exercise complete extraction.
pub async fn check_backup(package: &Path, work: &Path) -> anyhow::Result<Manifest> {
    check_package(package, work, false).await
}

async fn check_package(
    package: &Path,
    work: &Path,
    materialize_spool: bool,
) -> anyhow::Result<Manifest> {
    let input = package.to_owned();
    let destination = work.to_owned();
    let manifest =
        tokio::task::spawn_blocking(move || unpack(&input, &destination, materialize_spool))
            .await??;
    let mut conn = connect(&work.join("archive.db"), false).await?;
    let counts = validate_database(&mut conn).await?;
    ensure!(
        counts == manifest.counts,
        "Backup contents do not match manifest"
    );
    let identity: (String, String, i64) =
        sqlx::query_as("SELECT instance_id,generation,revision FROM archive_state")
            .fetch_one(&mut conn)
            .await?;
    ensure!(
        identity
            == (
                manifest.instance_id.clone(),
                manifest.generation.clone(),
                manifest.revision
            ),
        "Backup identity mismatch"
    );
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success=1")
            .fetch_one(&mut conn)
            .await?;
    ensure!(version == manifest.schema_version, "Backup schema mismatch");
    let files = crate::retention::files(&mut conn).await?;
    ensure!(
        files.len() + 1 == manifest.files.len(),
        "Backup file inventory mismatch"
    );
    for (hash, size) in files {
        identifier(&hash)?;
        ensure!(
            manifest
                .files
                .iter()
                .any(|f| f.name == format!("spool/{hash}")
                    && f.sha256 == hash
                    && f.size_bytes == size as u64),
            "Received file missing from backup"
        );
    }
    storage::ARCHIVE_MIGRATOR
        .run_direct(None, &mut conn, false)
        .await?;
    crate::retention::compact_payloads(&mut conn).await?;
    storage::compact_original_payloads(&mut conn).await?;
    validate_database(&mut conn).await?;
    conn.close().await?;
    Ok(manifest)
}

pub async fn assets(store: &Storage) -> anyhow::Result<Vec<Asset>> {
    Ok(
        sqlx::query_as("SELECT a.*,j.error_code AS error FROM recovery_assets a JOIN jobs j ON j.id=a.job_id ORDER BY a.created_at_ms DESC,a.id DESC")
            .fetch_all(&store.readers)
            .await?,
    )
}
pub async fn asset(store: &Storage, id: &str) -> anyhow::Result<Asset> {
    identifier(id)?;
    Ok(sqlx::query_as("SELECT a.*,j.error_code AS error FROM recovery_assets a JOIN jobs j ON j.id=a.job_id WHERE a.id=?")
        .bind(id)
        .fetch_one(&store.readers)
        .await?)
}
pub async fn enqueue(
    store: &Storage,
    kind: &str,
    parameters: serde_json::Value,
) -> anyhow::Result<String> {
    Ok(store.enqueue(kind, None, None, &parameters).await?)
}
pub async fn enqueue_asset(
    store: &Storage,
    kind: &str,
    format: &str,
    parameters: serde_json::Value,
) -> anyhow::Result<String> {
    let id = db::new_id();
    let mut tx = store.writer.begin().await?;
    let job = db::new_id();
    let inserted=sqlx::query("INSERT INTO jobs(id,kind,parameters_json,state,created_at_ms,updated_at_ms) SELECT ?,?,?,'queued',?,? FROM archive_state WHERE maintenance=0").bind(&job).bind(if kind=="export"{"recovery_export"}else{"recovery_backup"}).bind(parameters.to_string()).bind(auth::now_ms()).bind(auth::now_ms()).execute(&mut *tx).await?;
    ensure!(inserted.rows_affected() == 1, "Archive is in maintenance");
    sqlx::query(
        "INSERT INTO recovery_assets(id,job_id,kind,format,created_at_ms) VALUES(?,?,?,?,?)",
    )
    .bind(&id)
    .bind(job)
    .bind(kind)
    .bind(format)
    .bind(auth::now_ms())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}
async fn run(store: &Storage, job: &crate::jobs::Job, work: &Path) -> anyhow::Result<()> {
    let parameters: serde_json::Value = serde_json::from_str(&job.parameters_json)?;
    if job.kind == "recovery_verify" {
        phase(store, &job.id, "isolated_restore").await?;
        let id = parameters["asset_id"]
            .as_str()
            .context("Missing backup identifier")?;
        let asset = asset(store, id).await?;
        ensure!(
            asset.state == "ready" && asset.format == "zip",
            "Backup is unavailable"
        );
        let result = trial_restore(&asset_path(store, &asset)?, work).await;
        sqlx::query("UPDATE recovery_assets SET verified_at_ms=?,verification_error=? WHERE id=?")
            .bind(if result.is_ok() {
                Some(auth::now_ms())
            } else {
                None
            })
            .bind(if result.is_err() {
                Some("Restore trial failed. Check integrity, compatibility and free disk space.")
            } else {
                None
            })
            .bind(id)
            .execute(&store.writer)
            .await?;
        result?;
        return Ok(());
    }
    let asset: Asset = sqlx::query_as("SELECT a.*,j.error_code AS error FROM recovery_assets a JOIN jobs j ON j.id=a.job_id WHERE a.job_id=?")
        .bind(&job.id)
        .fetch_one(&store.readers)
        .await?;
    if asset.state == "ready" {
        return Ok(());
    }
    if job.kind == "recovery_inspect" {
        phase(store, &job.id, "inspecting_backup").await?;
        let path = asset_path(store, &asset)?;
        let manifest = trial_restore(&path, work).await?;
        sqlx::query("UPDATE recovery_assets SET state='ready',manifest_json=? WHERE id=?")
            .bind(serde_json::to_string(&manifest)?)
            .bind(&asset.id)
            .execute(&store.writer)
            .await?;
        return Ok(());
    }
    let output = work.join("output.partial");
    let manifest = if job.kind == "recovery_backup" {
        phase(store, &job.id, "creating_backup").await?;
        let manifest = create_backup(store, work, &output).await?;
        // The package owns the frozen bytes now. Release this extra database before
        // extracting a complete trial, so two expanded databases do not overlap.
        std::fs::remove_file(work.join("archive.db"))?;
        phase(store, &job.id, "checking_backup").await?;
        let check = work.join("creation-check");
        std::fs::create_dir(&check)?;
        check_backup(&output, &check).await?;
        Some(serde_json::to_string(&manifest)?)
    } else {
        phase(store, &job.id, "freezing_archive").await?;
        let selection: crate::exports::ExportSelection = serde_json::from_value(parameters)?;
        let snapshot = work.join("archive.db");
        freeze(store, &snapshot).await?;
        phase(store, &job.id, "writing_export").await?;
        let count = crate::exports::write(&snapshot, &output, &asset.format, &selection).await?;
        Some(json!({"selection":selection,"listens":count,"format_version":crate::exports::FORMAT_VERSION}).to_string())
    };
    let path = asset_path(store, &asset)?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    std::fs::rename(&output, &path)?;
    storage::sync_directory(&store.directory.join("backups"))?;
    sqlx::query("UPDATE recovery_assets SET state='ready',size_bytes=?,manifest_json=? WHERE id=?")
        .bind(i64::try_from(std::fs::metadata(&path)?.len())?)
        .bind(manifest)
        .bind(&asset.id)
        .execute(&store.writer)
        .await?;
    Ok(())
}
async fn phase(store: &Storage, id: &str, phase: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE jobs SET phase=?,updated_at_ms=? WHERE id=?")
        .bind(phase)
        .bind(auth::now_ms())
        .bind(id)
        .execute(&store.writer)
        .await?;
    Ok(())
}
pub async fn worker(store: Storage) {
    loop {
        for kind in [
            "recovery_backup",
            "recovery_export",
            "recovery_verify",
            "recovery_inspect",
        ] {
            if let Ok(Some(job)) = store.claim(kind).await {
                let _operation = store.recovery_operations.lock().await;
                if store.directory.join("recovery.pending").exists() {
                    continue;
                }
                let work = store
                    .directory
                    .join("backups")
                    .join(format!("work-{}", db::new_id()));
                let result = async {
                    std::fs::create_dir(&work)?;
                    sqlx::query("UPDATE jobs SET phase='preparing' WHERE id=?")
                        .bind(&job.id)
                        .execute(&store.writer)
                        .await?;
                    run(&store, &job, &work).await
                }
                .await;
                // Only this freshly generated work directory is removed, never the data root.
                let _ = std::fs::remove_dir_all(&work);
                if result.is_ok() {
                    let _=sqlx::query("UPDATE jobs SET state='completed',phase='finished',outcome='complete',updated_at_ms=? WHERE id=?").bind(auth::now_ms()).bind(&job.id).execute(&store.writer).await;
                } else {
                    let _=sqlx::query("UPDATE recovery_assets SET state='failed' WHERE job_id=? AND state!='ready'").bind(&job.id).execute(&store.writer).await;
                    let message = public_error(result.as_ref().expect_err("failed operation"));
                    let _=sqlx::query("UPDATE jobs SET state='failed',phase='finished',outcome='error',error_code=?,updated_at_ms=? WHERE id=?").bind(message).bind(auth::now_ms()).bind(&job.id).execute(&store.writer).await;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

struct RestorePreparation {
    path: PathBuf,
    published: bool,
}
impl Drop for RestorePreparation {
    fn drop(&mut self) {
        if !self.published {
            // Only the directory created for this attempt, never retained previous data.
            if std::fs::remove_dir_all(&self.path).is_err() {
                eprintln!(
                    "Could not remove an unpublished restore preparation; check storage permissions before retrying."
                );
            }
        }
    }
}
/// Confirmation is authenticated by the destination owner. A fresh verifier is prepared
/// before the journal is published, so recovery never opens unauthenticated setup.
pub async fn prepare_restore(store: &Storage, id: &str, password: String) -> anyhow::Result<()> {
    let _operation = store.recovery_operations.lock().await;
    ensure!(
        !store.directory.join("recovery.pending").exists(),
        "A restore is already awaiting restart"
    );
    ensure!(
        password.chars().count() >= 12 && password.len() <= 1024,
        "Choose a new password of at least 12 characters (maximum 1024 bytes)"
    );
    let asset = asset(store, id).await?;
    ensure!(
        asset.state == "ready" && asset.format == "zip",
        "Choose an available backup"
    );
    let stage_id = db::new_id();
    let stage = store.directory.join(format!("recovery-{stage_id}"));
    std::fs::create_dir(&stage)?;
    let mut preparation = RestorePreparation {
        path: stage.clone(),
        published: false,
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o700))?;
    }
    let new = stage.join("new");
    std::fs::create_dir(&new)?;
    // Until journal publication, failure or cancellation releases this attempt's staging.
    // The live databases remain untouched.
    trial_restore(&asset_path(store, &asset)?, &new).await?;
    let mut conn = connect(&new.join("archive.db"), false).await?;
    sqlx::query("UPDATE archive_state SET maintenance=0,generation=?,retained_from=revision")
        .bind(db::new_id())
        .execute(&mut conn)
        .await?;
    sqlx::query("DELETE FROM changes")
        .execute(&mut conn)
        .await?;
    sqlx::query("INSERT INTO settings(key,value) VALUES('recovery_paused','true') ON CONFLICT(key) DO UPDATE SET value='true'").execute(&mut conn).await?;
    sqlx::query("DELETE FROM settings WHERE key IN ('spotify_error','retry_at_ms','collection_error','now_playing_error','spotify_action_required','spotify_verified_at_ms','last_attempt_ms','last_sync_ms','last_sync_added')").execute(&mut conn).await?;
    conn.close().await?;
    std::fs::create_dir(new.join("private"))?;
    let private_path = new.join("private/auth.db");
    let mut private = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(&private_path)
            .create_if_missing(true),
    )
    .await?;
    storage::PRIVATE_MIGRATOR
        .run_direct(None, &mut private, false)
        .await?;
    let hash = tokio::task::spawn_blocking(move || {
        use argon2::{Argon2, PasswordHasher};
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|v| v.to_string())
            .map_err(|_| anyhow::anyhow!("Password preparation failed"))
    })
    .await??;
    sqlx::query("INSERT INTO owner VALUES(1,?,?)")
        .bind(hash)
        .bind(auth::now_ms())
        .execute(&mut private)
        .await?;
    private.close().await?;
    std::fs::create_dir_all(new.join("spool/incoming"))?;
    std::fs::create_dir(new.join("snapshots"))?;
    std::fs::create_dir(stage.join("previous"))?;
    for path in [new.join("archive.db"), private_path] {
        storage::sync_file(&path)?;
    }
    storage::sync_directory(&new.join("private"))?;
    storage::sync_directory(&new)?;
    storage::sync_directory(&stage)?;
    // Preserve the destination's saved-copy catalogue: those files are outside the
    // replaced archive, and were deliberately excluded from the portable package.
    let mut conn = connect(&new.join("archive.db"), false).await?;
    for saved in assets(store)
        .await?
        .into_iter()
        .filter(|a| a.state != "pending")
    {
        let state = if saved.state == "ready" {
            "completed"
        } else {
            "failed"
        };
        sqlx::query("INSERT OR IGNORE INTO jobs(id,kind,state,phase,created_at_ms,updated_at_ms,error_code) VALUES(?,'recovery_retained',?,'finished',?,?,?)").bind(&saved.job_id).bind(state).bind(saved.created_at_ms).bind(auth::now_ms()).bind(saved.error).execute(&mut conn).await?;
        sqlx::query("INSERT INTO recovery_assets(id,job_id,kind,format,state,created_at_ms,size_bytes,manifest_json,verified_at_ms,verification_error) VALUES(?,?,?,?,?,?,?,?,?,?)").bind(saved.id).bind(saved.job_id).bind(saved.kind).bind(saved.format).bind(saved.state).bind(saved.created_at_ms).bind(saved.size_bytes).bind(saved.manifest_json).bind(saved.verified_at_ms).bind(saved.verification_error).execute(&mut conn).await?;
    }
    conn.close().await?;
    storage::sync_file(&new.join("archive.db"))?;
    let marker = stage.join("ready");
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&marker)?;
    file.write_all(stage_id.as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&marker, store.directory.join("recovery.pending"))?;
    // Once published, preserve the resources even if a later sync/barrier fails.
    preparation.published = true;
    storage::sync_directory(&stage)?;
    storage::sync_directory(&store.directory)?;
    // From here startup can finish even if the process dies before the barrier commits.
    // Middleware refuses writes immediately when the durable marker exists.
    store.maintenance(true).await?;
    Ok(())
}

/// Idempotent rename protocol. Each old resource is kept under previous/. Moving the new
/// resource consumes its source and serves as its durable per-resource completion record.
pub async fn activate_pending(directory: &Path) -> anyhow::Result<()> {
    let marker = directory.join("recovery.pending");
    if !marker.exists() {
        return Ok(());
    }
    let id = std::fs::read_to_string(&marker)?;
    identifier(&id)?;
    let stage = directory.join(format!("recovery-{id}"));
    let new = stage.join("new");
    let previous = stage.join("previous");
    ensure!(
        previous.is_dir() && new.is_dir(),
        "Restore preparation is incomplete; retain the data directory and follow offline recovery instructions"
    );
    for name in [
        "archive.db-wal",
        "archive.db-shm",
        "archive.db",
        "private",
        "spool",
        "snapshots",
    ] {
        let active = directory.join(name);
        let old = previous.join(name);
        let replacement = new.join(name);
        let sidecar = name.ends_with("-wal") || name.ends_with("-shm");
        if replacement.exists() || sidecar {
            if active.exists() && !old.exists() {
                std::fs::rename(&active, &old)?;
                storage::sync_directory(&previous)?;
                storage::sync_directory(directory)?;
            }
            if replacement.exists() {
                ensure!(
                    !active.exists(),
                    "Restore activation conflict; manual recovery required"
                );
                std::fs::rename(replacement, &active)?;
                storage::sync_directory(&new)?;
                storage::sync_directory(directory)?;
            }
        } else {
            ensure!(
                active.exists(),
                "Restore activation lost a required resource; manual recovery required"
            );
        }
    }
    // Databases and all required files are now active. Old data and access storage remain
    // private on the destination, never in the downloadable backup.
    std::fs::rename(&marker, stage.join("activated"))?;
    storage::sync_directory(&stage)?;
    storage::sync_directory(directory)?;
    Ok(())
}

pub async fn paused(store: &Storage) -> bool {
    match crate::db::setting(&store.readers, "recovery_paused").await {
        Ok(value) => value.as_deref() == Some("true"),
        Err(_) => true,
    }
}

/// Offline recovery writes a NEW directory, never overwrites the failed installation.
/// Run using the image that performs recovery, then start the recorded previous image.
pub async fn recover_upgrade(
    directory: &Path,
    copy: &str,
    destination: &Path,
) -> anyhow::Result<()> {
    use fs2::FileExt;
    let suffix = copy
        .strip_prefix("pre-upgrade-")
        .context("Choose a pre-upgrade copy")?;
    identifier(suffix)?;
    let directory = directory.canonicalize()?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join("instance.lock"))?;
    lock.try_lock_exclusive()
        .context("Stop the application before offline recovery")?;
    ensure!(
        !destination.exists(),
        "Recovery destination must be a new directory; existing data is never overwritten"
    );
    let source = directory.join("backups").join(copy);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(source.join("complete.json"))?)?;
    ensure!(
        manifest["format"] == 1 && manifest["purpose"] == "pre-upgrade-private",
        "Not a completed pre-upgrade copy"
    );
    std::fs::create_dir_all(destination)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o700))?;
    }
    // A failed attempt is visibly incomplete; do not start it until recovery.complete exists.
    std::fs::write(
        destination.join("recovery.incomplete"),
        b"Offline upgrade recovery incomplete",
    )?;
    std::fs::create_dir(destination.join("private"))?;
    std::fs::create_dir(destination.join("spool"))?;
    for (name, target) in [
        ("archive.db", destination.join("archive.db")),
        ("auth.db", destination.join("private/auth.db")),
    ] {
        std::fs::copy(source.join(name), &target)?;
        if let Some(expected) = manifest["database_sha256"][name].as_str() {
            ensure!(
                hash_file(&target)?.0 == expected,
                "Pre-upgrade database checksum mismatch"
            );
        }
        let mut conn = connect(&target, true).await?;
        let integrity: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_all(&mut conn)
            .await?;
        ensure!(
            integrity == ["ok"],
            "Pre-upgrade database integrity check failed"
        );
        conn.close().await?;
        storage::sync_file(&target)?;
    }
    for entry in manifest["files"]
        .as_array()
        .context("Missing pre-upgrade file inventory")?
    {
        let hash = entry["sha256"]
            .as_str()
            .context("Missing received file hash")?;
        identifier(hash)?;
        let size = entry["size_bytes"]
            .as_u64()
            .context("Missing received file size")?;
        let target = destination.join("spool").join(hash);
        let spool = if manifest["spool"] == "snapshot" {
            source.join("spool")
        } else {
            directory.join("spool")
        };
        std::fs::copy(spool.join(hash), &target)?;
        let actual = hash_file(&target)?;
        ensure!(
            actual == (hash.to_owned(), size),
            "Pre-upgrade received file integrity check failed"
        );
        storage::sync_file(&target)?;
    }
    let complete = destination.join("recovery.complete");
    std::fs::write(&complete, serde_json::to_vec(&manifest)?)?;
    storage::sync_file(&complete)?;
    for folder in [
        destination.join("private"),
        destination.join("spool"),
        destination.to_owned(),
    ] {
        storage::sync_directory(&folder)?;
    }
    std::fs::remove_file(destination.join("recovery.incomplete"))?;
    storage::sync_directory(destination)?;
    println!(
        "Recovered into {}. Start the PREVIOUS pinned image/version using this new directory. Retain the failed directory. Source application: {}",
        destination.display(),
        manifest
            .get("source_app")
            .unwrap_or(&serde_json::Value::Null)
    );
    Ok(())
}
