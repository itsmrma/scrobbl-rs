//! Ask: natural-language questions answered by deterministic Rust over the
//! canonical listening selection. The model (later) only produces DSL.
pub mod dsl;
pub mod execute;
pub mod grammar;
pub mod grouping;
pub mod names;
pub mod parser;
pub mod prompt;
pub mod runtime;
pub mod schema;
pub mod streaks;
pub mod time;

use crate::{
    AppState, ai,
    ask::{
        dsl::{Chip, Entity, EntityKind, Metric, Query},
        parser::{ParsedQuestion, QueryParser, StubParser, UnavailableParser},
    },
    auth,
    error::Result,
};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
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

/// The Ask service seam. `AppState::new` installs the local model parser when
/// the `llama` feature is compiled in; tests install `Ask::stub()` and a
/// feature-off build keeps `Ask::unavailable()`.
#[derive(Clone)]
pub struct Ask {
    parser: Arc<dyn QueryParser>,
    pub runtime_available: bool,
    /// Whether the parser can attempt a question. For the local parser this is
    /// true and the gate checks the selected installed model separately.
    pub model_ready: bool,
    /// True when this Ask is backed by the real runtime rather than a stub.
    pub local: bool,
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
            local: false,
        }
    }

    /// Installed at `AppState::new`: the real runtime with the `llama` feature,
    /// or the unavailable parser in a `--no-default-features` build. No model
    /// is loaded here.
    pub fn installed(db: sqlx::SqlitePool, data_dir: PathBuf) -> Self {
        #[cfg(feature = "llama")]
        {
            Self {
                parser: Arc::new(parser::ModelParser::local(db, data_dir)),
                runtime_available: true,
                model_ready: true,
                local: true,
            }
        }
        #[cfg(not(feature = "llama"))]
        {
            let _ = (db, data_dir);
            Self::unavailable()
        }
    }

    pub fn stub() -> Self {
        Self {
            parser: Arc::new(StubParser),
            runtime_available: true,
            model_ready: true,
            local: false,
        }
    }

    pub fn with_parser(
        parser: Arc<dyn QueryParser>,
        runtime_available: bool,
        model_ready: bool,
        local: bool,
    ) -> Self {
        Self {
            parser,
            runtime_available,
            model_ready,
            local,
        }
    }

    pub fn available(&self) -> bool {
        self.runtime_available && self.model_ready && !self.model_loading()
    }

    /// Live model-loading state from the worker. Stubs never load a model.
    pub fn model_loading(&self) -> bool {
        self.parser.model_loading()
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
    if question.is_empty() || !question.chars().any(char::is_alphanumeric) {
        return Ok(Json(AskResponse {
            status: AskStatus::Unsupported,
            question,
            query: None,
            chips: Vec::new(),
            result: None,
            message: Some("Please write a question about your music archive.".into()),
            candidates: Vec::new(),
            runtime_available: state.ask.runtime_available,
        }));
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
    let stored = ai::read_settings(state).await?;
    if !stored.enabled {
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
    if state.ask.model_loading() {
        return Ok(Some(refusal(
            state,
            AskStatus::ModelUnavailable,
            "The AI model is loading. Try again in a moment.",
        )));
    }
    if state.ask.local {
        let readiness = ai::selected_model_readiness(state, &stored).await?;
        if !(readiness.installed && readiness.selected_available) {
            return Ok(Some(refusal(
                state,
                AskStatus::ModelMissing,
                "Download and select an AI model in Settings before asking questions.",
            )));
        }
    } else if !state.ask.model_ready {
        return Ok(Some(refusal(
            state,
            AskStatus::ModelMissing,
            "Download and select an AI model in Settings before asking questions.",
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

async fn run(
    state: &AppState,
    question: String,
    mut query: Query,
    context: Option<AskContext>,
) -> Result<AskResponse> {
    query.validate()?;
    let mut view = state.storage.read_view().await?;
    let conn = &mut *view.transaction;

    let failed = |query: &Query, message: String| AskResponse {
        status: AskStatus::Failed,
        question: question.clone(),
        query: Some(query.clone()),
        chips: query.chips(),
        result: None,
        message: Some(message),
        candidates: Vec::new(),
        runtime_available: state.ask.runtime_available,
    };

    // The page context is what the owner is looking at. It answers questions
    // that name nothing ("how many times did I play this?"), and narrows a
    // named track or album to the artist/album page it was asked from. It
    // never replaces something the question names explicitly.
    let mut soft_scope: Option<(EntityKind, String)> = None;
    if let Some(context) = context {
        let Some((context_id, context_name)) =
            names::resolve_id(conn, context.kind, &context.id).await?
        else {
            return Ok(failed(
                &query,
                "That context entity is not in your library.".into(),
            ));
        };
        let named = query
            .entity
            .as_ref()
            .filter(|entity| !entity.name.trim().is_empty() || entity.id.is_some());
        match named {
            None => {
                query.entity = Some(Entity {
                    kind: context.kind,
                    name: context_name,
                    id: Some(context_id),
                });
            }
            Some(entity)
                if entity.id.is_none()
                    && entity.kind == context.kind
                    && names::fold(&entity.name) == names::fold(&context_name) =>
            {
                query.entity = Some(Entity {
                    kind: context.kind,
                    name: context_name,
                    id: Some(context_id),
                });
            }
            Some(entity)
                if entity.id.is_none()
                    && query.scope.is_none()
                    && matches!(
                        (entity.kind, context.kind),
                        (EntityKind::Track | EntityKind::Album, EntityKind::Artist)
                            | (EntityKind::Track, EntityKind::Album)
                    ) =>
            {
                soft_scope = Some((context.kind, context_id));
            }
            Some(_) => {}
        }
    }

    // An explicit scope ("Show by Ado") must resolve before the entity.
    let mut hard_scope: Option<(EntityKind, String)> = None;
    if let Some(scope) = query.scope.clone() {
        let resolved = match scope.id.as_deref().filter(|id| !id.is_empty()) {
            Some(id) => names::resolve_id(conn, scope.kind, id)
                .await?
                .map(|(id, name)| names::Resolution::Unique { id, name })
                .unwrap_or(names::Resolution::NotFound),
            None => names::resolve(conn, scope.kind, &scope.name).await?,
        };
        match resolved {
            names::Resolution::Unique { id, name } => {
                query.scope = Some(Entity {
                    kind: scope.kind,
                    name,
                    id: Some(id.clone()),
                });
                hard_scope = Some((scope.kind, id));
            }
            names::Resolution::Ambiguous(candidates) => {
                // Keep the question answerable: a chosen candidate becomes the
                // scope through the chips, exactly like an ambiguous entity.
                return Ok(AskResponse {
                    status: AskStatus::Ambiguous,
                    question,
                    query: Some(query.clone()),
                    chips: query.chips(),
                    result: None,
                    message: Some(
                        "I found more than one artist with that name. Choose the one you meant."
                            .into(),
                    ),
                    candidates,
                    runtime_available: state.ask.runtime_available,
                });
            }
            names::Resolution::NotFound => {
                return Ok(failed(
                    &query,
                    format!(
                        "I couldn't find {} “{}” in your library.",
                        scope.kind.label().to_lowercase(),
                        scope.name.trim()
                    ),
                ));
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
                    return Ok(failed(&query, "That entity is not in your library.".into()));
                }
            }
        } else {
            let mut resolution = match hard_scope.as_ref().or(soft_scope.as_ref()) {
                Some((kind, id)) => {
                    names::resolve_in(conn, entity.kind, &entity.name, Some((*kind, id))).await?
                }
                None => names::resolve(conn, entity.kind, &entity.name).await?,
            };
            // The page is only a hint: a song by another artist is still found.
            if resolution == names::Resolution::NotFound
                && hard_scope.is_none()
                && soft_scope.is_some()
            {
                resolution = names::resolve(conn, entity.kind, &entity.name).await?;
            }
            match resolution {
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
                    let scope = query
                        .scope
                        .as_ref()
                        .map(|scope| format!(" by {}", scope.name.trim()))
                        .unwrap_or_default();
                    return Ok(failed(
                        &query,
                        format!(
                            "I couldn't find {} “{}”{scope} in your library.",
                            entity.kind.label().to_lowercase(),
                            entity.name.trim()
                        ),
                    ));
                }
            }
        }
    } else if let Some(scope) = query.scope.take() {
        // A scope without a named entity is the subject itself.
        query.entity = Some(scope);
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
