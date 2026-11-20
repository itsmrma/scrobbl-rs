//! Ask: natural-language music questions answered by local GGUF models
//! generating read-only SQLite queries executed in the Ask SQL sandbox.
pub mod fixture;
pub mod grammar;
pub mod history_link;
pub mod names;
pub mod parser;
pub mod projection;
pub mod prompt;
pub mod runtime;
pub mod sandbox;
pub mod schema;
pub mod training;

use crate::{
    AppState, ai,
    ask::{
        parser::{ParsedQuestion, Previous, QueryParser, StubParser, UnavailableParser},
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

const MAX_IN_FLIGHT_ASKS: usize = 16;
pub const MAX_QUESTION_CHARS: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AskStatus {
    Ok,
    Unsupported,
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
    pub message: Option<String>,
    pub runtime_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sql: Option<SqlAnswer>,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct AskRequest {
    pub question: String,
    /// The answer this question follows up on, when the owner continues from it.
    #[serde(default)]
    pub previous: Option<AskPrevious>,
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

/// The previous SQL answer summary for a follow-up question. Results are
/// never sent to the model.
#[derive(Clone, Debug, Deserialize, TS)]
pub struct SqlPrevious {
    pub title: String,
    pub computed: String,
    pub sql: String,
}

/// The previous question and its SQL summary. Results are never sent to the
/// model.
#[derive(Clone, Debug, Deserialize, TS)]
pub struct AskPrevious {
    pub question: String,
    #[serde(default)]
    #[ts(optional)]
    pub sql: Option<SqlPrevious>,
}

impl AskPrevious {
    /// Bounded copy for the prompt: a follow-up cannot smuggle a large payload.
    fn sanitized(&self) -> Previous {
        let clip = |text: &str, max: usize| text.chars().take(max).collect::<String>();
        Previous {
            question: clip(self.question.trim(), 500),
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
    fn register(&self, request_id: Option<&str>, timeout: Duration) -> Result<(String, AskCancel)> {
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
        entries.retain(|entry| !entry.cancel.is_cancelled());
        if entries.len() >= MAX_IN_FLIGHT_ASKS {
            return Err(AppError::Conflict(
                "Too many Ask questions are already in flight. Wait a moment and try again.".into(),
            ));
        }
        entries.push(InFlightEntry {
            id: id.clone(),
            cancel: cancel.clone(),
        });
        Ok((id, cancel))
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

/// The Ask service seam. `AppState::new` installs the local GGUF SQL parser when
/// the `llama` feature is compiled in; tests install `Ask::stub()` and a
/// feature-off build keeps `Ask::unavailable()`.
#[derive(Clone)]
pub struct Ask {
    parser: Arc<dyn QueryParser>,
    pub runtime_available: bool,
    pub model_ready: bool,
    /// True when this Ask is backed by the real local GGUF runtime rather than a stub.
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
    if let Some(instructions) = std::env::var("SCROBBL_ASK_SQL_MAX_INSTRUCTIONS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
    {
        limits.max_instructions = instructions;
    }
    if let Some(rows) = std::env::var("SCROBBL_ASK_SQL_MAX_ROWS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
    {
        limits.max_rows = rows.clamp(1, 10_000);
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

    pub fn installed(db: sqlx::SqlitePool, data_dir: PathBuf) -> Self {
        Self {
            parser: Arc::new(parser::ModelParser::local(db, data_dir)),
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
    ) -> Result<(AskCancel, InFlightGuard)> {
        let (id, cancel) = self.in_flight.register(request_id, timeout)?;
        let guard = InFlightGuard {
            registry: Arc::clone(&self.in_flight),
            id,
            cancel: cancel.clone(),
            finished: false,
        };
        Ok((cancel, guard))
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

    pub fn model_loading(&self) -> bool {
        self.parser.model_loading()
    }

    pub fn unload(&self) {
        self.parser.unload();
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

    pub async fn parse_with_cancel(
        &self,
        question: &str,
        previous: Option<&Previous>,
        cancel: &CancelToken,
    ) -> Result<ParsedQuestion> {
        self.parser
            .parse_with_cancel(question, previous, Some(cancel))
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
        message: Some("The question was cancelled (rule `cancelled`).".into()),
        runtime_available: state.ask.runtime_available,
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
    if question.chars().count() > MAX_QUESTION_CHARS {
        return Err(AppError::BadRequest(
            "Question is too long (maximum 2,000 characters).".into(),
        ));
    }
    if let Err(mut refusal) = gate(&state).await? {
        refusal.question = question;
        return Ok(Json(refusal));
    }
    if question.is_empty() || !question.chars().any(char::is_alphanumeric) {
        return Ok(Json(AskResponse {
            status: AskStatus::Unsupported,
            question,
            message: Some("Please write a question about your music archive.".into()),
            runtime_available: state.ask.runtime_available,
            sql: None,
        }));
    }
    let previous = request.previous.as_ref().map(AskPrevious::sanitized);
    let (ask_cancel, guard) = state
        .ask
        .register_in_flight(request.request_id.as_deref(), parser::inference_timeout())?;

    let parsed = tokio::select! {
        biased;
        () = ask_cancel.cancelled() => {
            guard.finish();
            return Ok(Json(cancelled_response(&state, question, None)));
        }
        res = state.ask.parse_with_cancel(
            &question,
            previous.as_ref(),
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
        ParsedQuestion::Sql(output) => {
            run_sql(&state, question, previous.as_ref(), output, &ask_cancel).await?
        }
        ParsedQuestion::Unsupported { reason } => AskResponse {
            status: AskStatus::Unsupported,
            question,
            message: Some(reason),
            runtime_available: state.ask.runtime_available,
            sql: None,
        },
    };
    guard.finish();
    Ok(Json(response))
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
        run_sql_once(&state, String::new(), output, None).await?,
    ))
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
    if !state.ask.runtime_available {
        return Ok(Err(refusal(
            state,
            AskStatus::ModelUnavailable,
            "The local AI runtime is not available in this build.",
        )));
    }
    if state.ask.model_loading() {
        return Ok(Err(refusal(
            state,
            AskStatus::ModelUnavailable,
            "The local AI model is loading. Try again in a moment.",
        )));
    }
    if state.ask.local {
        let readiness = ai::selected_model_readiness(state, &stored).await?;
        if !(readiness.installed && readiness.selected_available) {
            return Ok(Err(refusal(
                state,
                AskStatus::ModelMissing,
                "Download and select an AI model in Settings, or set a custom local .gguf path, before asking questions.",
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
        message: Some(message.into()),
        runtime_available: state.ask.runtime_available,
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

static SANDBOX_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn sandbox_slots() -> &'static Arc<Semaphore> {
    SANDBOX_SLOTS.get_or_init(|| Arc::new(Semaphore::new(1)))
}

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
        message: None,
        runtime_available: state.ask.runtime_available,
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
        message: Some(message),
        runtime_available: state.ask.runtime_available,
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
    ask_cancel: Option<&AskCancel>,
) -> Result<AskResponse> {
    match execute_sql_once(state, &output, ask_cancel).await? {
        SqlAttempt::Done(exec) => Ok(sql_ok_response(state, question, output, exec)),
        SqlAttempt::Cancelled => Ok(cancelled_response(state, question, Some(output))),
        SqlAttempt::Rejected {
            as_of_ms,
            rejection,
        } => Ok(sql_failed_response(
            state,
            question,
            output,
            as_of_ms,
            rejection.rule,
            rejection.message,
        )),
    }
}

async fn run_sql(
    state: &AppState,
    question: String,
    previous: Option<&Previous>,
    mut output: SqlModelOutput,
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
                                message: Some(reason),
                                runtime_available: state.ask.runtime_available,
                                sql: None,
                            });
                        }
                        Ok(None) => {}
                        Err(_) if ask_cancel.is_cancelled() => {
                            return Ok(cancelled_response(state, question, Some(output)));
                        }
                        Err(AppError::BadRequest(detail)) => {
                            repair_note = Some(detail);
                        }
                        Err(err) => return Err(err),
                    }
                }
                let message = if let Some(note) = repair_note {
                    format!(
                        "Could not repair the SQL query ({note}) {}",
                        rejection.message
                    )
                } else if round > 0 && rejection.rule.is_repairable() {
                    format!(
                        "Could not produce a valid query after {MAX_SQL_REPAIR_ROUNDS} repair attempts. {}",
                        rejection.message
                    )
                } else {
                    rejection.message
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_sql_sandbox_runs_one_query_at_a_time() {
        assert_eq!(sandbox_slots().available_permits(), 1);

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
