use crate::{
    AppState, auth,
    db::{self, Artist, Event, IngestReport, Track},
    error::{AppError, Result},
};
use axum::{
    Json,
    extract::{Multipart, State},
    http::HeaderMap,
};
use chrono::DateTime;
use serde::Serialize;
use serde_json::Value;

fn text<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key)?.as_str().filter(|v| !v.trim().is_empty())
}
pub fn spotify_uri(value: &str) -> bool {
    value
        .strip_prefix("spotify:track:")
        .is_some_and(|id| id.len() == 22 && id.bytes().all(|c| c.is_ascii_alphanumeric()))
}
pub fn parse_extended(bytes: &[u8]) -> Result<(Vec<Event>, IngestReport)> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let rows: Vec<Value> = serde_json::from_slice(bytes).map_err(|_| {
        AppError::BadRequest(
            "Upload a Spotify Extended Streaming History JSON array. Unzip the archive first."
                .into(),
        )
    })?;
    if rows.len() > 200_000 {
        return Err(AppError::BadRequest(
            "This file contains more than 200,000 records. Split it into smaller JSON files."
                .into(),
        ));
    }
    let mut events = Vec::with_capacity(rows.len());
    let mut report = IngestReport::default();
    for (index, row) in rows.into_iter().enumerate() {
        // Reject other formats rather than reporting a successful, empty import.
        if !row.is_object() || row.get("ts").is_none() || row.get("ms_played").is_none() {
            report.invalid += 1;
            if report.errors.len() < 10 {
                report.errors.push(format!(
                    "Row {}: expected extended-history fields ts and ms_played.",
                    index + 1
                ));
            }
            continue;
        }
        if row.get("spotify_episode_uri").is_some_and(|v| !v.is_null())
            || row
                .get("spotify_audiobook_uri")
                .is_some_and(|v| !v.is_null())
        {
            report.skipped += 1;
            continue;
        }
        let parsed = (|| -> std::result::Result<Event, &'static str> {
            let title =
                text(&row, "master_metadata_track_name").ok_or("Missing music track title.")?;
            let artist =
                text(&row, "master_metadata_album_artist_name").ok_or("Missing artist.")?;
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
            let key = db::metadata_id("event", &[&id, &end.to_string(), &ms.to_string()]);
            let album_id = album
                .as_deref()
                .map(|name| db::metadata_id("import-album", &[name, artist]));
            Ok(Event {
                source: "spotify_import",
                source_key: key,
                source_timestamp_ms: end,
                played_at_ms: end - ms,
                listened_ms: Some(ms),
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
            Ok(event) => events.push(event),
            Err(message) => {
                report.invalid += 1;
                if report.errors.len() < 10 {
                    report.errors.push(format!("Row {}: {message}", index + 1));
                }
            }
        }
    }
    Ok((events, report))
}
#[derive(Serialize)]
pub struct FileReport {
    pub filename: String,
    #[serde(flatten)]
    pub report: IngestReport,
}
#[derive(Serialize)]
pub struct ImportResponse {
    pub files: Vec<FileReport>,
    pub total: IngestReport,
}
pub async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<ImportResponse>> {
    auth::require_write(&state, &headers).await?;
    let _guard = state
        .imports
        .try_lock()
        .map_err(|_| AppError::Conflict("Another import is running. Please wait.".into()))?;
    let mut response = ImportResponse {
        files: Vec::new(),
        total: IngestReport::default(),
    };
    while let Some(field) = multipart.next_field().await.map_err(|_| {
        AppError::BadRequest(
            "The upload is invalid or exceeds 64 MiB. Upload fewer JSON files at a time.".into(),
        )
    })? {
        if response.files.len() >= 50 {
            return Err(AppError::BadRequest(
                "Upload at most 50 files at a time.".into(),
            ));
        }
        let filename = field
            .file_name()
            .unwrap_or("history.json")
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("history.json")
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect::<String>();
        let bytes = field.bytes().await.map_err(|_| {
            AppError::BadRequest(
                "The upload is invalid or exceeds 64 MiB. Upload fewer JSON files at a time."
                    .into(),
            )
        })?;
        let (events, mut report) = tokio::task::spawn_blocking(move || parse_extended(&bytes))
            .await
            .map_err(|_| AppError::Internal)??;
        let mut tx = state.db.begin().await?;
        report.add(&db::ingest(&mut tx, &events).await?);
        sqlx::query(
            "INSERT INTO import_jobs(filename, created_at_ms, report_json) VALUES (?, ?, ?)",
        )
        .bind(&filename)
        .bind(auth::now_ms())
        .bind(serde_json::to_string(&report).map_err(|_| AppError::Internal)?)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        response.total.add(&report);
        response.files.push(FileReport { filename, report });
    }
    if response.files.is_empty() {
        return Err(AppError::BadRequest(
            "Choose one or more JSON files.".into(),
        ));
    }
    Ok(Json(response))
}
