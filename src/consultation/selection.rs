//! Canonical predicates shared by consultation, history and frozen exports.
use crate::{
    auth,
    error::{AppError, Result},
};
use chrono::{Datelike, NaiveDate};
use serde::Deserialize;
use sqlx::{QueryBuilder, Sqlite};
#[derive(Default, Deserialize, serde::Serialize, Clone)]
pub struct Filters {
    pub period: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub q: Option<String>,
    pub page: Option<u32>,
    pub status: Option<String>,
    pub sort: Option<String>,
    pub page_size: Option<i64>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track: Option<String>,
    pub source: Option<String>,
    pub grain: Option<String>,
    pub bucket: Option<String>,
    pub bucket_start: Option<String>,
}
pub async fn selection(
    conn: &mut sqlx::SqliteConnection,
    filters: &Filters,
) -> Result<QueryBuilder<Sqlite>> {
    selection_at(conn, filters, true, auth::now_ms()).await
}
pub(crate) async fn selection_at(
    conn: &mut sqlx::SqliteConnection,
    filters: &Filters,
    metadata: bool,
    now: i64,
) -> Result<QueryBuilder<Sqlite>> {
    let mut b = QueryBuilder::new("WITH selected AS (SELECT l.*");
    if metadata {
        b.push(", t.title, t.album_id, t.spotify_url, a.name AS album, a.cover_url");
    }
    b.push(" FROM listens l");
    if metadata || filters.q.as_deref().is_some_and(|q| !q.trim().is_empty()) {
        b.push(" JOIN tracks t ON t.id=l.track_id LEFT JOIN albums a ON a.id=t.album_id");
    }
    b.push(" WHERE l.trashed_at_ms IS NULL");
    let parse = |s: &Option<String>| -> Result<Option<NaiveDate>> {
        s.as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| {
                NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .map_err(|_| AppError::BadRequest("Use dates in YYYY-MM-DD format.".into()))
            })
            .transpose()
    };
    let period = if filters.from.as_deref().is_some_and(|s| !s.is_empty())
        || filters.to.as_deref().is_some_and(|s| !s.is_empty())
        || filters.period.as_deref() == Some("custom")
    {
        crate::calendar::Period::Dates {
            from: parse(&filters.from)?,
            through: parse(&filters.to)?,
        }
    } else {
        match filters.period.as_deref().unwrap_or("all") {
            "all" => crate::calendar::Period::All,
            "today" => crate::calendar::Period::Today,
            "week" => crate::calendar::Period::Week,
            "month" => crate::calendar::Period::Month,
            "year" => crate::calendar::Period::Year,
            "7d" => crate::calendar::Period::RollingDays(7),
            "30d" => crate::calendar::Period::RollingDays(30),
            "365d" => crate::calendar::Period::RollingDays(365),
            _ => return Err(AppError::BadRequest("Unknown period.".into())),
        }
    };
    crate::calendar::filter(conn, &mut b, &period, now).await?;
    if let Some(id) = filters.artist.as_deref().filter(|s| !s.is_empty()) {
        b.push(" AND EXISTS(SELECT 1 FROM track_artists ta WHERE ta.track_id=l.track_id AND ta.artist_id=").push_bind(id.to_owned()).push(")");
    }
    if let Some(id) = filters.album.as_deref().filter(|s| !s.is_empty()) {
        b.push(" AND l.track_id IN (SELECT id FROM tracks WHERE album_id=")
            .push_bind(id.to_owned())
            .push(")");
    }
    if let Some(id) = filters.track.as_deref().filter(|s| !s.is_empty()) {
        b.push(" AND l.track_id=").push_bind(id.to_owned());
    }
    if let Some(source) = filters.source.as_deref().filter(|s| !s.is_empty()) {
        b.push(" AND EXISTS(SELECT 1 FROM observations o WHERE o.listen_id=l.id AND o.source=")
            .push_bind(source.to_owned())
            .push(")");
    }
    if let Some(grain) = &filters.bucket {
        let start = filters
            .bucket_start
            .as_deref()
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .ok_or_else(|| {
                AppError::BadRequest("A chart segment requires a valid start date.".into())
            })?;
        let (first, through) = crate::calendar::bucket_bounds(
            start,
            grain,
            i64::from(start.weekday().num_days_from_monday()),
        )?;
        if first != start {
            return Err(AppError::BadRequest("Invalid segment boundary.".into()));
        }
        if grain == "week" {
            b.push(" AND l.week_date=").push_bind(start.to_string());
        } else {
            b.push(" AND l.local_date>=")
                .push_bind(start.to_string())
                .push(" AND l.local_date<=")
                .push_bind(through.to_string());
        }
    }
    if let Some(q) = filters.q.as_deref().filter(|s| !s.trim().is_empty()) {
        if q.len() > 500 {
            return Err(AppError::BadRequest(
                "Search is limited to 500 bytes.".into(),
            ));
        }
        let escaped = q
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        b.push(" AND (t.title LIKE ").push_bind(pattern.clone()).push(" ESCAPE '\\' OR a.name LIKE ").push_bind(pattern.clone()).push(" ESCAPE '\\' OR EXISTS(SELECT 1 FROM track_artists ta JOIN artists ar ON ar.id=ta.artist_id WHERE ta.track_id=t.id AND ar.name LIKE ").push_bind(pattern).push(" ESCAPE '\\'))");
    }
    match filters.status.as_deref().unwrap_or("all") {
        "all" => {}
        "counted" => {
            b.push(" AND l.counted=1");
        }
        "short" => {
            b.push(" AND l.counted=0");
        }
        "actual" => {
            b.push(" AND l.duration_kind='actual'");
        }
        "review" => {
            b.push(" AND l.needs_review=1");
        }
        "estimated" => {
            b.push(" AND l.duration_kind IN ('estimated','unknown')");
        }
        _ => return Err(AppError::BadRequest("Unknown listen filter.".into())),
    }
    b.push(") ");
    Ok(b)
}
