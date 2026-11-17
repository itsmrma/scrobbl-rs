//! Ask: natural-language questions answered by deterministic Rust over the
//! canonical listening selection. The model (later) only produces DSL.
pub mod dsl;
pub mod execute;
pub mod fixture;
pub mod grammar;
pub mod grouping;
pub mod history_link;
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
        runtime::CancelToken,
        sandbox::{SandboxCancel, SandboxLimits, SandboxRule, SqlValue},
        schema::{ColumnFormat, SqlAnswerKind, SqlColumnSpec, SqlModelOutput},
    },
    auth,
    error::{AppError, Result},
};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, Semaphore};
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
    #[serde(default)]
    pub cell_links: Vec<Vec<Option<String>>>,
    #[serde(default)]
    pub history_href: Option<String>,
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
    /// Answer mode for this request only, overriding the stored per-engine
    /// mode. Never persisted; an unknown value is rejected with 400.
    #[serde(default)]
    #[ts(optional, type = "\"guided\" | \"sql\"")]
    pub mode: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, TS)]
pub struct AskCancelRequest {
    #[serde(default)]
    #[ts(optional)]
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AskCancelResponse {
    pub cancelled: bool,
}

/// The previous SQL-mode answer for a follow-up question. Results are never
/// sent to the model.
#[derive(Clone, Debug, Deserialize, TS)]
pub struct SqlPrevious {
    pub title: String,
    pub computed: String,
    pub sql: String,
}

/// The previous question and its interpretation chips or SQL summary. Results
/// are never sent to the model.
#[derive(Clone, Debug, Deserialize, TS)]
pub struct AskPrevious {
    pub question: String,
    #[serde(default)]
    pub chips: Vec<Chip>,
    #[serde(default)]
    #[ts(optional)]
    pub sql: Option<SqlPrevious>,
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
            sql: self
                .sql
                .as_ref()
                .filter(|prev| !prev.sql.trim().is_empty())
                .map(|prev| parser::SqlPrevious {
                    title: clip(prev.title.trim(), 200),
                    computed: clip(prev.computed.trim(), 500),
                    sql: clip(prev.sql.trim(), sandbox::MAX_SQL_BYTES),
                }),
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

#[derive(Clone, Debug, Deserialize, TS)]
pub struct AskSqlRequest {
    pub sql: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub computed: Option<String>,
    #[serde(default)]
    pub answer_kind: Option<SqlAnswerKind>,
    #[serde(default)]
    pub columns: Vec<SqlColumn>,
}

#[derive(Clone)]
pub struct AskCancel {
    pub token: CancelToken,
    pub sandbox: SandboxCancel,
    notify: Arc<Notify>,
}

impl AskCancel {
    fn new(timeout: Duration) -> Self {
        Self {
            token: CancelToken::new(timeout),
            sandbox: SandboxCancel::new(),
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn cancel(&self) {
        self.token.cancel();
        self.sandbox.cancel();
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled() || self.sandbox.is_cancelled()
    }

    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

struct InFlightEntry {
    id: String,
    cancel: AskCancel,
}

#[derive(Default)]
struct InFlightAsks {
    entries: Mutex<Vec<InFlightEntry>>,
    next_id: AtomicU64,
}

impl InFlightAsks {
    fn register(&self, request_id: Option<&str>, timeout: Duration) -> (String, AskCancel) {
        let id = request_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(128).collect::<String>())
            .unwrap_or_else(|| format!("ask-{}", self.next_id.fetch_add(1, Ordering::Relaxed)));
        let cancel = AskCancel::new(timeout);
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        entries.push(InFlightEntry {
            id: id.clone(),
            cancel: cancel.clone(),
        });
        (id, cancel)
    }

    fn remove(&self, id: &str) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(pos) = entries.iter().position(|entry| entry.id == id) {
            entries.remove(pos);
        }
    }

    fn cancel(&self, request_id: Option<&str>) -> bool {
        let targets: Vec<AskCancel> = {
            let entries = self
                .entries
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            entries
                .iter()
                .filter(
                    |entry| match request_id.map(str::trim).filter(|s| !s.is_empty()) {
                        Some(id) => entry.id == id,
                        None => true,
                    },
                )
                .filter(|entry| !entry.cancel.is_cancelled())
                .map(|entry| entry.cancel.clone())
                .collect()
        };
        let any = !targets.is_empty();
        for target in targets {
            target.cancel();
        }
        any
    }

    fn count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .len()
    }
}

struct InFlightGuard {
    registry: Arc<InFlightAsks>,
    id: String,
    cancel: AskCancel,
    finished: bool,
}

impl InFlightGuard {
    fn finish(mut self) {
        self.finished = true;
        self.registry.remove(&self.id);
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.cancel.cancel();
            self.registry.remove(&self.id);
        }
    }
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
    pub sql_limits: SandboxLimits,
    in_flight: Arc<InFlightAsks>,
}

fn sql_limits_from_env() -> SandboxLimits {
    let mut limits = SandboxLimits::default();
    if let Some(ms) = std::env::var("SCROBBL_ASK_SQL_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
    {
        limits.timeout = std::time::Duration::from_millis(ms);
    }
    limits
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
            sql_limits: sql_limits_from_env(),
            in_flight: Arc::new(InFlightAsks::default()),
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
            sql_limits: sql_limits_from_env(),
            in_flight: Arc::new(InFlightAsks::default()),
        }
    }

    pub fn stub() -> Self {
        Self {
            parser: Arc::new(StubParser),
            runtime_available: true,
            model_ready: true,
            local: false,
            projection: projection::ProjectionGate::default(),
            sql_limits: sql_limits_from_env(),
            in_flight: Arc::new(InFlightAsks::default()),
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
            sql_limits: sql_limits_from_env(),
            in_flight: Arc::new(InFlightAsks::default()),
        }
    }

    fn register_in_flight(
        &self,
        request_id: Option<&str>,
        timeout: Duration,
    ) -> (AskCancel, InFlightGuard) {
        let (id, cancel) = self.in_flight.register(request_id, timeout);
        let guard = InFlightGuard {
            registry: Arc::clone(&self.in_flight),
            id,
            cancel: cancel.clone(),
            finished: false,
        };
        (cancel, guard)
    }

    pub fn cancel_request(&self, request_id: Option<&str>) -> bool {
        self.in_flight.cancel(request_id)
    }

    pub fn active_count(&self) -> usize {
        self.in_flight.count()
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

    pub async fn parse_for_mode_with_cancel(
        &self,
        question: &str,
        previous: Option<&Previous>,
        mode: ai::AnswerMode,
        cancel: &CancelToken,
    ) -> Result<ParsedQuestion> {
        self.parser
            .parse_for_mode_with_cancel(question, previous, mode, Some(cancel))
            .await
    }

    pub async fn repair_sql(
        &self,
        question: &str,
        previous: Option<&Previous>,
        attempts: &[(&str, &str)],
        cancel: &CancelToken,
    ) -> Result<Option<ParsedQuestion>> {
        self.parser
            .repair_sql(question, previous, attempts, Some(cancel))
            .await
    }
}

fn cancelled_response(
    state: &AppState,
    question: String,
    sql_output: Option<SqlModelOutput>,
) -> AskResponse {
    AskResponse {
        status: AskStatus::Failed,
        question,
        query: None,
        chips: Vec::new(),
        result: None,
        message: Some("The question was cancelled (rule `cancelled`).".into()),
        candidates: Vec::new(),
        runtime_available: state.ask.runtime_available,
        earlier: Vec::new(),
        sql: sql_output.map(|output| SqlAnswer {
            title: output.title,
            computed: output.computed,
            answer_kind: output.answer_kind,
            sql: output.sql,
            columns: Vec::new(),
            rows: Vec::new(),
            truncated: false,
            rule_code: Some(SandboxRule::Cancelled),
            as_of_ms: None,
            cell_links: Vec::new(),
            history_href: None,
        }),
    }
}

pub async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<AskCancelRequest>>,
) -> Result<Json<AskCancelResponse>> {
    auth::session(&state, &headers).await?;
    let request_id = body.as_ref().and_then(|b| b.request_id.as_deref());
    let cancelled = state.ask.cancel_request(request_id);
    Ok(Json(AskCancelResponse { cancelled }))
}

pub async fn ask(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AskRequest>,
) -> Result<Json<AskResponse>> {
    auth::session(&state, &headers).await?;
    let question = request.question.trim().to_owned();
    let requested_mode = match request.mode.as_deref() {
        None => None,
        Some("guided") => Some(ai::AnswerMode::Guided),
        Some("sql") => Some(ai::AnswerMode::Sql),
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "Unknown answer mode `{other}`. Use `guided` or `sql`."
            )));
        }
    };
    let stored = match gate(&state).await? {
        Ok(stored) => stored,
        Err(mut refusal) => {
            refusal.question = question;
            return Ok(Json(refusal));
        }
    };
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
    let (ask_cancel, guard) = state
        .ask
        .register_in_flight(request.request_id.as_deref(), parser::inference_timeout());

    let parsed = tokio::select! {
        biased;
        () = ask_cancel.cancelled() => {
            guard.finish();
            return Ok(Json(cancelled_response(&state, question, None)));
        }
        res = state.ask.parse_for_mode_with_cancel(
            &question,
            previous.as_ref(),
            requested_mode.unwrap_or_else(|| stored.effective_answer_mode()),
            &ask_cancel.token,
        ) => match res {
            Ok(parsed) => parsed,
            Err(_) if ask_cancel.is_cancelled() => {
                guard.finish();
                return Ok(Json(cancelled_response(&state, question, None)));
            }
            Err(err) => {
                guard.finish();
                return Err(err);
            }
        }
    };
    let response = match parsed {
        ParsedQuestion::Query(query) => run(&state, question, query, request.context).await?,
        ParsedQuestion::Plan(steps) => run_plan(&state, question, steps, request.context).await?,
        ParsedQuestion::Sql(output) => {
            run_sql(
                &state,
                question,
                previous.as_ref(),
                output,
                !stored.uses_remote(),
                &ask_cancel,
            )
            .await?
        }
        ParsedQuestion::Unsupported { reason } => AskResponse {
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
        },
    };
    guard.finish();
    Ok(Json(response))
}

pub async fn query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AskQueryRequest>,
) -> Result<Json<AskResponse>> {
    auth::session(&state, &headers).await?;
    if let Err(refusal) = gate(&state).await? {
        return Ok(Json(refusal));
    }
    Ok(Json(run(&state, String::new(), request.query, None).await?))
}

/// Execute owner-edited SQL directly through the Ask SQL sandbox.
pub async fn sql(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AskSqlRequest>,
) -> Result<Json<AskResponse>> {
    auth::session(&state, &headers).await?;
    let stored = ai::read_settings(&state).await?;
    if !stored.enabled {
        return Ok(Json(refusal(
            &state,
            AskStatus::ModelUnavailable,
            "AI is off. Enable it in Settings to ask questions.",
        )));
    }
    let answer_kind = match request.answer_kind {
        Some(SqlAnswerKind::Unsupported) | None => SqlAnswerKind::Table,
        Some(kind) => kind,
    };
    let output = SqlModelOutput {
        sql: request.sql,
        title: request.title.unwrap_or_default(),
        computed: request.computed.unwrap_or_default(),
        answer_kind,
        columns: request
            .columns
            .into_iter()
            .map(|col| SqlColumnSpec {
                name: col.name,
                format: col.format,
            })
            .collect(),
    };
    Ok(Json(
        run_sql_once(&state, String::new(), output, false, None).await?,
    ))
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

async fn gate(state: &AppState) -> Result<std::result::Result<ai::StoredSettings, AskResponse>> {
    let stored = ai::read_settings(state).await?;
    if !stored.enabled {
        return Ok(Err(refusal(
            state,
            AskStatus::ModelUnavailable,
            "AI is off. Enable it in Settings to ask questions.",
        )));
    }
    if stored.uses_remote() {
        // Your own AI server: nothing to load or install locally.
        if !stored.remote_configured() {
            return Ok(Err(refusal(
                state,
                AskStatus::ModelMissing,
                "Enter the address and model name of your AI server in Settings before asking questions.",
            )));
        }
        return Ok(Ok(stored));
    }
    if !state.ask.runtime_available {
        return Ok(Err(refusal(
            state,
            AskStatus::ModelUnavailable,
            "The AI runtime is not available in this build.",
        )));
    }
    if state.ask.model_loading() {
        return Ok(Err(refusal(
            state,
            AskStatus::ModelUnavailable,
            "The AI model is loading. Try again in a moment.",
        )));
    }
    if state.ask.local {
        let readiness = ai::selected_model_readiness(state, &stored).await?;
        if !(readiness.installed && readiness.selected_available) {
            return Ok(Err(refusal(
                state,
                AskStatus::ModelMissing,
                "Download and select an AI model in Settings before asking questions.",
            )));
        }
    } else if !state.ask.model_ready {
        return Ok(Err(refusal(
            state,
            AskStatus::ModelMissing,
            "Download and select an AI model in Settings before asking questions.",
        )));
    }
    Ok(Ok(stored))
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

pub const MAX_SQL_REPAIR_ROUNDS: usize = 2;

struct SqlExecution {
    as_of_ms: Option<i64>,
    table: sandbox::SandboxOutput,
    columns: Vec<SqlColumn>,
    cell_links: Vec<Vec<Option<String>>>,
    history_href: Option<String>,
}

enum SqlAttempt {
    Done(SqlExecution),
    Rejected {
        as_of_ms: Option<i64>,
        rejection: sandbox::SandboxRejection,
    },
    Cancelled,
}

/// One sandbox execution process-wide. Two projects can share an instance and
/// two questions can arrive at once, so the SQL sandbox itself is serialized:
/// the permit is taken before the blocking task starts and released after it
/// (including on cancellation) by the guard's drop.
static SANDBOX_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn sandbox_slots() -> &'static Arc<Semaphore> {
    SANDBOX_SLOTS.get_or_init(|| Arc::new(Semaphore::new(1)))
}

/// Wait for the single sandbox slot, honouring cancellation while waiting.
async fn acquire_sandbox_slot(
    ask_cancel: Option<&AskCancel>,
) -> Result<Option<tokio::sync::OwnedSemaphorePermit>> {
    let slot = Arc::clone(sandbox_slots());
    match ask_cancel {
        Some(cancel) => tokio::select! {
            biased;
            () = cancel.cancelled() => Ok(None),
            permit = slot.acquire_owned() => permit.map(Some).map_err(|_| AppError::Internal),
        },
        None => slot
            .acquire_owned()
            .await
            .map(Some)
            .map_err(|_| AppError::Internal),
    }
}

async fn execute_sql_once(
    state: &AppState,
    output: &SqlModelOutput,
    ask_cancel: Option<&AskCancel>,
) -> Result<SqlAttempt> {
    let proj_path = match ask_cancel {
        Some(cancel) => tokio::select! {
            biased;
            () = cancel.cancelled() => return Ok(SqlAttempt::Cancelled),
            res = projection::for_query(&state.storage, &state.ask.projection) => res?,
        },
        None => projection::for_query(&state.storage, &state.ask.projection).await?,
    };
    let sql_text = output.sql.clone();
    let declared_columns = output.columns.clone();
    let answer_kind = output.answer_kind;
    let limits = state.ask.sql_limits;
    let sandbox_cancel = ask_cancel.map(|c| c.sandbox.clone());
    let (as_of_ms, outcome) = {
        let Some(_slot) = acquire_sandbox_slot(ask_cancel).await? else {
            return Ok(SqlAttempt::Cancelled);
        };
        let _read = match ask_cancel {
            Some(cancel) => tokio::select! {
                biased;
                () = cancel.cancelled() => return Ok(SqlAttempt::Cancelled),
                guard = state.ask.projection.read_lock() => guard,
            },
            None => state.ask.projection.read_lock().await,
        };
        tokio::task::spawn_blocking(move || {
            let as_of_ms = projection::read_built_at_ms(&proj_path);
            let outcome = sandbox::execute_with_cancel(
                &proj_path,
                &sql_text,
                limits,
                sandbox_cancel.as_ref(),
            )
            .map(|table| {
                let columns = match_columns(&table.columns, &declared_columns);
                let cell_links =
                    history_link::resolve_cell_links(&proj_path, &columns, &table.rows);
                let history_href = history_link::resolve(&proj_path, &sql_text, answer_kind);
                (table, columns, cell_links, history_href)
            });
            (as_of_ms, outcome)
        })
        .await
        .map_err(|_| AppError::Internal)?
    };
    Ok(match outcome {
        Ok((table, columns, cell_links, history_href)) => SqlAttempt::Done(SqlExecution {
            as_of_ms,
            table,
            columns,
            cell_links,
            history_href,
        }),
        Err(rejection) => SqlAttempt::Rejected {
            as_of_ms,
            rejection,
        },
    })
}

fn sql_ok_response(
    state: &AppState,
    question: String,
    output: SqlModelOutput,
    exec: SqlExecution,
) -> AskResponse {
    AskResponse {
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
            columns: exec.columns,
            rows: exec.table.rows,
            truncated: exec.table.truncated,
            rule_code: None,
            as_of_ms: exec.as_of_ms,
            cell_links: exec.cell_links,
            history_href: exec.history_href,
        }),
    }
}

fn sql_failed_response(
    state: &AppState,
    question: String,
    output: SqlModelOutput,
    as_of_ms: Option<i64>,
    rule: SandboxRule,
    message: String,
) -> AskResponse {
    AskResponse {
        status: AskStatus::Failed,
        question,
        query: None,
        chips: Vec::new(),
        result: None,
        message: Some(message),
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
            rule_code: Some(rule),
            as_of_ms,
            cell_links: Vec::new(),
            history_href: None,
        }),
    }
}

async fn run_sql_once(
    state: &AppState,
    question: String,
    output: SqlModelOutput,
    local_engine: bool,
    ask_cancel: Option<&AskCancel>,
) -> Result<AskResponse> {
    match execute_sql_once(state, &output, ask_cancel).await? {
        SqlAttempt::Done(exec) => Ok(sql_ok_response(state, question, output, exec)),
        SqlAttempt::Cancelled => Ok(cancelled_response(state, question, Some(output))),
        SqlAttempt::Rejected {
            as_of_ms,
            rejection,
        } => {
            let message = if local_engine && rejection.rule.is_repairable() {
                format!(
                    "{} Local catalog models can struggle with SQL — try Guided questions in Settings → AI or a larger model.",
                    rejection.message
                )
            } else {
                rejection.message
            };
            Ok(sql_failed_response(
                state,
                question,
                output,
                as_of_ms,
                rejection.rule,
                message,
            ))
        }
    }
}

async fn run_sql(
    state: &AppState,
    question: String,
    previous: Option<&Previous>,
    mut output: SqlModelOutput,
    local_engine: bool,
    ask_cancel: &AskCancel,
) -> Result<AskResponse> {
    let mut failed_attempts: Vec<(String, String)> = Vec::new();
    for round in 0..=MAX_SQL_REPAIR_ROUNDS {
        match execute_sql_once(state, &output, Some(ask_cancel)).await? {
            SqlAttempt::Done(exec) => return Ok(sql_ok_response(state, question, output, exec)),
            SqlAttempt::Cancelled => return Ok(cancelled_response(state, question, Some(output))),
            SqlAttempt::Rejected {
                as_of_ms,
                rejection,
            } => {
                if rejection.rule == SandboxRule::Cancelled {
                    return Ok(cancelled_response(state, question, Some(output)));
                }
                let mut repair_note: Option<String> = None;
                if rejection.rule.is_repairable() && round < MAX_SQL_REPAIR_ROUNDS {
                    failed_attempts.push((output.sql.clone(), rejection.model_error.clone()));
                    let refs: Vec<(&str, &str)> = failed_attempts
                        .iter()
                        .map(|(sql, err)| (sql.as_str(), err.as_str()))
                        .collect();
                    let repaired = tokio::select! {
                        biased;
                        () = ask_cancel.cancelled() => return Ok(cancelled_response(state, question, Some(output))),
                        res = state.ask.repair_sql(&question, previous, &refs, &ask_cancel.token) => res,
                    };
                    match repaired {
                        Ok(Some(ParsedQuestion::Sql(next_output))) => {
                            output = next_output;
                            continue;
                        }
                        Ok(Some(ParsedQuestion::Unsupported { reason })) => {
                            return Ok(AskResponse {
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
                            });
                        }
                        Ok(Some(ParsedQuestion::Query(_) | ParsedQuestion::Plan(_))) | Ok(None) => {
                        }
                        Err(_) if ask_cancel.is_cancelled() => {
                            return Ok(cancelled_response(state, question, Some(output)));
                        }
                        Err(AppError::BadRequest(detail)) => {
                            repair_note = Some(detail);
                        }
                        Err(err) => return Err(err),
                    }
                }
                let local_hint = if local_engine && rejection.rule.is_repairable() {
                    " Local catalog models can struggle with SQL — try Guided questions in Settings → AI or a larger model."
                } else {
                    ""
                };
                let message = if let Some(note) = repair_note {
                    format!(
                        "Could not repair the SQL query ({note}) {}{local_hint}",
                        rejection.message
                    )
                } else if round > 0 && rejection.rule.is_repairable() {
                    format!(
                        "Could not produce a valid query after {MAX_SQL_REPAIR_ROUNDS} repair attempts. {}{local_hint}",
                        rejection.message
                    )
                } else {
                    format!("{}{local_hint}", rejection.message)
                };
                return Ok(sql_failed_response(
                    state,
                    question,
                    output,
                    as_of_ms,
                    rejection.rule,
                    message,
                ));
            }
        }
    }
    Err(AppError::Internal)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// Both checks share the process-wide semaphore, so they run in one test.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_sql_sandbox_runs_one_query_at_a_time() {
        // The slot is process-wide: whatever project asks, one permit exists.
        assert_eq!(sandbox_slots().available_permits(), 1);

        // Three executions that each hold the slot across a blocking task never
        // overlap, and every permit comes back.
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..3 {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            tasks.push(tokio::spawn(async move {
                let _slot = acquire_sandbox_slot(None)
                    .await
                    .unwrap()
                    .expect("an uncancelled request always gets the slot");
                tokio::task::spawn_blocking(move || {
                    let running = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(running, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    active.fetch_sub(1, Ordering::SeqCst);
                })
                .await
                .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1, "sandboxes overlapped");
        assert_eq!(sandbox_slots().available_permits(), 1);

        // A cancelled question waiting for the slot never runs SQL.
        let held = sandbox_slots().clone().acquire_owned().await.unwrap();
        let cancel = AskCancel::new(Duration::from_secs(30));
        let waiter = cancel.clone();
        let handle = tokio::spawn(async move { acquire_sandbox_slot(Some(&waiter)).await });
        tokio::task::yield_now().await;
        cancel.cancel();
        let slot = handle.await.unwrap().unwrap();
        assert!(slot.is_none(), "a cancelled ask must not run SQL");
        drop(held);
        assert_eq!(sandbox_slots().available_permits(), 1);
    }
}
