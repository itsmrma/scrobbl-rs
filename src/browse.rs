//! Owner consultation: bounded projections of one coherent archive revision.
//! No source originals, duration estimation or timestamp interpretation lives here.
use crate::calendar::bucket_bounds;
use crate::{
    AppState, auth,
    calendar::Calendar,
    consultation::{self, Filters},
    dto::*,
    error::{AppError, Result},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{FromRow, Row, SqliteConnection};
use std::collections::{BTreeMap, BTreeSet};

pub fn paging(filters: &Filters) -> Result<(i64, i64)> {
    let size = filters.page_size.unwrap_or(50);
    if ![10, 25, 50, 100].contains(&size) {
        return Err(AppError::BadRequest(
            "Choose 10, 25, 50 or 100 results per page.".into(),
        ));
    }
    Ok((
        i64::from(filters.page.unwrap_or(1)).clamp(1, 1_000_000),
        size,
    ))
}
fn kind_table(kind: &str) -> Result<&'static str> {
    match kind {
        "artists" => Ok("artist_ranks"),
        "albums" => Ok("album_ranks"),
        "tracks" => Ok("track_ranks"),
        _ => Err(AppError::BadRequest(
            "Choose artists, albums or tracks.".into(),
        )),
    }
}
/// History choices include identities with only below-threshold listens.
/// A typeahead is bounded independently of the paginated result lists.
pub async fn choices(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(kind): Path<String>,
    Query(filters): Query<Filters>,
) -> Result<Json<Vec<EntityChoice>>> {
    auth::session(&state, &headers).await?;
    let q = filters.q.as_deref().unwrap_or("").trim();
    if q.len() > 500 {
        return Err(AppError::BadRequest(
            "Search is limited to 500 bytes.".into(),
        ));
    }
    let pattern = format!(
        "%{}%",
        q.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let join = match kind.as_str() {
        "artists" => {
            "artists e JOIN track_artists ta ON ta.artist_id=e.id JOIN tracks t ON t.id=ta.track_id"
        }
        "albums" => "albums e JOIN tracks t ON t.album_id=e.id",
        _ => return Err(AppError::BadRequest("Choose artists or albums.".into())),
    };
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
        "{}SELECT e.id,e.name,CASE WHEN COUNT(DISTINCT t.id)=1 THEN MIN(t.title) ELSE CAST(COUNT(DISTINCT t.id) AS TEXT)||' tracks' END AS subtitle FROM ",
        crate::library::METADATA
    ));
    query.push(join).push(" WHERE EXISTS(SELECT 1 FROM listens l WHERE l.track_id=t.id AND l.trashed_at_ms IS NULL) AND (e.name LIKE ").push_bind(pattern.clone()).push(" ESCAPE '\\' OR t.title LIKE ").push_bind(pattern).push(" ESCAPE '\\') GROUP BY e.id ORDER BY e.name COLLATE NOCASE,e.id LIMIT 50");
    let mut view = state.storage.read_view().await?;
    Ok(Json(
        query
            .build_query_as()
            .fetch_all(&mut *view.transaction)
            .await?,
    ))
}
pub async fn rankings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(kind): Path<String>,
    Query(filters): Query<Filters>,
) -> Result<Json<RankingPage>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    Ok(Json(
        ranking_page(&mut view.transaction, &kind, &filters, auth::now_ms()).await?,
    ))
}
pub async fn ranking_page(
    conn: &mut SqliteConnection,
    kind: &str,
    filters: &Filters,
    now: i64,
) -> Result<RankingPage> {
    let table = kind_table(kind)?;
    let (page, size) = paging(filters)?;
    let (primary, secondary) = match filters.sort.as_deref().unwrap_or("plays") {
        "plays" => ("plays", "listened_ms"),
        "time" => ("listened_ms", "plays"),
        _ => return Err(AppError::BadRequest("Unknown ranking order.".into())),
    };
    let mut query = consultation::selection_at(conn, filters, false, now).await?;
    query.push(include_str!("ranking-aggregates.sql"));
    // Rank over the entire selection before applying the page boundary.
    query.push(format!(", ranked AS (SELECT *, RANK() OVER(ORDER BY {primary} DESC) AS position FROM {table}) SELECT json_object('total',(SELECT COUNT(*) FROM ranked),'items',(SELECT json_group_array(json_object('id',id,'name',name,'subtitle',subtitle,'cover_url',cover_url,'plays',plays,'listened_ms',listened_ms,'position',position,'actual_ms',actual_ms,'estimated_ms',estimated_ms,'unknown_durations',unknown_durations)) FROM (SELECT * FROM ranked ORDER BY {primary} DESC,{secondary} DESC,name COLLATE NOCASE,id LIMIT "));
    query
        .push_bind(size)
        .push(" OFFSET ")
        .push_bind((page - 1) * size)
        .push(")))");
    let value: String = query.build_query_scalar().fetch_one(conn).await?;
    let value: serde_json::Value = serde_json::from_str(&value).map_err(|_| AppError::Internal)?;
    Ok(RankingPage {
        items: serde_json::from_value(value["items"].clone()).map_err(|_| AppError::Internal)?,
        total: value["total"].as_i64().ok_or(AppError::Internal)?,
        page,
        page_size: size,
    })
}
pub async fn music(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
    Query(mut filters): Query<Filters>,
) -> Result<Json<MusicPage>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    let conn = &mut *view.transaction;
    let id = crate::name_groups::canonical(conn, &kind, &id).await?;
    let sql = match kind.as_str() {
        "artists" => {
            filters.artist = Some(id.clone());
            "SELECT id,name,'/api/v1/artwork/artists/'||id AS cover_url,NULL AS album_id,NULL AS album FROM artists WHERE id=?"
        }
        "albums" => {
            filters.album = Some(id.clone());
            "SELECT id,name,cover_url,NULL AS album_id,NULL AS album FROM albums WHERE id=?"
        }
        "tracks" => {
            filters.track = Some(id.clone());
            "SELECT t.id,t.title AS name,a.cover_url,t.album_id,a.name AS album FROM tracks t LEFT JOIN albums a ON a.id=t.album_id WHERE t.id=?"
        }
        _ => {
            return Err(AppError::BadRequest(
                "Choose artists, albums or tracks.".into(),
            ));
        }
    };
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "{}{sql}",
        crate::library::METADATA
    )))
    .bind(&id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(AppError::NotFound)?;
    let artists = if kind == "tracks" {
        sqlx::query(sqlx::AssertSqlSafe(format!("{}SELECT a.id,a.name FROM track_artists ta JOIN artists a ON a.id=ta.artist_id WHERE ta.track_id=? ORDER BY ta.ordinal",crate::library::METADATA))).bind(&id).fetch_all(&mut *conn).await?.into_iter().map(|r|crate::db::Artist{id:r.get("id"),name:r.get("name")}).collect()
    } else {
        Vec::new()
    };
    let mut query = consultation::selection_at(conn, &filters, false, auth::now_ms()).await?;
    query.push("SELECT json_object('records',COUNT(*),'listens',COALESCE(SUM(counted),0),'tracks',COUNT(DISTINCT CASE WHEN counted=1 THEN track_id END),'actual_ms',COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='actual' THEN listened_ms END),0),'estimated_ms',COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='estimated' THEN listened_ms END),0),'unknown_durations',COALESCE(SUM(counted=1 AND duration_kind='unknown'),0)) FROM selected");
    let totals: String = query.build_query_scalar().fetch_one(&mut *conn).await?;
    Ok(Json(MusicPage {
        id,
        name: row.get("name"),
        kind,
        cover_url: row.get("cover_url"),
        album_id: row.get("album_id"),
        album: row.get("album"),
        artists,
        totals: serde_json::from_str(&totals).map_err(|_| AppError::Internal)?,
    }))
}

fn date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| AppError::BadRequest("Use dates in YYYY-MM-DD format.".into()))
}
fn calendar_day(calendar: &Calendar, time: i64) -> Result<NaiveDate> {
    date(&calendar.dates(time)?.0)
}
#[derive(FromRow)]
struct Daily {
    local_date: String,
    week_date: String,
    listens: i64,
    actual_ms: i64,
    estimated_ms: i64,
    unknown_durations: i64,
}
pub async fn activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filters): Query<Filters>,
) -> Result<Json<ActivityChart>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    Ok(Json(
        activity_chart(&mut view.transaction, &filters, auth::now_ms()).await?,
    ))
}
pub async fn activity_chart(
    conn: &mut SqliteConnection,
    filters: &Filters,
    now: i64,
) -> Result<ActivityChart> {
    let mut query = consultation::selection_at(conn, filters, false, now).await?;
    query.push("SELECT local_date,week_date,COUNT(*) AS listens,COALESCE(SUM(CASE WHEN duration_kind='actual' THEN listened_ms END),0) AS actual_ms,COALESCE(SUM(CASE WHEN duration_kind='estimated' THEN listened_ms END),0) AS estimated_ms,SUM(duration_kind='unknown') AS unknown_durations FROM selected WHERE counted=1 GROUP BY local_date,week_date ORDER BY local_date");
    let rows: Vec<Daily> = query.build_query_as().fetch_all(&mut *conn).await?;
    // Used calendars, plus the effective calendar when there are no matching listens.
    let mut query = consultation::selection_at(conn, filters, false, now).await?;
    query.push("SELECT DISTINCT c.* FROM calendars c JOIN selected s ON s.calendar_id=c.id");
    let mut calendars: Vec<Calendar> = query.build_query_as().fetch_all(&mut *conn).await?;
    if calendars.is_empty() {
        calendars.push(crate::calendar::current(conn).await?);
    }
    let period = filters.period.as_deref().unwrap_or("all");
    let custom = period == "custom"
        || filters.from.as_deref().is_some_and(|s| !s.is_empty())
        || filters.to.as_deref().is_some_and(|s| !s.is_empty());
    let min = rows.first().map(|r| date(&r.local_date)).transpose()?;
    let max = rows.last().map(|r| date(&r.local_date)).transpose()?;
    let mut ranges = Vec::new();
    for calendar in &calendars {
        let today = calendar_day(calendar, now)?;
        let (from, through) = if custom {
            (
                filters
                    .from
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(date)
                    .transpose()?
                    .or(min),
                filters
                    .to
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(date)
                    .transpose()?
                    .or(max),
            )
        } else {
            match period {
                "all" => (min, max),
                "today" => (Some(today), Some(today)),
                "week" => (
                    Some(bucket_bounds(today, "week", calendar.week_start)?.0),
                    Some(today),
                ),
                "month" => (
                    Some(bucket_bounds(today, "month", calendar.week_start)?.0),
                    Some(today),
                ),
                "year" => (
                    Some(bucket_bounds(today, "year", calendar.week_start)?.0),
                    Some(today),
                ),
                "7d" | "30d" | "365d" => {
                    let days = period
                        .trim_end_matches('d')
                        .parse::<i64>()
                        .map_err(|_| AppError::Internal)?;
                    (
                        Some(calendar_day(calendar, now - days * 86_400_000)?),
                        Some(today),
                    )
                }
                _ => return Err(AppError::BadRequest("Unknown period.".into())),
            }
        };
        if let (Some(from), Some(through)) = (from, through) {
            ranges.push((from, through, calendar.week_start));
        }
    }
    let span = ranges
        .iter()
        .map(|(f, t, _)| (*t - *f).num_days())
        .max()
        .unwrap_or(0);
    let grain = filters.grain.as_deref().unwrap_or(if span <= 31 {
        "day"
    } else if span <= 180 {
        "week"
    } else if span <= 1095 {
        "month"
    } else {
        "year"
    });
    // Validate even empty archives and avoid unbounded responses for extreme custom ranges.
    bucket_bounds(
        DateTime::<Utc>::from_timestamp_millis(now)
            .ok_or(AppError::Internal)?
            .date_naive(),
        grain,
        0,
    )?;
    let mut buckets = BTreeMap::<NaiveDate, ActivityBucket>::new();
    let partial = matches!(
        period,
        "today" | "week" | "month" | "year" | "7d" | "30d" | "365d"
    ) && !custom;
    for (from, through, week_start) in &ranges {
        let mut day = bucket_bounds(*from, grain, *week_start)?.0;
        while day <= *through {
            let (start, end) = bucket_bounds(day, grain, *week_start)?;
            let bucket = buckets.entry(start).or_insert_with(|| ActivityBucket {
                start: start.to_string(),
                through: end.to_string(),
                ..Default::default()
            });
            bucket.partial |= start < *from
                || end > *through
                || (partial && end >= *through)
                || (partial && period.ends_with('d') && start <= *from);
            if buckets.len() > 12000 {
                return Err(AppError::BadRequest("This chart exceeds 12,000 segments. Choose a coarser grouping or a shorter period.".into()));
            }
            day = end.succ_opt().ok_or_else(|| {
                AppError::BadRequest("Activity date outside supported range.".into())
            })?;
        }
    }
    for row in rows {
        let start = if grain == "week" {
            date(&row.week_date)?
        } else {
            bucket_bounds(date(&row.local_date)?, grain, 0)?.0
        };
        if let Some(bucket) = buckets.get_mut(&start) {
            bucket.listens += row.listens;
            bucket.actual_ms += row.actual_ms;
            bucket.estimated_ms += row.estimated_ms;
            bucket.unknown_durations += row.unknown_durations;
        }
    }
    Ok(ActivityChart {
        grain: grain.into(),
        buckets: buckets.into_values().collect(),
        mixed_week_starts: grain == "week"
            && calendars
                .iter()
                .map(|c| c.week_start)
                .collect::<BTreeSet<_>>()
                .len()
                > 1,
        partial,
    })
}
