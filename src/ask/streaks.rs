//! Listening streaks and milestones for the Ask surface.
//!
//! Both metrics describe the whole history of the selected entity (GLOSSARY
//! "Serie di giorni di ascolto" and "Traguardo di ascolto"), so the requested
//! period, year, month, weekday and hour are deliberately ignored. The source
//! filter is kept because it narrows provenance, not time: asking about a
//! specific source still asks about that source's whole history.
//!
//! The calculation is reused from `crate::browse::listening_milestones`, the
//! same function behind the listening calendar widget, so the thresholds and
//! streak semantics cannot drift between the two surfaces.
use crate::{
    ask::{
        AskResult, MilestonePoint, ResultLink, StreakResult, StreakSpan,
        dsl::{Metric, Query},
        execute::entity_links,
    },
    browse::listening_milestones,
    consultation::Filters,
    error::{AppError, Result},
};
use sqlx::SqliteConnection;

pub async fn execute(
    conn: &mut SqliteConnection,
    query: &Query,
    filters: &Filters,
    now: i64,
) -> Result<AskResult> {
    let history = listening_milestones(conn, &all_time_filters(filters), now).await?;

    let current = StreakSpan {
        days: history.current_streak.days,
        start: history.current_streak.start_date,
        end: history.current_streak.end_date,
    };
    let longest = StreakSpan {
        days: history.longest_streak.days,
        start: history.longest_streak.start_date,
        end: history.longest_streak.end_date,
    };
    let milestones: Vec<MilestonePoint> = history
        .thresholds
        .into_iter()
        .map(|threshold| MilestonePoint {
            count: threshold.count,
            reached_at_ms: threshold.reached_at_ms,
        })
        .collect();

    let (scalar, total) = match query.metric {
        Metric::Streak => (current.days, current.days),
        Metric::Milestones => (history.total_listens, history.total_listens),
        _ => return Err(AppError::Internal),
    };

    let mut links = entity_links(query);
    // The streak dates are the only local-date range this metric knows, so link
    // to them when there is a streak. The entity link above always applies.
    let range = if current.days > 0 {
        Some(&current)
    } else if longest.days > 0 {
        Some(&longest)
    } else {
        None
    };
    if let Some(span) = range
        && let (Some(start), Some(end)) = (&span.start, &span.end)
    {
        links.push(ResultLink {
            label: format!("Open {start} – {end}"),
            href: format!("/history?from={start}&to={end}"),
        });
    }

    Ok(AskResult {
        metric: query.metric,
        scalar: Some(scalar),
        time: None,
        rows: Vec::new(),
        first: None,
        last: None,
        streak: (query.metric == Metric::Streak).then_some(StreakResult { current, longest }),
        milestones: if query.metric == Metric::Milestones {
            milestones
        } else {
            Vec::new()
        },
        total,
        links,
    })
}

/// Achievements and streaks refer to the entity's whole history, independent
/// of the consulted period. Source is provenance, not time, so it survives.
fn all_time_filters(filters: &Filters) -> Filters {
    let mut all_time = filters.clone();
    all_time.period = Some("all".into());
    all_time.from = None;
    all_time.to = None;
    all_time.bucket = None;
    all_time.bucket_start = None;
    all_time.year = None;
    all_time.month = None;
    all_time.weekday = None;
    all_time.hour = None;
    all_time.include_totals = None;
    all_time
}
