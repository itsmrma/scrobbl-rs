//! Time listened and its coverage.
//!
//! Known time is the sum of actual and estimated playback time on counted
//! listens. Unknown duration is never zero: it stays a separate count with its
//! share of the counted listens, so the card can always show the coverage.
use crate::{
    ask::{AskResult, TimeCoverage, dsl::Query, execute::entity_links},
    consultation::{Filters, selection_at},
    error::Result,
};
use sqlx::{FromRow, SqliteConnection};

pub async fn execute(
    conn: &mut SqliteConnection,
    query: &Query,
    filters: &Filters,
    now: i64,
) -> Result<AskResult> {
    let mut selection = selection_at(conn, filters, false, now).await?;
    selection.push(
        "SELECT COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='actual' AND listened_ms IS NOT NULL THEN listened_ms END),0) AS actual_ms,COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='estimated' AND listened_ms IS NOT NULL THEN listened_ms END),0) AS estimated_ms,COALESCE(SUM(CASE WHEN counted=1 AND (duration_kind='unknown' OR listened_ms IS NULL) THEN 1 ELSE 0 END),0) AS unknown_listens,COALESCE(SUM(counted),0) AS total FROM selected",
    );
    let sums: TimeSums = selection.build_query_as().fetch_one(&mut *conn).await?;
    Ok(result(query, sums))
}

/// Accumulates the same split when the hour filter runs in Rust instead of SQL.
#[derive(Default, Clone, Copy, FromRow)]
pub struct TimeSums {
    pub actual_ms: i64,
    pub estimated_ms: i64,
    pub unknown_listens: i64,
    pub total: i64,
}

impl TimeSums {
    pub fn add(&mut self, counted: bool, listened_ms: Option<i64>, duration_kind: &str) {
        if !counted {
            return;
        }
        self.total += 1;
        match (duration_kind, listened_ms) {
            ("actual", Some(ms)) => self.actual_ms += ms,
            ("estimated", Some(ms)) => self.estimated_ms += ms,
            _ => self.unknown_listens += 1,
        }
    }

    pub fn coverage(self) -> TimeCoverage {
        TimeCoverage {
            known_ms: self.actual_ms + self.estimated_ms,
            actual_ms: self.actual_ms,
            estimated_ms: self.estimated_ms,
            unknown_listens: self.unknown_listens,
            unknown_percent: if self.total == 0 {
                0.0
            } else {
                self.unknown_listens as f64 / self.total as f64 * 100.0
            },
        }
    }
}

pub fn result(query: &Query, sums: TimeSums) -> AskResult {
    let coverage = sums.coverage();
    AskResult {
        metric: query.metric,
        scalar: Some(coverage.known_ms),
        time: Some(coverage),
        rows: Vec::new(),
        first: None,
        last: None,
        streak: None,
        milestones: Vec::new(),
        total: sums.total,
        links: entity_links(query),
    }
}
