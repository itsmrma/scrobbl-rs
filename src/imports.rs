//! Spotify source parsing and durable multipart receipt. Processing is independent of HTTP.
use crate::{
    AppState, auth,
    db::{self, Artist, Event, IngestReport, Track},
    error::{AppError, Result},
    jobs::SourceRow,
};
use axum::{
    Json,
    extract::{Multipart, State},
    http::HeaderMap,
};
use chrono::DateTime;
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
pub struct ImportLimits {
    pub upload_bytes: u64,
    pub expanded_bytes: u64,
    pub member_bytes: u64,
    pub row_bytes: u64,
    pub files: usize,
}
impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            upload_bytes: 256 * 1024 * 1024,
            expanded_bytes: 1024 * 1024 * 1024,
            member_bytes: 256 * 1024 * 1024,
            row_bytes: 2 * 1024 * 1024,
            files: 10000,
        }
    }
}
impl ImportLimits {
    pub fn from_env() -> anyhow::Result<Self> {
        let mut limits = Self::default();
        for (key, target) in [
            ("SCROBBL_IMPORT_UPLOAD_BYTES", &mut limits.upload_bytes),
            ("SCROBBL_IMPORT_EXPANDED_BYTES", &mut limits.expanded_bytes),
            ("SCROBBL_IMPORT_MEMBER_BYTES", &mut limits.member_bytes),
            ("SCROBBL_IMPORT_ROW_BYTES", &mut limits.row_bytes),
        ] {
            if let Ok(value) = std::env::var(key) {
                *target = value.parse()?;
            }
            anyhow::ensure!(
                *target > 0 && *target <= i64::MAX as u64,
                "{key} must be positive and fit in 64 bits"
            );
        }
        if let Ok(value) = std::env::var("SCROBBL_IMPORT_FILES") {
            limits.files = value.parse()?;
        }
        anyhow::ensure!(
            limits.files > 0 && limits.files <= 100000,
            "SCROBBL_IMPORT_FILES must be between 1 and 100000"
        );
        Ok(limits)
    }
}
pub async fn limits(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ImportLimits>> {
    auth::session(&state, &headers).await?;
    Ok(Json(state.config.import_limits))
}
fn text<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key)?.as_str().filter(|v| !v.trim().is_empty())
}
pub fn spotify_uri(value: &str) -> bool {
    value
        .strip_prefix("spotify:track:")
        .is_some_and(|id| id.len() == 22 && id.bytes().all(|c| c.is_ascii_alphanumeric()))
}
pub fn parse_row(row: Value) -> SourceRow {
    if row.get("spotify_episode_uri").is_some_and(|v| !v.is_null())
        || row
            .get("spotify_audiobook_uri")
            .is_some_and(|v| !v.is_null())
        || row.get("audiobook_uri").is_some_and(|v| !v.is_null())
    {
        return SourceRow::Skipped {
            raw: row,
            reason: "Podcast or audiobook excluded from this release.".into(),
        };
    }
    let parsed = (|| -> std::result::Result<Event, &'static str> {
        let title = text(&row, "master_metadata_track_name").ok_or("Missing music track title.")?;
        let artist = text(&row, "master_metadata_album_artist_name").ok_or("Missing artist.")?;
        let ms = row
            .get("ms_played")
            .and_then(Value::as_i64)
            .filter(|v| (0..=604_800_000).contains(v))
            .ok_or("Invalid ms_played (expected milliseconds from 0 to 7 days).")?;
        let end = DateTime::parse_from_rfc3339(text(&row, "ts").ok_or("Missing timestamp.")?)
            .map_err(|_| "Invalid timestamp (expected RFC3339 with timezone).")?
            .timestamp_millis();
        if !(0..=auth::now_ms() + 86_400_000).contains(&end) {
            return Err("Timestamp outside the supported range.");
        }
        let album = text(&row, "master_metadata_album_album_name").map(str::to_owned);
        let uri = text(&row, "spotify_track_uri");
        if uri.is_some_and(|uri| !spotify_uri(uri)) {
            return Err("Invalid Spotify track URI.");
        }
        let id = uri.map(str::to_owned).unwrap_or_else(|| {
            db::metadata_id(
                "local-track",
                &[title, artist, album.as_deref().unwrap_or("")],
            )
        });
        let key = if uri.is_some() {
            db::metadata_id("event", &[&id, &end.to_string(), &ms.to_string()])
        } else {
            db::metadata_id("uncertain-event", &[&db::payload_fingerprint(&row)])
        };
        let album_id = album
            .as_deref()
            .map(|name| db::metadata_id("import-album", &[name, artist]));
        Ok(Event {
            source: "spotify_import",
            source_key: key,
            source_timestamp_ms: end,
            played_at_ms: end - ms,
            listened_ms: Some(ms),
            duration_kind: db::DurationKind::Actual,
            counted: ms >= 30_000,
            track: Track {
                id,
                title: title.into(),
                album_id,
                album,
                cover_url: None,
                duration_ms: None,
                artists: vec![Artist {
                    id: db::metadata_id("import-artist", &[artist]),
                    name: artist.into(),
                }],
                structured: false,
            },
            raw: row.clone(),
        })
    })();

    match parsed {
        Ok(event) => SourceRow::from(event),
        Err(reason) => SourceRow::Invalid {
            raw: row,
            reason: reason.into(),
        },
    }
}
// Small in-memory convenience for existing domain tests. The production worker streams arrays.
pub fn parse_extended(bytes: &[u8]) -> Result<(Vec<Event>, IngestReport)> {
    let rows: Vec<Value> = serde_json::from_slice(
        bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes),
    )
    .map_err(|_| AppError::BadRequest("Expected a Spotify extended-history JSON array.".into()))?;
    let mut events = Vec::new();
    let mut report = IngestReport::default();
    for (index, row) in rows.into_iter().enumerate() {
        match parse_row(row) {
            SourceRow::Event(e) => events.push(*e),
            SourceRow::Invalid { reason, .. } => {
                report.invalid += 1;
                if report.errors.len() < 10 {
                    report.errors.push(format!("Row {}: {reason}", index + 1));
                }
            }
            SourceRow::Skipped { .. } => report.skipped += 1,
        }
    }
    Ok((events, report))
}
#[derive(Serialize, ts_rs::TS)]
pub struct ImportResponse {
    pub job_id: String,
}

struct ReceiptGuard {
    storage: crate::storage::Storage,
    id: String,
}
impl Drop for ReceiptGuard {
    fn drop(&mut self) {
        let storage = self.storage.clone();
        let id = self.id.clone();
        tokio::spawn(async move {
            let _=sqlx::query("UPDATE jobs SET state='failed',outcome='error',error_code='upload_incomplete: send the complete selection again',retry_at_ms=NULL WHERE id=? AND phase='receiving' AND state='waiting'").bind(id).execute(&storage.writer).await;
        });
    }
}
pub async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<ImportResponse>> {
    auth::require_write(&state, &headers).await?;
    // A receiving job is visible but cannot be claimed. Incomplete multipart bodies never enter processing.
    let id = db::new_id();
    sqlx::query("INSERT INTO jobs(id,kind,state,phase,filename,created_at_ms,updated_at_ms,retry_at_ms) SELECT ?,'spotify_import','waiting','receiving','Spotify history upload',?,?,9223372036854775807 FROM archive_state WHERE maintenance=0")
        .bind(&id).bind(auth::now_ms()).bind(auth::now_ms()).execute(&state.db).await?;
    if state.storage.job(&id).await.is_err() {
        return Err(AppError::Conflict("Archive is in maintenance.".into()));
    }
    let _receipt = ReceiptGuard {
        storage: state.storage.clone(),
        id: id.clone(),
    };
    let result = receive_fields(&state, &id, &mut multipart).await;
    if let Err(error) = result {
        sqlx::query("UPDATE jobs SET state='failed',outcome='error',error_code=?,retry_at_ms=NULL WHERE id=?")
            .bind(error.to_string()).bind(&id).execute(&state.db).await?;
        return Err(AppError::BadRequest(format!(
            "{error} Upload report is in Activities. Fully received files are retained; upload the complete selection again to process it."
        )));
    }
    sqlx::query("UPDATE jobs SET state='queued',phase='inventory',retry_at_ms=NULL,parameters_json=? WHERE id=?")
        .bind(json!(state.config.import_limits).to_string()).bind(&id).execute(&state.db).await?;
    Ok(Json(ImportResponse { job_id: id }))
}
async fn receive_fields(state: &AppState, id: &str, multipart: &mut Multipart) -> Result<()> {
    let mut total = 0u64;
    let mut ordinal = 0usize;
    while let Some(field)=multipart.next_field().await.map_err(|_|AppError::BadRequest("Upload incomplete or request byte limit exceeded. Send the entire ZIP or selection again.".into()))? {
        ordinal+=1;
        if ordinal>state.config.import_limits.files {return Err(AppError::BadRequest("Configured file-count limit exceeded.".into()));}
        let filename=field.file_name().unwrap_or("history.json").rsplit(['/', '\\']).next().unwrap_or("history.json").chars().filter(|c|!c.is_control()).take(200).collect::<String>();
        let mut reader=tokio_util::io::StreamReader::new(field.map_err(std::io::Error::other));
        let file_id=state.storage.receive(&mut reader,state.config.import_limits.upload_bytes.saturating_sub(total)).await.map_err(|_|AppError::BadRequest("Upload incomplete, configured upload byte limit exceeded, or server storage unavailable. Check storage and send the complete upload again.".into()))?;
        let size:i64=sqlx::query_scalar("SELECT size_bytes FROM source_files WHERE id=?").bind(&file_id).fetch_one(&state.db).await?;
        total+=size as u64;
        sqlx::query("INSERT INTO job_inputs(job_id,ordinal,file_id,filename) VALUES(?,?,?,?)").bind(id).bind(ordinal as i64).bind(&file_id).bind(&filename).execute(&state.db).await?;
        // Receipt details survive interruption. The worker replaces these with the complete member inventory.
        state.storage.record_member(id,&format!("{ordinal}: {filename}"),"received","received",None,None).await?;
    }
    if ordinal == 0 {
        return Err(AppError::BadRequest(
            "Choose a ZIP or one or more JSON files.".into(),
        ));
    }
    Ok(())
}
