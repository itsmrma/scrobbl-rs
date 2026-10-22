//! Bounded ZIP/JSON readers. No member is extracted to a client-supplied path.
use crate::{
    auth,
    error::{AppError, Result},
    imports::{ImportLimits, parse_row},
    jobs::{Job, SourceRow},
    storage::Storage,
};
use serde::{
    Deserializer,
    de::{self, SeqAccess, Visitor},
};
use serde_json::Value;
use sqlx::Row;
use std::{
    cell::Cell,
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    rc::Rc,
};
use tokio::sync::mpsc;

#[derive(Clone)]
struct Input {
    ordinal: i64,
    file_id: String,
    filename: String,
}
#[derive(Clone)]
struct Member {
    key: String,
    file_id: String,
    path: PathBuf,
    index: Option<usize>,
    kind: String,
    error: Option<String>,
}
enum Message {
    Inventory(Member),
    Classified {
        key: String,
        kind: String,
        count: Option<i64>,
        error: Option<String>,
    },
    Batch {
        key: String,
        first: i64,
        rows: Vec<SourceRow>,
    },
    Complete(String),
    Inventoried,
}
fn send(tx: &mpsc::Sender<Message>, message: Message) -> anyhow::Result<()> {
    tx.blocking_send(message)
        .map_err(|_| anyhow::anyhow!("Worker stopped"))
}
// Serde's array visitor reads one Value at a time. Count outside BufReader so read-ahead
// cannot hide an oversized row. A bounded channel supplies backpressure to the parser.
struct RowReader<R> {
    inner: R,
    bytes: Rc<Cell<u64>>,
    max: u64,
}
impl<R: Read> Read for RowReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.bytes.get() >= self.max {
            return Err(std::io::Error::other(
                "Configured JSON row byte limit exceeded",
            ));
        }
        let size = buf.len().min((self.max - self.bytes.get()) as usize);
        let n = self.inner.read(&mut buf[..size])?;
        self.bytes.set(self.bytes.get() + n as u64);
        Ok(n)
    }
}
fn array<R: Read>(
    reader: R,
    limits: &ImportLimits,
    mut row: impl FnMut(i64, Value) -> anyhow::Result<()>,
) -> anyhow::Result<i64> {
    struct Rows<'a, F> {
        counter: Rc<Cell<u64>>,
        row: &'a mut F,
    }
    impl<'de, F: FnMut(i64, Value) -> anyhow::Result<()>> Visitor<'de> for Rows<'_, F> {
        type Value = i64;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("JSON array")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<i64, A::Error> {
            let mut count = 0;
            loop {
                self.counter.set(0);
                let Some(value) = seq.next_element::<Value>()? else {
                    break;
                };
                count += 1;
                (self.row)(count, value).map_err(de::Error::custom)?;
            }
            Ok(count)
        }
    }
    let mut reader = BufReader::new(reader);
    if reader.fill_buf()?.starts_with(&[0xef, 0xbb, 0xbf]) {
        reader.consume(3);
    }
    let counter = Rc::new(Cell::new(0));
    let mut de = serde_json::Deserializer::from_reader(RowReader {
        inner: reader,
        bytes: counter.clone(),
        max: limits.row_bytes,
    });
    let count = de.deserialize_seq(Rows {
        counter,
        row: &mut row,
    })?;
    de.end()?;
    Ok(count)
}
struct ByteBudget<R> {
    inner: R,
    remaining: u64,
}
impl<R: Read> Read for ByteBudget<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let size = buf.len().min(self.remaining.saturating_add(1) as usize);
        let n = self.inner.read(&mut buf[..size])?;
        if n as u64 > self.remaining {
            return Err(std::io::Error::other(
                "Configured member byte limit exceeded",
            ));
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}
fn with_reader<T>(
    member: &Member,
    limit: u64,
    f: impl FnOnce(&mut dyn Read) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let file = File::open(&member.path)?;
    if let Some(index) = member.index {
        let mut zip = zip::ZipArchive::new(file)?;
        let entry = zip.by_index(index)?;
        let mut reader = ByteBudget {
            inner: entry,
            remaining: limit,
        };
        f(&mut reader)
    } else {
        let mut reader = ByteBudget {
            inner: file,
            remaining: limit,
        };
        f(&mut reader)
    }
}
fn inspect(
    storage: &Storage,
    inputs: &[Input],
    limits: &ImportLimits,
) -> anyhow::Result<Vec<Member>> {
    let mut members = Vec::new();
    let mut expanded = 0u64;
    for input in inputs {
        let path = storage.directory.join("spool").join(&input.file_id);
        let mut file = File::open(&path)?;
        let mut signature = [0u8; 4];
        let n = file.read(&mut signature)?;
        let is_zip = (n == 4 && signature[..2] == *b"PK")
            || input.filename.to_ascii_lowercase().ends_with(".zip");
        if is_zip {
            match zip::ZipArchive::new(File::open(&path)?) {
                Err(_) => members.push(Member {
                    key: format!("{}: {}", input.ordinal, input.filename),
                    file_id: input.file_id.clone(),
                    path,
                    index: None,
                    kind: "invalid".into(),
                    error: Some("zip_unreadable_inventory_incomplete".into()),
                }),
                Ok(mut archive) => {
                    anyhow::ensure!(
                        members.len().saturating_add(archive.len()) <= limits.files,
                        "file_count_limit_inventory_incomplete"
                    );
                    for index in 0..archive.len() {
                        let entry = archive.by_index_raw(index)?;
                        expanded = expanded.saturating_add(entry.size());
                        let error = if entry.enclosed_name().is_none() || entry.is_symlink() {
                            Some("unsafe_zip_path")
                        } else if entry.encrypted() {
                            Some("encrypted_zip_member")
                        } else if entry.size() > limits.member_bytes {
                            Some("member_byte_limit")
                        } else if expanded > limits.expanded_bytes {
                            Some("expanded_byte_limit")
                        } else {
                            None
                        };
                        members.push(Member {
                            key: format!(
                                "{}: {} / [{}] {}",
                                input.ordinal,
                                input.filename,
                                index + 1,
                                entry
                                    .name()?
                                    .chars()
                                    .filter(|c| !c.is_control())
                                    .take(1000)
                                    .collect::<String>()
                            ),
                            file_id: input.file_id.clone(),
                            path: path.clone(),
                            index: Some(index),
                            kind: if entry.is_dir() {
                                "directory"
                            } else {
                                "pending"
                            }
                            .into(),
                            error: error.map(str::to_owned),
                        });
                    }
                }
            }
        } else {
            let size = file.metadata()?.len();
            expanded = expanded.saturating_add(size);
            let error = if size > limits.member_bytes {
                Some("member_byte_limit")
            } else if expanded > limits.expanded_bytes {
                Some("expanded_byte_limit")
            } else {
                None
            };
            members.push(Member {
                key: format!("{}: {}", input.ordinal, input.filename),
                file_id: input.file_id.clone(),
                path,
                index: None,
                kind: "pending".into(),
                error: error.map(str::to_owned),
            });
        }
        anyhow::ensure!(
            members.len() <= limits.files,
            "file_count_limit_inventory_incomplete"
        );
    }
    Ok(members)
}
fn parse(
    storage: Storage,
    inputs: Vec<Input>,
    limits: ImportLimits,
    resume: HashMap<String, i64>,
    tx: mpsc::Sender<Message>,
) -> anyhow::Result<()> {
    let members = inspect(&storage, &inputs, &limits)?;
    for member in &members {
        send(&tx, Message::Inventory(member.clone()))?;
    }
    send(&tx, Message::Inventoried)?;
    for member in members {
        if member.error.is_some() || member.kind == "directory" {
            continue;
        }
        let mut music = false;
        let mut non_music = false;
        let classification = with_reader(&member, limits.member_bytes, |reader| {
            array(reader, &limits, |_, row| {
                music |= row.get("master_metadata_track_name").is_some()
                    || row.get("spotify_track_uri").is_some();
                non_music |= row.get("spotify_episode_uri").is_some_and(|v| !v.is_null())
                    || row
                        .get("spotify_audiobook_uri")
                        .is_some_and(|v| !v.is_null())
                    || row.get("audiobook_uri").is_some_and(|v| !v.is_null());
                // Extended history with damaged/missing music metadata still needs row diagnostics.
                music |= row.get("ts").is_some() && row.get("ms_played").is_some() && !non_music;
                Ok(())
            })
        });
        let (kind, count, error) = match classification {
            Ok(count) => (
                if music {
                    "music"
                } else if non_music {
                    "non_music"
                } else {
                    "foreign"
                },
                Some(count),
                None,
            ),
            Err(error) => {
                let limit_error = if error.to_string().contains("row byte limit") {
                    Some("json_row_byte_limit")
                } else if error.to_string().contains("member byte limit") {
                    Some("member_byte_limit")
                } else {
                    None
                };
                // Non-array valid JSON and ordinary non-JSON assets are unrelated, not musical failures.
                let foreign = with_reader(&member, limits.member_bytes, |reader| {
                    let mut buffer = BufReader::new(reader);
                    let prefix = buffer.fill_buf()?;
                    let first = prefix.iter().copied().find(|b| !b.is_ascii_whitespace());
                    if first == Some(b'[') {
                        return Ok(false);
                    }
                    let json_name = member.key.to_ascii_lowercase().ends_with(".json");
                    if !json_name {
                        return Ok(true);
                    }
                    let mut de = serde_json::Deserializer::from_reader(buffer);
                    serde::Deserialize::deserialize(&mut de)
                        .and_then(|_: de::IgnoredAny| de.end())
                        .map(|_| true)
                        .map_err(Into::into)
                })
                .unwrap_or(false);
                if foreign && limit_error.is_none() {
                    ("foreign", None, None)
                } else {
                    ("invalid",None,Some(limit_error.unwrap_or("json_unreadable_or_row_limit: check JSON syntax, row byte limit, ZIP CRC and compression support").into()))
                }
            }
        };
        send(
            &tx,
            Message::Classified {
                key: member.key.clone(),
                kind: kind.into(),
                count,
                error,
            },
        )?;
        if kind != "music" && kind != "non_music" {
            continue;
        }
        let skip = resume.get(&member.key).copied().unwrap_or(0);
        let mut batch = Vec::new();
        let mut first = skip + 1;
        let mut batch_bytes = 0usize;
        let parsed = with_reader(&member, limits.member_bytes, |reader| {
            array(reader, &limits, |index, row| {
                if index <= skip {
                    return Ok(());
                }
                let bytes = serde_json::to_vec(&row)?.len();
                if !batch.is_empty() && batch_bytes.saturating_add(bytes) > 1024 * 1024 {
                    let size = batch.len() as i64;
                    send(
                        &tx,
                        Message::Batch {
                            key: member.key.clone(),
                            first,
                            rows: std::mem::take(&mut batch),
                        },
                    )?;
                    first += size;
                    batch_bytes = 0;
                }
                batch_bytes += bytes;
                batch.push(parse_row(row));
                if batch.len() == 1000 {
                    let size = batch.len() as i64;
                    send(
                        &tx,
                        Message::Batch {
                            key: member.key.clone(),
                            first,
                            rows: std::mem::take(&mut batch),
                        },
                    )?;
                    first += size;
                    batch_bytes = 0;
                }
                Ok(())
            })
        });
        // Validation above completed before any row commit. Immutable spool makes replay deterministic.
        parsed?;
        if !batch.is_empty() {
            send(
                &tx,
                Message::Batch {
                    key: member.key.clone(),
                    first,
                    rows: batch,
                },
            )?;
        }
        send(&tx, Message::Complete(member.key))?;
    }
    Ok(())
}
pub async fn run(storage: &Storage, mut job: Job) -> Result<()> {
    let limits: ImportLimits =
        serde_json::from_str(&job.parameters_json).map_err(|_| AppError::Internal)?;
    let inputs = sqlx::query(
        "SELECT ordinal,file_id,filename FROM job_inputs WHERE job_id=? ORDER BY ordinal",
    )
    .bind(&job.id)
    .fetch_all(&storage.readers)
    .await?
    .into_iter()
    .map(|r| Input {
        ordinal: r.get("ordinal"),
        file_id: r.get("file_id"),
        filename: r.get("filename"),
    })
    .collect();
    let resume = sqlx::query("SELECT member,processed FROM job_members WHERE job_id=?")
        .bind(&job.id)
        .fetch_all(&storage.readers)
        .await?
        .into_iter()
        .map(|r| (r.get("member"), r.get("processed")))
        .collect();
    let (tx, mut rx) = mpsc::channel(2);
    let own = storage.clone();
    let parser = tokio::task::spawn_blocking(move || parse(own, inputs, limits, resume, tx));
    while let Some(message) = rx.recv().await {
        let mut transaction = storage.writer.begin().await?;
        crate::jobs::validate_lease(&mut transaction, &job).await?;
        match message {
            Message::Inventory(member) => {
                sqlx::query("DELETE FROM job_members WHERE job_id=? AND kind='received'")
                    .bind(&job.id)
                    .execute(&mut *transaction)
                    .await?;
                sqlx::query("INSERT INTO job_members(job_id,member,file_id,kind,state,error_code) VALUES(?,?,?,?,?,?) ON CONFLICT(job_id,member) DO UPDATE SET error_code=excluded.error_code")
                    .bind(&job.id).bind(member.key).bind(member.file_id).bind(&member.kind).bind(if member.error.is_some(){"failed"}else if member.kind=="directory"{"excluded"}else{"pending"}).bind(member.error).execute(&mut *transaction).await?;
            }
            Message::Inventoried => {
                sqlx::query("UPDATE jobs SET inventory_complete=1,phase='processing' WHERE id=?")
                    .bind(&job.id)
                    .execute(&mut *transaction)
                    .await?;
            }
            Message::Classified {
                key,
                kind,
                count,
                error,
            } => {
                sqlx::query("UPDATE job_members SET kind=?,expected_rows=?,error_code=?,state=? WHERE job_id=? AND member=?")
                    .bind(&kind).bind(count).bind(&error).bind(if error.is_some(){"failed"}else if kind=="foreign"{"excluded"}else{"processing"}).bind(&job.id).bind(key).execute(&mut *transaction).await?;
            }
            Message::Complete(key) => {
                sqlx::query("UPDATE job_members SET state=CASE WHEN invalid>0 THEN 'partial' WHEN kind='non_music' THEN 'excluded' ELSE 'completed' END WHERE job_id=? AND member=?").bind(&job.id).bind(key).execute(&mut *transaction).await?;
            }
            Message::Batch { key, first, rows } => {
                transaction.commit().await?;
                storage
                    .commit_member_batch(&job, &key, first, &rows, false)
                    .await?;
                job = storage.job(&job.id).await?;
                continue;
            }
        }
        transaction.commit().await?;
    }
    let parsed = parser.await.map_err(|_| AppError::Internal)?;
    job = storage.job(&job.id).await?;
    let mut transaction = storage.writer.begin().await?;
    crate::jobs::validate_lease(&mut transaction, &job).await?;
    let useful = job.processed > job.invalid;
    let failures:i64=sqlx::query_scalar("SELECT COUNT(*) FROM job_members WHERE job_id=? AND (state IN ('failed','pending','partial','received') OR error_code IS NOT NULL)").bind(&job.id).fetch_one(&mut *transaction).await?;
    let musical:i64=sqlx::query_scalar("SELECT COUNT(*) FROM job_members WHERE job_id=? AND kind='music' AND processed>invalid+skipped").bind(&job.id).fetch_one(&mut *transaction).await?;
    let error = parsed.err().map(|e| {
        if e.to_string()
            .contains("file_count_limit_inventory_incomplete")
        {
            "file_count_limit_inventory_incomplete: increase SCROBBL_IMPORT_FILES and upload again"
        } else {
            "inventory_or_processing_failed: check spool, disk space and ZIP directory"
        }
    });
    let outcome = if error.is_some() || failures > 0 || job.invalid > 0 {
        if useful { "partial" } else { "error" }
    } else if musical == 0 {
        "no_music"
    } else {
        "complete"
    };
    sqlx::query("UPDATE jobs SET state=?,outcome=?,phase='finished',error_code=?,updated_at_ms=? WHERE id=?")
        .bind(if outcome=="error"{"failed"}else{"completed"}).bind(outcome).bind(error).bind(auth::now_ms()).bind(&job.id).execute(&mut *transaction).await?;
    transaction.commit().await?;
    Ok(())
}
pub async fn worker(storage: Storage) {
    loop {
        match storage.claim("spotify_import").await {
            Ok(Some(job)) => {
                let id = job.id.clone();
                let attempt = job.attempt;
                match run(&storage, job).await {
                    Ok(()) => {
                        // Cleanup failure must not relabel a committed import as failed.
                        // The retention worker retries it independently.
                        let _ = crate::retention::cleanup(&storage).await;
                    }
                    Err(AppError::Conflict(_)) => {
                        // Maintenance also fences a running lease. Make its confirmed
                        // checkpoint takeable again without resurrecting cancelled work.
                        let _=sqlx::query("UPDATE jobs SET state='queued',updated_at_ms=? WHERE id=? AND attempt=? AND state='running' AND cancel_requested=0").bind(auth::now_ms()).bind(&id).bind(attempt).execute(&storage.writer).await;
                    }
                    Err(_) => {
                        if let Ok(current) = storage.job(&id).await
                            && storage.pause(&current,None,"import_storage_failed: check disk space and retry; confirmed rows remain saved").await.is_ok()
                        {
                            let _=sqlx::query("UPDATE jobs SET outcome=CASE WHEN processed>invalid THEN 'partial' ELSE 'error' END,phase='finished' WHERE id=?").bind(&id).execute(&storage.writer).await;
                        }
                    }
                }
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(300)).await,
        }
    }
}
