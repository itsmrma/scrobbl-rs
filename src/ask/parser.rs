//! Parser seam between the Ask API and the local SQL model interpreter.
//!
//! `ModelParser` loads a verified local GGUF model (from the curated catalog
//! or a configured custom local path) and generates read-only SQLite queries
//! constrained by the GBNF SQL grammar. `StubParser` is a deterministic phrase
//! map used in automated tests without weights.
use crate::{
    ask::{
        runtime::{CancelToken, ModelSpec, Runtime, RuntimeError},
        schema::{ColumnFormat, SqlAnswerKind, SqlColumnSpec, SqlModelOutput},
    },
    error::{AppError, Result},
};
use std::{future::Future, path::PathBuf, pin::Pin, time::Duration};

pub type ParseFuture<'a> = Pin<Box<dyn Future<Output = Result<ParsedQuestion>> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedQuestion {
    Sql(SqlModelOutput),
    Unsupported { reason: String },
}

/// What the previous SQL answer computed, so a follow-up ("and last year?") can
/// build on it without ever sending archive results to the model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SqlPrevious {
    pub title: String,
    pub computed: String,
    pub sql: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Previous {
    pub question: String,
    pub sql: Option<SqlPrevious>,
}

pub trait QueryParser: Send + Sync {
    fn parse<'a>(&'a self, question: &'a str) -> ParseFuture<'a>;

    fn parse_follow_up<'a>(
        &'a self,
        question: &'a str,
        _previous: Option<&'a Previous>,
    ) -> ParseFuture<'a> {
        self.parse(question)
    }

    fn parse_with_cancel<'a>(
        &'a self,
        question: &'a str,
        previous: Option<&'a Previous>,
        _cancel: Option<&'a CancelToken>,
    ) -> ParseFuture<'a> {
        self.parse_follow_up(question, previous)
    }

    /// Ask the model to repair a rejected or failing SQL query using only its
    /// previous SQL and the sanitized sandbox error. Returns `Ok(None)` when
    /// the parser does not support SQL repair.
    fn repair_sql<'a>(
        &'a self,
        _question: &'a str,
        _previous: Option<&'a Previous>,
        _attempts: &'a [(&'a str, &'a str)],
        _cancel: Option<&'a CancelToken>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ParsedQuestion>>> + Send + 'a>> {
        Box::pin(async { Ok(None) })
    }

    /// True while the real runtime is loading a model.
    fn model_loading(&self) -> bool {
        false
    }

    /// Ask the underlying worker to drop any loaded model from memory.
    fn unload(&self) {}
}

/// Deterministic English/Italian SQL phrase map used by CI and development.
pub struct StubParser;

impl QueryParser for StubParser {
    fn parse<'a>(&'a self, question: &'a str) -> ParseFuture<'a> {
        Box::pin(async move { Ok(stub_parse_sql(question)) })
    }

    fn parse_follow_up<'a>(
        &'a self,
        question: &'a str,
        previous: Option<&'a Previous>,
    ) -> ParseFuture<'a> {
        Box::pin(async move { Ok(stub_parse_sql_follow_up(question, previous)) })
    }
}

/// Stand-in until the compiled runtime and a local model are available.
pub struct UnavailableParser;

impl QueryParser for UnavailableParser {
    fn parse<'a>(&'a self, _question: &'a str) -> ParseFuture<'a> {
        Box::pin(async move {
            Ok(ParsedQuestion::Unsupported {
                reason: "The local AI runtime is not available in this build.".into(),
            })
        })
    }
}

pub fn stub_parse_sql_follow_up(question: &str, previous: Option<&Previous>) -> ParsedQuestion {
    let normalised = normalise(question);
    if let Some(prev) = previous
        && let Some(prev_sql) = prev.sql.as_ref()
        && let Some(year_str) = normalised
            .strip_prefix("and in ")
            .or_else(|| normalised.strip_prefix("e nel "))
            .or_else(|| normalised.strip_prefix("what about "))
        && let Ok(year) = year_str.trim().parse::<i32>()
    {
        return ParsedQuestion::Sql(SqlModelOutput {
            sql: format!(
                "SELECT COUNT(*) AS plays FROM ({}) WHERE 1 = 1 AND {year} = {year}",
                prev_sql.sql.trim().trim_end_matches(';')
            ),
            title: format!("Follow-up ({year})"),
            computed: format!("Follow-up based on previous query for {year}."),
            answer_kind: SqlAnswerKind::Value,
            columns: vec![SqlColumnSpec {
                name: "plays".into(),
                format: ColumnFormat::Count,
            }],
        });
    }
    stub_parse_sql(question)
}

/// Deterministic SQL-mode stub used when testing local SQL mode without a GGUF model.
pub fn stub_parse_sql(question: &str) -> ParsedQuestion {
    let normalised = normalise(question);
    if normalised.is_empty()
        || contains_any(
            &normalised,
            &["weather", "meteo", "poem", "poesia", "capital of"],
        )
    {
        return ParsedQuestion::Unsupported {
            reason: "I can only answer questions about your listening archive.".into(),
        };
    }
    if contains_any(&normalised, &["show me settings", "read settings"]) {
        return ParsedQuestion::Sql(SqlModelOutput {
            sql: "SELECT value FROM settings".into(),
            title: "Settings".into(),
            computed: "Attempted to read settings.".into(),
            answer_kind: SqlAnswerKind::Table,
            columns: Vec::new(),
        });
    }
    if contains_any(&normalised, &["invalid sql", "broken sql"]) {
        return ParsedQuestion::Sql(SqlModelOutput {
            sql: "SELECT * FROM nonexistent_table".into(),
            title: "Broken query".into(),
            computed: "Attempted to query a missing table.".into(),
            answer_kind: SqlAnswerKind::Table,
            columns: Vec::new(),
        });
    }
    if contains_any(
        &normalised,
        &["top tracks", "brani più ascoltati", "canzoni più ascoltate"],
    ) {
        return ParsedQuestion::Sql(SqlModelOutput {
            sql: "SELECT t.title AS track, COUNT(*) AS plays FROM listens l JOIN tracks t ON t.id = l.track_id WHERE l.counted = 1 GROUP BY t.id ORDER BY plays DESC, t.title ASC LIMIT 10".into(),
            title: "Top tracks".into(),
            computed: "Counted plays per track across all time.".into(),
            answer_kind: SqlAnswerKind::Ranking,
            columns: vec![
                SqlColumnSpec {
                    name: "track".into(),
                    format: ColumnFormat::Track,
                },
                SqlColumnSpec {
                    name: "plays".into(),
                    format: ColumnFormat::Count,
                },
            ],
        });
    }
    if contains_any(&normalised, &["top artists", "artisti più ascoltati"]) {
        return ParsedQuestion::Sql(SqlModelOutput {
            sql: "SELECT a.name AS artist, COUNT(DISTINCT l.id) AS plays FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id WHERE l.counted = 1 GROUP BY a.id ORDER BY plays DESC, a.name ASC LIMIT 10".into(),
            title: "Top artists".into(),
            computed: "Counted plays per artist across all time.".into(),
            answer_kind: SqlAnswerKind::Ranking,
            columns: vec![
                SqlColumnSpec {
                    name: "artist".into(),
                    format: ColumnFormat::Artist,
                },
                SqlColumnSpec {
                    name: "plays".into(),
                    format: ColumnFormat::Count,
                },
            ],
        });
    }
    if contains_any(
        &normalised,
        &["how many hours", "how long", "quante ore", "quanto tempo"],
    ) {
        return ParsedQuestion::Sql(SqlModelOutput {
            sql:
                "SELECT COALESCE(SUM(listened_ms), 0) AS listened_ms FROM listens WHERE counted = 1"
                    .into(),
            title: "Total listening time".into(),
            computed: "Total known listening time for counted plays across all time.".into(),
            answer_kind: SqlAnswerKind::Value,
            columns: vec![SqlColumnSpec {
                name: "listened_ms".into(),
                format: ColumnFormat::DurationMs,
            }],
        });
    }
    for prefix in [
        "how many times did i listen to ",
        "how many times have i listened to ",
        "how many times did i play ",
        "quante volte ho ascoltato ",
    ] {
        if let Some(name) = normalised.strip_prefix(prefix) {
            let escaped = name.trim().replace('\'', "''");
            return ParsedQuestion::Sql(SqlModelOutput {
                sql: format!(
                    "SELECT COUNT(DISTINCT l.id) AS plays FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id WHERE l.counted = 1 AND LOWER(a.name) = LOWER('{escaped}')"
                ),
                title: format!("Plays for {}", name.trim()),
                computed: format!("Counted plays for artist {} across all time.", name.trim()),
                answer_kind: SqlAnswerKind::Value,
                columns: vec![SqlColumnSpec {
                    name: "plays".into(),
                    format: ColumnFormat::Count,
                }],
            });
        }
    }
    if contains_any(
        &normalised,
        &["how many plays", "how many listens", "quanti ascolti"],
    ) {
        return ParsedQuestion::Sql(SqlModelOutput {
            sql: "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1".into(),
            title: "Total plays".into(),
            computed: "Total counted plays across all time.".into(),
            answer_kind: SqlAnswerKind::Value,
            columns: vec![SqlColumnSpec {
                name: "plays".into(),
                format: ColumnFormat::Count,
            }],
        });
    }
    ParsedQuestion::Unsupported {
        reason: "I couldn't translate that question in stub SQL mode. Try \"What are my top tracks?\", \"How many hours did I listen?\", or \"How many plays?\".".into(),
    }
}

const DEFAULT_TIMEOUT_MS: u64 = 90_000;

pub fn inference_timeout() -> Duration {
    std::env::var("SCROBBL_ASK_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(DEFAULT_TIMEOUT_MS))
}

/// Local GGUF parser backed by `llama.cpp`. Reads consent, model selection
/// (or custom GGUF path), threads, and unload timeout on every question and
/// produces schema-constrained JSON containing a read-only SQL query.
pub struct ModelParser {
    db: sqlx::SqlitePool,
    data_dir: PathBuf,
    runtime: Runtime,
    timeout: Duration,
}

impl ModelParser {
    pub fn local(db: sqlx::SqlitePool, data_dir: PathBuf) -> Self {
        #[cfg(feature = "llama")]
        let engine: Box<dyn crate::ask::runtime::Engine> =
            Box::new(crate::ask::runtime::LlamaEngine::new());
        #[cfg(not(feature = "llama"))]
        let engine: Box<dyn crate::ask::runtime::Engine> =
            Box::new(crate::ask::runtime::UnavailableEngine);
        Self::new(db, data_dir, Runtime::spawn(engine, 4), inference_timeout())
    }

    pub fn new(
        db: sqlx::SqlitePool,
        data_dir: PathBuf,
        runtime: Runtime,
        timeout: Duration,
    ) -> Self {
        Self {
            db,
            data_dir,
            runtime,
            timeout,
        }
    }

    pub fn runtime_state(&self) -> crate::ask::runtime::RuntimeState {
        self.runtime.state()
    }

    async fn parse_model(
        &self,
        question: &str,
        previous: Option<&Previous>,
        cancel: Option<&CancelToken>,
    ) -> Result<ParsedQuestion> {
        if cancel.is_some_and(CancelToken::is_cancelled) {
            return Err(runtime_error(RuntimeError::Cancelled));
        }
        let stored = crate::ai::read_settings_from(&self.db).await?;
        if !stored.enabled {
            return Ok(ParsedQuestion::Unsupported {
                reason: "AI is off. Enable it in Settings to ask questions.".into(),
            });
        }
        let spec = match self.local_spec(&stored)? {
            Ok(spec) => spec,
            Err(reason) => return Ok(ParsedQuestion::Unsupported { reason }),
        };
        let token = cancel
            .cloned()
            .unwrap_or_else(|| CancelToken::new(self.timeout));
        let today = active_calendar_today(&self.db).await;
        let prompt = crate::ask::prompt::build_sql_prompt_with_previous(question, previous, &today);
        let output = self
            .runtime
            .infer_with_cancel(&spec, &prompt, token)
            .await
            .map_err(runtime_error)?;
        interpret_local_sql(&output)
    }

    /// Resolve the active local model (curated catalog entry, local folder, or
    /// custom GGUF path) into the worker's spec.
    fn local_spec(
        &self,
        stored: &crate::ai::StoredSettings,
    ) -> Result<std::result::Result<ModelSpec, String>> {
        match crate::ai::resolve_local_model_path(&self.data_dir, stored)? {
            Ok((model_id, path)) => Ok(Ok(ModelSpec::new(
                model_id,
                path,
                stored.threads.clamp(1, 64),
                stored.unload_minutes.clamp(1, 1440),
            )
            .with_gpu_layers(crate::ai::effective_gpu_layers(stored.gpu_layers)))),
            Err(reason) => Ok(Err(reason)),
        }
    }

    async fn repair_sql_model(
        &self,
        question: &str,
        previous: Option<&Previous>,
        attempts: &[(&str, &str)],
        cancel: Option<&CancelToken>,
    ) -> Result<Option<ParsedQuestion>> {
        if cancel.is_some_and(CancelToken::is_cancelled) {
            return Err(runtime_error(RuntimeError::Cancelled));
        }
        let stored = crate::ai::read_settings_from(&self.db).await?;
        if !stored.enabled {
            return Ok(Some(ParsedQuestion::Unsupported {
                reason: "AI is off. Enable it in Settings to ask questions.".into(),
            }));
        }
        let spec = match self.local_spec(&stored)? {
            Ok(spec) => spec,
            Err(reason) => return Ok(Some(ParsedQuestion::Unsupported { reason })),
        };
        let today = active_calendar_today(&self.db).await;
        let prompt =
            crate::ask::prompt::build_sql_repair_prompt(question, previous, attempts, &today);
        let token = cancel
            .cloned()
            .unwrap_or_else(|| CancelToken::new(self.timeout));
        let raw = self
            .runtime
            .infer_with_cancel(&spec, &prompt, token)
            .await
            .map_err(runtime_error)?;
        interpret_local_sql(&raw).map(Some)
    }
}

async fn active_calendar_today(db: &sqlx::SqlitePool) -> String {
    let tz_name: Option<String> = sqlx::query_scalar(
        "SELECT c.timezone FROM calendars c JOIN settings s ON s.value = c.id WHERE s.key = 'calendar_id'",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    let now = chrono::Utc::now();
    if let Some(tz) = tz_name.and_then(|name| name.parse::<chrono_tz::Tz>().ok()) {
        return now.with_timezone(&tz).format("%Y-%m-%d").to_string();
    }
    now.format("%Y-%m-%d").to_string()
}

fn extract_json_payload(raw: &str) -> &str {
    let mut text = raw.trim();
    if let Some(after_think_open) = text.strip_prefix("<think>")
        && let Some((_, after)) = after_think_open.split_once("</think>")
    {
        text = after.trim();
    } else if let Some((before, after)) = text.split_once("</think>")
        && !before.trim_start().starts_with('{')
    {
        text = after.trim();
    }
    if let Some(rest) = text.strip_prefix("```") {
        let inner = match rest.split_once('\n') {
            Some((_, after_lang)) => after_lang,
            None => rest,
        };
        if let Some((body, _)) = inner.rsplit_once("```") {
            text = body.trim();
        }
    }
    text
}

pub fn interpret_local_sql(raw: &str) -> Result<ParsedQuestion> {
    let payload = extract_json_payload(raw);
    let output: SqlModelOutput = serde_json::from_str(payload).map_err(|_| {
        AppError::BadRequest(
            "The local model returned a SQL interpretation that does not match the expected format. Try rephrasing the question, choosing a larger Qwen3.5 model, or configuring a custom local GGUF model."
                .into(),
        )
    })?;
    if output.answer_kind == SqlAnswerKind::Unsupported {
        let reason = if output.computed.trim().is_empty() {
            "I can only answer questions about your listening archive.".to_owned()
        } else {
            output.computed.trim().to_owned()
        };
        return Ok(ParsedQuestion::Unsupported { reason });
    }
    Ok(ParsedQuestion::Sql(output))
}

fn runtime_error(error: RuntimeError) -> AppError {
    match error {
        RuntimeError::Busy => AppError::Conflict(
            "The local model is answering another question. Try again in a moment.".into(),
        ),
        RuntimeError::Timeout => AppError::BadRequest(
            "The local model did not answer in time. Try a smaller question or a different local model."
                .into(),
        ),
        RuntimeError::Cancelled => {
            AppError::BadRequest("The question was cancelled before the model answered.".into())
        }
        RuntimeError::Closed => AppError::Internal,
        RuntimeError::Failed(message) => AppError::BadRequest(format!(
            "The local model could not answer: {message}. Check the selected model or custom GGUF path in Settings → AI."
        )),
    }
}

impl QueryParser for ModelParser {
    fn parse<'a>(&'a self, question: &'a str) -> ParseFuture<'a> {
        Box::pin(async move { self.parse_model(question, None, None).await })
    }

    fn parse_follow_up<'a>(
        &'a self,
        question: &'a str,
        previous: Option<&'a Previous>,
    ) -> ParseFuture<'a> {
        Box::pin(async move { self.parse_model(question, previous, None).await })
    }

    fn parse_with_cancel<'a>(
        &'a self,
        question: &'a str,
        previous: Option<&'a Previous>,
        cancel: Option<&'a CancelToken>,
    ) -> ParseFuture<'a> {
        Box::pin(async move { self.parse_model(question, previous, cancel).await })
    }

    fn repair_sql<'a>(
        &'a self,
        question: &'a str,
        previous: Option<&'a Previous>,
        attempts: &'a [(&'a str, &'a str)],
        cancel: Option<&'a CancelToken>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ParsedQuestion>>> + Send + 'a>> {
        Box::pin(async move {
            self.repair_sql_model(question, previous, attempts, cancel)
                .await
        })
    }

    fn model_loading(&self) -> bool {
        self.runtime_state().loading
    }

    fn unload(&self) {
        self.runtime.unload();
    }
}

fn contains_any(question: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| question.contains(needle))
}

fn normalise(question: &str) -> String {
    question
        .trim()
        .trim_end_matches(['?', '.', '!', ' '])
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_generates_sql_for_english_and_italian_questions() {
        let ParsedQuestion::Sql(out) = stub_parse_sql("How many times did I listen to Björk?")
        else {
            panic!("expected SQL output");
        };
        assert_eq!(out.answer_kind, SqlAnswerKind::Value);
        assert!(out.sql.contains("LOWER('björk')"));

        let ParsedQuestion::Sql(out) = stub_parse_sql("Quante volte ho ascoltato Björk?") else {
            panic!("expected SQL output");
        };
        assert_eq!(out.answer_kind, SqlAnswerKind::Value);

        let ParsedQuestion::Sql(top) = stub_parse_sql("What are my top tracks?") else {
            panic!("expected SQL output");
        };
        assert_eq!(top.answer_kind, SqlAnswerKind::Ranking);
        assert!(top.sql.contains("GROUP BY t.id"));
    }

    #[test]
    fn stub_refuses_out_of_scope_questions_explicitly() {
        let ParsedQuestion::Unsupported { reason } = stub_parse_sql("What is the weather?") else {
            panic!("expected a refusal");
        };
        assert!(reason.contains("listening archive"));
    }
}
