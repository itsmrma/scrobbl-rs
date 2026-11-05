//! Ask: natural-language questions answered by deterministic Rust over the
//! canonical listening selection. The model (later) only produces DSL.
pub mod dsl;
pub mod execute;
pub mod names;
pub mod parser;

use crate::{
    AppState,
    ask::{
        dsl::{Chip, Entity, EntityKind, Metric, Query},
        parser::{ParsedQuestion, QueryParser, StubParser, UnavailableParser},
    },
    auth,
    error::Result,
};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AskStatus {
    Ok,
    Unsupported,
    Ambiguous,
    ModelUnavailable,
    ModelMissing,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AskResponse {
    pub status: AskStatus,
    pub question: String,
    pub query: Option<Query>,
    pub chips: Vec<Chip>,
    pub result: Option<AskResult>,
    pub message: Option<String>,
    pub candidates: Vec<EntityCandidate>,
    pub runtime_available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AskResult {
    pub metric: Metric,
    /// Count, or timestamp in milliseconds for first/last.
    pub scalar: Option<i64>,
    pub time: Option<TimeCoverage>,
    pub rows: Vec<ResultRow>,
    pub first: Option<ListenPoint>,
    pub last: Option<ListenPoint>,
    pub streak: Option<StreakResult>,
    pub milestones: Vec<MilestonePoint>,
    /// Row/result cardinality used by the deterministic card.
    pub total: i64,
    pub links: Vec<ResultLink>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct TimeCoverage {
    pub known_ms: i64,
    pub actual_ms: i64,
    pub estimated_ms: i64,
    pub unknown_listens: i64,
    pub unknown_percent: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ResultRow {
    pub label: String,
    pub key: String,
    pub plays: i64,
    pub time: Option<TimeCoverage>,
    pub href: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ListenPoint {
    pub played_at_ms: i64,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub track_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct StreakResult {
    pub current: StreakSpan,
    pub longest: StreakSpan,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct StreakSpan {
    pub days: i64,
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct MilestonePoint {
    pub count: i64,
    pub reached_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ResultLink {
    pub label: String,
    pub href: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EntityCandidate {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub kind: EntityKind,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct AskRequest {
    pub question: String,
    #[serde(default)]
    pub context: Option<AskContext>,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct AskContext {
    pub kind: EntityKind,
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct AskQueryRequest {
    pub query: Query,
}

/// The Ask service seam. `AppState::new` installs the unavailable parser; tests
/// and the current development page install `Ask::stub()`. #178 can replace the
/// parser with the model-backed adapter without touching the HTTP handlers.
#[derive(Clone)]
pub struct Ask {
    parser: Arc<dyn QueryParser>,
    pub runtime_available: bool,
    pub model_ready: bool,
    pub model_loading: bool,
}

impl Default for Ask {
    fn default() -> Self {
        Self::unavailable()
    }
}

impl Ask {
    pub fn unavailable() -> Self {
        Self {
            parser: Arc::new(UnavailableParser),
            runtime_available: false,
            model_ready: false,
            model_loading: false,
        }
    }

    pub fn stub() -> Self {
        Self {
            parser: Arc::new(StubParser),
            runtime_available: true,
            model_ready: true,
            model_loading: false,
        }
    }

    pub fn available(&self) -> bool {
        self.runtime_available && self.model_ready && !self.model_loading
    }

    pub async fn parse(&self, question: &str) -> Result<ParsedQuestion> {
        self.parser.parse(question).await
    }
}

pub async fn ask(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AskRequest>,
) -> Result<Json<AskResponse>> {
    auth::session(&state, &headers).await?;
    let question = request.question.trim().to_owned();
    if let Some(mut refusal) = gate(&state).await? {
        refusal.question = question;
        return Ok(Json(refusal));
    }
    let parsed = state.ask.parse(&question).await?;
    let query = match parsed {
        ParsedQuestion::Query(query) => query,
        ParsedQuestion::Unsupported { reason } => {
            return Ok(Json(AskResponse {
                status: AskStatus::Unsupported,
                question,
                query: None,
                chips: Vec::new(),
                result: None,
                message: Some(reason),
                candidates: Vec::new(),
                runtime_available: state.ask.runtime_available,
            }));
        }
    };
    Ok(Json(run(&state, question, query, request.context).await?))
}

pub async fn query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AskQueryRequest>,
) -> Result<Json<AskResponse>> {
    auth::session(&state, &headers).await?;
    if let Some(refusal) = gate(&state).await? {
        return Ok(Json(refusal));
    }
    Ok(Json(run(&state, String::new(), request.query, None).await?))
}

async fn gate(state: &AppState) -> Result<Option<AskResponse>> {
    if !enabled(state).await? {
        return Ok(Some(refusal(
            state,
            AskStatus::ModelUnavailable,
            "AI is off. Enable it in Settings to ask questions.",
        )));
    }
    if !state.ask.runtime_available {
        return Ok(Some(refusal(
            state,
            AskStatus::ModelUnavailable,
            "The AI runtime is not available in this build.",
        )));
    }
    if !state.ask.model_ready {
        return Ok(Some(refusal(
            state,
            AskStatus::ModelMissing,
            "Download and select an AI model in Settings before asking questions.",
        )));
    }
    if state.ask.model_loading {
        return Ok(Some(refusal(
            state,
            AskStatus::ModelUnavailable,
            "The AI model is loading. Try again in a moment.",
        )));
    }
    Ok(None)
}

fn refusal(state: &AppState, status: AskStatus, message: &str) -> AskResponse {
    AskResponse {
        status,
        question: String::new(),
        query: None,
        chips: Vec::new(),
        result: None,
        message: Some(message.into()),
        candidates: Vec::new(),
        runtime_available: state.ask.runtime_available,
    }
}

async fn enabled(state: &AppState) -> Result<bool> {
    Ok(crate::db::setting(&state.db, "ai_enabled")
        .await?
        .as_deref()
        == Some("1"))
}

async fn run(
    state: &AppState,
    question: String,
    mut query: Query,
    context: Option<AskContext>,
) -> Result<AskResponse> {
    query.validate()?;
    let mut view = state.storage.read_view().await?;
    let conn = &mut *view.transaction;

    if let Some(context) = context {
        match names::resolve_id(conn, context.kind, &context.id).await? {
            Some((id, name)) => {
                query.entity = Some(Entity {
                    kind: context.kind,
                    name,
                    id: Some(id),
                });
            }
            None => {
                return Ok(AskResponse {
                    status: AskStatus::Failed,
                    question,
                    query: Some(query.clone()),
                    chips: query.chips(),
                    result: None,
                    message: Some("That context entity is not in your library.".into()),
                    candidates: Vec::new(),
                    runtime_available: state.ask.runtime_available,
                });
            }
        }
    }

    if let Some(entity) = query.entity.clone() {
        if let Some(id) = entity.id.as_deref().filter(|id| !id.is_empty()) {
            match names::resolve_id(conn, entity.kind, id).await? {
                Some((id, name)) => {
                    query.entity = Some(Entity {
                        kind: entity.kind,
                        name,
                        id: Some(id),
                    });
                }
                None => {
                    return Ok(AskResponse {
                        status: AskStatus::Failed,
                        question,
                        query: Some(query.clone()),
                        chips: query.chips(),
                        result: None,
                        message: Some("That entity is not in your library.".into()),
                        candidates: Vec::new(),
                        runtime_available: state.ask.runtime_available,
                    });
                }
            }
        } else {
            match names::resolve(conn, entity.kind, &entity.name).await? {
                names::Resolution::Unique { id, name } => {
                    query.entity = Some(Entity {
                        kind: entity.kind,
                        name,
                        id: Some(id),
                    });
                }
                names::Resolution::Ambiguous(candidates) => {
                    return Ok(AskResponse {
                        status: AskStatus::Ambiguous,
                        question,
                        query: Some(query.clone()),
                        chips: query.chips(),
                        result: None,
                        message: Some(
                            "I found more than one match. Choose the one you meant.".into(),
                        ),
                        candidates,
                        runtime_available: state.ask.runtime_available,
                    });
                }
                names::Resolution::NotFound => {
                    return Ok(AskResponse {
                        status: AskStatus::Failed,
                        question,
                        query: Some(query.clone()),
                        chips: query.chips(),
                        result: None,
                        message: Some(format!(
                            "I couldn't find {} “{}” in your library.",
                            entity.kind.label().to_lowercase(),
                            entity.name.trim()
                        )),
                        candidates: Vec::new(),
                        runtime_available: state.ask.runtime_available,
                    });
                }
            }
        }
    }

    let chips = query.chips();
    match execute::execute(conn, &query, auth::now_ms()).await? {
        execute::Outcome::Unsupported(message) => Ok(AskResponse {
            status: AskStatus::Unsupported,
            question,
            query: Some(query),
            chips,
            result: None,
            message: Some(message),
            candidates: Vec::new(),
            runtime_available: state.ask.runtime_available,
        }),
        execute::Outcome::Result(result) => Ok(AskResponse {
            status: AskStatus::Ok,
            question,
            query: Some(query),
            chips,
            result: Some(*result),
            message: None,
            candidates: Vec::new(),
            runtime_available: state.ask.runtime_available,
        }),
    }
}
