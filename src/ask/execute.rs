//! Execute the supported Ask metrics on the canonical consultation selection.
//!
//! Filters, assigned calendars and timezone boundaries come from
//! `crate::consultation::selection_at`, so Ask can never drift from the listening
//! pages. Only plays, first play and last play run in this slice; the other
//! metrics are rejected explicitly by the handler.
use crate::{
    ask::{
        AskResult, ListenPoint, ResultLink,
        dsl::{EntityKind, Metric, Period, Query, Weekday},
        time,
    },
    consultation::{Filters, selection_at},
    error::{AppError, Result},
};
use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Tz;
use sqlx::{FromRow, SqliteConnection};

pub enum Outcome {
    Result(Box<AskResult>),
    Unsupported(String),
}

pub async fn execute(conn: &mut SqliteConnection, query: &Query, now: i64) -> Result<Outcome> {
    if !matches!(
        query.metric,
        Metric::Plays | Metric::TimeListened | Metric::FirstPlay | Metric::LastPlay
    ) {
        return Ok(Outcome::Unsupported(unsupported_metric_message()));
    }
    let filters = filters_for(query)?;
    if query.hour.is_some() {
        // The assigned-calendar hour needs the calendar timezone, which portable
        // SQLite date functions cannot apply correctly across DST. Keep the
        // canonical selection and filter the bounded result in Rust.
        return Ok(Outcome::Result(Box::new(
            execute_with_hour(conn, query, &filters, now).await?,
        )));
    }
    match query.metric {
        Metric::Plays => {
            let mut count = selection_at(conn, &filters, false, now).await?;
            count.push("SELECT COALESCE(SUM(counted),0) FROM selected");
            let plays: i64 = count.build_query_scalar().fetch_one(&mut *conn).await?;
            Ok(Outcome::Result(Box::new(AskResult {
                metric: query.metric,
                scalar: Some(plays),
                time: None,
                rows: Vec::new(),
                first: None,
                last: None,
                streak: None,
                milestones: Vec::new(),
                total: plays,
                links: entity_links(query),
            })))
        }
        Metric::TimeListened => Ok(Outcome::Result(Box::new(
            time::execute(conn, query, &filters, now).await?,
        ))),
        Metric::FirstPlay | Metric::LastPlay => {
            let point = point_query(conn, &filters, query.metric, now).await?;
            Ok(Outcome::Result(Box::new(point_result(query, point))))
        }
        _ => unreachable!(),
    }
}

async fn execute_with_hour(
    conn: &mut SqliteConnection,
    query: &Query,
    filters: &Filters,
    now: i64,
) -> Result<AskResult> {
    let mut rows_query = selection_at(conn, filters, true, now).await?;
    rows_query.push(
        "SELECT s.played_at_ms,s.track_id,s.title,s.album,s.counted,s.listened_ms,s.duration_kind,s.local_date,(SELECT timezone FROM calendars WHERE id=s.calendar_id) AS timezone,(SELECT group_concat(name, ', ') FROM (SELECT ar.name FROM track_artists ta JOIN artists ar ON ar.id=ta.artist_id WHERE ta.track_id=s.track_id ORDER BY ta.ordinal)) AS artist FROM selected s ORDER BY s.played_at_ms,s.id",
    );
    let rows: Vec<RawPoint> = rows_query.build_query_as().fetch_all(&mut *conn).await?;
    let hour = u32::from(query.hour.ok_or(AppError::Internal)?);
    let mut matching = Vec::new();
    for row in rows {
        let tz: Tz = row.timezone.parse().map_err(|_| AppError::Internal)?;
        let local_hour = DateTime::<Utc>::from_timestamp_millis(row.played_at_ms)
            .ok_or(AppError::Internal)?
            .with_timezone(&tz)
            .hour();
        if local_hour == hour {
            matching.push(row);
        }
    }
    let counted = || matching.iter().filter(|row| row.counted);
    match query.metric {
        Metric::Plays => {
            let plays = counted().count() as i64;
            Ok(AskResult {
                metric: query.metric,
                scalar: Some(plays),
                time: None,
                rows: Vec::new(),
                first: None,
                last: None,
                streak: None,
                milestones: Vec::new(),
                total: plays,
                links: entity_links(query),
            })
        }
        Metric::TimeListened => {
            let mut sums = time::TimeSums::default();
            for row in counted() {
                sums.add(row.counted, row.listened_ms, &row.duration_kind);
            }
            Ok(time::result(query, sums))
        }
        Metric::FirstPlay | Metric::LastPlay => {
            let row = if query.metric == Metric::FirstPlay {
                counted().next().cloned()
            } else {
                counted().next_back().cloned()
            };
            Ok(point_result(query, row.map(ListenPointWithDate::from)))
        }
        _ => unreachable!(),
    }
}

async fn point_query(
    conn: &mut SqliteConnection,
    filters: &Filters,
    metric: Metric,
    now: i64,
) -> Result<Option<ListenPointWithDate>> {
    let mut query = selection_at(conn, filters, true, now).await?;
    let order = if metric == Metric::FirstPlay {
        "ASC"
    } else {
        "DESC"
    };
    query.push(format!(
        "SELECT s.played_at_ms,s.track_id,s.title,s.album,s.local_date,(SELECT group_concat(name, ', ') FROM (SELECT ar.name FROM track_artists ta JOIN artists ar ON ar.id=ta.artist_id WHERE ta.track_id=s.track_id ORDER BY ta.ordinal)) AS artist FROM selected s WHERE s.counted=1 ORDER BY s.played_at_ms {order},s.id {order} LIMIT 1"
    ));
    Ok(query
        .build_query_as::<PointRow>()
        .fetch_optional(&mut *conn)
        .await?
        .map(ListenPointWithDate::from))
}

fn point_result(query: &Query, entry: Option<ListenPointWithDate>) -> AskResult {
    let found = entry.is_some();
    let mut links = entity_links(query);
    if let Some(entry) = &entry {
        links.push(ResultLink {
            label: "Open track".into(),
            href: format!("/music/tracks/{}", entry.point.track_id),
        });
        if !entry.local_date.is_empty() {
            links.push(ResultLink {
                label: format!("Played on {}", entry.local_date),
                href: format!("/history?from={date}&to={date}", date = entry.local_date),
            });
        }
    }
    let point = entry.map(|entry| entry.point);
    AskResult {
        metric: query.metric,
        scalar: point.as_ref().map(|point| point.played_at_ms),
        time: None,
        rows: Vec::new(),
        first: (query.metric == Metric::FirstPlay)
            .then_some(point.clone())
            .flatten(),
        last: (query.metric == Metric::LastPlay)
            .then_some(point)
            .flatten(),
        streak: None,
        milestones: Vec::new(),
        total: if found { 1 } else { 0 },
        links,
    }
}

fn filters_for(query: &Query) -> Result<Filters> {
    let mut filters = Filters {
        period: Some(period_key(query.period).to_owned()),
        year: query.year,
        month: query.month,
        weekday: query.weekday.map(weekday_key).map(str::to_owned),
        hour: query.hour,
        source: query.source.clone(),
        ..Filters::default()
    };
    if let Some(entity) = &query.entity {
        let id = entity
            .id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| AppError::BadRequest("The entity was not resolved.".into()))?;
        match entity.kind {
            EntityKind::Artist => filters.artist = Some(id.to_owned()),
            EntityKind::Album => filters.album = Some(id.to_owned()),
            EntityKind::Track => filters.track = Some(id.to_owned()),
        }
    }
    Ok(filters)
}

fn period_key(period: Period) -> &'static str {
    match period {
        Period::AllTime => "all",
        Period::Today => "today",
        Period::Yesterday => "yesterday",
        Period::ThisWeek => "week",
        Period::LastWeek => "last_week",
        Period::ThisMonth => "month",
        Period::LastMonth => "last_month",
        Period::ThisYear => "year",
        Period::LastYear => "last_year",
    }
}

fn weekday_key(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Monday => "monday",
        Weekday::Tuesday => "tuesday",
        Weekday::Wednesday => "wednesday",
        Weekday::Thursday => "thursday",
        Weekday::Friday => "friday",
        Weekday::Saturday => "saturday",
        Weekday::Sunday => "sunday",
    }
}

pub fn unsupported_metric_message() -> String {
    "I can currently answer only play counts, listening time, first play and last play (metrics: plays, time_listened, first_play, last_play).".into()
}

pub(crate) fn entity_links(query: &Query) -> Vec<ResultLink> {
    let Some(entity) = &query.entity else {
        return Vec::new();
    };
    let Some(id) = entity.id.as_deref() else {
        return Vec::new();
    };
    let (segment, label) = match entity.kind {
        EntityKind::Artist => ("artists", "View artist"),
        EntityKind::Album => ("albums", "Open album"),
        EntityKind::Track => ("tracks", "Open track"),
    };
    vec![ResultLink {
        label: label.into(),
        href: format!("/music/{segment}/{id}"),
    }]
}

#[derive(FromRow, Clone)]
struct RawPoint {
    played_at_ms: i64,
    track_id: String,
    title: String,
    album: Option<String>,
    counted: bool,
    listened_ms: Option<i64>,
    duration_kind: String,
    local_date: String,
    timezone: String,
    artist: Option<String>,
}

impl From<RawPoint> for ListenPointWithDate {
    fn from(row: RawPoint) -> Self {
        Self {
            point: ListenPoint {
                played_at_ms: row.played_at_ms,
                title: row.title,
                artist: row.artist.unwrap_or_default(),
                album: row.album,
                track_id: row.track_id,
            },
            local_date: row.local_date,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ListenPointWithDate {
    pub point: ListenPoint,
    pub local_date: String,
}

#[derive(FromRow)]
struct PointRow {
    played_at_ms: i64,
    track_id: String,
    title: String,
    album: Option<String>,
    local_date: String,
    artist: Option<String>,
}

impl From<PointRow> for ListenPointWithDate {
    fn from(row: PointRow) -> Self {
        Self {
            point: ListenPoint {
                played_at_ms: row.played_at_ms,
                title: row.title,
                artist: row.artist.unwrap_or_default(),
                album: row.album,
                track_id: row.track_id,
            },
            local_date: row.local_date,
        }
    }
}
