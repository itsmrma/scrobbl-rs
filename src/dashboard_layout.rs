//! Dashboard modular layout persistence and schema validation.
use crate::{
    AppState, auth, db,
    error::{AppError, Result},
};
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
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
    "history",
    "listening_calendar",
    "listening_milestones",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutTemplate {
    Home,
    Artist,
    Album,
    Track,
}

impl LayoutTemplate {
    pub fn parse(s: Option<&str>) -> Result<Self> {
        match s {
            None | Some("") | Some("home") => Ok(Self::Home),
            Some("artist") => Ok(Self::Artist),
            Some("album") => Ok(Self::Album),
            Some("track") => Ok(Self::Track),
            Some(other) => Err(AppError::BadRequest(format!(
                "Invalid template parameter '{other}'. Expected 'home', 'artist', 'album', or 'track'."
            ))),
        }
    }

    pub fn setting_key(&self) -> &'static str {
        match self {
            Self::Home => "dashboard_layout",
            Self::Artist => "dashboard_layout_artist",
            Self::Album => "dashboard_layout_album",
            Self::Track => "dashboard_layout_track",
        }
    }

    pub fn is_music_template(&self) -> bool {
        !matches!(self, Self::Home)
    }

    pub fn default_layout(&self) -> DashboardLayout {
        match self {
            Self::Home => DashboardLayout::default_home(),
            Self::Artist => DashboardLayout::default_artist(),
            Self::Album => DashboardLayout::default_album(),
            Self::Track => DashboardLayout::default_track(),
        }
    }

    pub fn is_widget_allowed(&self, widget_type: &str) -> bool {
        match self {
            Self::Home => matches!(
                widget_type,
                "totals"
                    | "unified_rankings"
                    | "activity_chart"
                    | "recent_listens"
                    | "top_artists"
                    | "top_albums"
                    | "top_tracks"
                    | "now_playing"
                    | "listening_patterns"
                    | "discovery_ratio"
            ),
            Self::Artist => matches!(
                widget_type,
                "totals"
                    | "activity_chart"
                    | "listening_patterns"
                    | "recent_listens"
                    | "history"
                    | "top_albums"
                    | "top_tracks"
                    | "discovery_ratio"
                    | "listening_calendar"
                    | "listening_milestones"
            ),
            Self::Album => matches!(
                widget_type,
                "totals"
                    | "activity_chart"
                    | "listening_patterns"
                    | "recent_listens"
                    | "history"
                    | "top_tracks"
                    | "discovery_ratio"
                    | "listening_calendar"
                    | "listening_milestones"
            ),
            Self::Track => matches!(
                widget_type,
                "totals"
                    | "activity_chart"
                    | "listening_patterns"
                    | "recent_listens"
                    | "history"
                    | "listening_calendar"
                    | "listening_milestones"
            ),
        }
    }
}

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
        Self::default_home()
    }
}

impl DashboardLayout {
    pub fn default_home() -> Self {
        Self {
            version: 0,
            widgets: vec![
                WidgetConfig {
                    id: "now_playing".into(),
                    widget_type: "now_playing".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "totals".into(),
                    widget_type: "totals".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "top_artists".into(),
                    widget_type: "top_artists".into(),
                    col_span: 2,
                    period_override: None,
                },
                WidgetConfig {
                    id: "top_albums".into(),
                    widget_type: "top_albums".into(),
                    col_span: 2,
                    period_override: None,
                },
                WidgetConfig {
                    id: "top_tracks".into(),
                    widget_type: "top_tracks".into(),
                    col_span: 2,
                    period_override: None,
                },
                WidgetConfig {
                    id: "activity_chart".into(),
                    widget_type: "activity_chart".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "listening_patterns".into(),
                    widget_type: "listening_patterns".into(),
                    col_span: 3,
                    period_override: None,
                },
                WidgetConfig {
                    id: "recent_listens".into(),
                    widget_type: "recent_listens".into(),
                    col_span: 3,
                    period_override: None,
                },
            ],
        }
    }

    pub fn default_artist() -> Self {
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
                    id: "activity_chart".into(),
                    widget_type: "activity_chart".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "listening_patterns".into(),
                    widget_type: "listening_patterns".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "top_albums".into(),
                    widget_type: "top_albums".into(),
                    col_span: 3,
                    period_override: None,
                },
                WidgetConfig {
                    id: "top_tracks".into(),
                    widget_type: "top_tracks".into(),
                    col_span: 3,
                    period_override: None,
                },
                WidgetConfig {
                    id: "history".into(),
                    widget_type: "history".into(),
                    col_span: 6,
                    period_override: None,
                },
            ],
        }
    }

    pub fn default_album() -> Self {
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
                    id: "activity_chart".into(),
                    widget_type: "activity_chart".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "listening_patterns".into(),
                    widget_type: "listening_patterns".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "top_tracks".into(),
                    widget_type: "top_tracks".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "history".into(),
                    widget_type: "history".into(),
                    col_span: 6,
                    period_override: None,
                },
            ],
        }
    }

    pub fn default_track() -> Self {
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
                    id: "activity_chart".into(),
                    widget_type: "activity_chart".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "listening_patterns".into(),
                    widget_type: "listening_patterns".into(),
                    col_span: 6,
                    period_override: None,
                },
                WidgetConfig {
                    id: "history".into(),
                    widget_type: "history".into(),
                    col_span: 6,
                    period_override: None,
                },
            ],
        }
    }

    pub fn validate(&self, template: LayoutTemplate) -> Result<()> {
        let max_widgets = 32;
        if template.is_music_template() {
            if self.widgets.len() > max_widgets {
                return Err(AppError::BadRequest(
                    "Layout must contain at most 32 widgets.".into(),
                ));
            }
        } else if self.widgets.is_empty() || self.widgets.len() > max_widgets {
            return Err(AppError::BadRequest(
                "Dashboard layout must contain between 1 and 32 widgets.".into(),
            ));
        }

        let mut seen_ids = std::collections::HashSet::new();
        let mut seen_types = std::collections::HashSet::new();

        for w in &self.widgets {
            if !seen_ids.insert(&w.id) {
                return Err(AppError::BadRequest(format!(
                    "Duplicate widget ID '{}'.",
                    w.id
                )));
            }
            if !seen_types.insert(&w.widget_type) {
                return Err(AppError::BadRequest(format!(
                    "Duplicate widget type '{}'.",
                    w.widget_type
                )));
            }

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
            if !template.is_widget_allowed(&w.widget_type) {
                return Err(AppError::BadRequest(format!(
                    "Widget type '{}' is not supported on {:?} template.",
                    w.widget_type, template
                )));
            }
            if !ALLOWED_SPANS.contains(&w.col_span) {
                return Err(AppError::BadRequest(format!(
                    "Invalid column span {}. Must be 2, 3, 4, or 6.",
                    w.col_span
                )));
            }
            if w.widget_type == "listening_milestones" && w.period_override.is_some() {
                return Err(AppError::BadRequest(
                    "Listening milestones does not support period override.".into(),
                ));
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

pub async fn read(
    conn: &mut SqliteConnection,
    template: LayoutTemplate,
) -> Result<DashboardLayout> {
    let key = template.setting_key();
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
        .bind(key)
        .fetch_optional(conn)
        .await?;
    let layout = match value {
        Some(v) => serde_json::from_str(&v).map_err(|_| AppError::Internal)?,
        None => template.default_layout(),
    };
    layout.validate(template)?;
    Ok(layout)
}

#[derive(Debug, Deserialize)]
pub struct LayoutParams {
    pub template: Option<String>,
}

pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<LayoutParams>,
) -> Result<Json<DashboardLayout>> {
    auth::session(&state, &headers).await?;
    let template = LayoutTemplate::parse(params.template.as_deref())?;
    let mut view = state.storage.read_view().await?;
    Ok(Json(read(&mut view.transaction, template).await?))
}

pub async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<LayoutParams>,
    Json(layout): Json<DashboardLayout>,
) -> Result<Json<DashboardLayout>> {
    auth::require_write(&state, &headers).await?;
    let template = LayoutTemplate::parse(params.template.as_deref())?;
    layout.validate(template)?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision",
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::Conflict("Archive is in maintenance.".into()))?;

    let current = read(&mut tx, template).await?;
    if current.version != layout.version {
        return Err(AppError::Conflict(
            "Dashboard layout changed in another session. Reload before saving.".into(),
        ));
    }
    let mut to_save = layout;
    to_save.version = current.version + 1;
    let serialized = serde_json::to_string(&to_save).map_err(|_| AppError::Internal)?;
    db::set_setting(&mut tx, template.setting_key(), &serialized).await?;
    tx.commit().await?;
    Ok(Json(to_save))
}
