//! Grouping, ordering, limits and the distinct metrics.
//!
//! Every bucket is built from the canonical consultation selection, so the
//! assigned calendar of each listen (`local_date`, `week_date`, timezone and
//! week start) decides its day, week, month, year, hour and weekday. Only
//! counted listens contribute numbers. Time coverage stays separate from counts
//! and unknown durations are never treated as zero.
use crate::{
    ask::{
        AskResult, ResultRow, TimeCoverage,
        dsl::{GroupBy, Limit, Metric, Query, SortBy, SortOrder},
    },
    calendar,
    consultation::{Filters, selection_at},
    error::{AppError, Result},
};
use chrono::{DateTime, Datelike, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use sqlx::{FromRow, SqliteConnection};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The metrics and shapes owned by this module. Plays are grouped only when the
/// query asks for a grouping; the distinct metrics always run here.
pub fn handles(query: &Query) -> bool {
    matches!(
        query.metric,
        Metric::DistinctTracks | Metric::DistinctArtists | Metric::DistinctAlbums
    ) || (query.metric == Metric::Plays && query.group_by.is_some())
}

pub async fn execute(
    conn: &mut SqliteConnection,
    query: &Query,
    filters: &Filters,
    now: i64,
) -> Result<AskResult> {
    // Weekday ordering follows the effective calendar. Assigned week dates
    // remain per-listen, as the canonical selection already stores them.
    let order_week_start = calendar::current(conn).await?.week_start;

    let mut selection = selection_at(conn, filters, true, now).await?;
    selection.push(
        "SELECT s.played_at_ms,s.local_date,s.week_date,s.counted,\
         s.listened_ms,s.duration_kind,s.track_id,s.title,s.album_id,s.album,\
         c.timezone,\
         (SELECT COALESCE(json_group_array(json_object('id',id,'name',name)),'[]') FROM \
          (SELECT ar.id,ar.name FROM track_artists ta JOIN artists ar ON ar.id=ta.artist_id \
           WHERE ta.track_id=s.track_id ORDER BY ta.ordinal)) AS artists \
         FROM selected s JOIN calendars c ON c.id=s.calendar_id \
         ORDER BY s.played_at_ms,s.id",
    );
    let listens: Vec<RawListen> = selection.build_query_as().fetch_all(&mut *conn).await?;

    let mut buckets = BTreeMap::<String, Bucket>::new();
    let mut zones = HashMap::<String, Tz>::new();
    for listen in listens {
        if !listen.counted {
            continue;
        }
        if let Some(hour) = query.hour {
            let tz = zone(&mut zones, &listen.timezone)?;
            let local_hour = DateTime::<Utc>::from_timestamp_millis(listen.played_at_ms)
                .ok_or(AppError::Internal)?
                .with_timezone(&tz)
                .hour();
            if local_hour != u32::from(hour) {
                continue;
            }
        }
        let artists: Vec<ArtistRef> =
            serde_json::from_str(&listen.artists).map_err(|_| AppError::Internal)?;
        for spec in bucket_specs(&listen, query.group_by, &mut zones, order_week_start)? {
            buckets
                .entry(spec.key.clone())
                .or_insert_with(|| Bucket::new(spec))
                .add(&listen, &artists);
        }
    }

    let mut buckets: Vec<Bucket> = buckets.into_values().collect();
    for bucket in &mut buckets {
        bucket.value = bucket.metric_value(query.metric);
    }
    sort_buckets(&mut buckets, query);

    if query.group_by.is_none() {
        // Ungrouped distinct metrics: a scalar and no rows.
        let value = buckets
            .into_iter()
            .next()
            .map(|bucket| bucket.value)
            .unwrap_or(0);
        return Ok(AskResult {
            metric: query.metric,
            scalar: Some(value),
            time: None,
            rows: Vec::new(),
            first: None,
            last: None,
            streak: None,
            milestones: Vec::new(),
            total: value,
            links: entity_links(query),
        });
    }

    if let Some(limit) = query.limit {
        let limit = limit.value().clamp(1, Limit::MAX);
        buckets.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    }
    let rows: Vec<ResultRow> = buckets.into_iter().map(ResultRow::from).collect();
    let total = i64::try_from(rows.len()).unwrap_or(i64::MAX);
    Ok(AskResult {
        metric: query.metric,
        scalar: None,
        time: None,
        rows,
        first: None,
        last: None,
        streak: None,
        milestones: Vec::new(),
        total,
        links: entity_links(query),
    })
}

fn bucket_specs(
    listen: &RawListen,
    group_by: Option<GroupBy>,
    zones: &mut HashMap<String, Tz>,
    order_week_start: i64,
) -> Result<Vec<BucketSpec>> {
    let spec = match group_by {
        None => BucketSpec::new("total", "", ""),
        Some(GroupBy::Day) => BucketSpec::temporal(listen.local_date.clone()),
        Some(GroupBy::Week) => BucketSpec::temporal(listen.week_date.clone()),
        Some(GroupBy::Month) => {
            let key = listen
                .local_date
                .get(..7)
                .ok_or(AppError::Internal)?
                .to_owned();
            BucketSpec::temporal(key)
        }
        Some(GroupBy::Year) => {
            let key = listen
                .local_date
                .get(..4)
                .ok_or(AppError::Internal)?
                .to_owned();
            BucketSpec::temporal(key)
        }
        Some(GroupBy::Hour) => {
            let tz = zone(zones, &listen.timezone)?;
            let hour = DateTime::<Utc>::from_timestamp_millis(listen.played_at_ms)
                .ok_or(AppError::Internal)?
                .with_timezone(&tz)
                .hour();
            let label = format!("{hour:02}:00");
            BucketSpec::temporal(label)
        }
        Some(GroupBy::Weekday) => {
            let date = NaiveDate::parse_from_str(&listen.local_date, "%Y-%m-%d")
                .map_err(|_| AppError::Internal)?;
            let label = weekday_label(date.weekday());
            let index =
                (i64::from(date.weekday().num_days_from_monday()) - order_week_start).rem_euclid(7);
            BucketSpec {
                key: label.to_lowercase(),
                label: label.to_owned(),
                href: None,
                sort_date: format!("{index:02}"),
            }
        }
        Some(GroupBy::Artist) => {
            let artists: Vec<ArtistRef> =
                serde_json::from_str(&listen.artists).map_err(|_| AppError::Internal)?;
            return Ok(artists
                .into_iter()
                .map(|artist| BucketSpec {
                    key: artist.id.clone(),
                    label: artist.name,
                    href: Some(format!("/music/artists/{}", artist.id)),
                    sort_date: String::new(),
                })
                .collect());
        }
        Some(GroupBy::Album) => match &listen.album_id {
            Some(id) => BucketSpec {
                key: id.clone(),
                label: listen
                    .album
                    .clone()
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "Unknown album".into()),
                href: Some(format!("/music/albums/{id}")),
                sort_date: String::new(),
            },
            None => BucketSpec {
                key: "unknown-album".into(),
                label: "Unknown album".into(),
                href: None,
                sort_date: String::new(),
            },
        },
        Some(GroupBy::Track) => BucketSpec {
            key: listen.track_id.clone(),
            label: listen.title.clone(),
            href: Some(format!("/music/tracks/{}", listen.track_id)),
            sort_date: String::new(),
        },
    };
    Ok(vec![spec])
}

fn weekday_label(weekday: chrono::Weekday) -> &'static str {
    match weekday {
        chrono::Weekday::Mon => "Monday",
        chrono::Weekday::Tue => "Tuesday",
        chrono::Weekday::Wed => "Wednesday",
        chrono::Weekday::Thu => "Thursday",
        chrono::Weekday::Fri => "Friday",
        chrono::Weekday::Sat => "Saturday",
        chrono::Weekday::Sun => "Sunday",
    }
}

fn zone(zones: &mut HashMap<String, Tz>, name: &str) -> Result<Tz> {
    if let Some(tz) = zones.get(name) {
        return Ok(*tz);
    }
    let tz: Tz = name.parse().map_err(|_| AppError::Internal)?;
    zones.insert(name.to_owned(), tz);
    Ok(tz)
}

fn sort_buckets(buckets: &mut [Bucket], query: &Query) {
    let descending = query.order == SortOrder::Desc;
    buckets.sort_by(|a, b| {
        let primary = match query.sort_by {
            SortBy::Metric => a.value.cmp(&b.value),
            SortBy::Date => a.sort_date.cmp(&b.sort_date),
        };
        let primary = if descending {
            primary.reverse()
        } else {
            primary
        };
        // Deterministic tie-breaking: the label always ascends, then the key.
        primary
            .then_with(|| a.label.cmp(&b.label))
            .then_with(|| a.key.cmp(&b.key))
    });
}

fn entity_links(query: &Query) -> Vec<crate::ask::ResultLink> {
    let Some(entity) = &query.entity else {
        return Vec::new();
    };
    let (Some(id), kind) = (entity.id.as_deref(), entity.kind) else {
        return Vec::new();
    };
    let (segment, label) = match kind {
        crate::ask::dsl::EntityKind::Artist => ("artists", "View artist"),
        crate::ask::dsl::EntityKind::Album => ("albums", "Open album"),
        crate::ask::dsl::EntityKind::Track => ("tracks", "Open track"),
    };
    vec![crate::ask::ResultLink {
        label: label.into(),
        href: format!("/music/{segment}/{id}"),
    }]
}

#[derive(FromRow)]
struct RawListen {
    played_at_ms: i64,
    local_date: String,
    week_date: String,
    counted: bool,
    listened_ms: Option<i64>,
    duration_kind: String,
    track_id: String,
    title: String,
    album_id: Option<String>,
    album: Option<String>,
    timezone: String,
    artists: String,
}

#[derive(Deserialize)]
struct ArtistRef {
    id: String,
    name: String,
}

struct BucketSpec {
    key: String,
    label: String,
    href: Option<String>,
    sort_date: String,
}

impl BucketSpec {
    fn new(key: impl Into<String>, label: impl Into<String>, sort_date: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            href: None,
            sort_date: sort_date.into(),
        }
    }

    fn temporal(key: String) -> Self {
        Self {
            sort_date: key.clone(),
            label: key.clone(),
            key,
            href: None,
        }
    }
}

#[derive(Default)]
struct Bucket {
    key: String,
    label: String,
    href: Option<String>,
    sort_date: String,
    counted: i64,
    plays: i64,
    tracks: BTreeSet<String>,
    albums: BTreeSet<String>,
    artists: BTreeSet<String>,
    actual_ms: i64,
    estimated_ms: i64,
    unknown_listens: i64,
    value: i64,
}

impl Bucket {
    fn new(spec: BucketSpec) -> Self {
        Self {
            key: spec.key,
            label: spec.label,
            href: spec.href,
            sort_date: spec.sort_date,
            ..Self::default()
        }
    }

    fn add(&mut self, listen: &RawListen, artists: &[ArtistRef]) {
        self.counted += 1;
        self.plays += 1;
        self.tracks.insert(listen.track_id.clone());
        if let Some(album) = &listen.album_id {
            self.albums.insert(album.clone());
        }
        for artist in artists {
            self.artists.insert(artist.id.clone());
        }
        match listen.duration_kind.as_str() {
            "actual" => self.actual_ms += listen.listened_ms.unwrap_or(0),
            "estimated" => self.estimated_ms += listen.listened_ms.unwrap_or(0),
            "unknown" => self.unknown_listens += 1,
            _ => {}
        }
    }

    fn metric_value(&self, metric: Metric) -> i64 {
        match metric {
            Metric::DistinctTracks => i64::try_from(self.tracks.len()).unwrap_or(i64::MAX),
            Metric::DistinctArtists => i64::try_from(self.artists.len()).unwrap_or(i64::MAX),
            Metric::DistinctAlbums => i64::try_from(self.albums.len()).unwrap_or(i64::MAX),
            _ => self.plays,
        }
    }

    fn coverage(&self) -> Option<TimeCoverage> {
        if self.counted == 0 {
            return None;
        }
        Some(TimeCoverage {
            known_ms: self.actual_ms + self.estimated_ms,
            actual_ms: self.actual_ms,
            estimated_ms: self.estimated_ms,
            unknown_listens: self.unknown_listens,
            unknown_percent: (self.unknown_listens as f64 / self.counted as f64) * 100.0,
        })
    }
}

impl From<Bucket> for ResultRow {
    fn from(bucket: Bucket) -> Self {
        let time = bucket.coverage();
        Self {
            label: bucket.label,
            key: bucket.key,
            plays: bucket.value,
            time,
            href: bucket.href,
        }
    }
}
