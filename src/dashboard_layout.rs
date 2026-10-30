//! Dashboard modular layout persistence and schema validation.
use crate::{
    AppState, auth, db,
    error::{AppError, Result},
};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

pub const ALLOWED_SPANS: &[u8] = &[2, 3, 4, 6];
pub const VALID_WIDGET_TYPES: &[&str] = &[
    "totals",
    "unified_rankings",
    "activity_chart",
    "recent_listens",
    "top_artists",
    "top_albums",
    "top_tracks",
    "now_playing",
    "listening_patterns",
    "discovery_ratio",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WidgetConfig {
    pub id: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub widget_type: String,
    #[ts(type = "2 | 3 | 4 | 6")]
    pub col_span: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_override: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct DashboardLayout {
    pub version: i64,
    pub widgets: Vec<WidgetConfig>,
}

impl Default for DashboardLayout {
    fn default() -> Self {
        Self {
            version: 0,
            widgets: vec![
                WidgetConfig {
                    id: "totals".into(),
                    widget_type: "totals".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "unified_rankings".into(),
                    widget_type: "unified_rankings".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "activity_chart".into(),
                    widget_type: "activity_chart".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "recent_listens".into(),
                    widget_type: "recent_listens".into(),
                    col_span: 6,
                    period_override: None,
                },
            ],
        }
    }
}

impl DashboardLayout {
    pub fn validate(&self) -> Result<()> {
        if self.widgets.is_empty() || self.widgets.len() > 32 {
            return Err(AppError::BadRequest(
                "Dashboard layout must contain between 1 and 32 widgets.".into(),
            ));
        }
        for w in &self.widgets {
            if w.id.is_empty()
                || w.id.len() > 64
                || !w
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(AppError::BadRequest("Invalid widget ID.".into()));
            }
            if !VALID_WIDGET_TYPES.contains(&w.widget_type.as_str()) {
                return Err(AppError::BadRequest(format!(
                    "Unknown widget type '{}'.",
                    w.widget_type
                )));
            }
            if !ALLOWED_SPANS.contains(&w.col_span) {
                return Err(AppError::BadRequest(format!(
                    "Invalid column span {}. Must be 2, 3, 4, or 6.",
                    w.col_span
                )));
            }
            if let Some(ref period) = w.period_override
                && !crate::preferences::PERIODS.contains(&period.as_str())
            {
                return Err(AppError::BadRequest("Invalid period override.".into()));
            }
        }
        Ok(())
    }
}

pub async fn read(conn: &mut SqliteConnection) -> Result<DashboardLayout> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='dashboard_layout'")
            .fetch_optional(conn)
            .await?;
    let layout = match value {
        Some(v) => serde_json::from_str(&v).map_err(|_| AppError::Internal)?,
        None => DashboardLayout::default(),
    };
    layout.validate()?;
    Ok(layout)
}

pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DashboardLayout>> {
    auth::session(&state, &headers).await?;
    let mut view = state.storage.read_view().await?;
    Ok(Json(read(&mut view.transaction).await?))
}

pub async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(layout): Json<DashboardLayout>,
) -> Result<Json<DashboardLayout>> {
    auth::require_write(&state, &headers).await?;
    layout.validate()?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision",
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::Conflict("Archive is in maintenance.".into()))?;

    let current = read(&mut tx).await?;
    if current.version != layout.version {
        return Err(AppError::Conflict(
            "Dashboard layout changed in another session. Reload before saving.".into(),
        ));
    }
    let mut to_save = layout;
    to_save.version = current.version + 1;
    let serialized = serde_json::to_string(&to_save).map_err(|_| AppError::Internal)?;
    db::set_setting(&mut tx, "dashboard_layout", &serialized).await?;
    tx.commit().await?;
    Ok(Json(to_save))
}
