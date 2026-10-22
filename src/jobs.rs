//! Durable jobs. Adapters parse outside the writer, then commit bounded batches here.
use crate::{
    auth,
    db::{self, Event, IngestReport},
    error::{AppError, Result},
    storage::Storage,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub file_id: Option<String>,
    pub filename: Option<String>,
    pub parameters_json: String,
    pub attempt: i64,
    pub checkpoint: i64,
    pub processed: i64,
    pub added: i64,
    pub reconciled: i64,
    pub duplicates: i64,
    pub originals: i64,
    pub variants: i64,
    pub invalid: i64,
    pub skipped: i64,
    pub ambiguous: i64,
    pub cancel_requested: bool,
    pub retry_at_ms: Option<i64>,
    pub error_code: Option<String>,
    pub phase: String,
    pub outcome: Option<String>,
    pub inventory_complete: bool,
    pub below_threshold: i64,
}

impl Storage {
    /// Files are content-addressed and become visible to the queue only after EOF and fsync.
    /// Names supplied by clients are display data, never filesystem paths.
    pub async fn receive<R: AsyncRead + Unpin>(
        &self,
        reader: &mut R,
        max_bytes: u64,
    ) -> anyhow::Result<String> {
        let _files = self.spool_operations.read().await;
        self.receive_locked(reader, max_bytes).await
    }

    /// The caller holds the file gate through publication AND attachment to a job.
    pub(crate) async fn receive_locked<R: AsyncRead + Unpin>(
        &self,
        reader: &mut R,
        max_bytes: u64,
    ) -> anyhow::Result<String> {
        let incoming = self.directory.join("spool/incoming").join(db::new_id());
        let result=async {
            let mut file=tokio::fs::OpenOptions::new().write(true).create_new(true).open(&incoming).await?;
            let mut hash=Sha256::new(); let mut size=0u64; let mut buffer=[0u8;65536];
            loop {
                let count=reader.read(&mut buffer).await?;
                if count==0 {break;}
                size=size.checked_add(count as u64).ok_or_else(||anyhow::anyhow!("Upload too large"))?;
                anyhow::ensure!(size<=max_bytes,"Upload exceeds configured spool limit");
                hash.update(&buffer[..count]); file.write_all(&buffer[..count]).await?;
            }
            file.sync_all().await?; drop(file);
            let hash=format!("{:x}",hash.finalize());
            let destination=self.directory.join("spool").join(&hash);
            // Hard link publishes without replacing an existing immutable file, including on Windows.
            match tokio::fs::hard_link(&incoming,&destination).await {
                Ok(())=>{}, Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>{}, Err(e)=>return Err(e.into())
            }
            crate::storage::sync_directory(&self.directory.join("spool"))?;
            sqlx::query("INSERT INTO source_files(id,content_hash,size_bytes,received_at_ms) VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET retained=1,received_at_ms=excluded.received_at_ms")
                .bind(&hash).bind(&hash).bind(i64::try_from(size)?).bind(auth::now_ms()).execute(&self.writer).await?;
            Ok::<_,anyhow::Error>(hash)
        }.await;
        // An aborted future can leave an incoming file; it is never processable or marked complete.
        let _ = tokio::fs::remove_file(incoming).await;
        result
    }

    pub async fn enqueue(
        &self,
        kind: &str,
        file_id: Option<&str>,
        filename: Option<&str>,
        parameters: &Value,
    ) -> Result<String> {
        let _files = self.spool_operations.read().await;
        if let Some(file_id) = file_id {
            let retained: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM source_files WHERE id=? AND retained=1)",
            )
            .bind(file_id)
            .fetch_one(&self.writer)
            .await?;
            if !retained {
                return Err(AppError::Conflict(
                    "Source file was released. Upload it again to start another import.".into(),
                ));
            }
        }
        let id = db::new_id();
        let inserted=sqlx::query("INSERT INTO jobs(id,kind,file_id,filename,parameters_json,state,created_at_ms,updated_at_ms) SELECT ?,?,?,?,?,'queued',?,? FROM archive_state WHERE singleton=1 AND maintenance=0")
            .bind(&id).bind(kind).bind(file_id).bind(filename).bind(parameters.to_string()).bind(auth::now_ms()).bind(auth::now_ms()).execute(&self.writer).await?;
        if inserted.rows_affected() != 1 {
            return Err(AppError::Conflict("Archive is in maintenance.".into()));
        }
        Ok(id)
    }

    pub async fn job(&self, id: &str) -> Result<Job> {
        sqlx::query_as("SELECT * FROM jobs WHERE id=?")
            .bind(id)
            .fetch_optional(&self.readers)
            .await?
            .ok_or_else(|| AppError::BadRequest("Unknown job.".into()))
    }
    pub async fn activities(&self) -> Result<Vec<Job>> {
        Ok(
            sqlx::query_as("SELECT * FROM jobs ORDER BY created_at_ms DESC,id DESC LIMIT 100")
                .fetch_all(&self.readers)
                .await?,
        )
    }
    /// Atomic claim and attempt fencing. Each worker claims only kinds it can execute.
    pub async fn claim(&self, kind: &str) -> Result<Option<Job>> {
        if self.directory.join("recovery.pending").exists()
            || (!kind.starts_with("recovery_") && crate::recovery::paused(self).await)
        {
            return Ok(None);
        }
        Ok(sqlx::query_as("UPDATE jobs SET state='running',attempt=attempt+1,updated_at_ms=? WHERE id=(SELECT id FROM jobs WHERE kind=? AND state IN ('queued','waiting') AND cancel_requested=0 AND (retry_at_ms IS NULL OR retry_at_ms<=?) ORDER BY created_at_ms,id LIMIT 1) AND NOT EXISTS(SELECT 1 FROM archive_state WHERE maintenance=1) RETURNING *")
            .bind(auth::now_ms()).bind(kind).bind(auth::now_ms()).fetch_optional(&self.writer).await?)
    }
    pub async fn cancel(&self, id: &str) -> Result<()> {
        sqlx::query("UPDATE jobs SET cancel_requested=1,state='cancelled',updated_at_ms=? WHERE id=? AND state IN ('queued','waiting','running')")
            .bind(auth::now_ms()).bind(id).execute(&self.writer).await?;
        Ok(())
    }
    /// Persist a provider wait or a terminal failure without losing the last checkpoint.
    pub async fn pause(&self, lease: &Job, retry_at_ms: Option<i64>, code: &str) -> Result<()> {
        let changed=sqlx::query("UPDATE jobs SET state=?,retry_at_ms=?,error_code=?,updated_at_ms=? WHERE id=? AND attempt=? AND state='running' AND cancel_requested=0")
            .bind(if retry_at_ms.is_some(){"waiting"}else{"failed"}).bind(retry_at_ms).bind(code).bind(auth::now_ms()).bind(&lease.id).bind(lease.attempt).execute(&self.writer).await?;
        if changed.rows_affected() != 1 {
            return Err(stale());
        }
        Ok(())
    }
    pub async fn retry(&self, id: &str) -> Result<()> {
        let _files = self.spool_operations.read().await;
        let mut tx = self.writer.begin().await?;
        sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision").fetch_optional(&mut *tx).await?.ok_or_else(||AppError::Conflict("Archive is in maintenance.".into()))?;
        let released: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM source_files f JOIN jobs j ON j.id=? WHERE f.retained=0 AND (j.file_id=f.id OR EXISTS(SELECT 1 FROM job_inputs i WHERE i.job_id=j.id AND i.file_id=f.id)))")
            .bind(id).fetch_one(&mut *tx).await?;
        if released {
            return Err(AppError::Conflict(
                "Source file was released. Upload it again to retry.".into(),
            ));
        }
        let another_calendar: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs target WHERE target.id=? AND target.kind='calendar' AND EXISTS(SELECT 1 FROM jobs other WHERE other.id!=target.id AND other.kind='calendar' AND other.state IN ('queued','running','waiting')))").bind(id).fetch_one(&mut *tx).await?;
        if another_calendar {
            return Err(AppError::Conflict(
                "Another calendar recalculation is already pending.".into(),
            ));
        }
        sqlx::query("UPDATE jobs SET state='queued',cancel_requested=0,retry_at_ms=NULL,error_code=NULL,outcome=NULL WHERE id=? AND (state IN ('failed','cancelled') OR outcome='partial') AND phase!='receiving'")
            .bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    /// Rows must be a contiguous checkpoint interval. Invalid/skipped fingerprints and reasons persist too.
    /// If parsing fails, omit this call: no progress is acknowledged.
    pub async fn commit_batch(
        &self,
        lease: &Job,
        member: &str,
        rows: &[SourceRow],
        complete: bool,
    ) -> Result<IngestReport> {
        self.commit_member_batch(lease, member, lease.checkpoint + 1, rows, complete)
            .await
    }
    /// For ZIP/multi-file sources, source row numbers are independent of the global checkpoint.
    pub async fn commit_member_batch(
        &self,
        lease: &Job,
        member: &str,
        first_row: i64,
        rows: &[SourceRow],
        complete: bool,
    ) -> Result<IngestReport> {
        let mut tx = self.writer.begin().await?;
        let report = self
            .commit_member_batch_in(&mut tx, lease, member, first_row, rows, complete)
            .await?;
        tx.commit().await?;
        Ok(report)
    }
    pub(crate) async fn commit_member_batch_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        lease: &Job,
        member: &str,
        first_row: i64,
        rows: &[SourceRow],
        complete: bool,
    ) -> Result<IngestReport> {
        if rows.len() > 1000 {
            return Err(AppError::BadRequest(
                "A job batch may contain at most 1000 rows.".into(),
            ));
        }
        validate_lease(tx, lease).await?;
        sqlx::query("INSERT OR IGNORE INTO job_members(job_id,member,kind,state) VALUES(?,?,'music','processing')").bind(&lease.id).bind(member).execute(&mut **tx).await?;
        let position: i64 =
            sqlx::query_scalar("SELECT processed FROM job_members WHERE job_id=? AND member=?")
                .bind(&lease.id)
                .bind(member)
                .fetch_one(&mut **tx)
                .await?;
        if first_row != position + 1 {
            return Err(stale());
        }
        let events: Vec<Event> = rows
            .iter()
            .filter_map(|r| match r {
                SourceRow::Event(e) => Some(e.as_ref().clone()),
                _ => None,
            })
            .collect();
        let below_threshold = events.iter().filter(|e| !e.counted).count() as i64;
        let file_id: Option<String> =
            sqlx::query_scalar("SELECT file_id FROM job_members WHERE job_id=? AND member=?")
                .bind(&lease.id)
                .bind(member)
                .fetch_one(&mut **tx)
                .await?;
        let file_id = file_id.or_else(|| lease.file_id.clone());
        let report = crate::ingest::with_connection(tx, |conn| {
            crate::ingest::import_rows(
                conn,
                &events,
                &lease.id,
                file_id.as_deref(),
                member,
                first_row,
                rows,
            )
        })
        .await?;
        sqlx::query("UPDATE job_members SET processed=processed+? WHERE job_id=? AND member=?")
            .bind(rows.len() as i64)
            .bind(&lease.id)
            .bind(member)
            .execute(&mut **tx)
            .await?;
        if complete {
            sqlx::query("UPDATE job_members SET state=CASE WHEN EXISTS(SELECT 1 FROM source_occurrences o WHERE o.job_id=job_members.job_id AND o.member=job_members.member AND o.outcome='invalid') THEN 'partial' ELSE 'completed' END,expected_rows=processed WHERE job_id=? AND member=?")
                .bind(&lease.id).bind(member).execute(&mut **tx).await?;
        }
        sqlx::query("UPDATE job_members SET originals=originals+?,variants=variants+?,duplicates=duplicates+?,added=added+?,reconciled=reconciled+?,invalid=invalid+?,skipped=skipped+?,below_threshold=below_threshold+? WHERE job_id=? AND member=?")
            .bind(report.originals as i64).bind(report.variants as i64).bind(report.duplicates as i64).bind(report.added as i64).bind(report.reconciled as i64).bind(report.invalid as i64).bind(report.skipped as i64).bind(below_threshold).bind(&lease.id).bind(member).execute(&mut **tx).await?;
        sqlx::query("UPDATE jobs SET below_threshold=below_threshold+? WHERE id=?")
            .bind(below_threshold)
            .bind(&lease.id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE jobs SET checkpoint=checkpoint+?,processed=processed+?,added=added+?,reconciled=reconciled+?,duplicates=duplicates+?,originals=originals+?,variants=variants+?,invalid=invalid+?,skipped=skipped+?,ambiguous=ambiguous+?,state=?,updated_at_ms=? WHERE id=?")
            .bind(rows.len() as i64).bind(rows.len() as i64).bind(report.added as i64).bind(report.reconciled as i64).bind(report.duplicates as i64).bind(report.originals as i64).bind(report.variants as i64).bind(report.invalid as i64).bind(report.skipped as i64).bind(report.ambiguous as i64)
            .bind(if complete{"completed"}else{"running"}).bind(auth::now_ms()).bind(&lease.id).execute(&mut **tx).await?;
        Ok(report)
    }
    /// Inventory can represent unreadable/excluded/unprocessed files without inventing row counts.
    pub async fn record_member(
        &self,
        job_id: &str,
        member: &str,
        kind: &str,
        state: &str,
        expected_rows: Option<i64>,
        error_code: Option<&str>,
    ) -> Result<()> {
        if expected_rows.is_some_and(|n| n < 0) {
            return Err(AppError::BadRequest("Invalid row count.".into()));
        }
        sqlx::query("INSERT INTO job_members(job_id,member,kind,state,expected_rows,error_code) VALUES(?,?,?,?,?,?) ON CONFLICT(job_id,member) DO UPDATE SET kind=excluded.kind,state=excluded.state,expected_rows=excluded.expected_rows,error_code=excluded.error_code")
            .bind(job_id).bind(member).bind(kind).bind(state).bind(expected_rows).bind(error_code).execute(&self.writer).await?;
        Ok(())
    }
}

pub enum SourceRow {
    Event(Box<Event>),
    Invalid { raw: Value, reason: String },
    Skipped { raw: Value, reason: String },
}
impl From<Event> for SourceRow {
    fn from(event: Event) -> Self {
        Self::Event(Box::new(event))
    }
}
pub(crate) fn stale() -> AppError {
    AppError::Conflict("Job cancelled or checkpoint/attempt no longer current.".into())
}
pub(crate) async fn validate_lease(conn: &mut sqlx::SqliteConnection, lease: &Job) -> Result<()> {
    // Acquiring a write reservation before reading avoids SQLite read-to-write upgrade races.
    let row=sqlx::query("UPDATE jobs SET updated_at_ms=? WHERE id=? AND state='running' AND cancel_requested=0 AND attempt=? AND checkpoint=? AND NOT EXISTS(SELECT 1 FROM archive_state WHERE maintenance=1) RETURNING id")
        .bind(auth::now_ms()).bind(&lease.id).bind(lease.attempt).bind(lease.checkpoint).fetch_optional(conn).await?;
    if row.is_none() {
        return Err(stale());
    }
    Ok(())
}
