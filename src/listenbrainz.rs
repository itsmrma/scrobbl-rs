//! ListenBrainz source adapter. Files are immutable before a durable job can see them.
use crate::{
    AppState, auth,
    db::{self, Artist, DurationKind, Event, Track},
    error::{AppError, Result},
    jobs::{Job, SourceRow},
    storage::Storage,
};
use axum::{
    Json,
    extract::{Multipart, Path as AxumPath, Query, State},
    http::HeaderMap,
};
use chrono::{NaiveDate, TimeZone};
use futures_util::TryStreamExt;
use serde::{
    Deserialize, Serialize,
    de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fmt,
    io::{BufRead, BufReader, Read},
    path::Path,
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct Limits {
    pub upload_bytes: u64,
    pub expanded_bytes: u64,
    pub members: usize,
    pub row_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            upload_bytes: 512 * 1024 * 1024,
            expanded_bytes: 2 * 1024 * 1024 * 1024,
            members: 10_000,
            row_bytes: 1024 * 1024,
        }
    }
}
impl Limits {
    pub fn from_env() -> anyhow::Result<Self> {
        let mut limits = Self::default();
        for (name, value) in [
            ("SCROBBL_LB_UPLOAD_BYTES", &mut limits.upload_bytes),
            ("SCROBBL_LB_EXPANDED_BYTES", &mut limits.expanded_bytes),
        ] {
            if let Ok(raw) = std::env::var(name) {
                *value = raw.parse()?;
            }
        }
        if let Ok(raw) = std::env::var("SCROBBL_LB_MEMBERS") {
            limits.members = raw.parse()?;
        }
        if let Ok(raw) = std::env::var("SCROBBL_LB_ROW_BYTES") {
            limits.row_bytes = raw.parse()?;
        }
        anyhow::ensure!(
            limits.upload_bytes > 0
                && limits.expanded_bytes > 0
                && limits.members > 0
                && limits.row_bytes > 0,
            "ListenBrainz limits must be positive"
        );
        Ok(limits)
    }
}
fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)?.as_str().filter(|s| !s.trim().is_empty())
}
fn seconds(v: &Value) -> Option<i64> {
    let n = v.as_f64()?;
    (n.is_finite() && n.fract() == 0.0 && (0.0..=253_402_300_799.0).contains(&n))
        .then_some(n as i64)
}
fn duration(v: &Value, scale: f64) -> Option<i64> {
    let n = v.as_f64()? * scale;
    (n.is_finite() && (0.0..=604_800_000.0).contains(&n)).then_some(n.round() as i64)
}
fn spotify_id(info: &Value) -> Option<String> {
    let s = text(info, "spotify_id")?;
    if crate::imports::spotify_uri(s) {
        return Some(s.into());
    }
    let url = url::Url::parse(s).ok()?;
    if url.scheme() != "https" || url.host_str() != Some("open.spotify.com") {
        return None;
    }
    let id = url.path().strip_prefix("/track/")?;
    let uri = format!("spotify:track:{id}");
    crate::imports::spotify_uri(&uri).then_some(uri)
}
/// No verified playback ID is currently supplied by ListenBrainz. recording_msid is
/// a recording identifier. Ignore response enrichment/transport fields for reimport
/// identity, retaining all of them in raw variants and source occurrences.
pub fn parse(row: Value, account: Option<&str>) -> SourceRow {
    let parsed = (|| -> std::result::Result<Event, &str> {
        let row_account = text(&row, "user_name").map(str::to_lowercase);
        let account = account.map(str::to_lowercase);
        if row_account
            .as_deref()
            .zip(account.as_deref())
            .is_some_and(|(a, b)| a != b)
        {
            return Err("Row account disagrees with source account.");
        }
        let account = account.as_deref().or(row_account.as_deref());
        let time = seconds(&row["listened_at"])
            .filter(|s| *s <= auth::now_ms() / 1000 + 86400)
            .ok_or("Missing or invalid listened_at: expected integer UTC seconds.")?
            * 1000;
        let meta = &row["track_metadata"];
        let artist = text(meta, "artist_name").ok_or("Missing artist_name.")?;
        let title = text(meta, "track_name").ok_or("Missing track_name.")?;
        let album = text(meta, "release_name");
        let info = &meta["additional_info"];
        let actual = duration(&info["duration_played"], 1000.0);
        let total =
            duration(&info["duration_ms"], 1.0).or_else(|| duration(&info["duration"], 1000.0));
        let (listened_ms, duration_kind) = if let Some(ms) = actual {
            (Some(ms), DurationKind::Actual)
        } else if let Some(ms) = total {
            (Some(ms), DurationKind::Estimated)
        } else {
            (None, DurationKind::Unknown)
        };
        let mut identity = row.clone();
        if let Some(obj) = identity.as_object_mut() {
            obj.remove("inserted_at");
            obj.remove("recording_msid");
            obj.remove("user_name");
            obj.remove("user_id");
            obj.insert("listened_at".into(), json!(time / 1000));
        }
        if let Some(meta) = identity
            .get_mut("track_metadata")
            .and_then(Value::as_object_mut)
        {
            meta.remove("mbid_mapping");
            meta.remove("recording_msid");
        }
        normalize_numbers(&mut identity);
        let source_key = db::metadata_id(
            "listenbrainz-v1",
            &[account.unwrap_or(""), &db::payload_fingerprint(&identity)],
        );
        let id = spotify_id(info).unwrap_or_else(|| db::metadata_id("lb-track", &[&source_key]));
        Ok(Event {
            source: "listenbrainz",
            source_key,
            source_timestamp_ms: time,
            played_at_ms: time,
            listened_ms,
            duration_kind,
            counted: actual.is_none_or(|ms| ms >= 30_000),
            track: Track {
                id,
                title: title.into(),
                album_id: album.map(|a| db::metadata_id("lb-album", &[artist, a])),
                album: album.map(str::to_owned),
                cover_url: None,
                duration_ms: total,
                artists: vec![Artist {
                    id: db::metadata_id("lb-artist", &[artist]),
                    name: artist.into(),
                }],
                structured: false,
            },
            raw: row.clone(),
        })
    })();
    match parsed {
        Ok(event) => event.into(),
        Err(reason) => SourceRow::Invalid {
            raw: row,
            reason: reason.into(),
        },
    }
}

fn normalize_numbers(value: &mut Value) {
    match value {
        Value::Number(n) => {
            if let Some(v) = n
                .as_f64()
                .filter(|v| v.fract() == 0.0 && v.abs() < 9_007_199_254_740_992.0)
            {
                *value = json!(v as i64);
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                normalize_numbers(v);
            }
        }
        Value::Array(array) => {
            for v in array {
                normalize_numbers(v);
            }
        }
        _ => {}
    }
}
/// Warnings are persisted beside accepted occurrences; bad optional durations do not
/// discard an otherwise valid listen or silently become actual listening time.
pub fn duration_warning(row: &Value) -> Option<String> {
    let info = &row["track_metadata"]["additional_info"];
    let mut warnings = Vec::new();
    for (key, scale) in [
        ("duration_played", 1000.0),
        ("duration_ms", 1.0),
        ("duration", 1000.0),
    ] {
        if info.get(key).is_some_and(|v| !v.is_null()) && duration(&info[key], scale).is_none() {
            warnings.push(format!(
                "Invalid {key}; source fingerprint retained; ignored for duration."
            ));
        }
    }
    if duration(&info["duration_ms"], 1.0)
        .zip(duration(&info["duration"], 1000.0))
        .is_some_and(|(a, b)| (a - b).abs() > 1)
    {
        warnings.push("Conflicting total durations; duration_ms used as an estimate.".into());
    }
    if duration(&info["duration_played"], 1000.0)
        .zip(duration(&info["duration_ms"], 1.0).or_else(|| duration(&info["duration"], 1000.0)))
        .is_some_and(|(a, b)| a > b)
    {
        warnings.push(
            "Actual listening time exceeds track duration; source values disagree; compact storage keeps normalized duration and source fingerprints.".into(),
        );
    }
    (!warnings.is_empty()).then(|| warnings.join(" "))
}

// Streaming JSON array / payload.listens reader. A validation pass determines the
// account before rows are emitted, regardless of object field order. No Vec of listens.
struct Document<'a> {
    level: u8,
    emit: &'a mut dyn FnMut(Value) -> std::result::Result<(), String>,
}
impl<'de> DeserializeSeed<'de> for Document<'_> {
    type Value = Option<String>;
    fn deserialize<D: de::Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Document<'_> {
    type Value = Option<String>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a listen array or payload.listens response")
    }
    fn visit_seq<A: SeqAccess<'de>>(
        self,
        mut seq: A,
    ) -> std::result::Result<Self::Value, A::Error> {
        if self.level == 1 {
            return Err(de::Error::custom(
                "Submission payloads are not history responses",
            ));
        }
        while let Some(row) = seq.next_element::<Value>()? {
            (self.emit)(row).map_err(de::Error::custom)?;
        }
        Ok(None)
    }
    fn visit_map<A: MapAccess<'de>>(
        self,
        mut map: A,
    ) -> std::result::Result<Self::Value, A::Error> {
        let mut found = false;
        let mut account = None;
        while let Some(key) = map.next_key::<String>()? {
            match (self.level, key.as_str()) {
                (0, "payload") | (1, "listens") => {
                    if found {
                        return Err(de::Error::custom("Duplicate listen payload"));
                    }
                    found = true;
                    let a = map.next_value_seed(Document {
                        level: self.level + 1,
                        emit: self.emit,
                    })?;
                    if a.is_some() {
                        account = a;
                    }
                }
                (1, "user_id" | "user_name") => {
                    let value = map.next_value::<Value>()?;
                    if let Some(s) = value.as_str().filter(|s| !s.is_empty()) {
                        account = Some(s.to_lowercase());
                    }
                }
                (1, "playing_now") => {
                    if map.next_value::<bool>()? {
                        return Err(de::Error::custom("Playing now is not listening history"));
                    }
                }
                (0, "listen_type") => {
                    return Err(de::Error::custom(
                        "Submission payloads are not history responses",
                    ));
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if !found || self.level > 1 {
            return Err(de::Error::custom("Expected payload.listens"));
        }
        Ok(account)
    }
}
struct QuotaReader<R> {
    reader: R,
    remaining: std::rc::Rc<std::cell::Cell<usize>>,
}
impl<R: Read> Read for QuotaReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.remaining.get();
        if remaining == 0 {
            return Err(std::io::Error::other(
                "JSON row or metadata exceeds configured limit",
            ));
        }
        let limit = bytes.len().min(remaining);
        let n = self.reader.read(&mut bytes[..limit])?;
        self.remaining.set(remaining - n);
        Ok(n)
    }
}
fn read_json(
    reader: impl Read,
    row_bytes: usize,
    emit: &mut dyn FnMut(Value) -> std::result::Result<(), String>,
) -> anyhow::Result<Option<String>> {
    let mut reader = BufReader::new(reader);
    if reader.fill_buf()?.starts_with(&[0xef, 0xbb, 0xbf]) {
        reader.consume(3);
    }
    let remaining = std::rc::Rc::new(std::cell::Cell::new(row_bytes));
    let mut de = serde_json::Deserializer::from_reader(QuotaReader {
        reader,
        remaining: remaining.clone(),
    });
    let mut bounded = |row| {
        remaining.set(row_bytes);
        emit(row)
    };
    let account = Document {
        level: 0,
        emit: &mut bounded,
    }
    .deserialize(&mut de)?;
    de.end()?;
    Ok(account)
}

enum Message {
    Account(Option<String>),
    Inventory {
        member: String,
        kind: String,
    },
    Row {
        member: String,
        line: i64,
        account: Option<String>,
        row: Value,
        invalid: Option<String>,
    },
    End {
        member: String,
        count: Option<i64>,
        error: Option<String>,
    },
}
fn send(tx: &mpsc::Sender<Message>, message: Message) -> anyhow::Result<()> {
    tx.blocking_send(message)
        .map_err(|_| anyhow::anyhow!("Worker stopped"))
}
fn jsonl(
    reader: impl Read,
    member: &str,
    account: Option<&str>,
    limits: &Limits,
    tx: &mpsc::Sender<Message>,
) -> anyhow::Result<i64> {
    let mut reader = BufReader::new(reader);
    let mut number = 0;
    loop {
        let mut bytes = Vec::new();
        let n = reader
            .by_ref()
            .take(limits.row_bytes as u64 + 1)
            .read_until(b'\n', &mut bytes)?;
        if n == 0 {
            break;
        }
        anyhow::ensure!(
            n <= limits.row_bytes,
            "JSONL row exceeds configured byte limit"
        );
        number += 1;
        let content = if number == 1 {
            bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)
        } else {
            &bytes
        };
        let (row, invalid) = match serde_json::from_slice::<Value>(content) {
            Ok(v) => (v, None),
            Err(_) => (
                json!({"unparsed_utf8":String::from_utf8_lossy(content)}),
                Some("Invalid JSONL row.".into()),
            ),
        };
        send(
            tx,
            Message::Row {
                member: member.into(),
                line: number,
                account: account.map(str::to_owned),
                row,
                invalid,
            },
        )?;
    }
    Ok(number)
}
fn music_member(name: &str) -> bool {
    let parts: Vec<_> = name.split('/').collect();
    parts.len() == 3
        && parts[0] == "listens"
        && parts[1].len() == 4
        && parts[1].bytes().all(|b| b.is_ascii_digit())
        && parts[2]
            .strip_suffix(".jsonl")
            .and_then(|s| s.parse::<u8>().ok())
            .is_some_and(|m| (1..=12).contains(&m))
}
fn parse_file(
    path: &Path,
    name: &str,
    limits: &Limits,
    account_hint: Option<&str>,
    tx: &mpsc::Sender<Message>,
) -> anyhow::Result<()> {
    let file = std::fs::File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.len() <= limits.upload_bytes,
        "File exceeds upload limit"
    );
    if name.to_ascii_lowercase().ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(file)?;
        anyhow::ensure!(zip.len() <= limits.members, "ZIP member limit exceeded");
        let mut names = BTreeSet::new();
        let mut total = 0u64;
        // Validate and inventory the entire central directory before parsing a member.
        for index in 0..zip.len() {
            let entry = zip.by_index(index)?;
            let name = entry.name()?.into_owned();
            anyhow::ensure!(
                !name.contains('\\')
                    && !name.starts_with('/')
                    && !name.contains(':')
                    && !name.split('/').any(|p| p == "..")
                    && entry.enclosed_name().is_some(),
                "Unsafe ZIP member"
            );
            anyhow::ensure!(names.insert(name.clone()), "Duplicate ZIP member");
            anyhow::ensure!(
                entry.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
                "ZIP symlinks are unsupported"
            );
            total = total
                .checked_add(entry.size())
                .ok_or_else(|| anyhow::anyhow!("ZIP size overflow"))?;
            anyhow::ensure!(
                total <= limits.expanded_bytes,
                "ZIP expanded byte limit exceeded"
            );
            send(
                tx,
                Message::Inventory {
                    member: name.clone(),
                    kind: if music_member(&name) {
                        "music"
                    } else {
                        "excluded"
                    }
                    .into(),
                },
            )?;
        }
        let account = if names.contains("user.json") {
            let mut reader = zip.by_name("user.json")?.take(65537);
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 65536, "user.json exceeds metadata limit");
            let user: Value = serde_json::from_slice(&bytes)?;
            text(&user, "username").map(str::to_lowercase)
        } else {
            None
        };
        if account
            .as_deref()
            .zip(account_hint)
            .is_some_and(|(a, b)| a != b)
        {
            anyhow::bail!("Account disagrees with ZIP user.json");
        }
        let account = account.as_deref().or(account_hint);
        send(tx, Message::Account(account.map(str::to_owned)))?;
        for index in 0..zip.len() {
            let entry = zip.by_index(index)?;
            let member = entry.name()?.into_owned();
            if !music_member(&member) {
                continue;
            }
            let expected = entry.size();
            let result = jsonl(entry.take(expected + 1), &member, account, limits, tx);
            send(
                tx,
                Message::End {
                    member,
                    count: result.as_ref().ok().copied(),
                    error: result.err().map(|_| {
                        "Unreadable JSONL member or row limit exceeded; original ZIP retained."
                            .into()
                    }),
                },
            )?;
        }
    } else {
        anyhow::ensure!(
            file.metadata()?.len() <= limits.expanded_bytes,
            "File exceeds expanded byte limit"
        );
        send(
            tx,
            Message::Inventory {
                member: name.into(),
                kind: "music".into(),
            },
        )?;
        if name.to_ascii_lowercase().ends_with(".jsonl") {
            send(tx, Message::Account(account_hint.map(str::to_owned)))?;
            let result = jsonl(
                file.take(limits.expanded_bytes + 1),
                name,
                account_hint,
                limits,
                tx,
            );
            send(
                tx,
                Message::End {
                    member: name.into(),
                    count: result.as_ref().ok().copied(),
                    error: result.err().map(|_| {
                        "Unreadable JSONL or row limit exceeded; original retained.".into()
                    }),
                },
            )?;
        } else {
            // Validate before publishing any array rows. Malformed JSON never silently
            // acknowledges a prefix as a complete file; other uploaded files still run.
            let account = read_json(file, limits.row_bytes, &mut |_| Ok(()))?;
            if account
                .as_deref()
                .zip(account_hint)
                .is_some_and(|(a, b)| a != b)
            {
                anyhow::bail!("Account disagrees with JSON response");
            }
            let account = account.as_deref().or(account_hint);
            send(tx, Message::Account(account.map(str::to_owned)))?;
            let mut count = 0;
            read_json(std::fs::File::open(path)?, limits.row_bytes, &mut |row| {
                count += 1;
                send(
                    tx,
                    Message::Row {
                        member: name.into(),
                        line: count,
                        account: account.map(str::to_owned),
                        row,
                        invalid: None,
                    },
                )
                .map_err(|e| e.to_string())
            })?;
            send(
                tx,
                Message::End {
                    member: name.into(),
                    count: Some(count),
                    error: None,
                },
            )?;
        }
    }
    Ok(())
}

async fn finish(storage: &Storage, lease: &Job, code: Option<&str>) -> Result<()> {
    let mut tx = storage.writer.begin().await?;
    crate::jobs::validate_lease(&mut tx, lease).await?;
    sqlx::query("UPDATE jobs SET state='completed',phase='finished',outcome=CASE WHEN ? IS NULL THEN 'complete' ELSE 'partial' END,error_code=?,retry_at_ms=NULL,updated_at_ms=? WHERE id=?").bind(code).bind(code).bind(auth::now_ms()).bind(&lease.id).execute(&mut *tx).await?;
    crate::artwork::enqueue_missing(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}
async fn flush(
    storage: &Storage,
    lease: &mut Job,
    member: &str,
    first: i64,
    rows: &mut Vec<SourceRow>,
) -> Result<()> {
    if !rows.is_empty() {
        storage
            .commit_member_batch(lease, member, first, rows, false)
            .await?;
        *lease = storage.job(&lease.id).await?;
        rows.clear();
    }
    Ok(())
}
pub async fn import_file(storage: &Storage, mut lease: Job, limits: Limits) -> Result<()> {
    let file = lease.file_id.as_ref().ok_or(AppError::Internal)?;
    let path = storage.directory.join("spool").join(file);
    let name = lease.filename.clone().ok_or(AppError::Internal)?;
    let (sender, mut receiver) = mpsc::channel(8);
    let account_hint = serde_json::from_str::<Value>(&lease.parameters_json)
        .ok()
        .and_then(|v| v["account"].as_str().map(str::to_owned));
    let parser = tokio::task::spawn_blocking(move || {
        parse_file(&path, &name, &limits, account_hint.as_deref(), &sender)
    });
    let mut rows = Vec::new();
    let mut active = String::new();
    let mut first = 1;
    let mut position = 0;
    let mut partial = false;
    let mut batch_bytes = 0;
    while let Some(message) = receiver.recv().await {
        match message {
            Message::Account(account) => {
                let mut parameters: Value =
                    serde_json::from_str(&lease.parameters_json).map_err(|_| AppError::Internal)?;
                parameters["account"] = json!(account);
                let mut tx = storage.writer.begin().await?;
                crate::jobs::validate_lease(&mut tx, &lease).await?;
                sqlx::query("UPDATE jobs SET parameters_json=?,phase='processing',inventory_complete=1 WHERE id=?")
                    .bind(parameters.to_string())
                    .bind(&lease.id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                lease = storage.job(&lease.id).await?;
            }
            Message::Inventory { member, kind } => {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM job_members WHERE job_id=? AND member=?)",
                )
                .bind(&lease.id)
                .bind(&member)
                .fetch_one(&storage.readers)
                .await?;
                if !exists {
                    storage
                        .record_member(
                            &lease.id,
                            &member,
                            &kind,
                            if kind == "excluded" {
                                "excluded"
                            } else {
                                "pending"
                            },
                            if kind == "excluded" { Some(0) } else { None },
                            None,
                        )
                        .await?;
                }
            }
            Message::Row {
                member,
                line,
                account,
                row,
                invalid,
            } => {
                if member != active {
                    flush(storage, &mut lease, &active, first, &mut rows).await?;
                    active = member;
                    position = sqlx::query_scalar(
                        "SELECT processed FROM job_members WHERE job_id=? AND member=?",
                    )
                    .bind(&lease.id)
                    .bind(&active)
                    .fetch_one(&storage.readers)
                    .await?;
                }
                if line <= position {
                    continue;
                }
                if rows.is_empty() {
                    first = line;
                }
                batch_bytes += row.to_string().len();
                rows.push(if let Some(reason) = invalid {
                    SourceRow::Invalid { raw: row, reason }
                } else {
                    parse(row, account.as_deref())
                });
                if rows.len() == 250 || batch_bytes >= 4 * 1024 * 1024 {
                    flush(storage, &mut lease, &active, first, &mut rows).await?;
                    batch_bytes = 0;
                }
            }
            Message::End {
                member,
                count,
                error,
            } => {
                flush(storage, &mut lease, &active, first, &mut rows).await?;
                batch_bytes = 0;
                let invalid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM source_occurrences WHERE job_id=? AND member=? AND outcome='invalid')").bind(&lease.id).bind(&member).fetch_one(&storage.readers).await?;
                partial |= error.is_some() || invalid;
                storage
                    .record_member(
                        &lease.id,
                        &member,
                        "music",
                        if error.is_some() || invalid {
                            "partial"
                        } else {
                            "completed"
                        },
                        count,
                        error.as_deref(),
                    )
                    .await?;
            }
        }
    }
    flush(storage, &mut lease, &active, first, &mut rows).await?;
    match parser.await.map_err(|_| AppError::Internal)? {
        Ok(()) => {
            finish(
                storage,
                &lease,
                partial.then_some("partial_import_review_report"),
            )
            .await
        }
        Err(_) => {
            storage
                .pause(
                    &lease,
                    None,
                    "invalid_archive_or_configured_limit_review_original",
                )
                .await
        }
    }
}

pub async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    let limits = Limits::from_env().map_err(|_| AppError::Internal)?;
    let mut jobs = Vec::new();
    // Queue each completely received file. A later interrupted multipart field never
    // invalidates a preceding complete file, and an incomplete field is never queued.
    let mut account = None;
    while let Some(field) = multipart.next_field().await.map_err(|_| {
        AppError::BadRequest(
            "Upload interrupted. Complete files remain in Activities; retry incomplete files."
                .into(),
        )
    })? {
        if field.name() == Some("account") {
            if !jobs.is_empty() {
                return Err(AppError::BadRequest(
                    "Account must precede uploaded files.".into(),
                ));
            }
            let mut reader =
                tokio_util::io::StreamReader::new(field.map_err(std::io::Error::other));
            let mut bytes = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(
                &mut tokio::io::AsyncReadExt::take(&mut reader, 257),
                &mut bytes,
            )
            .await
            .map_err(|_| AppError::BadRequest("Incomplete account field.".into()))?;
            if bytes.len() > 256 {
                return Err(AppError::BadRequest("Account name is too long.".into()));
            }
            let name = String::from_utf8(bytes)
                .map_err(|_| AppError::BadRequest("Invalid account name.".into()))?;
            if name.trim().len() > 64 || name.chars().any(|c| c.is_control()) {
                return Err(AppError::BadRequest("Invalid account name.".into()));
            }
            account = (!name.trim().is_empty()).then(|| name.trim().to_lowercase());
            continue;
        }
        if jobs.len() >= 50 {
            return Err(AppError::BadRequest(
                "Choose at most 50 files per upload.".into(),
            ));
        }
        let filename = field
            .file_name()
            .unwrap_or("listens.json")
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("listens.json")
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect::<String>();
        if ![".json", ".jsonl", ".zip"]
            .iter()
            .any(|s| filename.to_ascii_lowercase().ends_with(s))
        {
            return Err(AppError::BadRequest(
                "Choose ListenBrainz JSON, JSONL or ZIP files.".into(),
            ));
        }
        let mut reader = tokio_util::io::StreamReader::new(field.map_err(std::io::Error::other));
        let _files = state.storage.spool_operations.read().await;
        let id=state.storage.receive_locked(&mut reader,limits.upload_bytes).await.map_err(|_|AppError::BadRequest("Upload incomplete, exceeds the configured limit, or could not be saved. Complete files remain in Activities; reload this file.".into()))?;
        let job = state
            .storage
            .enqueue_locked(
                "listenbrainz_file",
                Some(&id),
                Some(&filename),
                &json!({"limits":limits,"account":account}),
            )
            .await?;
        jobs.push(job);
    }
    if jobs.is_empty() {
        return Err(AppError::BadRequest("Choose at least one file.".into()));
    }
    Ok(Json(json!({"jobs":jobs})))
}
pub async fn limits(State(_state): State<AppState>, headers: HeaderMap) -> Result<Json<Limits>> {
    auth::session(&_state, &headers).await?;
    Ok(Json(Limits::from_env().map_err(|_| AppError::Internal)?))
}
#[derive(Deserialize)]
pub struct RowQuery {
    pub offset: Option<i64>,
}
pub async fn originals(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<RowQuery>,
) -> Result<Json<Value>> {
    auth::session(&state, &headers).await?;
    let job = state.storage.job(&id).await?;
    if !job.kind.starts_with("listenbrainz_") {
        return Err(AppError::BadRequest("Not a ListenBrainz activity.".into()));
    }
    let offset = query.offset.unwrap_or(0).max(0);
    let rows=sqlx::query("SELECT member,row_number,outcome,reason,'null' AS raw_json,file_id,variant_id FROM source_occurrences WHERE job_id=? ORDER BY member,row_number LIMIT 10 OFFSET ?").bind(&id).bind(offset).fetch_all(&state.read_db).await?;
    use sqlx::Row;
    let rows:Vec<Value>=rows.into_iter().map(|row|json!({"member":row.get::<String,_>("member"),"row_number":row.get::<i64,_>("row_number"),"outcome":row.get::<String,_>("outcome"),"reason":row.get::<Option<String>,_>("reason"),"file_id":row.get::<Option<String>,_>("file_id"),"variant_id":row.get::<Option<String>,_>("variant_id"),"raw":Value::Null,"raw_available":false})).collect();
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_occurrences WHERE job_id=?")
        .bind(&id)
        .fetch_one(&state.read_db)
        .await?;
    let files:Vec<(String,String)>=sqlx::query_as("SELECT file_id,COALESCE(filename,'original upload') FROM jobs WHERE id=? AND file_id IN (SELECT id FROM source_files WHERE retained=1) UNION SELECT file_id,member FROM listenbrainz_pages WHERE job_id=? AND file_id IN (SELECT id FROM source_files WHERE retained=1)").bind(&id).bind(&id).fetch_all(&state.read_db).await?;
    Ok(Json(
        json!({"rows":rows,"total":total,"offset":offset,"files":files.into_iter().map(|(file_id,name)|json!({"file_id":file_id,"name":name})).collect::<Vec<_>>()}),
    ))
}
pub async fn original_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((id, file_id)): AxumPath<(String, String)>,
) -> Result<axum::response::Response> {
    auth::session(&state, &headers).await?;
    if file_id.len() != 64 || !file_id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::Forbidden);
    }
    let _files = state.storage.spool_operations.read().await;
    let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=? AND kind LIKE 'listenbrainz_%' AND (file_id=? OR EXISTS(SELECT 1 FROM listenbrainz_pages p WHERE p.job_id=jobs.id AND p.file_id=?)))").bind(id).bind(&file_id).bind(&file_id).fetch_one(&state.read_db).await?;
    if !allowed {
        return Err(AppError::Forbidden);
    }
    let retained: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM source_files WHERE id=? AND retained=1)")
            .bind(&file_id)
            .fetch_one(&state.read_db)
            .await?;
    if !retained {
        return Err(AppError::NotFound);
    }
    let file = tokio::fs::File::open(state.storage.directory.join("spool").join(file_id))
        .await
        .map_err(|_| AppError::Internal)?;
    use axum::response::IntoResponse;
    Ok((
        [
            ("content-type", "application/octet-stream"),
            (
                "content-disposition",
                "attachment; filename=\"listenbrainz-source.bin\"",
            ),
        ],
        axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
}

#[derive(Deserialize)]
pub struct FetchRequest {
    pub username: String,
    pub from: Option<String>,
    pub to: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct FetchParameters {
    pub username: String,
    pub timezone: String,
    pub start: i64,
    pub end: i64,
    pub cursor: i64,
    pub pages: u32,
}
pub fn fetch_parameters(
    form: FetchRequest,
    timezone: &str,
    now_ms: i64,
) -> Result<FetchParameters> {
    let username = form.username.trim().to_lowercase();
    if username.is_empty() || username.len() > 64 || username.chars().any(|c| c.is_control()) {
        return Err(AppError::BadRequest("Enter a MusicBrainz username.".into()));
    }
    let tz: chrono_tz::Tz = timezone.parse().map_err(|_| AppError::Internal)?;
    let bound = |date: Option<String>, next: bool| -> Result<Option<i64>> {
        date.filter(|s|!s.is_empty()).map(|s|{
            let date=NaiveDate::parse_from_str(&s,"%Y-%m-%d").map_err(|_|AppError::BadRequest("Use YYYY-MM-DD dates.".into()))?;
            let date=if next {date.succ_opt().ok_or_else(||AppError::BadRequest("Date out of range.".into()))?}else{date};
            tz.from_local_datetime(&date.and_hms_opt(0,0,0).ok_or(AppError::Internal)?).single().map(|d|d.timestamp()).ok_or_else(||AppError::BadRequest("This date has an ambiguous or missing midnight in the instance time zone.".into()))
        }).transpose()
    };
    let end = bound(form.to, true)?
        .unwrap_or(now_ms / 1000 + 1)
        .min(now_ms / 1000 + 1);
    let start = bound(form.from, false)?.unwrap_or(end - 14 * 86400);
    if start < 0 || start >= end {
        return Err(AppError::BadRequest(
            "Choose a nonempty date range ending after its start.".into(),
        ));
    }
    if end - start > 31 * 86400 {
        return Err(AppError::BadRequest("For more than 31 days, upload the official ListenBrainz ZIP from Settings → Export. No token is needed here.".into()));
    }
    Ok(FetchParameters {
        username,
        timezone: timezone.into(),
        start,
        end,
        cursor: end,
        pages: 0,
    })
}
pub async fn fetch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<FetchRequest>,
) -> Result<Json<Value>> {
    auth::require_write(&state, &headers).await?;
    let mut conn = state.read_db.acquire().await?;
    let calendar = crate::calendar::current(&mut conn).await?;
    let params = fetch_parameters(form, &calendar.timezone, auth::now_ms())?;
    let id = state
        .storage
        .enqueue(
            "listenbrainz_fetch",
            None,
            Some(&format!("ListenBrainz · {}", params.username)),
            &json!(params),
        )
        .await?;
    Ok(Json(json!({"jobs":[id],"interval":params})))
}

/// Process a validated descending page. Delay the oldest second until the next
/// overlapping page, so a normal page boundary never drops equal timestamps.
pub struct Page {
    pub rows: Vec<Value>,
    pub next: i64,
    pub complete: bool,
    pub doubtful: bool,
}
pub fn page(value: &Value, params: &FetchParameters) -> std::result::Result<Page, &'static str> {
    if text(&value["payload"], "user_id").is_some_and(|name| name.to_lowercase() != params.username)
    {
        return Err("listenbrainz_account_mismatch");
    }
    let rows = value["payload"]["listens"]
        .as_array()
        .ok_or("malformed_page")?;
    if rows.len() > 1000 || value["payload"]["playing_now"].as_bool() == Some(true) {
        return Err("malformed_page");
    }
    let mut previous = params.cursor;
    for row in rows {
        let ts = seconds(&row["listened_at"]).ok_or("malformed_page")?;
        if ts >= params.cursor || ts > previous {
            return Err("nonadvancing_or_unsorted_page");
        }
        previous = ts;
    }
    if rows.is_empty() {
        return Ok(Page {
            rows: vec![],
            next: params.start,
            complete: true,
            doubtful: false,
        });
    }
    let oldest =
        seconds(&rows.last().ok_or("malformed_page")?["listened_at"]).ok_or("malformed_page")?;
    let full = rows.len() == 1000;
    let stuck = full && seconds(&rows[0]["listened_at"]) == Some(oldest);
    let complete = !full || oldest < params.start;
    let selected = rows
        .iter()
        .filter(|r| {
            let ts = seconds(&r["listened_at"]).unwrap_or(0);
            ts >= params.start && (complete || stuck || ts > oldest)
        })
        .cloned()
        .collect();
    Ok(Page {
        rows: selected,
        next: oldest + 1,
        complete: complete && !stuck,
        doubtful: stuck,
    })
}
fn wait_seconds(headers: &reqwest::header::HeaderMap) -> u64 {
    let number = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
    };
    let reset = if number("x-ratelimit-remaining") == Some(0) {
        number("x-ratelimit-reset-in").unwrap_or(60)
    } else {
        1
    };
    let date_wait = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
        .map(|d| (d.timestamp() - auth::now_ms() / 1000).max(0) as u64);
    reset
        .max(number("retry-after").or(date_wait).unwrap_or(1))
        .max(1)
}
async fn fetch_page(
    storage: &Storage,
    mut lease: Job,
    client: &reqwest::Client,
    base: &str,
) -> Result<()> {
    let mut params: FetchParameters =
        serde_json::from_str(&lease.parameters_json).map_err(|_| AppError::Internal)?;
    let mut url = url::Url::parse(base).map_err(|_| AppError::Internal)?;
    url.path_segments_mut()
        .map_err(|_| AppError::Internal)?
        .extend(["1", "user", &params.username, "listens"]);
    url.query_pairs_mut()
        .append_pair("count", "1000")
        .append_pair("max_ts", &params.cursor.to_string());
    // A persistent global gate is reserved before I/O, so separate jobs and process
    // restarts cannot exceed one request per second or bypass a provider wait.
    let now = auth::now_ms();
    let next: i64 =
        sqlx::query_scalar("SELECT next_request_ms FROM listenbrainz_gate WHERE singleton=1")
            .fetch_one(&storage.writer)
            .await?;
    if next > now {
        return storage
            .pause(&lease, Some(next), "listenbrainz_rate_wait")
            .await;
    }
    sqlx::query("UPDATE listenbrainz_gate SET next_request_ms=? WHERE singleton=1")
        .bind(now + 1000)
        .execute(&storage.writer)
        .await?;
    let response = match client.get(url).send().await {
        Ok(r) => r,
        Err(_) => {
            return storage
                .pause(&lease, Some(now + 30_000), "listenbrainz_network_wait")
                .await;
        }
    };
    let wait = i64::try_from(wait_seconds(response.headers()))
        .unwrap_or(i64::MAX)
        .saturating_mul(1000);
    sqlx::query(
        "UPDATE listenbrainz_gate SET next_request_ms=MAX(next_request_ms,?) WHERE singleton=1",
    )
    .bind(auth::now_ms().saturating_add(wait))
    .execute(&storage.writer)
    .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return storage
            .pause(&lease, None, "listenbrainz_username_not_found")
            .await;
    }
    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
        || response.status().is_server_error()
    {
        return storage
            .pause(
                &lease,
                Some(auth::now_ms().saturating_add(wait.max(30_000))),
                "listenbrainz_service_wait",
            )
            .await;
    }
    if !response.status().is_success() {
        return storage
            .pause(&lease, None, "listenbrainz_request_rejected")
            .await;
    }
    let mut response = response;
    let mut bytes = Vec::new();
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(_) => {
                return storage
                    .pause(
                        &lease,
                        Some(auth::now_ms() + 30_000),
                        "listenbrainz_network_wait",
                    )
                    .await;
            }
        };
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return storage
                .pause(&lease, None, "listenbrainz_page_too_large_use_zip")
                .await;
        }
        bytes.extend_from_slice(&chunk);
    }
    // A fully received page is retained even if its JSON/order cannot be interpreted.
    let member = format!("page-{}-attempt-{}", params.pages + 1, lease.attempt);
    {
        // A repeated page can share a hash with a completed acquisition. Keep
        // receipt and attachment under one lease, just like multipart uploads.
        let _files = storage.spool_operations.read().await;
        let file_id = storage
            .receive_locked(&mut bytes.as_slice(), 8 * 1024 * 1024)
            .await
            .map_err(|_| AppError::Internal)?;
        sqlx::query("INSERT INTO listenbrainz_pages(job_id,member,file_id) VALUES(?,?,?) ON CONFLICT(job_id,member) DO UPDATE SET file_id=excluded.file_id").bind(&lease.id).bind(&member).bind(file_id).execute(&storage.writer).await?;
    }
    let value = match serde_json::from_slice::<Value>(&bytes) {
        Ok(v) => v,
        Err(_) => {
            storage
                .record_member(
                    &lease.id,
                    &member,
                    "music",
                    "unreadable",
                    None,
                    Some("listenbrainz_malformed_page_use_zip"),
                )
                .await?;
            return storage
                .pause(&lease, None, "listenbrainz_malformed_page_use_zip")
                .await;
        }
    };
    let result = match page(&value, &params) {
        Ok(p) => p,
        Err(code) => {
            storage
                .record_member(&lease.id, &member, "music", "unprocessed", None, Some(code))
                .await?;
            return storage.pause(&lease, None, code).await;
        }
    };
    params.cursor = result.next;
    params.pages += 1;
    let rows: Vec<_> = result
        .rows
        .into_iter()
        .map(|row| parse(row, Some(&params.username)))
        .collect();
    let doubtful = result.doubtful || (!result.complete && params.pages >= 10);
    let terminal = result.complete || doubtful;
    let mut parameters = json!(params);
    if doubtful {
        parameters["_terminal_code"] = json!("coverage_uncertain_use_official_zip");
    } else if terminal
        && (lease.invalid > 0 || rows.iter().any(|r| matches!(r, SourceRow::Invalid { .. })))
    {
        parameters["_terminal_code"] = json!("partial_import_review_report");
    }
    storage
        .commit_source_batch(&lease, &member, 1, &rows, terminal, Some(&parameters))
        .await?;
    lease = storage.job(&lease.id).await?;
    storage
        .record_member(
            &lease.id,
            &member,
            "music",
            if doubtful || rows.iter().any(|r| matches!(r, SourceRow::Invalid { .. })) {
                "partial"
            } else {
                "completed"
            },
            Some(rows.len() as i64),
            parameters["_terminal_code"].as_str(),
        )
        .await?;
    if terminal {
        return Ok(());
    }
    storage
        .pause(
            &lease,
            Some(auth::now_ms().saturating_add(wait)),
            "listenbrainz_rate_wait",
        )
        .await
}
pub async fn worker(storage: Storage) {
    let client = reqwest::Client::builder()
        .user_agent(concat!(
            "scrobbl.rs/",
            env!("CARGO_PKG_VERSION"),
            " ( https://github.com/itsmrma/scrobbl-rs )"
        ))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("HTTP client");
    loop {
        for kind in ["listenbrainz_file", "listenbrainz_fetch"] {
            match storage.claim(kind).await {
                Ok(Some(lease)) => {
                    let result = if kind == "listenbrainz_file" {
                        let limits = serde_json::from_str::<Value>(&lease.parameters_json)
                            .ok()
                            .and_then(|v| serde_json::from_value(v["limits"].clone()).ok())
                            .unwrap_or_default();
                        import_file(&storage, lease.clone(), limits).await
                    } else {
                        fetch_page(
                            &storage,
                            lease.clone(),
                            &client,
                            "https://api.listenbrainz.org/",
                        )
                        .await
                    };
                    if result.is_err() {
                        let current = storage.job(&lease.id).await;
                        if let Ok(current) = current {
                            let _ = storage
                                .pause(
                                    &current,
                                    None,
                                    "listenbrainz_processing_failed_saved_rows_retained",
                                )
                                .await;
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("ListenBrainz queue: {e}"),
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;
    use axum::{Router, http::StatusCode, response::IntoResponse, routing::get};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };
    type Answers = Arc<Mutex<VecDeque<(StatusCode, Value)>>>;
    async fn mock(answers: Answers) -> (String, tokio::task::JoinHandle<()>) {
        let app=Router::new().route("/1/user/listener/listens",get(move |headers:HeaderMap,Query(query):Query<std::collections::HashMap<String,String>>| {
            let answers=answers.clone();async move {
                assert!(headers["user-agent"].to_str().unwrap().contains("scrobbl"));
                assert_eq!(query.get("count").map(String::as_str),Some("1000"));assert!(!query.contains_key("min_ts"));
                let (status,value)=answers.lock().unwrap().pop_front().expect("No unexpected extra page requests");
                (status,[("x-ratelimit-remaining","0"),("x-ratelimit-reset-in","7")],Json(value)).into_response()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, handle)
    }
    fn row(ts: i64) -> Value {
        json!({"listened_at":ts,"track_metadata":{"artist_name":"A","track_name":"T"}})
    }
    async fn ready(store: &Storage, id: &str) {
        sqlx::query("UPDATE listenbrainz_gate SET next_request_ms=0")
            .execute(&store.writer)
            .await
            .unwrap();
        sqlx::query("UPDATE jobs SET retry_at_ms=0 WHERE id=?")
            .bind(id)
            .execute(&store.writer)
            .await
            .unwrap();
    }
    async fn enqueue(store: &Storage) -> String {
        store
            .enqueue(
                "listenbrainz_fetch",
                None,
                None,
                &json!(FetchParameters {
                    username: "listener".into(),
                    timezone: "UTC".into(),
                    start: 100,
                    end: 10000,
                    cursor: 10000,
                    pages: 0
                }),
            )
            .await
            .unwrap()
    }
    #[tokio::test]
    async fn public_fetch_preserves_progress_waits_across_restart_and_finishes_atomically() {
        let first: Vec<_> = (1000..2000).rev().map(row).collect();
        let answers: Answers = Arc::new(Mutex::new(VecDeque::from([
            (
                StatusCode::OK,
                json!({"payload":{"user_id":"listener","listens":first}}),
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"error":"temporarily unavailable"}),
            ),
            (
                StatusCode::OK,
                json!({"payload":{"user_id":"listener","listens":[row(1000),row(999)]}}),
            ),
        ])));
        let (base, server) = mock(answers.clone()).await;
        let client = reqwest::Client::builder()
            .user_agent("scrobbl-test/1.0")
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = db::open(dir.path()).await.unwrap();
        let id = enqueue(&store).await;
        fetch_page(
            &store,
            store.claim("listenbrainz_fetch").await.unwrap().unwrap(),
            &client,
            &base,
        )
        .await
        .unwrap();
        let saved = store.job(&id).await.unwrap();
        assert_eq!(saved.processed, 999);
        assert_eq!(saved.state, "waiting");
        assert!(saved.retry_at_ms.unwrap() >= auth::now_ms() + 6000);
        assert!(store.claim("listenbrainz_fetch").await.unwrap().is_none());
        store.close().await;
        let store = db::open(dir.path()).await.unwrap();
        assert!(store.claim("listenbrainz_fetch").await.unwrap().is_none());
        ready(&store, &id).await;
        fetch_page(
            &store,
            store.claim("listenbrainz_fetch").await.unwrap().unwrap(),
            &client,
            &base,
        )
        .await
        .unwrap();
        assert_eq!(store.job(&id).await.unwrap().processed, 999);
        assert_eq!(store.job(&id).await.unwrap().state, "waiting");
        store.cancel(&id).await.unwrap();
        store.close().await;
        let store = db::open(dir.path()).await.unwrap();
        assert!(store.claim("listenbrainz_fetch").await.unwrap().is_none());
        store.retry(&id).await.unwrap();
        ready(&store, &id).await;
        fetch_page(
            &store,
            store.claim("listenbrainz_fetch").await.unwrap().unwrap(),
            &client,
            &base,
        )
        .await
        .unwrap();
        let saved = store.job(&id).await.unwrap();
        assert_eq!(saved.state, "completed");
        assert_eq!(saved.processed, 1001);
        assert_eq!(saved.added, 1001);
        assert!(saved.error_code.is_none());
        assert!(answers.lock().unwrap().is_empty());
        let unlinked: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM source_occurrences WHERE file_id IS NULL")
                .fetch_one(&store.readers)
                .await
                .unwrap();
        assert_eq!(unlinked, 0);
        store.close().await;
        let store = db::open(dir.path()).await.unwrap();
        assert!(store.claim("listenbrainz_fetch").await.unwrap().is_none());
        store.close().await;
        server.abort();
    }
    #[tokio::test]
    async fn saturated_timestamp_group_is_saved_but_never_reported_complete() {
        let answers: Answers = Arc::new(Mutex::new(VecDeque::from([(
            StatusCode::OK,
            json!({"payload":{"listens":vec![row(1000);1000]}}),
        )])));
        let (base, server) = mock(answers).await;
        let client = reqwest::Client::builder()
            .user_agent("scrobbl-test/1.0")
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = db::open(dir.path()).await.unwrap();
        let id = enqueue(&store).await;
        fetch_page(
            &store,
            store.claim("listenbrainz_fetch").await.unwrap().unwrap(),
            &client,
            &base,
        )
        .await
        .unwrap();
        let job = store.job(&id).await.unwrap();
        assert_eq!(job.processed, 1000);
        assert_eq!(job.state, "completed");
        assert_eq!(
            job.error_code.as_deref(),
            Some("coverage_uncertain_use_official_zip")
        );
        store.close().await;
        server.abort();
    }
    #[tokio::test]
    async fn not_found_rate_limit_empty_and_malformed_pages_are_distinct() {
        let answers: Answers = Arc::new(Mutex::new(VecDeque::from([
            (StatusCode::NOT_FOUND, json!({})),
            (StatusCode::TOO_MANY_REQUESTS, json!({})),
            (StatusCode::OK, json!({"payload":{"listens":[]}})),
            (StatusCode::OK, json!({"payload":{"listens":[row(10000)]}})),
        ])));
        let (base, server) = mock(answers).await;
        let client = reqwest::Client::builder()
            .user_agent("scrobbl-test/1.0")
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = db::open(dir.path()).await.unwrap();
        let id = enqueue(&store).await;
        fetch_page(
            &store,
            store.claim("listenbrainz_fetch").await.unwrap().unwrap(),
            &client,
            &base,
        )
        .await
        .unwrap();
        assert_eq!(
            store.job(&id).await.unwrap().error_code.as_deref(),
            Some("listenbrainz_username_not_found")
        );
        let id = enqueue(&store).await;
        ready(&store, &id).await;
        fetch_page(
            &store,
            store.claim("listenbrainz_fetch").await.unwrap().unwrap(),
            &client,
            &base,
        )
        .await
        .unwrap();
        assert_eq!(store.job(&id).await.unwrap().state, "waiting");
        assert_eq!(store.job(&id).await.unwrap().processed, 0);
        ready(&store, &id).await;
        fetch_page(
            &store,
            store.claim("listenbrainz_fetch").await.unwrap().unwrap(),
            &client,
            &base,
        )
        .await
        .unwrap();
        assert_eq!(store.job(&id).await.unwrap().state, "completed");
        assert_eq!(store.job(&id).await.unwrap().processed, 0);
        let id = enqueue(&store).await;
        ready(&store, &id).await;
        fetch_page(
            &store,
            store.claim("listenbrainz_fetch").await.unwrap().unwrap(),
            &client,
            &base,
        )
        .await
        .unwrap();
        assert_eq!(store.job(&id).await.unwrap().state, "failed");
        assert_eq!(
            store.job(&id).await.unwrap().error_code.as_deref(),
            Some("nonadvancing_or_unsorted_page")
        );
        store.close().await;
        server.abort();
    }
}
