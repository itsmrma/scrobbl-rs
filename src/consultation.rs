//! Shared coherent read and revision primitives. Public wire schemas belong to the API module.
use crate::{
    error::{AppError, Result},
    storage::Storage,
};
use serde::Serialize;
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use sqlx::{FromRow, Sqlite, Transaction};
use std::{path::Path, time::Duration};

#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct Identity {
    pub instance_id: String,
    pub generation: String,
    pub revision: i64,
    pub retained_from: i64,
}
pub struct ReadView {
    pub identity: Identity,
    pub transaction: Transaction<'static, Sqlite>,
}
#[derive(Debug, Serialize, FromRow)]
pub struct Change {
    pub revision: i64,
    pub listen_id: String,
    pub operation: String,
    pub replacement_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Listen {
    pub id: String,
    pub track_id: String,
    pub played_at_ms: i64,
    pub first_acquired_at_ms: i64,
    pub calendar_id: String,
    pub local_date: String,
    pub week_date: String,
    pub listened_ms: Option<i64>,
    pub duration_kind: String,
    pub counted: bool,
    pub needs_review: bool,
    pub trashed_at_ms: Option<i64>,
    pub revision: i64,
}
/// Internal paging position; the external API is responsible for signing/encoding its cursor.
pub struct Position {
    pub played_at_ms: i64,
    pub id: String,
}
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub instance_id: String,
    pub generation: String,
    pub revision: i64,
    pub expires_at_ms: i64,
}
pub struct SnapshotLimits {
    pub max_copies: usize,
    pub max_total_bytes: u64,
    pub ttl: Duration,
    pub preparation_timeout: Duration,
}
impl Default for SnapshotLimits {
    fn default() -> Self {
        Self {
            max_copies: 4,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            ttl: Duration::from_secs(900),
            preparation_timeout: Duration::from_secs(60),
        }
    }
}
impl Storage {
    /// A bounded server-side operation may share this transaction; never hold it for client paging.
    pub async fn read_view(&self) -> Result<ReadView> {
        let mut transaction = self.readers.begin().await?;
        let identity=sqlx::query_as("SELECT instance_id,generation,revision,retained_from FROM archive_state WHERE singleton=1").fetch_one(&mut *transaction).await?;
        Ok(ReadView {
            identity,
            transaction,
        })
    }
    pub async fn changes(&self, cursor: &Identity, limit: i64) -> Result<(Identity, Vec<Change>)> {
        let mut view = self.read_view().await?;
        if cursor.instance_id != view.identity.instance_id
            || cursor.generation != view.identity.generation
            || cursor.revision < view.identity.retained_from
            || cursor.revision > view.identity.revision
        {
            return Err(AppError::Conflict(
                "Cursor incompatible; create a complete copy.".into(),
            ));
        }
        // A revision is indivisible; the soft limit never truncates its records.
        let changes:Vec<Change>=sqlx::query_as("SELECT * FROM changes WHERE revision>? AND revision <= COALESCE((SELECT revision FROM changes WHERE revision>? ORDER BY revision,listen_id LIMIT 1 OFFSET ?),?) ORDER BY revision,listen_id")
            .bind(cursor.revision).bind(cursor.revision).bind(limit.clamp(1,10000)-1).bind(view.identity.revision).fetch_all(&mut *view.transaction).await?;
        if let Some(last) = changes.last() {
            view.identity.revision = last.revision;
        }
        Ok((view.identity, changes))
    }
    pub async fn prune_changes(&self, through: i64) -> Result<()> {
        let mut tx = self.writer.begin().await?;
        let revision:Option<i64>=sqlx::query_scalar("UPDATE archive_state SET retained_from=? WHERE singleton=1 AND retained_from<=? AND revision>=? RETURNING revision")
            .bind(through).bind(through).bind(through).fetch_optional(&mut *tx).await?;
        if revision.is_none() {
            return Err(AppError::BadRequest(
                "Invalid change retention boundary.".into(),
            ));
        }
        sqlx::query("DELETE FROM changes WHERE revision<=?")
            .bind(through)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Frozen SQLite copies live on the server, not as an indefinitely pinned live transaction.
    /// These are internal assets: callers read projections, never serve the database file.
    pub async fn materialize(&self, limits: &SnapshotLimits) -> anyhow::Result<Snapshot> {
        let _guard = self.snapshots.lock().await;
        let root = self.directory.join("snapshots");
        let mut count = 0usize;
        let mut used = 0u64;
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("partial") {
                // The materialization mutex guarantees this is an abandoned preparation.
                std::fs::remove_file(path)?;
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("db") {
                continue;
            }
            match snapshot_metadata(&path).await {
                Ok(meta) if meta.expires_at_ms > crate::auth::now_ms() => {
                    count += 1;
                    used += entry.metadata()?.len();
                }
                _ => std::fs::remove_file(path)?,
            }
        }
        anyhow::ensure!(count < limits.max_copies, "Snapshot count quota exceeded");
        let bytes: i64 = sqlx::query_scalar(
            "SELECT page_count*page_size FROM pragma_page_count(),pragma_page_size()",
        )
        .fetch_one(&self.readers)
        .await?;
        anyhow::ensure!(
            used.saturating_add(bytes as u64) <= limits.max_total_bytes,
            "Snapshot byte quota exceeded"
        );
        let id = crate::db::new_id();
        let partial = root.join(format!("{id}.partial"));
        let result=tokio::time::timeout(limits.preparation_timeout,async {
            let mut source=SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(self.directory.join("archive.db")).read_only(true)).await?;
            sqlx::query("VACUUM INTO ?").bind(partial.to_str().ok_or_else(||anyhow::anyhow!("Non-UTF8 snapshot path"))?).execute(&mut source).await?;
            source.close().await?;
            let mut frozen=SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&partial)).await?;
            let identity:Identity=sqlx::query_as("SELECT instance_id,generation,revision,retained_from FROM archive_state WHERE singleton=1").fetch_one(&mut frozen).await?;
            let snapshot=Snapshot{id:id.clone(),instance_id:identity.instance_id,generation:identity.generation,revision:identity.revision,
                expires_at_ms:crate::auth::now_ms().checked_add(i64::try_from(limits.ttl.as_millis())?).ok_or_else(||anyhow::anyhow!("Invalid TTL"))?};
            sqlx::query("CREATE TABLE snapshot_metadata(value TEXT NOT NULL)").execute(&mut frozen).await?;
            sqlx::query("INSERT INTO snapshot_metadata VALUES(?)").bind(serde_json::to_string(&snapshot)?).execute(&mut frozen).await?;
            let integrity:String=sqlx::query_scalar("PRAGMA quick_check").fetch_one(&mut frozen).await?;
            anyhow::ensure!(integrity=="ok","Incomplete snapshot");
            frozen.close().await?;
            anyhow::ensure!(used.saturating_add(std::fs::metadata(&partial)?.len())<=limits.max_total_bytes,"Snapshot byte quota exceeded");
            crate::storage::sync_file(&partial)?;
            std::fs::rename(&partial,root.join(format!("{id}.db")))?;
            crate::storage::sync_directory(&root)?;
            Ok::<_,anyhow::Error>(snapshot)
        }).await;
        match result {
            Ok(Ok(snapshot)) => Ok(snapshot),
            other => {
                let _ = std::fs::remove_file(&partial);
                match other {
                    Ok(Err(e)) => Err(e),
                    Err(e) => Err(e.into()),
                    _ => unreachable!(),
                }
            }
        }
    }

    pub async fn snapshot_page(
        &self,
        id: &str,
        after: Option<&Position>,
        limit: i64,
    ) -> anyhow::Result<(Snapshot, Vec<Listen>)> {
        anyhow::ensure!(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid snapshot ID"
        );
        let _guard = self.snapshots.lock().await;
        let path = self.directory.join("snapshots").join(format!("{id}.db"));
        let snapshot = snapshot_metadata(&path).await?;
        anyhow::ensure!(
            snapshot.id == id && snapshot.expires_at_ms > crate::auth::now_ms(),
            "Snapshot expired or incompatible; restart complete copy"
        );
        let view = self.read_view().await?;
        anyhow::ensure!(
            view.identity.instance_id == snapshot.instance_id
                && view.identity.generation == snapshot.generation,
            "Snapshot generation incompatible; restart complete copy"
        );
        drop(view);
        let mut conn = SqliteConnection::connect_with(
            &SqliteConnectOptions::new().filename(path).read_only(true),
        )
        .await?;
        let rows = if let Some(after) = after {
            sqlx::query_as("SELECT * FROM listens WHERE trashed_at_ms IS NULL AND (played_at_ms,id)>(?,?) ORDER BY played_at_ms,id LIMIT ?")
                .bind(after.played_at_ms).bind(&after.id).bind(limit.clamp(1,1000)).fetch_all(&mut conn).await?
        } else {
            sqlx::query_as("SELECT * FROM listens WHERE trashed_at_ms IS NULL ORDER BY played_at_ms,id LIMIT ?").bind(limit.clamp(1,1000)).fetch_all(&mut conn).await?
        };
        conn.close().await?;
        Ok((snapshot, rows))
    }
}

async fn snapshot_metadata(path: &Path) -> anyhow::Result<Snapshot> {
    let mut conn =
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(path).read_only(true))
            .await?;
    let metadata: String = sqlx::query_scalar("SELECT value FROM snapshot_metadata")
        .fetch_one(&mut conn)
        .await?;
    conn.close().await?;
    Ok(serde_json::from_str(&metadata)?)
}
