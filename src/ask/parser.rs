//! Parser seam between the Ask API and the question interpreter.
//!
//! The real local model adapter arrives in #178. It must implement the same
//! object-safe `QueryParser` trait and return only DSL, never SQL, rows or IDs.
//! CI and the current app use `StubParser`, a deterministic phrase map.
use crate::{
    ask::{
        dsl::{Entity, EntityKind, Metric, Period, Query, SortBy, SortOrder},
        runtime::{ModelSpec, Runtime, RuntimeError},
        schema::ModelOutput,
    },
    error::{AppError, Result},
};
use std::{future::Future, path::PathBuf, pin::Pin, time::Duration};

pub type ParseFuture<'a> = Pin<Box<dyn Future<Output = Result<ParsedQuestion>> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedQuestion {
    Query(Query),
    Unsupported { reason: String },
}

pub trait QueryParser: Send + Sync {
    fn parse<'a>(&'a self, question: &'a str) -> ParseFuture<'a>;

    /// True while the real runtime is loading a model. Stub and unavailable
    /// parsers never load anything.
    fn model_loading(&self) -> bool {
        false
    }
}

/// Deterministic English/Italian phrase map used by CI and development.
pub struct StubParser;

impl QueryParser for StubParser {
    fn parse<'a>(&'a self, question: &'a str) -> ParseFuture<'a> {
        Box::pin(async move { Ok(stub_parse(question)) })
    }
}

/// Stand-in until the compiled runtime and a downloaded model are available.
/// The handler also checks `runtime_available`, so this is only a safety net.
pub struct UnavailableParser;

impl QueryParser for UnavailableParser {
    fn parse<'a>(&'a self, _question: &'a str) -> ParseFuture<'a> {
        Box::pin(async move {
            Ok(ParsedQuestion::Unsupported {
                reason: "The AI runtime is not available in this build.".into(),
            })
        })
    }
}

pub fn unsupported_question_message() -> String {
    "I can answer questions about play counts, listening time, distinct tracks, artists and albums, first plays, last plays, listening streaks, milestones and grouped top lists (metrics: plays, time_listened, distinct_tracks, distinct_artists, distinct_albums, first_play, last_play, streak, milestones). I cannot write SQL or answer questions outside the archive. Try “How many times did I listen to Björk?” or “How many hours did I listen?”".into()
}

pub fn stub_parse(question: &str) -> ParsedQuestion {
    let question = normalise(question);
    if question.is_empty() {
        return ParsedQuestion::Unsupported {
            reason: unsupported_question_message(),
        };
    }

    if contains_any(
        &question,
        &["how many hours", "how long", "quante ore", "quanto tempo"],
    ) {
        return ParsedQuestion::Query(default_query(Metric::TimeListened));
    }
    if contains_any(
        &question,
        &[
            "distinct artist",
            "different artist",
            "distinct album",
            "distinct track",
            "quanti artisti",
            "quanti album",
            "quanti brani diversi",
        ],
    ) {
        let metric = if contains_any(&question, &["album"]) {
            Metric::DistinctAlbums
        } else if contains_any(&question, &["track", "brani"]) {
            Metric::DistinctTracks
        } else {
            Metric::DistinctArtists
        };
        return ParsedQuestion::Query(default_query(metric));
    }
    if contains_any(&question, &["streak", "serie di giorni"]) {
        return ParsedQuestion::Query(default_query(Metric::Streak));
    }
    if contains_any(&question, &["milestone", "traguardo"]) {
        return ParsedQuestion::Query(default_query(Metric::Milestones));
    }

    // First/last listening. Entity literals stay untouched apart from the
    // transport-level normalisation above; resolution happens in Rust.
    for (prefix, metric) in [
        ("when did i first listen to ", Metric::FirstPlay),
        ("when did i first play ", Metric::FirstPlay),
        ("when did i last listen to ", Metric::LastPlay),
        ("when did i last play ", Metric::LastPlay),
        ("when did i start listening to ", Metric::FirstPlay),
    ] {
        if let Some(literal) = question.strip_prefix(prefix) {
            return entity_query(metric, literal);
        }
    }
    for (prefix, suffix, metric) in [
        (
            "quando ho ascoltato ",
            " per la prima volta",
            Metric::FirstPlay,
        ),
        ("quando ho ascoltato ", " l'ultima volta", Metric::LastPlay),
        ("quando ho iniziato ad ascoltare ", "", Metric::FirstPlay),
    ] {
        if let Some(rest) = question.strip_prefix(prefix) {
            if !suffix.is_empty() && !rest.ends_with(suffix) {
                continue;
            }
            let literal = rest.strip_suffix(suffix).unwrap_or(rest);
            return entity_query(metric, literal);
        }
    }

    for prefix in [
        "how many times did i listen to ",
        "how many times have i listened to ",
        "how many times did i play ",
        "how many plays for ",
        "how many plays of ",
        "how many listens for ",
    ] {
        if let Some(literal) = question.strip_prefix(prefix) {
            return entity_query(Metric::Plays, literal);
        }
    }
    for prefix in [
        "quante volte ho ascoltato ",
        "quante volte ho riprodotto ",
        "quanti ascolti per ",
        "quanti ascolti di ",
    ] {
        if let Some(literal) = question.strip_prefix(prefix) {
            return entity_query(Metric::Plays, literal);
        }
    }

    if let Some(period) = period_only_question(&question) {
        return ParsedQuestion::Query(period);
    }

    // A question about a named entity with an unrecognised metric is still a
    // refusal, not an empty answer.
    ParsedQuestion::Unsupported {
        reason: unsupported_question_message(),
    }
}

fn entity_query(metric: Metric, literal: &str) -> ParsedQuestion {
    match entity_from(literal) {
        Some(entity) => {
            let mut query = default_query(metric);
            query.entity = Some(entity);
            ParsedQuestion::Query(query)
        }
        None => ParsedQuestion::Unsupported {
            reason: "Tell me which artist, album or track you mean.".into(),
        },
    }
}

fn entity_from(literal: &str) -> Option<Entity> {
    let mut rest = literal.trim();
    let mut kind = EntityKind::Artist;
    for (prefix, candidate) in [
        ("the artist ", EntityKind::Artist),
        ("artist ", EntityKind::Artist),
        ("the album ", EntityKind::Album),
        ("album ", EntityKind::Album),
        ("the track ", EntityKind::Track),
        ("track ", EntityKind::Track),
        ("the song ", EntityKind::Track),
        ("song ", EntityKind::Track),
        ("l'artista ", EntityKind::Artist),
        ("l'album ", EntityKind::Album),
        ("il brano ", EntityKind::Track),
        ("la canzone ", EntityKind::Track),
    ] {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            kind = candidate;
            rest = stripped;
            break;
        }
    }
    let name = rest.trim();
    (!name.is_empty()).then(|| Entity {
        kind,
        name: name.to_owned(),
        id: None,
    })
}

fn period_only_question(question: &str) -> Option<Query> {
    for prefix in [
        "how many plays",
        "how many listens",
        "how many times",
        "quanti ascolti",
        "quante volte",
    ] {
        let Some(suffix) = question.strip_prefix(prefix) else {
            continue;
        };
        let suffix = suffix.trim();
        let mut query = default_query(Metric::Plays);
        if suffix.is_empty() {
            return Some(query);
        }
        if let Some(rest) = suffix.strip_prefix("in ")
            && let Ok(year) = rest.parse::<i32>()
        {
            query.year = Some(year);
            query.period = Period::AllTime;
            return Some(query);
        }
        if let Some(rest) = suffix.strip_prefix("nel ")
            && let Ok(year) = rest.parse::<i32>()
        {
            query.year = Some(year);
            query.period = Period::AllTime;
            return Some(query);
        }
        let period = match suffix {
            "today" | "oggi" => Some(Period::Today),
            "yesterday" | "ieri" => Some(Period::Yesterday),
            "this week" | "questa settimana" => Some(Period::ThisWeek),
            "last week" | "la settimana scorsa" | "settimana scorsa" => Some(Period::LastWeek),
            "this month" | "questo mese" => Some(Period::ThisMonth),
            "last month" | "il mese scorso" | "mese scorso" => Some(Period::LastMonth),
            "this year" | "quest'anno" => Some(Period::ThisYear),
            "last year" | "l'anno scorso" | "anno scorso" => Some(Period::LastYear),
            _ => None,
        }?;
        query.period = period;
        return Some(query);
    }
    None
}

fn default_query(metric: Metric) -> Query {
    Query {
        metric,
        entity: None,
        period: Period::AllTime,
        year: None,
        month: None,
        weekday: None,
        hour: None,
        source: None,
        group_by: None,
        sort_by: SortBy::Metric,
        order: SortOrder::Desc,
        limit: None,
    }
}

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub fn inference_timeout() -> Duration {
    std::env::var("SCROBBL_ASK_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(DEFAULT_TIMEOUT_MS))
}

/// The real local parser. It reads consent, model selection, threads and
/// unload minutes from the archive settings on every question, resolves the
/// installed file from the filesystem, asks the dedicated worker and maps the
/// grammar-constrained JSON to the shared DSL.
pub struct ModelParser {
    db: sqlx::SqlitePool,
    data_dir: PathBuf,
    runtime: Runtime,
    timeout: Duration,
}

impl ModelParser {
    #[cfg(feature = "llama")]
    pub fn local(db: sqlx::SqlitePool, data_dir: PathBuf) -> Self {
        Self::new(
            db,
            data_dir,
            Runtime::spawn(Box::new(crate::ask::runtime::LlamaEngine::new()), 4),
            inference_timeout(),
        )
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

    async fn parse_model(&self, question: &str) -> Result<ParsedQuestion> {
        let stored = crate::ai::read_settings_from(&self.db).await?;
        if !stored.enabled {
            return Ok(ParsedQuestion::Unsupported {
                reason: "AI is off. Enable it in Settings to ask questions.".into(),
            });
        }
        let Some(model_id) = stored.active_model else {
            return Ok(ParsedQuestion::Unsupported {
                reason: "Download and select an AI model in Settings before asking questions."
                    .into(),
            });
        };
        let Some(catalog_model) = crate::ai::catalog::find(&model_id) else {
            return Ok(ParsedQuestion::Unsupported {
                reason: "The selected model is not in the curated catalog. Choose another model."
                    .into(),
            });
        };
        if !catalog_model.is_available() {
            return Ok(ParsedQuestion::Unsupported {
                reason: "The selected model is not available for questions in this release. Choose another model."
                    .into(),
            });
        }

        let root = match stored.model_dir.as_deref() {
            Some(dir) => PathBuf::from(dir),
            None => self.data_dir.join("models"),
        };
        let dir = crate::ai::models::model_dir(&root, &model_id)?;
        let installed = crate::ai::models::scan(&root)
            .into_iter()
            .find(|entry| entry.id == model_id);
        if !installed.is_some_and(|entry| {
            entry.usable() && entry.file.as_deref() == Some(catalog_model.download.file.as_str())
        }) {
            return Ok(ParsedQuestion::Unsupported {
                reason: "The selected model is not installed or did not pass verification. Download it again or choose another model."
                    .into(),
            });
        }
        let path = crate::ai::models::model_file(&dir, &catalog_model.download.file)?;

        let spec = ModelSpec::new(
            model_id,
            path,
            stored.threads.clamp(1, 64),
            stored.unload_minutes.clamp(1, 1440),
        );
        let prompt = crate::ask::prompt::build_prompt(question);
        let output = self
            .runtime
            .infer(&spec, &prompt, self.timeout)
            .await
            .map_err(runtime_error)?;

        let output: ModelOutput = serde_json::from_str(output.trim()).map_err(|_| {
            AppError::BadRequest(
                "The local model returned an interpretation that does not match the question format. Try rephrasing the question."
                    .into(),
            )
        })?;
        output.into_parsed()
    }
}

fn runtime_error(error: RuntimeError) -> AppError {
    match error {
        RuntimeError::Busy => AppError::Conflict(
            "The local model is answering another question. Try again in a moment.".into(),
        ),
        RuntimeError::Timeout => {
            AppError::BadRequest("The local model did not answer in time. Try again.".into())
        }
        RuntimeError::Cancelled => {
            AppError::BadRequest("The question was cancelled before the model answered.".into())
        }
        RuntimeError::Closed => AppError::Internal,
        RuntimeError::Failed(message) => AppError::BadRequest(format!(
            "The local model could not answer: {message}. Check the selected model in Settings."
        )),
    }
}

impl QueryParser for ModelParser {
    fn parse<'a>(&'a self, question: &'a str) -> ParseFuture<'a> {
        Box::pin(async move { self.parse_model(question).await })
    }

    fn model_loading(&self) -> bool {
        self.runtime_state().loading
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
    fn parses_english_and_italian_counts() {
        let ParsedQuestion::Query(query) = stub_parse("How many times did I listen to Björk?")
        else {
            panic!("expected a query");
        };
        assert_eq!(query.metric, Metric::Plays);
        assert_eq!(
            query.entity,
            Some(Entity {
                kind: EntityKind::Artist,
                name: "björk".into(),
                id: None,
            })
        );

        let ParsedQuestion::Query(query) = stub_parse("Quante volte ho ascoltato Björk?") else {
            panic!("expected a query");
        };
        assert_eq!(query.metric, Metric::Plays);
    }

    #[test]
    fn parses_first_last_and_periods() {
        let ParsedQuestion::Query(query) = stub_parse("When did I first listen to Radiohead?")
        else {
            panic!("expected a query");
        };
        assert_eq!(query.metric, Metric::FirstPlay);

        let ParsedQuestion::Query(query) = stub_parse("quando ho ascoltato Björk l'ultima volta")
        else {
            panic!("expected a query");
        };
        assert_eq!(query.metric, Metric::LastPlay);

        let ParsedQuestion::Query(query) = stub_parse("How many plays in 2024?") else {
            panic!("expected a query");
        };
        assert_eq!(query.year, Some(2024));
        assert_eq!(query.period, Period::AllTime);

        let ParsedQuestion::Query(query) = stub_parse("quanti ascolti quest'anno") else {
            panic!("expected a query");
        };
        assert_eq!(query.period, Period::ThisYear);
    }

    #[test]
    fn refuses_out_of_scope_questions_explicitly() {
        let ParsedQuestion::Unsupported { reason } = stub_parse("What is the weather?") else {
            panic!("expected a refusal");
        };
        assert!(reason.contains("play counts"));
        assert!(reason.contains("first plays"));
        assert!(reason.contains("last plays"));
    }

    #[test]
    fn the_stub_is_deterministic_and_has_no_free_form_numbers() {
        let first = stub_parse("How many times did I listen to Björk?");
        let second = stub_parse("  how many times did i listen to björk  ");
        assert_eq!(first, second);
        let ParsedQuestion::Query(query) = first else {
            panic!("expected a query");
        };
        assert!(query.limit.is_none());
    }
}
