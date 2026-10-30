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
use chrono::{DateTime, Datelike, NaiveDate, Utc};
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
    let pattern = crate::db::search_pattern(q)?;
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
    query.push(join).push(" WHERE EXISTS(SELECT 1 FROM listens l WHERE l.track_id=t.id AND l.trashed_at_ms IS NULL) AND e.name LIKE ").push_bind(pattern).push(" ESCAPE '\\' GROUP BY e.id ORDER BY e.name COLLATE NOCASE,e.id LIMIT 50");
    let mut view = state.storage.read_view().await?;
    Ok(Json(
        query
            .build_query_as()
            .fetch_all(&mut *view.transaction)
            .await?,
    ))
}
/// Library search includes all non-trashed identities, even when every listen
/// is below the counting threshold. Corrections and name groups match music pages.
pub async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(kind): Path<String>,
    Query(filters): Query<Filters>,
) -> Result<Json<Vec<SearchItem>>> {
    auth::session(&state, &headers).await?;
    let q = filters.q.as_deref().unwrap_or("").trim();
    let pattern = crate::db::search_pattern(q)?;
    if q.is_empty() {
        return Ok(Json(vec![]));
    }
    let (join, name, subtitle) = match kind.as_str() {
        "artists" => (
            "artists e JOIN track_artists ta ON ta.artist_id=e.id JOIN tracks t ON t.id=ta.track_id",
            "e.name",
            "CAST(COUNT(DISTINCT t.album_id) AS TEXT)||CASE WHEN COUNT(DISTINCT t.album_id)=1 THEN ' album · ' ELSE ' albums · ' END||CAST(COUNT(DISTINCT t.id) AS TEXT)||CASE WHEN COUNT(DISTINCT t.id)=1 THEN ' track' ELSE ' tracks' END",
        ),
        "albums" => (
            "albums e JOIN tracks t ON t.album_id=e.id",
            "e.name",
            "COALESCE((SELECT group_concat(name, ', ') FROM (SELECT DISTINCT a.name FROM artists a JOIN track_artists ta ON ta.artist_id=a.id JOIN tracks at ON at.id=ta.track_id WHERE at.album_id=e.id ORDER BY a.name COLLATE NOCASE)), '')",
        ),
        "tracks" => (
            "tracks e JOIN tracks t ON t.id=e.id LEFT JOIN albums al ON al.id=e.album_id",
            "e.title",
            "COALESCE((SELECT group_concat(name, ', ') FROM (SELECT a.name FROM artists a JOIN track_artists ta ON ta.artist_id=a.id WHERE ta.track_id=e.id ORDER BY ta.ordinal)), '')||CASE WHEN al.name IS NULL THEN '' ELSE ' · '||al.name END",
        ),
        _ => {
            return Err(AppError::BadRequest(
                "Choose artists, albums or tracks.".into(),
            ));
        }
    };
    // Presentation groups do not merge archive identities. Keep every edition
    // accessible and use the most-played identity as the primary destination.
    let key = if kind == "tracks" {
        "json_array(lower(trim(e.title)),COALESCE((SELECT group_concat(artist_id) FROM (SELECT artist_id FROM track_artists WHERE track_id=e.id ORDER BY artist_id)),e.id))"
    } else {
        "e.id"
    };
    let cover = match kind.as_str() {
        "artists" => "'/api/v1/artwork/artists/'||e.id",
        "albums" => "e.cover_url",
        _ => "'/api/v1/artwork/albums/'||e.album_id",
    };
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
        "{}, search_rows AS (SELECT e.id,{name} AS name,{subtitle} AS subtitle,{cover} AS cover_url,{key} AS search_key,COUNT(DISTINCT CASE WHEN l.counted=1 THEN l.id END) AS plays FROM {join} JOIN listens l ON l.track_id=t.id AND l.trashed_at_ms IS NULL WHERE {name} LIKE ",
        crate::library::METADATA
    ));
    query.push_bind(pattern).push(" ESCAPE '\\' GROUP BY e.id), ordered AS (SELECT *,SUM(plays) OVER(PARTITION BY search_key) AS total_plays,ROW_NUMBER() OVER(PARTITION BY search_key ORDER BY plays DESC,name COLLATE NOCASE,id) AS edition_rank FROM search_rows) SELECT json_object('id',o.id,'name',o.name,'subtitle',o.subtitle,'cover_url',o.cover_url,'plays',o.total_plays,'editions',json((SELECT json_group_array(json_object('id',id,'subtitle',subtitle,'cover_url',cover_url,'plays',plays)) FROM (SELECT * FROM search_rows WHERE search_key=o.search_key ORDER BY plays DESC,name COLLATE NOCASE,id)))) FROM ordered o WHERE edition_rank=1 ORDER BY total_plays DESC,name COLLATE NOCASE,id LIMIT 50");
    let mut view = state.storage.read_view().await?;
    let rows: Vec<String> = query
        .build_query_scalar()
        .fetch_all(&mut *view.transaction)
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| serde_json::from_str(&row).map_err(|_| AppError::Internal))
            .collect::<Result<_>>()?,
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
    } else if kind == "albums" {
        sqlx::query(sqlx::AssertSqlSafe(format!("{}SELECT a.id,a.name FROM tracks t JOIN track_artists ta ON ta.track_id=t.id JOIN artists a ON a.id=ta.artist_id WHERE t.album_id=? GROUP BY a.id ORDER BY MIN(ta.ordinal),a.name,a.id",crate::library::METADATA))).bind(&id).fetch_all(&mut *conn).await?.into_iter().map(|r|crate::db::Artist{id:r.get("id"),name:r.get("name")}).collect()
    } else {
        Vec::new()
    };
    let totals: Totals = if filters.include_totals == Some(false) {
        Totals {
            records: 0,
            listens: 0,
            tracks: 0,
            actual_ms: 0,
            estimated_ms: 0,
            unknown_durations: 0,
        }
    } else {
        let mut query = consultation::selection_at(conn, &filters, false, auth::now_ms()).await?;
        query.push("SELECT json_object('records',COUNT(*),'listens',COALESCE(SUM(counted),0),'tracks',COUNT(DISTINCT CASE WHEN counted=1 THEN track_id END),'actual_ms',COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='actual' THEN listened_ms END),0),'estimated_ms',COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='estimated' THEN listened_ms END),0),'unknown_durations',COALESCE(SUM(counted=1 AND duration_kind='unknown'),0)) FROM selected");
        let totals: String = query.build_query_scalar().fetch_one(&mut *conn).await?;
        serde_json::from_str(&totals).map_err(|_| AppError::Internal)?
    };
    Ok(Json(MusicPage {
        id,
        name: row.get("name"),
        kind,
        cover_url: row.get("cover_url"),
        album_id: row.get("album_id"),
        album: row.get("album"),
        artists,
        totals,
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
    calendar_id: String,
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
pub async fn discovery_ratio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(mut filters): Query<Filters>,
) -> Result<Json<DiscoveryRatio>> {
    auth::session(&state, &headers).await?;
    filters.normalize_scope()?;
    if filters.track.is_some() {
        return Err(AppError::BadRequest(
            "Discovery ratio is not supported for tracks.".into(),
        ));
    }
    let mut view = state.storage.read_view().await?;
    Ok(Json(
        discovery_ratio_calc(&mut view.transaction, &filters, auth::now_ms()).await?,
    ))
}

pub async fn calendar(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(mut filters): Query<Filters>,
) -> Result<Json<ListeningCalendar>> {
    auth::session(&state, &headers).await?;
    filters.normalize_scope()?;
    if filters.artist.is_none() && filters.album.is_none() && filters.track.is_none() {
        return Err(AppError::BadRequest(
            "Entity scope (artist, album, or track) is required.".into(),
        ));
    }
    let mut view = state.storage.read_view().await?;
    Ok(Json(
        listening_calendar(&mut view.transaction, &filters, auth::now_ms()).await?,
    ))
}

pub async fn milestones(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(mut filters): Query<Filters>,
) -> Result<Json<ListeningMilestones>> {
    auth::session(&state, &headers).await?;
    filters.normalize_scope()?;
    if filters.artist.is_none() && filters.album.is_none() && filters.track.is_none() {
        return Err(AppError::BadRequest(
            "Entity scope (artist, album, or track) is required.".into(),
        ));
    }
    let mut view = state.storage.read_view().await?;
    Ok(Json(
        listening_milestones(&mut view.transaction, &filters, auth::now_ms()).await?,
    ))
}

fn all_time_filters(filters: &Filters) -> Filters {
    let mut f = filters.clone();
    f.period = Some("all".into());
    f.from = None;
    f.to = None;
    f.bucket = None;
    f.bucket_start = None;
    f.year = None;
    f.include_totals = None;
    f
}

pub async fn listening_calendar(
    conn: &mut SqliteConnection,
    filters: &Filters,
    now: i64,
) -> Result<ListeningCalendar> {
    let all_time_filters = all_time_filters(filters);
    let mut all_time_query =
        consultation::selection_at(conn, &all_time_filters, false, now).await?;
    all_time_query.push(
        "SELECT DISTINCT CAST(substr(local_date, 1, 4) AS INTEGER) AS y FROM selected WHERE counted = 1 ORDER BY y DESC",
    );
    let all_time_rows = all_time_query.build().fetch_all(&mut *conn).await?;
    let mut years: Vec<i32> = Vec::new();
    for r in all_time_rows {
        let y: i32 = r.try_get("y")?;
        years.push(y);
    }

    let calendar = crate::calendar::current(conn).await?;
    let today = calendar_day(&calendar, now)?;
    let period = filters.period.as_deref().unwrap_or("all");
    let custom = period == "custom"
        || filters.from.as_deref().is_some_and(|s| !s.is_empty())
        || filters.to.as_deref().is_some_and(|s| !s.is_empty());

    let target_year = filters.year.or_else(|| {
        if period == "all" && !custom {
            years.first().copied()
        } else {
            None
        }
    });

    let mut effective_filters = filters.clone();
    if let Some(y) = target_year {
        effective_filters.year = Some(y);
    }

    let mut query = consultation::selection_at(conn, &effective_filters, false, now).await?;
    query.push(
        "SELECT local_date, COUNT(*) AS listens FROM selected WHERE counted = 1 GROUP BY local_date ORDER BY local_date ASC",
    );
    let rows = query.build().fetch_all(&mut *conn).await?;
    let mut counts = BTreeMap::new();
    let mut total_listens = 0i64;
    for r in rows {
        let d: String = r.try_get("local_date")?;
        let c: i64 = r.try_get("listens")?;
        total_listens += c;
        counts.insert(d, c);
    }

    let (from, through) = if let Some(y) = target_year {
        let year_start = NaiveDate::from_ymd_opt(y, 1, 1)
            .ok_or_else(|| AppError::BadRequest("Invalid year.".into()))?;
        let year_end = if y < today.year() {
            NaiveDate::from_ymd_opt(y, 12, 31)
                .ok_or_else(|| AppError::BadRequest("Invalid year.".into()))?
        } else {
            today
        };
        if !years.contains(&y) {
            years.push(y);
            years.sort_by(|a, b| b.cmp(a));
        }
        (Some(year_start), Some(year_end))
    } else if custom {
        let min_d = counts.keys().next().and_then(|s| date(s).ok());
        let max_d = counts.keys().next_back().and_then(|s| date(s).ok());
        (
            filters
                .from
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(date)
                .transpose()?
                .or(min_d),
            filters
                .to
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(date)
                .transpose()?
                .or(max_d),
        )
    } else {
        match period {
            "all" => {
                let min_d = counts.keys().next().and_then(|s| date(s).ok());
                let max_d = counts.keys().next_back().and_then(|s| date(s).ok());
                (min_d, max_d)
            }
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
                    Some(calendar_day(&calendar, now - days * 86_400_000)?),
                    Some(today),
                )
            }
            _ => return Err(AppError::BadRequest("Unknown period.".into())),
        }
    };

    let mut days = Vec::new();
    let min_date = from.map(|d| d.to_string());
    let max_date = through.map(|d| d.to_string());

    if let (Some(from_date), Some(through_date)) = (from, through) {
        if (through_date - from_date).num_days() > 4000 {
            return Err(AppError::BadRequest(
                "Calendar range exceeds 4,000 days. Please choose a shorter period or navigate by year."
                    .into(),
            ));
        }
        let mut curr = from_date;
        while curr <= through_date {
            let d_str = curr.to_string();
            let count = counts.get(&d_str).copied().unwrap_or(0);
            days.push(CalendarDay { date: d_str, count });
            curr += chrono::Duration::days(1);
        }
    }

    Ok(ListeningCalendar {
        days,
        min_date,
        max_date,
        total_listens,
        years,
    })
}

pub async fn listening_milestones(
    conn: &mut SqliteConnection,
    filters: &Filters,
    now: i64,
) -> Result<ListeningMilestones> {
    let all_time_filters = all_time_filters(filters);

    let mut total_query = consultation::selection_at(conn, &all_time_filters, false, now).await?;
    total_query.push("SELECT COUNT(*) FROM selected WHERE counted = 1");
    let total_listens: i64 = total_query
        .build_query_scalar()
        .fetch_one(&mut *conn)
        .await?;

    if total_listens == 0 {
        return Ok(ListeningMilestones {
            total_listens: 0,
            first_listen: None,
            last_listen: None,
            thresholds: Vec::new(),
            longest_streak: MilestoneStreak {
                days: 0,
                start_date: None,
                end_date: None,
            },
            current_streak: MilestoneStreak {
                days: 0,
                start_date: None,
                end_date: None,
            },
        });
    }

    let mut first_query = consultation::selection_at(conn, &all_time_filters, true, now).await?;
    first_query.push(
        "SELECT s.played_at_ms, s.title FROM selected s WHERE s.counted = 1 ORDER BY s.played_at_ms ASC, s.id ASC LIMIT 1",
    );
    let first_row = first_query.build().fetch_optional(&mut *conn).await?;
    let first_listen = first_row.map(|r| ListenMilestone {
        played_at_ms: r.get("played_at_ms"),
        track_title: r.get("title"),
    });

    let mut last_query = consultation::selection_at(conn, &all_time_filters, true, now).await?;
    last_query.push(
        "SELECT s.played_at_ms, s.title FROM selected s WHERE s.counted = 1 ORDER BY s.played_at_ms DESC, s.id DESC LIMIT 1",
    );
    let last_row = last_query.build().fetch_optional(&mut *conn).await?;
    let last_listen = last_row.map(|r| ListenMilestone {
        played_at_ms: r.get("played_at_ms"),
        track_title: r.get("title"),
    });

    let mut thresholds = Vec::new();
    if total_listens >= 10 {
        let mut thresh_query =
            consultation::selection_at(conn, &all_time_filters, false, now).await?;
        thresh_query.push(
            "SELECT row_num, played_at_ms FROM (                 SELECT ROW_NUMBER() OVER (ORDER BY played_at_ms ASC, id ASC) AS row_num, played_at_ms                 FROM selected                 WHERE counted = 1             ) WHERE row_num IN (10, 50, 100, 250, 500, 1000, 2500, 5000, 10000, 25000, 50000, 100000)             ORDER BY row_num ASC",
        );
        let rows = thresh_query.build().fetch_all(&mut *conn).await?;
        for r in rows {
            thresholds.push(MilestoneThreshold {
                count: r.get("row_num"),
                reached_at_ms: r.get("played_at_ms"),
            });
        }
    }

    let mut dates_query = consultation::selection_at(conn, &all_time_filters, false, now).await?;
    dates_query
        .push("SELECT DISTINCT local_date FROM selected WHERE counted = 1 ORDER BY local_date ASC");
    let date_rows = dates_query.build().fetch_all(&mut *conn).await?;

    let dates: Vec<NaiveDate> = date_rows
        .iter()
        .filter_map(|r| {
            let s: String = r.get("local_date");
            date(&s).ok()
        })
        .collect();

    let mut longest_days = 0i64;
    let mut longest_start: Option<NaiveDate> = None;
    let mut longest_end: Option<NaiveDate> = None;

    let mut streak_days = 0i64;
    let mut streak_start: Option<NaiveDate> = None;
    let mut prev_date: Option<NaiveDate> = None;

    for &curr in &dates {
        match prev_date {
            None => {
                streak_days = 1;
                streak_start = Some(curr);
            }
            Some(prev) => {
                if curr == prev + chrono::Duration::days(1) {
                    streak_days += 1;
                } else {
                    if streak_days > longest_days {
                        longest_days = streak_days;
                        longest_start = streak_start;
                        longest_end = Some(prev);
                    }
                    streak_days = 1;
                    streak_start = Some(curr);
                }
            }
        }
        prev_date = Some(curr);
    }

    if streak_days > longest_days {
        longest_days = streak_days;
        longest_start = streak_start;
        longest_end = prev_date;
    }

    let calendar = crate::calendar::current(conn).await?;
    let today = calendar_day(&calendar, now)?;
    let yesterday = today - chrono::Duration::days(1);

    let (current_days, current_start, current_end) = if let Some(&last_date) = dates.last() {
        if last_date == today || last_date == yesterday {
            (streak_days, streak_start, Some(last_date))
        } else {
            (0, None, None)
        }
    } else {
        (0, None, None)
    };

    Ok(ListeningMilestones {
        total_listens,
        first_listen,
        last_listen,
        thresholds,
        longest_streak: MilestoneStreak {
            days: longest_days,
            start_date: longest_start.map(|d| d.to_string()),
            end_date: longest_end.map(|d| d.to_string()),
        },
        current_streak: MilestoneStreak {
            days: current_days,
            start_date: current_start.map(|d| d.to_string()),
            end_date: current_end.map(|d| d.to_string()),
        },
    })
}
pub async fn discovery_ratio_calc(
    conn: &mut SqliteConnection,
    filters: &Filters,
    now: i64,
) -> Result<DiscoveryRatio> {
    let mut query = consultation::selection_at(conn, filters, false, now).await?;
    query.push(
        "SELECT \
            COALESCE(SUM(CASE WHEN first_played_ms >= min_period_played_ms THEN 1 ELSE 0 END), 0) AS discoveries, \
            COALESCE(SUM(CASE WHEN first_played_ms < min_period_played_ms THEN 1 ELSE 0 END), 0) AS repeats \
        FROM ( \
            SELECT \
                s.track_id, \
                MIN(s.played_at_ms) AS min_period_played_ms, \
                ( \
                    SELECT MIN(l.played_at_ms) \
                    FROM listens l \
                    WHERE l.track_id = s.track_id \
                      AND l.counted = 1 \
                      AND l.trashed_at_ms IS NULL \
                ) AS first_played_ms \
            FROM selected s \
            WHERE s.counted = 1 \
            GROUP BY s.track_id \
        )",
    );
    let row = query.build().fetch_one(&mut *conn).await?;
    let discoveries: i64 = row.try_get("discoveries")?;
    let repeats: i64 = row.try_get("repeats")?;
    let total = discoveries + repeats;
    let discovery_percent = if total > 0 {
        (discoveries as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    Ok(DiscoveryRatio {
        discoveries,
        repeats,
        total,
        discovery_percent,
    })
}
pub async fn activity_chart(
    conn: &mut SqliteConnection,
    filters: &Filters,
    now: i64,
) -> Result<ActivityChart> {
    let mut query = consultation::selection_at(conn, filters, false, now).await?;
    query.push(
        "SELECT calendar_id,local_date,week_date,COALESCE(SUM(counted=1),0) AS listens,COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='actual' THEN listened_ms END),0) AS actual_ms,COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='estimated' THEN listened_ms END),0) AS estimated_ms,COALESCE(SUM(counted=1 AND duration_kind='unknown'),0) AS unknown_durations FROM selected GROUP BY calendar_id,local_date,week_date ORDER BY local_date",
    );
    let rows: Vec<Daily> = query.build_query_as().fetch_all(&mut *conn).await?;
    // Used calendars, plus the effective calendar when there are no matching listens.
    let calendar_ids: BTreeSet<&str> = rows.iter().map(|r| r.calendar_id.as_str()).collect();
    let calendars: Vec<Calendar> = if calendar_ids.is_empty() {
        vec![crate::calendar::current(conn).await?]
    } else {
        let mut cal_query = sqlx::QueryBuilder::new("SELECT * FROM calendars WHERE id IN (");
        let mut separated = cal_query.separated(",");
        for id in calendar_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        cal_query.build_query_as().fetch_all(&mut *conn).await?
    };
    let period = filters.period.as_deref().unwrap_or("all");
    let custom = period == "custom"
        || filters.from.as_deref().is_some_and(|s| !s.is_empty())
        || filters.to.as_deref().is_some_and(|s| !s.is_empty());
    let min = rows
        .iter()
        .find(|r| r.listens > 0)
        .map(|r| date(&r.local_date))
        .transpose()?;
    let max = rows
        .iter()
        .rev()
        .find(|r| r.listens > 0)
        .map(|r| date(&r.local_date))
        .transpose()?;
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
    let grain = filters.grain.as_deref().unwrap_or(if span <= 90 {
        "day"
    } else if span <= 9125 {
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
        if row.listens == 0 {
            continue;
        }
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
