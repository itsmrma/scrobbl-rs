//! Execute the supported Ask metrics on the canonical consultation selection.
//!
//! Filters, assigned calendars and timezone boundaries come from
//! `crate::consultation::selection_at`, so Ask can never drift from the listening
//! pages. Plays, listening time, distinct counts, first/last play, streaks and
//! milestones, ranking position, share of plays and listening days execute here
//! or in their owning module. Grouping applies to plays and the distinct
//! metrics; grouping other metrics is not implemented.
use crate::{
    ask::{
        AskResult, ListenPoint, ResultLink,
        dsl::{EntityKind, GroupBy, Metric, Period, Query, Weekday},
        grouping, time,
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
    let mut outcome = execute_metric(conn, query, now).await?;
    if let Outcome::Result(result) = &mut outcome {
        decorate(query, result);
    }
    Ok(outcome)
}

async fn execute_metric(conn: &mut SqliteConnection, query: &Query, now: i64) -> Result<Outcome> {
    if !matches!(
        query.metric,
        Metric::Plays
            | Metric::TimeListened
            | Metric::DistinctTracks
            | Metric::DistinctArtists
            | Metric::DistinctAlbums
            | Metric::FirstPlay
            | Metric::LastPlay
            | Metric::Streak
            | Metric::Milestones
            | Metric::Rank
            | Metric::Share
            | Metric::ListeningDays
    ) {
        return Ok(Outcome::Unsupported(unsupported_metric_message()));
    }
    if query.group_by.is_some() && !grouping::handles(query) {
        return Ok(Outcome::Unsupported(unsupported_grouping_message(
            query.metric,
        )));
    }
    let mut filters = filters_for(query)?;
    if let Some(track) = filters.track.clone() {
        filters.track_any = crate::ask::names::track_editions(conn, &track).await?;
    }
    if matches!(query.metric, Metric::Rank | Metric::Share) {
        if query.entity.is_none() {
            return Ok(Outcome::Unsupported(
                "Ranking position and share of plays need an artist, album or track.".into(),
            ));
        }
        if query.hour.is_some() {
            return Ok(Outcome::Unsupported(
                "Ranking position and share of plays cannot be filtered by hour yet.".into(),
            ));
        }
    }
    if grouping::handles(query) {
        return Ok(Outcome::Result(Box::new(
            grouping::execute(conn, query, &filters, now).await?,
        )));
    }
    if matches!(query.metric, Metric::Streak | Metric::Milestones) {
        // Streaks and milestones always read the entity's whole history, so the
        // requested hour never narrows them and the Rust hour pass is skipped.
        return Ok(Outcome::Result(Box::new(
            crate::ask::streaks::execute(conn, query, &filters, now).await?,
        )));
    }
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
        Metric::ListeningDays => {
            let mut days = selection_at(conn, &filters, false, now).await?;
            days.push("SELECT COUNT(DISTINCT local_date) FROM selected WHERE counted=1");
            let days: i64 = days.build_query_scalar().fetch_one(&mut *conn).await?;
            Ok(Outcome::Result(Box::new(scalar_result(
                query,
                Some(days),
                days,
            ))))
        }
        Metric::Share => {
            let mut entity_plays = selection_at(conn, &filters, false, now).await?;
            entity_plays.push("SELECT COALESCE(SUM(counted),0) FROM selected");
            let plays: i64 = entity_plays
                .build_query_scalar()
                .fetch_one(&mut *conn)
                .await?;
            let all = without_entity(&filters);
            let mut total = selection_at(conn, &all, false, now).await?;
            total.push("SELECT COALESCE(SUM(counted),0) FROM selected");
            let total: i64 = total.build_query_scalar().fetch_one(&mut *conn).await?;
            // scalar = entity plays, total = all plays in the same selection.
            Ok(Outcome::Result(Box::new(scalar_result(
                query,
                Some(plays),
                total,
            ))))
        }
        Metric::Rank => {
            let entity = query.entity.as_ref().ok_or(AppError::Internal)?;
            let id = entity.id.as_deref().ok_or(AppError::Internal)?;
            let all = without_entity(&filters);
            let mut ranking = selection_at(conn, &all, true, now).await?;
            // Same keys as the rankings pages: artist and album name groups,
            // tracks by id. Editions of the target song count as the target.
            let target = match entity.kind {
                EntityKind::Artist => crate::name_groups::canonical(conn, "artists", id)
                    .await
                    .unwrap_or_else(|_| id.to_owned()),
                EntityKind::Album => crate::name_groups::canonical(conn, "albums", id)
                    .await
                    .unwrap_or_else(|_| id.to_owned()),
                EntityKind::Track => id.to_owned(),
            };
            let editions = filters.track_any.clone();
            if entity.kind == EntityKind::Artist {
                ranking.push(
                    "SELECT k AS key, SUM(counted) AS plays FROM (SELECT COALESCE(m.group_id,ta.artist_id) AS k, s.counted FROM selected s JOIN track_artists ta ON ta.track_id=s.track_id LEFT JOIN name_group_members m ON m.kind='artists' AND m.entity_id=ta.artist_id) GROUP BY k",
                );
            } else if entity.kind == EntityKind::Album {
                ranking.push(
                    "SELECT COALESCE(m.group_id,s.album_id) AS key, SUM(s.counted) AS plays FROM selected s LEFT JOIN name_group_members m ON m.kind='albums' AND m.entity_id=s.album_id WHERE s.album_id IS NOT NULL GROUP BY 1",
                );
            } else {
                ranking.push(
                    "SELECT s.track_id AS key, SUM(s.counted) AS plays FROM selected s GROUP BY 1",
                );
            }
            let rows: Vec<(String, i64)> = ranking.build_query_as().fetch_all(&mut *conn).await?;
            let mut totals: std::collections::BTreeMap<String, i64> = Default::default();
            for (key, plays) in rows {
                let key = if editions.contains(&key) {
                    target.clone()
                } else {
                    key
                };
                *totals.entry(key).or_default() += plays;
            }
            totals.retain(|_, plays| *plays > 0);
            let own = totals.get(&target).copied().unwrap_or(0);
            let rank =
                (own > 0).then(|| 1 + totals.values().filter(|plays| **plays > own).count() as i64);
            let mut result = scalar_result(query, rank, totals.len() as i64);
            result.rows.push(crate::ask::ResultRow {
                label: entity.name.clone(),
                key: target,
                plays: own,
                time: None,
                href: None,
            });
            Ok(Outcome::Result(Box::new(result)))
        }
        _ => unreachable!(),
    }
}

fn without_entity(filters: &Filters) -> Filters {
    Filters {
        artist: None,
        album: None,
        track: None,
        track_any: Vec::new(),
        ..filters.clone()
    }
}

fn scalar_result(query: &Query, scalar: Option<i64>, total: i64) -> AskResult {
    AskResult {
        metric: query.metric,
        scalar,
        time: None,
        rows: Vec::new(),
        first: None,
        last: None,
        streak: None,
        milestones: Vec::new(),
        total,
        links: entity_links(query),
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
        Metric::ListeningDays => {
            let mut days: Vec<&str> = counted().map(|row| row.local_date.as_str()).collect();
            days.sort_unstable();
            days.dedup();
            let days = days.len() as i64;
            Ok(scalar_result(query, Some(days), days))
        }
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
                href: history_path(query, &Window::Dates(&entry.local_date, &entry.local_date)),
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

/// Which listens a History link shows besides the entity filter.
enum Window<'a> {
    /// The period, year, month, weekday and source the question used.
    Question,
    /// One calendar range, replacing the question's period.
    Dates(&'a str, &'a str),
    /// One assigned-calendar week, by the date its week starts on.
    Week(&'a str),
    /// The question's selection narrowed to one weekday.
    Weekday(&'a str),
}

fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A Listening history address that shows exactly the listens an answer was
/// computed from. It reuses the History filters, so the list always matches
/// the Ask selection (the hour is the one filter History cannot express).
fn history_path(query: &Query, window: &Window) -> String {
    let mut params: Vec<(&str, String)> = Vec::new();
    if let Some(entity) = &query.entity
        && let Some(id) = entity.id.as_deref().filter(|id| !id.is_empty())
    {
        let key = match entity.kind {
            EntityKind::Artist => "artist",
            EntityKind::Album => "album",
            EntityKind::Track => "track",
        };
        params.push((key, id.to_owned()));
    }
    match window {
        Window::Dates(from, to) => {
            params.push(("period", "custom".into()));
            params.push(("from", (*from).to_owned()));
            params.push(("to", (*to).to_owned()));
        }
        Window::Week(start) => {
            params.push(("period", "all".into()));
            params.push(("bucket", "week".into()));
            params.push(("bucket_start", (*start).to_owned()));
        }
        Window::Question | Window::Weekday(_) => {
            if query.from.is_some() || query.to.is_some() {
                params.push(("period", "custom".into()));
                if let Some(from) = &query.from {
                    params.push(("from", from.clone()));
                }
                if let Some(to) = &query.to {
                    params.push(("to", to.clone()));
                }
            } else if let Some(year) = query.year {
                params.push(("year", year.to_string()));
            } else {
                params.push(("period", period_key(query.period).to_owned()));
            }
            if let Some(month) = query.month {
                params.push(("month", month.to_string()));
            }
            match window {
                Window::Weekday(day) => params.push(("weekday", (*day).to_owned())),
                _ => {
                    if let Some(day) = query.weekday {
                        params.push(("weekday", weekday_key(day).to_owned()));
                    }
                }
            }
        }
    }
    if let Some(source) = query.source.as_deref().filter(|s| !s.is_empty()) {
        params.push(("source", source.to_owned()));
    }
    let query_string = params
        .iter()
        .map(|(key, value)| format!("{key}={}", encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("/history?{query_string}")
}

/// The first and last calendar day of a grouped bucket label, when the label
/// is a day, a month or a year.
fn bucket_dates(group: GroupBy, label: &str) -> Option<(String, String)> {
    use chrono::{Datelike, NaiveDate};
    match group {
        GroupBy::Day => NaiveDate::parse_from_str(label, "%Y-%m-%d")
            .ok()
            .map(|day| (day.to_string(), day.to_string())),
        GroupBy::Month => {
            let first = NaiveDate::parse_from_str(&format!("{label}-01"), "%Y-%m-%d").ok()?;
            let next = if first.month() == 12 {
                NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)?
            } else {
                NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)?
            };
            Some((first.to_string(), next.pred_opt()?.to_string()))
        }
        GroupBy::Year => {
            let year: i32 = label.parse().ok()?;
            Some((format!("{year:04}-01-01"), format!("{year:04}-12-31")))
        }
        _ => None,
    }
}

/// The inclusive date range of a bucket label, including a week that starts on
/// the label's date.
pub(crate) fn bucket_range(group: GroupBy, label: &str) -> Option<(String, String)> {
    if group == GroupBy::Week {
        let start = chrono::NaiveDate::parse_from_str(label, "%Y-%m-%d").ok()?;
        return Some((
            start.to_string(),
            (start + chrono::Days::new(6)).to_string(),
        ));
    }
    bucket_dates(group, label)
}

/// Every answer points at the listens behind it: the whole selection, and for
/// day, week, month, year and weekday groupings each bucket on its own.
fn decorate(query: &Query, result: &mut AskResult) {
    for row in &mut result.rows {
        if row.href.is_some() {
            continue;
        }
        row.href = match query.group_by {
            Some(group @ (GroupBy::Day | GroupBy::Month | GroupBy::Year)) => {
                bucket_dates(group, &row.label)
                    .map(|(from, to)| history_path(query, &Window::Dates(&from, &to)))
            }
            Some(GroupBy::Week) => Some(history_path(query, &Window::Week(&row.label))),
            Some(GroupBy::Weekday) => Some(history_path(query, &Window::Weekday(&row.key))),
            _ => None,
        };
    }
    let has_point = matches!(query.metric, Metric::FirstPlay | Metric::LastPlay);
    if query.hour.is_none() && !has_point && query.group_by.is_none() {
        result.links.push(ResultLink {
            label: "See these listens".into(),
            href: history_path(query, &Window::Question),
        });
    }
    if let [only] = result.rows.as_slice()
        && let Some(href) = only.href.as_deref().filter(|h| h.starts_with("/history?"))
    {
        let href = href.to_owned();
        result.links.push(ResultLink {
            label: "See these listens".into(),
            href,
        });
    }
    if query.hour.is_none() && query.group_by.is_some() && result.rows.len() > 1 {
        result.links.push(ResultLink {
            label: "See these listens".into(),
            href: history_path(query, &Window::Question),
        });
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
        from: query.from.clone(),
        to: query.to.clone(),
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
        Period::Last7Days => "7d",
        Period::Last30Days => "30d",
        Period::Last90Days => "90d",
        Period::Last180Days => "180d",
        Period::Last365Days => "365d",
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
    "I can answer play counts, listening time, distinct tracks/artists/albums, first and last play, streaks, milestones, ranking position, share of plays and listening days.".into()
}

pub fn unsupported_grouping_message(metric: Metric) -> String {
    format!(
        "Grouping is only supported for play counts and distinct tracks, artists or albums. The metric “{}” cannot be grouped.",
        metric.label()
    )
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
