//! Ask: natural-language questions answered by deterministic Rust over the
//! canonical listening selection. The model (later) only produces DSL.
pub mod dsl;
pub mod execute;
pub mod grammar;
pub mod grouping;
pub mod names;
pub mod parser;
pub mod projection;
pub mod prompt;
pub mod remote;
pub mod runtime;
pub mod sandbox;
pub mod schema;
pub mod streaks;
pub mod time;

use crate::{
    AppState, ai,
    ask::{
        dsl::{Chip, Entity, EntityKind, Metric, Query},
        parser::{ParsedQuestion, PlanStep, Previous, QueryParser, StubParser, UnavailableParser},
        sandbox::{SandboxLimits, SandboxRule, SqlValue},
        schema::{ColumnFormat, SqlAnswerKind, SqlColumnSpec, SqlModelOutput},
    },
    auth,
    error::{AppError, Result},
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
pub struct SqlColumn {
    pub name: String,
    pub format: ColumnFormat,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SqlAnswer {
    pub title: String,
    pub computed: String,
    pub answer_kind: SqlAnswerKind,
    pub sql: String,
    pub columns: Vec<SqlColumn>,
    pub rows: Vec<Vec<SqlValue>>,
    pub truncated: bool,
    pub rule_code: Option<SandboxRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub as_of_ms: Option<i64>,
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
    /// Steps that ran before this one when the question was chained, oldest
    /// first. This response is always the last step that ran.
    pub earlier: Vec<AskStep>,
    /// Free-question (SQL mode) answer or rejection details, present only in SQL mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sql: Option<SqlAnswer>,
}

/// An answered step of a chained question.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AskStep {
    pub query: Option<Query>,
    pub chips: Vec<Chip>,
    pub result: Option<AskResult>,
}

impl From<AskResponse> for AskStep {
    fn from(response: AskResponse) -> Self {
        Self {
            query: response.query,
            chips: response.chips,
            result: response.result,
        }
    }
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
    /// The answer this question follows up on, when the owner continues from it.
    #[serde(default)]
    pub previous: Option<AskPrevious>,
}

/// The previous question and its interpretation chips. Results are never sent
/// to the model.
#[derive(Clone, Debug, Deserialize, TS)]
pub struct AskPrevious {
    pub question: String,
    pub chips: Vec<Chip>,
}

impl AskPrevious {
    /// Bounded copy for the prompt: a follow-up cannot be used to smuggle a
    /// large payload to the AI server.
    fn sanitized(&self) -> Previous {
        let clip = |text: &str, max: usize| text.chars().take(max).collect::<String>();
        Previous {
            question: clip(self.question.trim(), 500),
            interpretation: self
                .chips
                .iter()
                .take(20)
                .map(|chip| (clip(&chip.label, 40), clip(&chip.value, 200)))
                .collect(),
        }
    }
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
    pub projection: projection::ProjectionGate,
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
            projection: projection::ProjectionGate::default(),
        }
    }

    /// Installed at `AppState::new`: the real runtime with the `llama` feature,
    /// or the unavailable parser in a `--no-default-features` build. No model
    /// is loaded here.
    pub fn installed(db: sqlx::SqlitePool, data_dir: PathBuf) -> Self {
        Self {
            parser: Arc::new(parser::ModelParser::local(db, data_dir)),
            // Without the `llama` feature only a remote AI server can answer;
            // the gate checks the provider before this flag.
            runtime_available: cfg!(feature = "llama"),
            model_ready: true,
            local: true,
            projection: projection::ProjectionGate::default(),
        }
    }

    pub fn stub() -> Self {
        Self {
            parser: Arc::new(StubParser),
            runtime_available: true,
            model_ready: true,
            local: false,
            projection: projection::ProjectionGate::default(),
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
            projection: projection::ProjectionGate::default(),
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

    pub async fn parse_follow_up(
        &self,
        question: &str,
        previous: Option<&Previous>,
    ) -> Result<ParsedQuestion> {
        self.parser.parse_follow_up(question, previous).await
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
            earlier: Vec::new(),
            sql: None,
        }));
    }
    let previous = request.previous.as_ref().map(AskPrevious::sanitized);
    let parsed = state
        .ask
        .parse_follow_up(&question, previous.as_ref())
        .await?;
    let query = match parsed {
        ParsedQuestion::Query(query) => query,
        ParsedQuestion::Plan(steps) => {
            return Ok(Json(
                run_plan(&state, question, steps, request.context).await?,
            ));
        }
        ParsedQuestion::Sql(output) => {
            return Ok(Json(run_sql(&state, question, output).await?));
        }
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
                earlier: Vec::new(),
                sql: None,
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

/// Run a chained question step by step. A step that builds on an earlier one
/// takes that step's row from the executed result, never from the model.
async fn run_plan(
    state: &AppState,
    question: String,
    steps: Vec<PlanStep>,
    context: Option<AskContext>,
) -> Result<AskResponse> {
    let total = steps.len();
    let mut done: Vec<AskResponse> = Vec::new();
    for (index, step) in steps.into_iter().enumerate() {
        let mut query = step.query;
        if let Some(bind) = step.bind
            && let Err(message) = bind_step(&mut query, done.get(bind.step), bind.step, bind.row)
        {
            return Ok(AskResponse {
                status: AskStatus::Failed,
                question,
                chips: query.chips(),
                query: Some(query),
                result: None,
                message: Some(message),
                candidates: Vec::new(),
                runtime_available: state.ask.runtime_available,
                earlier: done.into_iter().map(AskStep::from).collect(),
                sql: None,
            });
        }
        let page = if index == 0 { context.clone() } else { None };
        let mut response = run(state, question.clone(), query, page).await?;
        if response.status != AskStatus::Ok || index + 1 == total {
            response.earlier = done.into_iter().map(AskStep::from).collect();
            return Ok(response);
        }
        done.push(response);
    }
    Err(crate::error::AppError::Internal)
}

/// Fill a step's entity or dates from one row of an earlier step's result.
fn bind_step(
    query: &mut Query,
    earlier: Option<&AskResponse>,
    step: usize,
    row: usize,
) -> std::result::Result<(), String> {
    let number = step + 1;
    let (Some(source), Some(result)) = (
        earlier.and_then(|response| response.query.as_ref()),
        earlier.and_then(|response| response.result.as_ref()),
    ) else {
        return Err(format!(
            "Step {number} did not produce a result to build on."
        ));
    };
    let Some(group) = source.group_by else {
        return Err(format!(
            "Step {number} did not group its answer, so there is no row to build on."
        ));
    };
    let Some(found) = result.rows.get(row) else {
        return Err(format!("Step {number} has no row {} to build on.", row + 1));
    };
    // A chained step carries forward what the earlier step filtered on and
    // this step left unset ("that month" still means that artist's month).
    if query.entity.is_none() && query.scope.is_none() {
        query.entity = source.entity.clone();
    }
    if query.period == dsl::Period::AllTime
        && query.year.is_none()
        && query.from.is_none()
        && query.to.is_none()
    {
        query.period = source.period;
        query.year = source.year;
        query.from = source.from.clone();
        query.to = source.to.clone();
    }
    query.month = query.month.or(source.month);
    query.weekday = query.weekday.or(source.weekday);
    query.hour = query.hour.or(source.hour);
    query.source = query.source.clone().or_else(|| source.source.clone());
    use dsl::GroupBy;
    match group {
        GroupBy::Artist | GroupBy::Album | GroupBy::Track => {
            if found.key == "unknown-album" {
                return Err(format!("Row {} of step {number} has no album.", row + 1));
            }
            query.entity = Some(Entity {
                kind: match group {
                    GroupBy::Artist => EntityKind::Artist,
                    GroupBy::Album => EntityKind::Album,
                    _ => EntityKind::Track,
                },
                name: found.label.clone(),
                id: Some(found.key.clone()),
            });
            query.scope = None;
        }
        GroupBy::Day | GroupBy::Month | GroupBy::Year | GroupBy::Week => {
            let (from, to) = execute::bucket_range(group, &found.label)
                .ok_or_else(|| format!("Row {} of step {number} is not a date.", row + 1))?;
            query.from = Some(from);
            query.to = Some(to);
            query.period = dsl::Period::AllTime;
            query.year = None;
            query.month = None;
        }
        GroupBy::Weekday => {
            query.weekday = serde_json::from_value(serde_json::Value::String(found.key.clone()))
                .map(Some)
                .map_err(|_| format!("Row {} of step {number} is not a weekday.", row + 1))?;
        }
        GroupBy::Hour => {
            query.hour = found
                .label
                .get(..2)
                .and_then(|hour| hour.parse::<u8>().ok())
                .map(Some)
                .ok_or_else(|| format!("Row {} of step {number} is not an hour.", row + 1))?;
        }
    }
    Ok(())
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
    if stored.uses_remote() {
        // Your own AI server: nothing to load or install locally.
        if !stored.remote_configured() {
            return Ok(Some(refusal(
                state,
                AskStatus::ModelMissing,
                "Enter the address and model name of your AI server in Settings before asking questions.",
            )));
        }
        return Ok(None);
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
        earlier: Vec::new(),
        sql: None,
    }
}

fn match_columns(actual: &[String], declared: &[SqlColumnSpec]) -> Vec<SqlColumn> {
    actual
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let format = declared
                .iter()
                .find(|spec| spec.name.eq_ignore_ascii_case(name))
                .or_else(|| {
                    (declared.len() == actual.len())
                        .then(|| declared.get(index))
                        .flatten()
                })
                .map(|spec| spec.format)
                .unwrap_or(ColumnFormat::Text);
            SqlColumn {
                name: name.clone(),
                format,
            }
        })
        .collect()
}

async fn run_sql(
    state: &AppState,
    question: String,
    output: SqlModelOutput,
) -> Result<AskResponse> {
    let proj_path = projection::for_query(&state.storage, &state.ask.projection).await?;
    let sql_text = output.sql.clone();
    let (as_of_ms, outcome) = {
        let _read = state.ask.projection.read_lock().await;
        tokio::task::spawn_blocking(move || {
            let as_of_ms = projection::read_built_at_ms(&proj_path);
            let outcome = sandbox::execute(&proj_path, &sql_text, SandboxLimits::default());
            (as_of_ms, outcome)
        })
        .await
        .map_err(|_| AppError::Internal)?
    };
    match outcome {
        Ok(table) => {
            let columns = match_columns(&table.columns, &output.columns);
            Ok(AskResponse {
                status: AskStatus::Ok,
                question,
                query: None,
                chips: Vec::new(),
                result: None,
                message: None,
                candidates: Vec::new(),
                runtime_available: state.ask.runtime_available,
                earlier: Vec::new(),
                sql: Some(SqlAnswer {
                    title: output.title,
                    computed: output.computed,
                    answer_kind: output.answer_kind,
                    sql: output.sql,
                    columns,
                    rows: table.rows,
                    truncated: table.truncated,
                    rule_code: None,
                    as_of_ms,
                }),
            })
        }
        Err(rejection) => Ok(AskResponse {
            status: AskStatus::Failed,
            question,
            query: None,
            chips: Vec::new(),
            result: None,
            message: Some(rejection.message),
            candidates: Vec::new(),
            runtime_available: state.ask.runtime_available,
            earlier: Vec::new(),
            sql: Some(SqlAnswer {
                title: output.title,
                computed: output.computed,
                answer_kind: output.answer_kind,
                sql: output.sql,
                columns: Vec::new(),
                rows: Vec::new(),
                truncated: false,
                rule_code: Some(rejection.rule),
                as_of_ms,
            }),
        }),
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
        earlier: Vec::new(),
        sql: None,
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
                    earlier: Vec::new(),
                    sql: None,
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
                        earlier: Vec::new(),
                        sql: None,
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
            earlier: Vec::new(),
            sql: None,
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
            earlier: Vec::new(),
            sql: None,
        }),
    }
}
