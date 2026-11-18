#![allow(clippy::unwrap_used, clippy::disallowed_methods)]
//! Offline eval harness for local Ask SQL models (Qwen3 or custom GGUF).
//!
//! Reads `evals/ask/sql_corpus.jsonl`, builds the fixed synthetic fixture from
//! `ask::fixture`, runs one local GGUF file via `ask::runtime::Session` with
//! the shipped prompt and GBNF SQL grammar, executes generated SQL inside the
//! read-only sandbox (with up to `MAX_SQL_REPAIR_ROUNDS` repair rounds), and
//! scores the typed result against the expected values. Never touches the
//! network or downloads anything.
//!
//! ```text
//! cargo run --locked --release --bin ask-eval -- \
//!   --model C:\models\Qwen3-1.7B-Q4_K_M.gguf \
//!   --label qwen3-1.7b-q4_k_m \
//!   --threads 8 \
//!   --out C:\temp\qwen3-1.7b-sql.json
//! ```
use scrobbl_rs::{
    ai::models,
    ask::{
        fixture, prompt,
        runtime::{self, ModelSpec, Session},
        sandbox::{self, SandboxLimits, SqlValue},
        schema::{SqlAnswerKind, SqlModelOutput},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

const DEFAULT_SQL_CORPUS: &str = "evals/ask/sql_corpus.jsonl";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_WARMUPS: usize = 2;
const MAX_SQL_REPAIR_ROUNDS: usize = 2;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ask-eval: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse(std::env::args().skip(1))?;
    if args.help {
        eprintln!("{}", Args::usage());
        return Ok(());
    }
    models::validate_gguf_file(&args.model)?;

    let cases = load_sql_corpus(&args.sql_corpus)?;
    if cases.is_empty() {
        return Err("the SQL corpus is empty".into());
    }
    let cases = match args.limit {
        Some(limit) if limit < cases.len() => cases[..limit].to_vec(),
        _ => cases,
    };

    let spec = ModelSpec::new(
        args.label.clone(),
        args.model.clone(),
        args.threads.clamp(1, 64),
        15,
    );
    eprintln!(
        "ask-eval: loading {} ({} threads)",
        args.model.display(),
        spec.threads
    );
    let mut session = Session::open(spec.clone(), args.timeout)
        .map_err(|error| format!("could not load the model: {error}"))?;

    for case in cases.iter().take(args.warmups.min(cases.len())) {
        eprintln!("ask-eval: warm-up {}", case.id);
        let _ = session.run(&prompt::build_sql_prompt(&case.question, fixture::TODAY));
    }

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| format!("could not start the async runtime: {error}"))?;
    runtime.block_on(run_sql_cases(&mut session, &spec, &args, &cases))
}

#[derive(Clone, Debug)]
struct Args {
    model: PathBuf,
    label: String,
    threads: u32,
    limit: Option<usize>,
    out: Option<PathBuf>,
    report: Option<PathBuf>,
    timeout: Duration,
    warmups: usize,
    sql_corpus: PathBuf,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            model: PathBuf::new(),
            label: String::new(),
            threads: 4,
            limit: None,
            out: None,
            report: None,
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            warmups: DEFAULT_WARMUPS,
            sql_corpus: PathBuf::from(DEFAULT_SQL_CORPUS),
            help: false,
        }
    }
}

impl Args {
    fn usage() -> &'static str {
        "usage: ask-eval --model <file.gguf> [options]\n\
         \n\
         options:\n\
         \x20 --model <path>      local GGUF file to evaluate (required)\n\
         \x20 --label <name>      label stored in the result (default: file name)\n\
         \x20 --corpus <path>     SQL JSONL corpus (default: evals/ask/sql_corpus.jsonl)\n\
         \x20 --threads <n>       inference threads (default: 4)\n\
         \x20 --limit <n>         evaluate only the first n cases\n\
         \x20 --warmups <n>       uncounted warm-up questions (default: 2)\n\
         \x20 --timeout-ms <n>    per-question timeout (default: 120000)\n\
         \x20 --out <path.json>   write JSON plus a Markdown report next to it\n\
         \x20 --report <path.md>  override the Markdown report path\n\
         \x20 --help              show this help"
    }

    fn parse(mut arguments: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut args = Args::default();
        while let Some(argument) = arguments.next() {
            let mut value = || {
                arguments
                    .next()
                    .ok_or_else(|| format!("{argument} needs a value"))
            };
            match argument.as_str() {
                "--help" | "-h" => args.help = true,
                "--model" => args.model = PathBuf::from(value()?),
                "--label" => args.label = value()?,
                "--corpus" | "--sql-corpus" => args.sql_corpus = PathBuf::from(value()?),
                "--threads" => args.threads = parse_u32("--threads", &value()?)?,
                "--limit" => args.limit = Some(parse_usize("--limit", &value()?)?),
                "--warmups" => args.warmups = parse_usize("--warmups", &value()?)?,
                "--timeout-ms" => {
                    args.timeout = Duration::from_millis(parse_u64("--timeout-ms", &value()?)?)
                }
                "--out" => args.out = Some(PathBuf::from(value()?)),
                "--report" => args.report = Some(PathBuf::from(value()?)),
                other => return Err(format!("unknown argument {other:?}\n{}", Self::usage())),
            }
        }
        if args.help {
            return Ok(args);
        }
        if args.model.as_os_str().is_empty() {
            return Err(format!("--model is required\n{}", Self::usage()));
        }
        if args.label.is_empty() {
            args.label = file_name(&args.model);
        }
        if args.warmups > 1000 {
            return Err("--warmups is unreasonably large".into());
        }
        Ok(args)
    }
}

fn parse_u32(flag: &str, value: &str) -> Result<u32, String> {
    value
        .parse()
        .map_err(|_| format!("{flag} expects a positive integer, got {value:?}"))
}

fn parse_u64(flag: &str, value: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| format!("{flag} expects a positive integer, got {value:?}"))
}

fn parse_usize(flag: &str, value: &str) -> Result<usize, String> {
    let parsed: usize = value
        .parse()
        .map_err(|_| format!("{flag} expects a positive integer, got {value:?}"))?;
    if parsed == 0 {
        return Err(format!("{flag} must be greater than zero"));
    }
    Ok(parsed)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[derive(Clone, Debug, Deserialize)]
struct SqlCase {
    id: String,
    lang: String,
    question: String,
    #[serde(default)]
    tags: Vec<String>,
    reference_sql: String,
    expected: ExpectedTable,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct ExpectedTable {
    columns: Vec<String>,
    rows: Vec<Vec<Cell>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
enum Cell {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Correct,
    HonestFailure,
    WrongButPlausible,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Correct => "correct",
            Self::HonestFailure => "honest_failure",
            Self::WrongButPlausible => "wrong_but_plausible",
        }
    }
}

#[derive(Clone, Debug)]
struct SqlObservation {
    outcome: Outcome,
    rule_code: Option<String>,
    detail: Option<String>,
    latency_ms: f64,
    ttft_ms: f64,
}

fn load_sql_corpus(path: &Path) -> Result<Vec<SqlCase>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    parse_sql_corpus(&text, &path.display().to_string())
}

fn parse_sql_corpus(text: &str, origin: &str) -> Result<Vec<SqlCase>, String> {
    let mut cases = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let case: SqlCase = serde_json::from_str(line)
            .map_err(|error| format!("{origin}:{}: {error}", index + 1))?;
        validate_sql_case(&case, &mut ids)
            .map_err(|error| format!("{origin}:{}: {error}", index + 1))?;
        cases.push(case);
    }
    Ok(cases)
}

fn validate_sql_case(case: &SqlCase, ids: &mut BTreeSet<String>) -> Result<(), String> {
    let id = &case.id;
    if !ids.insert(id.clone()) {
        return Err(format!("{id}: duplicate id"));
    }
    if !matches!(case.lang.as_str(), "en" | "it") {
        return Err(format!("{id}: language must be en or it"));
    }
    if !id.starts_with(&format!("sql-{}-", case.lang)) {
        return Err(format!("{id}: id must start with sql-{}-", case.lang));
    }
    if case.question.trim().is_empty() {
        return Err(format!("{id}: the question is empty"));
    }
    if case.reference_sql.trim().is_empty() {
        return Err(format!("{id}: reference_sql is empty"));
    }
    let expected = &case.expected;
    if expected.columns.is_empty() {
        return Err(format!("{id}: expected.columns is empty"));
    }
    if expected.rows.is_empty() {
        return Err(format!(
            "{id}: expected.rows is empty; use an explicit result"
        ));
    }
    for (row, cells) in expected.rows.iter().enumerate() {
        if cells.len() != expected.columns.len() {
            return Err(format!(
                "{id}: expected row {} has {} cells for {} columns",
                row + 1,
                cells.len(),
                expected.columns.len()
            ));
        }
    }
    Ok(())
}

fn cell_matches(expected: &Cell, actual: &SqlValue) -> bool {
    match (expected, actual) {
        (Cell::Null, SqlValue::Null) => true,
        (Cell::Integer(a), SqlValue::Integer(b)) => a == b,
        (Cell::Text(a), SqlValue::Text(b)) => a == b,
        (Cell::Integer(a), SqlValue::Real(b)) => (*a as f64 - *b).abs() <= 1e-6,
        (Cell::Real(a), SqlValue::Integer(b)) => (*a - *b as f64).abs() <= 1e-6,
        (Cell::Real(a), SqlValue::Real(b)) => (a - b).abs() <= 1e-6,
        _ => false,
    }
}

fn render_value(value: &SqlValue) -> String {
    match value {
        SqlValue::Null => "null".into(),
        SqlValue::Integer(value) => value.to_string(),
        SqlValue::Real(value) => value.to_string(),
        SqlValue::Text(value) => format!("{value:?}"),
    }
}

fn render_cell(cell: &Cell) -> String {
    match cell {
        Cell::Null => "null".into(),
        Cell::Integer(value) => value.to_string(),
        Cell::Real(value) => value.to_string(),
        Cell::Text(value) => format!("{value:?}"),
    }
}

fn compare_result(expected: &ExpectedTable, output: &sandbox::SandboxOutput) -> Result<(), String> {
    if output.truncated {
        return Err("the result was truncated by the output cap".into());
    }
    if output.columns.len() != expected.columns.len() {
        return Err(format!(
            "expected {} columns, got {}",
            expected.columns.len(),
            output.columns.len()
        ));
    }
    if output.rows.len() != expected.rows.len() {
        return Err(format!(
            "expected {} rows, got {}",
            expected.rows.len(),
            output.rows.len()
        ));
    }
    for (row, (expected_row, actual_row)) in expected.rows.iter().zip(&output.rows).enumerate() {
        if expected_row.len() != actual_row.len() {
            return Err(format!("row {} has a different column count", row + 1));
        }
        for (column, (expected_cell, actual_cell)) in
            expected_row.iter().zip(actual_row).enumerate()
        {
            if !cell_matches(expected_cell, actual_cell) {
                return Err(format!(
                    "cell ({}, {}) expected {}, got {}",
                    row + 1,
                    expected.columns[column],
                    render_cell(expected_cell),
                    render_value(actual_cell)
                ));
            }
        }
    }
    Ok(())
}

fn parse_sql_output(reply: &str) -> Result<Option<SqlModelOutput>, String> {
    let output: SqlModelOutput =
        serde_json::from_str(reply.trim()).map_err(|error| format!("schema: {error}"))?;
    if output.answer_kind == SqlAnswerKind::Unsupported || output.sql.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(output))
}

fn observe_sql_case(session: &mut Session, case: &SqlCase, projection: &Path) -> SqlObservation {
    let generation = match session.run(&prompt::build_sql_prompt(&case.question, fixture::TODAY)) {
        Ok(generated) => generated,
        Err(error) => {
            return SqlObservation {
                outcome: Outcome::HonestFailure,
                rule_code: None,
                detail: Some(format!("inference: {error}")),
                latency_ms: 0.0,
                ttft_ms: 0.0,
            };
        }
    };
    let mut total_ms = generation.total_ms;
    let first_ttft = generation.ttft_ms;
    let fail = |detail: String, latency_ms: f64| SqlObservation {
        outcome: Outcome::HonestFailure,
        rule_code: None,
        detail: Some(detail),
        latency_ms,
        ttft_ms: first_ttft,
    };

    let mut output = match parse_sql_output(&generation.text) {
        Ok(Some(output)) => output,
        Ok(None) => return fail("unsupported".into(), total_ms),
        Err(error) => return fail(error, total_ms),
    };

    let mut attempts: Vec<(String, String)> = Vec::new();
    for round in 0..=MAX_SQL_REPAIR_ROUNDS {
        match sandbox::execute(projection, &output.sql, SandboxLimits::default()) {
            Ok(table) => {
                return match compare_result(&case.expected, &table) {
                    Ok(()) => SqlObservation {
                        outcome: Outcome::Correct,
                        rule_code: None,
                        detail: None,
                        latency_ms: total_ms,
                        ttft_ms: first_ttft,
                    },
                    Err(detail) => SqlObservation {
                        outcome: Outcome::WrongButPlausible,
                        rule_code: None,
                        detail: Some(detail),
                        latency_ms: total_ms,
                        ttft_ms: first_ttft,
                    },
                };
            }
            Err(rejection) => {
                if rejection.rule.is_repairable() && round < MAX_SQL_REPAIR_ROUNDS {
                    attempts.push((output.sql.clone(), rejection.model_error.clone()));
                    let refs: Vec<(&str, &str)> = attempts
                        .iter()
                        .map(|(sql, error)| (sql.as_str(), error.as_str()))
                        .collect();
                    let repair_prompt = prompt::build_sql_repair_prompt(
                        &case.question,
                        None,
                        &refs,
                        fixture::TODAY,
                    );
                    match session.run(&repair_prompt) {
                        Ok(generated) => {
                            total_ms += generated.total_ms;
                            match parse_sql_output(&generated.text) {
                                Ok(Some(next)) => {
                                    output = next;
                                    continue;
                                }
                                Ok(None) => {
                                    return fail("unsupported after repair".into(), total_ms);
                                }
                                Err(error) => {
                                    return fail(format!("repair output: {error}"), total_ms);
                                }
                            }
                        }
                        Err(error) => return fail(format!("repair request: {error}"), total_ms),
                    }
                }
                return SqlObservation {
                    outcome: Outcome::HonestFailure,
                    rule_code: Some(rejection.rule_name().to_owned()),
                    detail: Some(rejection.model_error),
                    latency_ms: total_ms,
                    ttft_ms: first_ttft,
                };
            }
        }
    }
    fail("repair rounds exhausted".into(), total_ms)
}

#[derive(Clone, Debug, Default)]
struct SqlCounts {
    cases: usize,
    correct: usize,
    honest_failure: usize,
    wrong_but_plausible: usize,
    latencies: Vec<f64>,
    ttfts: Vec<f64>,
}

impl SqlCounts {
    fn add(&mut self, observation: &SqlObservation) {
        self.cases += 1;
        match observation.outcome {
            Outcome::Correct => self.correct += 1,
            Outcome::HonestFailure => self.honest_failure += 1,
            Outcome::WrongButPlausible => self.wrong_but_plausible += 1,
        }
        if observation.latency_ms > 0.0 {
            self.latencies.push(observation.latency_ms);
            self.ttfts.push(observation.ttft_ms);
        }
    }

    fn summary(&self) -> SqlSummary {
        SqlSummary {
            cases: self.cases,
            correct: self.correct,
            honest_failure: self.honest_failure,
            wrong_but_plausible: self.wrong_but_plausible,
            p50_ms: percentile(&self.latencies, 50.0),
            p95_ms: percentile(&self.latencies, 95.0),
            ttft_p50_ms: percentile(&self.ttfts, 50.0),
            ttft_p95_ms: percentile(&self.ttfts, 95.0),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct SqlCaseResult {
    id: String,
    lang: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    rule_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct SqlRunResult {
    schema: u32,
    fixture: SqlFixtureInfo,
    model: SqlModelInfo,
    corpus: SqlCorpusInfo,
    overall: SqlSummary,
    by_language: BTreeMap<String, SqlSummary>,
    cases: Vec<SqlCaseResult>,
}

#[derive(Clone, Debug, Serialize)]
struct SqlFixtureInfo {
    version: u32,
    today: String,
}

#[derive(Clone, Debug, Serialize)]
struct SqlModelInfo {
    label: String,
    file: String,
    threads: u32,
    context_tokens: u32,
    max_new_tokens: i32,
}

#[derive(Clone, Debug, Serialize)]
struct SqlCorpusInfo {
    path: String,
    cases: usize,
    by_language: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize)]
struct SqlSummary {
    cases: usize,
    correct: usize,
    honest_failure: usize,
    wrong_but_plausible: usize,
    p50_ms: f64,
    p95_ms: f64,
    ttft_p50_ms: f64,
    ttft_p95_ms: f64,
}

impl SqlRunResult {
    fn markdown(&self) -> String {
        let mut report = String::new();
        report.push_str(&format!("# Ask local SQL eval - {}\n\n", self.model.label));
        report.push_str(&format!(
            "- Model file: `{}`\n- Threads: {}\n- Context: {} tokens, greedy, max {} new tokens\n- Fixture: version {}, today `{}`\n",
            self.model.file,
            self.model.threads,
            self.model.context_tokens,
            self.model.max_new_tokens,
            self.fixture.version,
            self.fixture.today
        ));
        let languages = self
            .corpus
            .by_language
            .iter()
            .map(|(language, count)| format!("{language} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        report.push_str(&format!(
            "- Corpus: `{}` ({} cases: {languages})\n\n",
            self.corpus.path, self.corpus.cases
        ));
        report.push_str(
            "| set | cases | correct | honest failure | wrong but plausible | p50 | p95 | TTFT p50 | TTFT p95 |\n\
             | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
        );
        report.push_str(&sql_row("all", &self.overall));
        for (language, summary) in &self.by_language {
            report.push_str(&sql_row(language, summary));
        }
        let failures: Vec<&SqlCaseResult> = self
            .cases
            .iter()
            .filter(|case| case.outcome != "correct")
            .collect();
        if failures.is_empty() {
            report.push_str("\nEvery case returned the expected result.\n");
        } else {
            report.push_str("\n| case | language | outcome | rule | detail |\n| --- | --- | --- | --- | --- |\n");
            for case in failures {
                report.push_str(&format!(
                    "| {} | {} | {} | {} | {} |\n",
                    case.id,
                    case.lang,
                    case.outcome,
                    case.rule_code.as_deref().unwrap_or("-"),
                    case.detail.as_deref().unwrap_or("-").replace('|', "\\|")
                ));
            }
        }
        report
    }
}

fn sql_row(label: &str, summary: &SqlSummary) -> String {
    format!(
        "| {label} | {} | {} | {} | {} | {:.0} ms | {:.0} ms | {:.0} ms | {:.0} ms |\n",
        summary.cases,
        summary.correct,
        summary.honest_failure,
        summary.wrong_but_plausible,
        summary.p50_ms,
        summary.p95_ms,
        summary.ttft_p50_ms,
        summary.ttft_p95_ms,
    )
}

async fn run_sql_cases(
    session: &mut Session,
    spec: &ModelSpec,
    args: &Args,
    cases: &[SqlCase],
) -> Result<(), String> {
    let data_dir = std::env::temp_dir().join(format!(
        "scrobbl-ask-eval-{}-{}",
        std::process::id(),
        scrobbl_rs::auth::random_secret()
    ));
    std::fs::create_dir_all(&data_dir)
        .map_err(|error| format!("could not create {}: {error}", data_dir.display()))?;
    let evaluation = evaluate_sql(session, spec, args, cases, &data_dir).await;
    let _ = std::fs::remove_dir_all(&data_dir);
    evaluation
}

async fn evaluate_sql(
    session: &mut Session,
    spec: &ModelSpec,
    args: &Args,
    cases: &[SqlCase],
    data_dir: &Path,
) -> Result<(), String> {
    eprintln!(
        "ask-eval: building fixture version {} in {}",
        fixture::FIXTURE_VERSION,
        data_dir.display()
    );
    let fixture = fixture::build(data_dir)
        .await
        .map_err(|error| format!("could not build the fixed fixture: {error}"))?;

    let mut overall = SqlCounts::default();
    let mut by_language: BTreeMap<String, SqlCounts> = BTreeMap::new();
    let mut results = Vec::with_capacity(cases.len());
    for (index, case) in cases.iter().enumerate() {
        let observation = observe_sql_case(session, case, fixture.projection_path());
        eprintln!(
            "ask-eval: {}/{} {} {}",
            index + 1,
            cases.len(),
            case.id,
            observation.outcome.as_str()
        );
        overall.add(&observation);
        by_language
            .entry(case.lang.clone())
            .or_default()
            .add(&observation);
        results.push(SqlCaseResult {
            id: case.id.clone(),
            lang: case.lang.clone(),
            tags: case.tags.clone(),
            outcome: observation.outcome.as_str(),
            rule_code: observation.rule_code,
            detail: observation.detail,
        });
    }

    let corpus_languages = cases.iter().fold(BTreeMap::new(), |mut acc, case| {
        *acc.entry(case.lang.clone()).or_insert(0) += 1;
        acc
    });

    let result = SqlRunResult {
        schema: 1,
        fixture: SqlFixtureInfo {
            version: fixture::FIXTURE_VERSION,
            today: fixture::TODAY.to_owned(),
        },
        model: SqlModelInfo {
            label: args.label.clone(),
            file: file_name(&args.model),
            threads: spec.threads,
            context_tokens: runtime::SQL_CONTEXT_TOKENS,
            max_new_tokens: runtime::MAX_SQL_NEW_TOKENS,
        },
        corpus: SqlCorpusInfo {
            path: args.sql_corpus.display().to_string(),
            cases: cases.len(),
            by_language: corpus_languages,
        },
        overall: overall.summary(),
        by_language: by_language
            .iter()
            .map(|(language, counts)| (language.clone(), counts.summary()))
            .collect(),
        cases: results,
    };

    let json = serde_json::to_string_pretty(&result)
        .map_err(|error| format!("could not serialize the result: {error}"))?;
    match &args.out {
        Some(path) => {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
            }
            std::fs::write(path, format!("{json}\n"))
                .map_err(|error| format!("could not write {}: {error}", path.display()))?;
            let report_path = args
                .report
                .clone()
                .unwrap_or_else(|| path.with_extension("md"));
            std::fs::write(&report_path, result.markdown())
                .map_err(|error| format!("could not write {}: {error}", report_path.display()))?;
            eprintln!(
                "ask-eval: wrote {} and {}",
                path.display(),
                report_path.display()
            );
        }
        None => println!("{json}"),
    }
    Ok(())
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((percentile / 100.0) * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    round(sorted[index], 1)
}

fn round(value: f64, places: u32) -> f64 {
    let factor = 10_f64.powi(places as i32);
    (value * factor).round() / factor
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQL_CORPUS: &str = include_str!("../../evals/ask/sql_corpus.jsonl");

    #[test]
    fn percentiles_use_nearest_rank() {
        let values = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(percentile(&values, 50.0), 20.0);
        assert_eq!(percentile(&values, 95.0), 40.0);
        assert_eq!(percentile(&[], 50.0), 0.0);
    }

    #[test]
    fn the_sql_corpus_is_valid_and_balanced() {
        let cases = parse_sql_corpus(SQL_CORPUS, "sql corpus").expect("the SQL corpus must parse");
        assert!(
            (18..=40).contains(&cases.len()),
            "expected 18-40 SQL cases, got {}",
            cases.len()
        );
        let english = cases.iter().filter(|case| case.lang == "en").count();
        let italian = cases.iter().filter(|case| case.lang == "it").count();
        assert_eq!(english + italian, cases.len());
        assert!(
            english.abs_diff(italian) <= 1,
            "languages are unbalanced: {english}/{italian}"
        );
        for tag in [
            "device",
            "country",
            "shuffle",
            "offline",
            "streak",
            "ranking",
            "gap",
            "first_play",
            "combined_conditions",
            "hour",
        ] {
            assert!(
                cases.iter().any(|case| case.tags.iter().any(|t| t == tag)),
                "no case is tagged {tag}"
            );
        }
    }

    #[tokio::test]
    async fn every_reference_query_returns_its_expected_result_on_the_fixed_fixture() {
        let cases = parse_sql_corpus(SQL_CORPUS, "sql corpus").expect("the SQL corpus must parse");
        let dir = tempfile::tempdir().unwrap();
        let fixture = fixture::build(dir.path()).await.unwrap();
        for case in &cases {
            let output = sandbox::execute(
                fixture.projection_path(),
                &case.reference_sql,
                SandboxLimits::default(),
            )
            .unwrap_or_else(|rejection| {
                panic!(
                    "{}: the reference SQL was rejected: {}",
                    case.id, rejection.model_error
                )
            });
            compare_result(&case.expected, &output)
                .unwrap_or_else(|error| panic!("{}: {error}", case.id));
        }
    }
}
