//! Browser contracts. The checked-in TypeScript declarations are checked by a Rust test.
use crate::db::{Artist, DurationKind, IngestReport};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Preferences {
    pub name: String,
    pub initial_period: String,
    pub hour_format: String,
    pub timezone: String,
    pub week_start: i64,
    pub version: i64,
}
#[derive(Deserialize, TS)]
pub struct SavePreferences {
    pub preferences: Preferences,
    pub calendar_scope: Option<CalendarScope>,
}
#[derive(Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CalendarScope {
    Future,
    All,
}
#[derive(Serialize, TS)]
pub struct SavedPreferences {
    pub preferences: Preferences,
    pub job_id: Option<String>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct Listen {
    pub id: String,
    pub track_id: String,
    pub title: String,
    pub album: Option<String>,
    pub cover_url: Option<String>,
    pub spotify_url: Option<String>,
    pub played_at_ms: i64,
    pub listened_ms: Option<i64>,
    pub duration_kind: DurationKind,
    pub counted: bool,
    pub needs_review: bool,
    pub artists: Vec<Artist>,
    pub sources: Vec<String>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct History {
    pub items: Vec<Listen>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    pub timezone: String,
}
#[derive(Serialize, Deserialize, TS)]
pub struct Totals {
    pub records: i64,
    pub listens: i64,
    pub tracks: i64,
    pub actual_ms: i64,
    pub estimated_ms: i64,
    pub unknown_durations: i64,
}
#[derive(Serialize, Deserialize, TS)]
pub struct Ranking {
    pub id: String,
    pub name: String,
    pub subtitle: Option<String>,
    pub cover_url: Option<String>,
    pub plays: i64,
    pub listened_ms: i64,
}
#[derive(Serialize, Deserialize, TS)]
pub struct Rankings {
    pub artists: Vec<Ranking>,
    pub albums: Vec<Ranking>,
    pub tracks: Vec<Ranking>,
}
#[derive(Serialize, Deserialize, TS)]
pub struct Dashboard {
    pub totals: Totals,
    pub rankings: Rankings,
    pub recent: Vec<Listen>,
    pub timezone: String,
}
#[derive(Serialize, Deserialize, TS)]
pub struct NowPlaying {
    pub title: String,
    pub artists: Vec<Artist>,
    pub album: Option<String>,
    pub cover_url: Option<String>,
    pub spotify_url: String,
    pub duration_ms: Option<i64>,
    pub progress_ms: Option<i64>,
    pub observed_at_ms: i64,
    pub source_timestamp_ms: Option<i64>,
    pub requested_at_ms: i64,
    pub source_requested_at_ms: i64,
    pub response_received_at_ms: i64,
    pub served_at_ms: i64,
}
#[derive(Serialize, Deserialize, TS)]
pub struct ImportJob {
    pub filename: String,
    pub created_at_ms: i64,
    pub report: IngestReport,
}
#[derive(Serialize, Deserialize, TS)]
pub struct NowPlayingTiming {
    pub source_timestamp_ms: Option<i64>,
    pub requested_at_ms: i64,
    pub source_requested_at_ms: i64,
    pub response_received_at_ms: i64,
}
#[derive(Serialize, TS, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SpotifyIntervals {
    pub sync_seconds: u64,
    pub now_playing_seconds: u64,
}
#[derive(Serialize, TS)]
pub struct SpotifyStatus {
    pub configured: bool,
    pub connected: bool,
    pub credentials_from_env: bool,
    pub redirect_uri: String,
    pub sync_seconds: u64,
    pub now_playing_seconds: u64,
    pub connection: String,
    pub collection: String,
    pub last_attempt_ms: Option<i64>,
    pub last_sync_ms: Option<i64>,
    pub last_added: Option<i64>,
    pub verified_at_ms: Option<i64>,
    pub last_error: Option<String>,
    pub now_playing_error: Option<String>,
    pub now_playing_timing: Option<NowPlayingTiming>,
    pub coverage_warning: Option<String>,
    pub retry_at_ms: Option<i64>,
    pub imports: Vec<ImportJob>,
    pub review_count: i64,
}
#[derive(Serialize, TS)]
pub struct Activity {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub filename: Option<String>,
    pub attempt: i64,
    pub processed: i64,
    pub added: i64,
    pub reconciled: i64,
    pub duplicates: i64,
    pub invalid: i64,
    pub skipped: i64,
    pub ambiguous: i64,
    pub retry_at_ms: Option<i64>,
    pub error_code: Option<String>,
    pub expected_rows: Option<i64>,
    pub phase: String,
    pub outcome: Option<String>,
    pub inventory_complete: bool,
    pub originals: i64,
    pub variants: i64,
    pub below_threshold: i64,
    pub target_timezone: Option<String>,
    pub target_week_start: Option<i64>,
}
#[derive(Serialize, sqlx::FromRow, TS)]
pub struct ActivityMember {
    pub originals: i64,
    pub variants: i64,
    pub duplicates: i64,
    pub added: i64,
    pub reconciled: i64,
    pub invalid: i64,
    pub skipped: i64,
    pub below_threshold: i64,
    pub member: String,
    pub kind: String,
    pub state: String,
    pub expected_rows: Option<i64>,
    pub processed: i64,
    pub error_code: Option<String>,
}
#[derive(Serialize, TS)]
pub struct ActivityDetail {
    pub issues: Vec<RowIssue>,
    pub activity: Activity,
    pub members: Vec<ActivityMember>,
}

#[derive(Serialize, sqlx::FromRow, TS)]
pub struct RowIssue {
    pub member: String,
    pub row_number: i64,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_contract_is_current() {
        let config = ts_rs::Config::default().with_large_int("number");
        let types = [
            Preferences::decl(&config),
            SavePreferences::decl(&config),
            CalendarScope::decl(&config),
            SavedPreferences::decl(&config),
            Artist::decl(&config),
            DurationKind::decl(&config),
            IngestReport::decl(&config),
            Listen::decl(&config),
            History::decl(&config),
            Totals::decl(&config),
            Ranking::decl(&config),
            Rankings::decl(&config),
            Dashboard::decl(&config),
            NowPlaying::decl(&config),
            NowPlayingTiming::decl(&config),
            ImportJob::decl(&config),
            SpotifyIntervals::decl(&config),
            SpotifyStatus::decl(&config),
            Activity::decl(&config),
            ActivityMember::decl(&config),
            ActivityDetail::decl(&config),
            RowIssue::decl(&config),
            crate::imports::ImportLimits::decl(&config),
            crate::imports::ImportResponse::decl(&config),
            crate::exports::ExportSelection::decl(&config),
            crate::recovery::InventoryFile::decl(&config),
            crate::recovery::Manifest::decl(&config),
            crate::recovery::Asset::decl(&config),
        ];
        let content = format!(
            "// Generated from Rust DTOs. Run UPDATE_BROWSER_TYPES=1 cargo test browser_contract_is_current.\n{}\n",
            types.map(|t| format!("export {t}")).join("\n")
        );
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend/src/generated.ts");
        if std::env::var_os("UPDATE_BROWSER_TYPES").is_some() {
            std::fs::write(&path, &content).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(path).unwrap().replace("\r\n", "\n"),
            content,
            "Regenerate the browser contract"
        );
    }
}
