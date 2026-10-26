//! Filesystem ownership and startup. Never opens or migrates the personal prototype.
use anyhow::{Context, ensure};
use fs2::FileExt;
use sqlx::{
    Connection, Row, SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

pub static ARCHIVE_MIGRATOR: Migrator = sqlx::migrate!("./release-migrations");
pub(crate) static PRIVATE_MIGRATOR: Migrator = sqlx::migrate!("./private-migrations");

#[derive(Clone)]
pub struct Storage {
    pub writer: SqlitePool,
    pub readers: SqlitePool,
    pub private: SqlitePool,
    pub directory: PathBuf,
    lock: Arc<Mutex<Option<File>>>,
    pub(crate) snapshots: Arc<tokio::sync::Mutex<()>>,
    pub(crate) requests: Arc<tokio::sync::RwLock<()>>,
    pub(crate) recovery_operations: Arc<tokio::sync::Mutex<()>>,
    pub(crate) spool_operations: Arc<tokio::sync::RwLock<()>>,
}

fn private_directory(path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub async fn open(directory: &Path) -> anyhow::Result<Storage> {
    ensure!(
        !directory.join("recovery.incomplete").exists(),
        "Offline recovery is incomplete. Do not start this directory; retry recovery into a new destination."
    );
    ensure!(
        !directory.join("scrobbl.db").exists(),
        "Prototype database detected. Use a new SCROBBL_DATA_DIR; no prototype migration is supported."
    );
    private_directory(directory)?;
    let directory = directory.canonicalize()?;
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("instance.lock"))?;
    lock_file
        .try_lock_exclusive()
        .context("Another process owns this data directory")?;
    crate::recovery::activate_pending(&directory).await?;
    for child in [
        "private",
        "spool",
        "spool/incoming",
        "snapshots",
        "cache",
        "backups",
    ] {
        private_directory(&directory.join(child))?;
    }
    let archive_path = directory.join("archive.db");
    let private_path = directory.join("private/auth.db");
    let existing = archive_path.exists();
    // If either half of an existing release is missing, do not silently start public setup.
    ensure!(
        archive_path.exists() == private_path.exists(),
        "Incomplete data directory: recover the missing archive/private database before starting"
    );
    let options = options(&archive_path);
    // A bounded import batch touches several indexes. Keep its working pages in
    // one modest writer cache and checkpoint less often; FULL durability stays on.
    let writer = pool(
        options
            .clone()
            .pragma("cache_size", "-131072")
            .pragma("journal_size_limit", "8388608")
            .pragma("wal_autocheckpoint", "16384"),
        1,
    )
    .await?;
    let private = match pool(options_for_private(&private_path), 1).await {
        Ok(pool) => pool,
        Err(error) => {
            writer.close().await;
            return Err(error);
        }
    };
    let result = async {
        let version: String = sqlx::query_scalar("SELECT sqlite_version()").fetch_one(&writer).await?;
        let parts: Vec<u32> = version.split('.').map(str::parse).collect::<Result<_,_>>()?;
        ensure!(parts.as_slice() >= [3,51,3].as_slice(), "SQLite {version} does not include the required WAL fix");
        if existing && table_exists(&writer,"archive_state").await? {
            ensure!(table_exists(&private,"owner").await?,"Private archive is incomplete; recover access storage before starting");
            let maintenance:bool=sqlx::query_scalar("SELECT maintenance FROM archive_state WHERE singleton=1").fetch_one(&writer).await?;
            ensure!(!maintenance,"Archive is in maintenance; complete recovery before migration/startup");
        }
        // No workers exist yet and the instance lock excludes other app processes.
        let pending = has_pending(&writer, &ARCHIVE_MIGRATOR).await? || has_pending(&private, &PRIVATE_MIGRATOR).await?;
        if pending && table_exists(&writer, "archive_state").await? {
            pre_upgrade_copy(&directory, &writer, &private).await?;
        }
        ARCHIVE_MIGRATOR.run(&writer).await?;
        {
            let mut conn = writer.acquire().await?;
            crate::retention::compact_payloads(&mut conn).await?;
            compact_original_payloads(&mut conn).await?;
        }
        PRIVATE_MIGRATOR.run(&private).await?;
        sqlx::query("INSERT INTO settings(key,value) VALUES('app_build',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value").bind(serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"revision":option_env!("SCROBBL_BUILD_REF").unwrap_or("development")}).to_string()).execute(&writer).await?;
        recover_access(&writer, &private).await?;
        sqlx::query("UPDATE jobs SET state='failed',outcome='error',error_code='upload_incomplete',retry_at_ms=NULL WHERE kind='spotify_import' AND phase='receiving' AND cancel_requested=0").execute(&writer).await?;
        sqlx::query("UPDATE jobs SET state='failed',phase='finished',outcome='error',error_code='Restore preparation interrupted. Confirm restore again with a new password.' WHERE kind='recovery_restore' AND state IN ('queued','running','waiting') AND cancel_requested=0").execute(&writer).await?;
        sqlx::query("UPDATE jobs SET state=CASE WHEN cancel_requested=1 THEN 'cancelled' ELSE 'queued' END, updated_at_ms=? WHERE state='running'")
            .bind(crate::auth::now_ms()).execute(&writer).await?;
        sqlx::query("UPDATE jobs SET state='cancelled' WHERE cancel_requested=1 AND state IN ('queued','waiting')").execute(&writer).await?;
        let maintenance: bool = sqlx::query_scalar("SELECT maintenance FROM archive_state WHERE singleton=1").fetch_one(&writer).await?;
        ensure!(!maintenance, "Archive is in maintenance; complete recovery before starting workers");
        Ok::<_,anyhow::Error>(())
    }.await;
    if let Err(error) = result {
        writer.close().await;
        private.close().await;
        return Err(error);
    }
    let readers = match pool(options.read_only(true).pragma("query_only", "ON"), 4).await {
        Ok(pool) => pool,
        Err(error) => {
            writer.close().await;
            private.close().await;
            return Err(error);
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for base in [&archive_path, &private_path] {
            for suffix in ["", "-wal", "-shm"] {
                let path = PathBuf::from(format!("{}{suffix}", base.display()));
                if path.exists() {
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
                }
            }
        }
    }
    Ok(Storage {
        writer,
        readers,
        private,
        directory,
        lock: Arc::new(Mutex::new(Some(lock_file))),
        snapshots: Default::default(),
        requests: Default::default(),
        recovery_operations: Default::default(),
        spool_operations: Default::default(),
    })
}

/// Release the old wide rows' free pages once, before starting any readers/workers.
/// Keep the marker until VACUUM succeeds so an interrupted upgrade retries it.
pub(crate) async fn compact_original_payloads(
    conn: &mut sqlx::SqliteConnection,
) -> anyhow::Result<()> {
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM settings WHERE key='original_payload_compaction_pending')",
    )
    .fetch_one(&mut *conn)
    .await?;
    if pending {
        sqlx::query("VACUUM").execute(&mut *conn).await?;
        sqlx::query("DELETE FROM settings WHERE key='original_payload_compaction_pending'")
            .execute(&mut *conn)
            .await?;
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

fn options(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .busy_timeout(Duration::from_secs(30))
        .pragma("cache_size", "-8192")
}
fn options_for_private(path: &Path) -> SqliteConnectOptions {
    options(path).pragma("secure_delete", "ON")
}
async fn pool(options: SqliteConnectOptions, size: u32) -> anyhow::Result<SqlitePool> {
    Ok(SqlitePoolOptions::new()
        .max_connections(size)
        .acquire_timeout(Duration::from_secs(120))
        .connect_with(options)
        .await?)
}
async fn table_exists(pool: &SqlitePool, name: &str) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
    )
    .bind(name)
    .fetch_one(pool)
    .await?)
}
async fn has_pending(pool: &SqlitePool, migrator: &Migrator) -> anyhow::Result<bool> {
    if !table_exists(pool, "_sqlx_migrations").await? {
        return Ok(true);
    }
    let applied: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success=1")
            .fetch_all(pool)
            .await?;
    Ok(migrator.iter().any(|m| !applied.contains(&m.version)))
}

/// Startup-only safety copy, including private credentials. Not the user-facing backup format.
async fn pre_upgrade_copy(
    directory: &Path,
    archive: &SqlitePool,
    private: &SqlitePool,
) -> anyhow::Result<()> {
    let destination = directory
        .join("backups")
        .join(format!("pre-upgrade-{}", crate::auth::random_secret()));
    private_directory(&destination)?;
    for (name, pool) in [("archive.db", archive), ("auth.db", private)] {
        let target = destination.join(name);
        sqlx::query("VACUUM INTO ?")
            .bind(target.to_str().context("Non-UTF8 backup path")?)
            .execute(pool)
            .await?;
        let mut verification = sqlx::SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(&target)
                .read_only(true),
        )
        .await?;
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&mut verification)
            .await?;
        ensure!(
            integrity == "ok",
            "Pre-upgrade copy failed integrity verification"
        );
        verification.close().await?;
        sync_file(&target)?;
    }
    let mut files = Vec::new();
    let spool = destination.join("spool");
    private_directory(&spool)?;
    for (hash, size) in crate::retention::files(&mut *archive.acquire().await?).await? {
        ensure!(
            std::fs::metadata(directory.join("spool").join(&hash))?.len() == size as u64,
            "Spool file missing or incomplete before upgrade"
        );
        ensure!(
            crate::recovery::hash_file(&directory.join("spool").join(&hash))?.0 == hash,
            "Spool checksum mismatch before upgrade"
        );
        // A completed import can now release its live source. Keep this technical
        // copy independently recoverable; immutable hard links add no data blocks.
        let source = directory.join("spool").join(&hash);
        let target = spool.join(&hash);
        if std::fs::hard_link(&source, &target).is_err() {
            std::fs::copy(&source, &target)?;
        }
        sync_file(&target)?;
        files.push(serde_json::json!({"sha256":hash,"size_bytes":size}));
    }
    sync_directory(&spool)?;
    let archive_schema: Option<i64> =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success=1")
            .fetch_one(archive)
            .await?;
    let private_schema: Option<i64> =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success=1")
            .fetch_one(private)
            .await?;
    let content = serde_json::to_vec(
        &serde_json::json!({"format":1,"purpose":"pre-upgrade-private","source_app":crate::db::setting(archive,"app_build").await?,"target_app_version":env!("CARGO_PKG_VERSION"),"target_revision":option_env!("SCROBBL_BUILD_REF").unwrap_or("development"),"created_at_ms":crate::auth::now_ms(),"archive_schema":archive_schema,"private_schema":private_schema,"spool":"snapshot","files":files,"database_sha256":{"archive.db":crate::recovery::hash_file(&destination.join("archive.db"))?.0,"auth.db":crate::recovery::hash_file(&destination.join("auth.db"))?.0}}),
    )?;
    let manifest = destination.join("complete.json");
    std::fs::write(&manifest, content)?;
    sync_file(&manifest)?;
    sync_directory(&destination)?;
    sync_directory(&directory.join("backups"))?;
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
pub(crate) fn sync_file(path: &Path) -> std::io::Result<()> {
    // FlushFileBuffers on Windows requires a handle opened for writing.
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .sync_all()
}

pub async fn recover_access(archive: &SqlitePool, private: &SqlitePool) -> anyhow::Result<()> {
    for row in sqlx::query("SELECT id FROM access_operations WHERE completed=0")
        .fetch_all(private)
        .await?
    {
        sqlx::query(crate::spotify::RESET_STATUS_SQL)
            .execute(archive)
            .await?;
        sqlx::query("UPDATE access_operations SET completed=1 WHERE id=?")
            .bind(row.get::<String, _>("id"))
            .execute(private)
            .await?;
    }
    Ok(())
}

impl Storage {
    /// Recovery owns the surrounding drain/close protocol; this is the durable write barrier.
    pub async fn maintenance(&self, enabled: bool) -> anyhow::Result<()> {
        sqlx::query("UPDATE archive_state SET maintenance=? WHERE singleton=1")
            .bind(enabled)
            .execute(&self.writer)
            .await?;
        Ok(())
    }
    /// Recovery calls this while the durable maintenance barrier is held.
    pub async fn rotate_generation(&self) -> anyhow::Result<()> {
        let mut tx = self.writer.begin().await?;
        let result=sqlx::query("UPDATE archive_state SET generation=?,retained_from=revision WHERE singleton=1 AND maintenance=1")
            .bind(crate::db::new_id()).execute(&mut *tx).await?;
        ensure!(
            result.rows_affected() == 1,
            "Generation can change only in maintenance"
        );
        sqlx::query("DELETE FROM changes").execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    /// Call only after requests and workers have drained. Releasing clones cannot unlock early.
    pub async fn close(&self) {
        self.readers.close().await;
        self.writer.close().await;
        self.private.close().await;
        self.lock.lock().expect("instance lock poisoned").take();
    }
}
