//! Offline eval harness for the Ask parser and for Ask free questions (SQL).
//!
//! The guided mode reads a versioned JSONL corpus, runs the shipped prompt,
//! grammar and `ModelOutput` path against one local GGUF file, and writes a
//! compact JSON result plus a Markdown report. It never runs in the normal test
//! suite and never touches the network: the model is passed by path.
//!
//! The SQL mode reads `evals/ask/sql_corpus.jsonl`, builds the fixed synthetic
//! fixture from `ask::fixture`, asks one OpenAI-compatible server for SQL and
//! scores the typed result against the expected values. It runs only when
//! explicitly requested and requires a server the operator names.
//!
//! ```text
//! cargo run --locked --release --bin ask-eval -- \
//!   --model C:\models\Qwen3-1.7B-Q4_K_M.gguf \
//!   --label qwen3-1.7b-q4_k_m \
//!   --corpus evals/ask/corpus.jsonl \
//!   --threads 8 \
//!   --out C:\temp\qwen3-1.7b.json
//!
//! cargo run --locked --release --bin ask-eval -- --sql \
//!   --base-url https://api.example.com/v1 --remote-model example-model \
//!   --label example --out C:\temp\example-sql.json
//! ```
use scrobbl_rs::ask::{
    dsl::{Limit, Query},
    fixture,
    parser::ParsedQuestion,
    prompt,
    remote::{self, RemoteConfig},
    runtime::{self, Generation, ModelSpec, Session},
    sandbox::{self, SandboxLimits, SandboxRule, SqlValue},
    schema::{self, ModelOutput, SqlAnswerKind, SqlModelOutput},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

const DEFAULT_CORPUS: &str = "evals/ask/corpus.jsonl";
const DEFAULT_SQL_CORPUS: &str = "evals/ask/sql_corpus.jsonl";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_WARMUPS: usize = 2;
const MAX_SQL_REPAIR_ROUNDS: usize = 2;
const SLOTS: [&str; 7] = [
    "target", "name", "period", "weekday", "group_by", "limit", "order",
];

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
    if args.sql {
        return run_sql(&args);
    }

    let cases = load_corpus(&args.corpus)?;
    let cases = match args.limit {
        Some(limit) if limit < cases.len() => cases[..limit].to_vec(),
        _ => cases,
    };
    if cases.is_empty() {
        return Err("the corpus is empty".into());
    }

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
        let _ = session.run(&prompt::build_prompt(&case.question));
    }

    let mut overall = Counts::default();
    let mut by_language: BTreeMap<String, Counts> = BTreeMap::new();
    for (index, case) in cases.iter().enumerate() {
        let observation = observe(&mut session, case);
        if observation.error.is_some() {
            eprintln!(
                "ask-eval: {}/{} {} failed: {}",
                index + 1,
                cases.len(),
                case.id,
                observation.error.as_deref().unwrap_or("unknown error")
            );
        } else if (index + 1) % 20 == 0 {
            eprintln!("ask-eval: {}/{}", index + 1, cases.len());
        }
        overall.add(case, &observation);
        by_language
            .entry(case.lang.clone())
            .or_default()
            .add(case, &observation);
    }

    let corpus_languages: BTreeMap<String, usize> =
        cases.iter().fold(BTreeMap::new(), |mut acc, case| {
            *acc.entry(case.lang.clone()).or_insert(0) += 1;
            acc
        });

    let result = RunResult {
        schema: 1,
        model: ModelInfo {
            label: args.label.clone(),
            file: file_name(&args.model),
            threads: spec.threads,
            context_tokens: runtime::CONTEXT_TOKENS,
            max_new_tokens: runtime::MAX_NEW_TOKENS,
        },
        corpus: CorpusInfo {
            path: args.corpus.display().to_string(),
            cases: cases.len(),
            by_language: corpus_languages,
        },
        overall: overall.summary(),
        by_language: by_language
            .iter()
            .map(|(language, counts)| (language.clone(), counts.summary()))
            .collect(),
        failed_case_ids: overall.failed_ids.clone(),
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

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Args {
    model: PathBuf,
    label: String,
    corpus: PathBuf,
    threads: u32,
    limit: Option<usize>,
    out: Option<PathBuf>,
    report: Option<PathBuf>,
    timeout: Duration,
    warmups: usize,
    sql: bool,
    sql_corpus: PathBuf,
    base_url: Option<String>,
    remote_model: Option<String>,
    api_key: Option<String>,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            model: PathBuf::new(),
            label: String::new(),
            corpus: PathBuf::from(DEFAULT_CORPUS),
            threads: 4,
            limit: None,
            out: None,
            report: None,
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            warmups: DEFAULT_WARMUPS,
            sql: false,
            sql_corpus: PathBuf::from(DEFAULT_SQL_CORPUS),
            base_url: None,
            remote_model: None,
            api_key: None,
            help: false,
        }
    }
}

impl Args {
    fn usage() -> &'static str {
        "usage: ask-eval --model <file.gguf> [options]\n\
         \x20      ask-eval --sql --base-url <url> --remote-model <name> [options]\n\
         \n\
         options:\n\
         \x20 --model <path>      GGUF file to evaluate in guided mode (required there)\n\
         \x20 --label <name>      label stored in the result (default: file stem or model name)\n\
         \x20 --corpus <path>     guided JSONL corpus (default: evals/ask/corpus.jsonl)\n\
         \x20 --threads <n>       inference threads (default: 4)\n\
         \x20 --limit <n>         evaluate only the first n cases\n\
         \x20 --warmups <n>       uncounted warm-up questions (default: 2)\n\
         \x20 --timeout-ms <n>    per-question timeout (default: 120000)\n\
         \x20 --out <path.json>   write JSON plus a Markdown report next to it\n\
         \x20 --report <path.md>  override the Markdown report path\n\
         \x20 --sql               score free questions (SQL) by result on the fixed fixture\n\
         \x20 --sql-corpus <path> SQL JSONL corpus (default: evals/ask/sql_corpus.jsonl)\n\
         \x20 --base-url <url>    OpenAI-compatible server base URL (SQL mode)\n\
         \x20 --remote-model <n>  model name sent to that server (SQL mode)\n\
         \x20 --api-key <key>     optional bearer token (SQL mode; prefer SCROBBL_ASK_EVAL_API_KEY)\n\
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
                "--corpus" => args.corpus = PathBuf::from(value()?),
                "--threads" => args.threads = parse_u32("--threads", &value()?)?,
                "--limit" => args.limit = Some(parse_usize("--limit", &value()?)?),
                "--warmups" => args.warmups = parse_usize("--warmups", &value()?)?,
                "--timeout-ms" => {
                    args.timeout = Duration::from_millis(parse_u64("--timeout-ms", &value()?)?)
                }
                "--out" => args.out = Some(PathBuf::from(value()?)),
                "--report" => args.report = Some(PathBuf::from(value()?)),
                "--sql" => args.sql = true,
                "--sql-corpus" => args.sql_corpus = PathBuf::from(value()?),
                "--base-url" => args.base_url = Some(value()?),
                "--remote-model" => args.remote_model = Some(value()?),
                "--api-key" => args.api_key = Some(value()?),
                other => return Err(format!("unknown argument {other:?}\n{}", Self::usage())),
            }
        }
        if args.help {
            return Ok(args);
        }
        if args.sql {
            if args
                .base_url
                .as_deref()
                .is_none_or(|url| url.trim().is_empty())
            {
                return Err(format!(
                    "--base-url is required in SQL mode\n{}",
                    Self::usage()
                ));
            }
            if args
                .remote_model
                .as_deref()
                .is_none_or(|model| model.trim().is_empty())
            {
                return Err(format!(
                    "--remote-model is required in SQL mode\n{}",
                    Self::usage()
                ));
            }
            if args.label.is_empty() {
                args.label = args.remote_model.clone().unwrap_or_default();
            }
            if args.warmups > 1000 {
                return Err("--warmups is unreasonably large".into());
            }
            return Ok(args);
        }
        if args.model.as_os_str().is_empty() {
            return Err(format!("--model is required\n{}", Self::usage()));
        }
        if !args.model.is_file() {
            return Err(format!("model file not found: {}", args.model.display()));
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

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
struct Case {
    id: String,
    lang: String,
    question: String,
    #[serde(default)]
    tags: Vec<String>,
    expected: Expected,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Expected {
    metric: String,
    target: String,
    name: String,
    period: String,
    weekday: String,
    group_by: String,
    limit: String,
    order: String,
}

fn load_corpus(path: &Path) -> Result<Vec<Case>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    parse_corpus(&text, &path.display().to_string())
}

fn parse_corpus(text: &str, origin: &str) -> Result<Vec<Case>, String> {
    let mut cases = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let case: Case = serde_json::from_str(line)
            .map_err(|error| format!("{origin}:{}: {error}", index + 1))?;
        validate_case(&case).map_err(|error| format!("{origin}:{}: {error}", index + 1))?;
        cases.push(case);
    }
    Ok(cases)
}

fn validate_case(case: &Case) -> Result<(), String> {
    let id = &case.id;
    if !matches!(case.lang.as_str(), "en" | "it") {
        return Err(format!("{id}: language must be en or it"));
    }
    if !schema::ModelMetric::VALUES.contains(&case.expected.metric.as_str()) {
        return Err(format!("{id}: unknown metric {:?}", case.expected.metric));
    }
    if !schema::ModelTarget::VALUES.contains(&case.expected.target.as_str()) {
        return Err(format!("{id}: unknown target {:?}", case.expected.target));
    }
    if !schema::ModelPeriod::VALUES.contains(&case.expected.period.as_str()) {
        return Err(format!("{id}: unknown period {:?}", case.expected.period));
    }
    if !schema::ModelWeekday::VALUES.contains(&case.expected.weekday.as_str()) {
        return Err(format!("{id}: unknown weekday {:?}", case.expected.weekday));
    }
    if !schema::ModelGroupBy::VALUES.contains(&case.expected.group_by.as_str()) {
        return Err(format!(
            "{id}: unknown group_by {:?}",
            case.expected.group_by
        ));
    }
    if canonical_limit(&case.expected.limit).is_none() {
        return Err(format!("{id}: unknown limit {:?}", case.expected.limit));
    }
    if !matches!(case.expected.order.as_str(), "asc" | "desc") {
        return Err(format!("{id}: unknown order {:?}", case.expected.order));
    }
    if case.expected.metric != "unsupported" {
        let target = case.expected.target.as_str();
        let name_empty = case.expected.name.trim().is_empty();
        match target {
            "none" if !name_empty => return Err(format!("{id}: a name is set but target is none")),
            "artist" | "album" | "track" if name_empty => {
                return Err(format!("{id}: target {target} needs a literal name"));
            }
            _ => {}
        }
    }
    Ok(())
}

fn canonical_limit(value: &str) -> Option<&'static str> {
    match value {
        "none" | "0" => Some("none"),
        "one" | "1" => Some("one"),
        "five" | "5" => Some("five"),
        "ten" | "10" => Some("ten"),
        "twenty_five" | "25" => Some("twenty_five"),
        "fifty" | "50" => Some("fifty"),
        "hundred" | "100" => Some("hundred"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// One observation
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct Observation {
    schema_valid: bool,
    refused: bool,
    slots: Slots,
    latency_ms: f64,
    ttft_ms: f64,
    error: Option<String>,
}

/// The eight model-facing values, as the strings the schema uses.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Slots {
    metric: String,
    target: String,
    name: String,
    period: String,
    weekday: String,
    group_by: String,
    limit: String,
    order: String,
}

impl Default for Slots {
    fn default() -> Self {
        Self {
            metric: String::new(),
            target: "none".into(),
            name: String::new(),
            period: "all_time".into(),
            weekday: "any".into(),
            group_by: "none".into(),
            limit: "none".into(),
            order: "desc".into(),
        }
    }
}

fn observe(session: &mut Session, case: &Case) -> Observation {
    let prompt = prompt::build_prompt(&case.question);
    let generation = match session.run(&prompt) {
        Ok(generation) => generation,
        Err(error) => {
            return Observation {
                error: Some(error.to_string()),
                ..Observation::default()
            };
        }
    };
    observation_from_generation(&generation)
}

fn observation_from_generation(generation: &Generation) -> Observation {
    let mut observation = Observation {
        latency_ms: generation.total_ms,
        ttft_ms: generation.ttft_ms,
        ..Observation::default()
    };
    let output: ModelOutput = match serde_json::from_str(generation.text.trim()) {
        Ok(output) => output,
        Err(error) => {
            observation.error = Some(format!("schema: {error}"));
            return observation;
        }
    };
    observation.schema_valid = true;
    match output.into_parsed() {
        Ok(ParsedQuestion::Unsupported { .. }) => {
            observation.refused = true;
            observation.slots.metric = "unsupported".into();
        }
        Ok(ParsedQuestion::Query(query)) => {
            observation.slots = Slots::from_query(&query);
        }
        // The local model's schema has no steps or SQL.
        Ok(ParsedQuestion::Plan(_)) | Ok(ParsedQuestion::Sql(_)) => {
            observation.error = Some("validation: unexpected plan".into());
        }
        Err(error) => {
            observation.error = Some(format!("validation: {error}"));
        }
    }
    observation
}

impl Slots {
    fn from_query(query: &Query) -> Self {
        let (target, name) = match &query.entity {
            Some(entity) => (enum_name(&entity.kind), entity.name.trim().to_string()),
            None => ("none".to_string(), String::new()),
        };
        Self {
            metric: enum_name(&query.metric),
            target,
            name,
            period: enum_name(&query.period),
            weekday: query
                .weekday
                .map(|weekday| enum_name(&weekday))
                .unwrap_or_else(|| "any".into()),
            group_by: query
                .group_by
                .map(|group_by| enum_name(&group_by))
                .unwrap_or_else(|| "none".into()),
            limit: query.limit.map(limit_name).unwrap_or("none").into(),
            order: enum_name(&query.order),
        }
    }
}

fn enum_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn limit_name(limit: Limit) -> &'static str {
    match limit {
        Limit::One => "one",
        Limit::Five => "five",
        Limit::Ten => "ten",
        Limit::TwentyFive => "twenty_five",
        Limit::Fifty => "fifty",
        Limit::Hundred => "hundred",
    }
}

fn slot_value<'a>(slots: &'a Slots, slot: &str) -> &'a str {
    match slot {
        "target" => &slots.target,
        "name" => &slots.name,
        "period" => &slots.period,
        "weekday" => &slots.weekday,
        "group_by" => &slots.group_by,
        "limit" => &slots.limit,
        "order" => &slots.order,
        _ => "",
    }
}

fn expected_value<'a>(expected: &'a Expected, slot: &str) -> &'a str {
    match slot {
        "target" => &expected.target,
        "name" => &expected.name,
        "period" => &expected.period,
        "weekday" => &expected.weekday,
        "group_by" => &expected.group_by,
        "limit" => &expected.limit,
        "order" => &expected.order,
        _ => "",
    }
}

/// A slot counts only when the expected or the predicted interpretation makes
/// it meaningful. Defaults such as `period: all_time`, `weekday: any` and
/// `group_by: none` are agreement noise, not skill.
fn slot_active(slot: &str, value: &str) -> bool {
    match slot {
        "target" => value != "none",
        "name" => !normalize_name(value).is_empty(),
        "period" => value != "all_time",
        "weekday" => value != "any",
        "group_by" => value != "none",
        "limit" => value != "none",
        "order" => value == "asc",
        _ => false,
    }
}

fn canonicalize(slot: &str, value: &str) -> String {
    match slot {
        "name" => normalize_name(value),
        "limit" => canonical_limit(value).unwrap_or("none").to_string(),
        _ => value.to_string(),
    }
}

fn normalize_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| match character {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            '\u{00a0}' => ' ',
            other => other,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct Counts {
    cases: usize,
    schema_valid: usize,
    intent_correct: usize,
    exact_correct: usize,
    expected_refusals: usize,
    predicted_refusals: usize,
    refusal_tp: usize,
    refusal_fp: usize,
    refusal_fn: usize,
    slot_tp: usize,
    slot_fp: usize,
    slot_fn: usize,
    metric_wrong: usize,
    period_wrong: usize,
    name_wrong: usize,
    weekday_wrong: usize,
    injection_failures: usize,
    empty_failures: usize,
    errors: usize,
    latencies: Vec<f64>,
    ttfts: Vec<f64>,
    failed_ids: Vec<String>,
}

impl Counts {
    fn add(&mut self, case: &Case, observation: &Observation) {
        self.cases += 1;
        if observation.schema_valid {
            self.schema_valid += 1;
        }
        let expected_refused = case.expected.metric == "unsupported";
        if expected_refused {
            self.expected_refusals += 1;
        }
        if observation.refused {
            self.predicted_refusals += 1;
        }
        match (expected_refused, observation.refused) {
            (true, true) => self.refusal_tp += 1,
            (true, false) => self.refusal_fn += 1,
            (false, true) => self.refusal_fp += 1,
            (false, false) => {}
        }

        let intent_correct = observation.slots.metric == case.expected.metric;
        if intent_correct {
            self.intent_correct += 1;
        }
        if observation.error.is_some() {
            self.errors += 1;
        }
        if observation.schema_valid {
            self.latencies.push(observation.latency_ms);
            self.ttfts.push(observation.ttft_ms);
        }

        let mut case_failed = !observation.schema_valid || !intent_correct;
        if observation.slots.metric != case.expected.metric && !expected_refused {
            self.metric_wrong += 1;
        }

        if !expected_refused {
            // Exact match is over the filter slots; the metric is scored
            // separately as `intent`, so a wrong metric never hides a correct
            // filter interpretation (and vice versa).
            let exact = SLOTS.iter().all(|slot| {
                canonicalize(slot, slot_value(&observation.slots, slot))
                    == canonicalize(slot, expected_value(&case.expected, slot))
            });
            if exact {
                self.exact_correct += 1;
            }

            for slot in SLOTS {
                let expected = canonicalize(slot, expected_value(&case.expected, slot));
                let predicted = canonicalize(slot, slot_value(&observation.slots, slot));
                let expected_active = slot_active(slot, &expected);
                let predicted_active = slot_active(slot, &predicted);
                match (expected_active, predicted_active) {
                    (true, true) if expected == predicted => self.slot_tp += 1,
                    (true, true) => {
                        self.slot_fn += 1;
                        self.slot_fp += 1;
                        case_failed = true;
                    }
                    (true, false) => {
                        self.slot_fn += 1;
                        case_failed = true;
                    }
                    (false, true) => {
                        self.slot_fp += 1;
                        case_failed = true;
                    }
                    (false, false) => {}
                }
                if expected != predicted {
                    match slot {
                        "period" => self.period_wrong += 1,
                        "name" => self.name_wrong += 1,
                        "weekday" => self.weekday_wrong += 1,
                        _ => {}
                    }
                }
            }
        } else if !observation.refused {
            case_failed = true;
        }

        if case_failed {
            self.failed_ids.push(case.id.clone());
        }
        if case.tags.iter().any(|tag| tag == "injection") && !observation.refused {
            self.injection_failures += 1;
        }
        if case
            .tags
            .iter()
            .any(|tag| tag == "empty" || tag == "gibberish")
            && !observation.refused
        {
            self.empty_failures += 1;
        }
    }

    fn summary(&self) -> Summary {
        let slot_precision = ratio(self.slot_tp, self.slot_tp + self.slot_fp);
        let slot_recall = ratio(self.slot_tp, self.slot_tp + self.slot_fn);
        let refusal_true_negatives = self
            .cases
            .saturating_sub(self.expected_refusals + self.predicted_refusals)
            .saturating_add(self.refusal_tp);
        Summary {
            cases: self.cases,
            schema_valid: self.schema_valid,
            schema_valid_rate: ratio(self.schema_valid, self.cases),
            intent: Accuracy {
                correct: self.intent_correct,
                accuracy: ratio(self.intent_correct, self.cases),
            },
            slots: SlotScores {
                tp: self.slot_tp,
                fp: self.slot_fp,
                fn_: self.slot_fn,
                precision: slot_precision,
                recall: slot_recall,
                f1: f1(slot_precision, slot_recall),
            },
            refusals: RefusalScores {
                expected: self.expected_refusals,
                predicted: self.predicted_refusals,
                tp: self.refusal_tp,
                fp: self.refusal_fp,
                fn_: self.refusal_fn,
                precision: ratio(self.refusal_tp, self.predicted_refusals),
                recall: ratio(self.refusal_tp, self.expected_refusals),
                accuracy: ratio(self.refusal_tp + refusal_true_negatives, self.cases),
            },
            exact_match: Accuracy {
                correct: self.exact_correct,
                accuracy: ratio(self.exact_correct, self.cases - self.expected_refusals),
            },
            latency: Latency {
                p50_ms: percentile(&self.latencies, 50.0),
                p95_ms: percentile(&self.latencies, 95.0),
                ttft_p50_ms: percentile(&self.ttfts, 50.0),
                ttft_p95_ms: percentile(&self.ttfts, 95.0),
                mean_ms: round(mean(&self.latencies), 1),
            },
            errors: self.errors,
            failures: FailureCounts {
                metric: self.metric_wrong,
                period: self.period_wrong,
                name: self.name_wrong,
                weekday: self.weekday_wrong,
                refusal: self.refusal_fp + self.refusal_fn,
                injection: self.injection_failures,
                empty: self.empty_failures,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct RunResult {
    schema: u32,
    model: ModelInfo,
    corpus: CorpusInfo,
    overall: Summary,
    by_language: BTreeMap<String, Summary>,
    failed_case_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct ModelInfo {
    label: String,
    file: String,
    threads: u32,
    context_tokens: u32,
    max_new_tokens: i32,
}

#[derive(Clone, Debug, Serialize)]
struct CorpusInfo {
    path: String,
    cases: usize,
    by_language: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize)]
struct Summary {
    cases: usize,
    schema_valid: usize,
    schema_valid_rate: f64,
    intent: Accuracy,
    slots: SlotScores,
    refusals: RefusalScores,
    exact_match: Accuracy,
    latency: Latency,
    errors: usize,
    failures: FailureCounts,
}

#[derive(Clone, Debug, Serialize)]
struct Accuracy {
    correct: usize,
    accuracy: f64,
}

#[derive(Clone, Debug, Serialize)]
struct SlotScores {
    tp: usize,
    fp: usize,
    #[serde(rename = "fn")]
    fn_: usize,
    precision: f64,
    recall: f64,
    f1: f64,
}

#[derive(Clone, Debug, Serialize)]
struct RefusalScores {
    expected: usize,
    predicted: usize,
    tp: usize,
    fp: usize,
    #[serde(rename = "fn")]
    fn_: usize,
    precision: f64,
    recall: f64,
    accuracy: f64,
}

#[derive(Clone, Debug, Serialize)]
struct Latency {
    p50_ms: f64,
    p95_ms: f64,
    ttft_p50_ms: f64,
    ttft_p95_ms: f64,
    mean_ms: f64,
}

#[derive(Clone, Debug, Serialize)]
struct FailureCounts {
    metric: usize,
    period: usize,
    name: usize,
    weekday: usize,
    refusal: usize,
    injection: usize,
    empty: usize,
}

impl RunResult {
    fn markdown(&self) -> String {
        let mut report = String::new();
        // ASCII only: the report is read on Windows consoles whose default
        // codepage is not UTF-8.
        report.push_str(&format!("# Ask eval - {}\n\n", self.model.label));
        report.push_str(&format!(
            "- Model file: `{}`\n- Threads: {}\n- Context: {} tokens, greedy, max {} new tokens\n",
            self.model.file,
            self.model.threads,
            self.model.context_tokens,
            self.model.max_new_tokens
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
            "| set | cases | JSON valid | intent | slot P | slot R | slot F1 | refusal P | refusal R | refusal acc | exact | p50 | p95 | TTFT p50 | TTFT p95 |\n\
             | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
        );
        report.push_str(&row("all", &self.overall));
        for (language, summary) in &self.by_language {
            report.push_str(&row(language, summary));
        }

        let failures = &self.overall.failures;
        report.push_str(&format!(
            "\nFailures: metric {}, period {}, name {}, weekday {}, refusal {}, injection {}, empty/gibberish {}.\n",
            failures.metric,
            failures.period,
            failures.name,
            failures.weekday,
            failures.refusal,
            failures.injection,
            failures.empty
        ));
        if !self.failed_case_ids.is_empty() {
            report.push_str(&format!(
                "\nFailed cases: {}.\n",
                self.failed_case_ids.join(", ")
            ));
        }
        report
    }
}

fn row(label: &str, summary: &Summary) -> String {
    format!(
        "| {label} | {} | {}/{} | {:.2}% | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.0} ms | {:.0} ms | {:.0} ms | {:.0} ms |\n",
        summary.cases,
        summary.schema_valid,
        summary.cases,
        summary.intent.accuracy * 100.0,
        summary.slots.precision,
        summary.slots.recall,
        summary.slots.f1,
        summary.refusals.precision,
        summary.refusals.recall,
        summary.refusals.accuracy,
        summary.exact_match.accuracy,
        summary.latency.p50_ms,
        summary.latency.p95_ms,
        summary.latency.ttft_p50_ms,
        summary.latency.ttft_p95_ms,
    )
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        round(numerator as f64 / denominator as f64, 4)
    }
}

fn f1(precision: f64, recall: f64) -> f64 {
    if precision + recall == 0.0 {
        0.0
    } else {
        round(2.0 * precision * recall / (precision + recall), 4)
    }
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

/// Nearest-rank percentile, matching the spike methodology.
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

// ---------------------------------------------------------------------------
// SQL mode: score free questions by result on the fixed fixture
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
struct SqlCase {
    id: String,
    lang: String,
    question: String,
    /// `expressible` when the frozen guided vocabulary can answer the question.
    guided: String,
    #[serde(default)]
    tags: Vec<String>,
    /// The query that defines the expected answer. The model-free test runs it
    /// against the fixture and asserts that it returns `expected`.
    reference_sql: String,
    expected: ExpectedTable,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct ExpectedTable {
    columns: Vec<String>,
    rows: Vec<Vec<Cell>>,
}

/// A typed expected cell, written as a JSON scalar in the corpus.
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
    if !matches!(case.guided.as_str(), "expressible" | "not_expressible") {
        return Err(format!(
            "{id}: guided must be expressible or not_expressible, got {:?}",
            case.guided
        ));
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

/// Compare a typed SQL result to the expected table. Rows are ordered: a
/// ranking is scored on its order, and every reference query orders it
/// deterministically.
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

fn is_repairable_rule(rule: SandboxRule) -> bool {
    !matches!(
        rule,
        SandboxRule::Cancelled | SandboxRule::Timeout | SandboxRule::ProjectionUnavailable
    )
}

/// Validate the model's JSON and honour an explicit refusal.
fn parse_sql_output(reply: &str) -> Result<Option<SqlModelOutput>, String> {
    let output: SqlModelOutput =
        serde_json::from_str(reply).map_err(|error| format!("schema: {error}"))?;
    if output.answer_kind == SqlAnswerKind::Unsupported || output.sql.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(output))
}

async fn observe_sql_case(
    config: &RemoteConfig,
    case: &SqlCase,
    projection: &Path,
    timeout: Duration,
) -> SqlObservation {
    let fail = |detail: String| SqlObservation {
        outcome: Outcome::HonestFailure,
        rule_code: None,
        detail: Some(detail),
    };
    let reply = match remote::infer_sql(config, &case.question, None, fixture::TODAY, timeout).await
    {
        Ok(reply) => reply,
        Err(error) => return fail(format!("request: {error}")),
    };
    let mut output = match parse_sql_output(&reply) {
        Ok(Some(output)) => output,
        Ok(None) => return fail("unsupported".into()),
        Err(error) => return fail(error),
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
                    },
                    Err(detail) => SqlObservation {
                        outcome: Outcome::WrongButPlausible,
                        rule_code: None,
                        detail: Some(detail),
                    },
                };
            }
            Err(rejection) => {
                if is_repairable_rule(rejection.rule) && round < MAX_SQL_REPAIR_ROUNDS {
                    attempts.push((output.sql.clone(), rejection.model_error.clone()));
                    let references: Vec<(&str, &str)> = attempts
                        .iter()
                        .map(|(sql, error)| (sql.as_str(), error.as_str()))
                        .collect();
                    let repaired = remote::infer_sql_repair(
                        config,
                        &case.question,
                        None,
                        &references,
                        fixture::TODAY,
                        timeout,
                    )
                    .await;
                    match repaired {
                        Ok(reply) => match parse_sql_output(&reply) {
                            Ok(Some(next)) => {
                                output = next;
                                continue;
                            }
                            Ok(None) => return fail("unsupported after repair".into()),
                            Err(error) => return fail(format!("repair output: {error}")),
                        },
                        Err(error) => return fail(format!("repair request: {error}")),
                    }
                }
                return SqlObservation {
                    outcome: Outcome::HonestFailure,
                    rule_code: Some(rejection.rule_name().to_owned()),
                    detail: Some(rejection.model_error),
                };
            }
        }
    }
    fail("repair rounds exhausted".into())
}

#[derive(Clone, Debug, Default)]
struct SqlCounts {
    cases: usize,
    correct: usize,
    honest_failure: usize,
    wrong_but_plausible: usize,
}

impl SqlCounts {
    fn add(&mut self, outcome: Outcome) {
        self.cases += 1;
        match outcome {
            Outcome::Correct => self.correct += 1,
            Outcome::HonestFailure => self.honest_failure += 1,
            Outcome::WrongButPlausible => self.wrong_but_plausible += 1,
        }
    }

    fn summary(&self) -> SqlSummary {
        SqlSummary {
            cases: self.cases,
            correct: self.correct,
            honest_failure: self.honest_failure,
            wrong_but_plausible: self.wrong_but_plausible,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct SqlCaseResult {
    id: String,
    lang: String,
    guided: String,
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
    mode: &'static str,
    fixture: SqlFixtureInfo,
    model: SqlModelInfo,
    corpus: SqlCorpusInfo,
    overall: SqlSummary,
    by_language: BTreeMap<String, SqlSummary>,
    by_guided: BTreeMap<String, SqlSummary>,
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
    base_url_host: String,
    model: String,
}

#[derive(Clone, Debug, Serialize)]
struct SqlCorpusInfo {
    path: String,
    cases: usize,
    by_language: BTreeMap<String, usize>,
    by_guided: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize)]
struct SqlSummary {
    cases: usize,
    correct: usize,
    honest_failure: usize,
    wrong_but_plausible: usize,
}

impl SqlRunResult {
    fn markdown(&self) -> String {
        let mut report = String::new();
        report.push_str(&format!("# Ask SQL eval - {}\n\n", self.model.label));
        report.push_str(&format!(
            "- Mode: free questions (SQL), scored by typed result\n- Server: `{}`\n- Model: `{}`\n- Fixture: version {}, today `{}`\n",
            self.model.base_url_host, self.model.model, self.fixture.version, self.fixture.today
        ));
        let languages = self
            .corpus
            .by_language
            .iter()
            .map(|(language, count)| format!("{language} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        let guided = self
            .corpus
            .by_guided
            .iter()
            .map(|(kind, count)| format!("{kind} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        report.push_str(&format!(
            "- Corpus: `{}` ({} cases: {languages}; {guided})\n\n",
            self.corpus.path, self.corpus.cases
        ));
        report.push_str(
            "| set | cases | correct | honest failure | wrong but plausible |\n\
             | --- | ---: | ---: | ---: | ---: |\n",
        );
        report.push_str(&sql_row("all", &self.overall));
        for (language, summary) in &self.by_language {
            report.push_str(&sql_row(language, summary));
        }
        for (kind, summary) in &self.by_guided {
            report.push_str(&sql_row(kind, summary));
        }
        let failures: Vec<&SqlCaseResult> = self
            .cases
            .iter()
            .filter(|case| case.outcome != "correct")
            .collect();
        if failures.is_empty() {
            report.push_str("\nEvery case returned the expected result.\n");
        } else {
            report.push_str("\n| case | language | guided vocabulary | outcome | rule | detail |\n| --- | --- | --- | --- | --- | --- |\n");
            for case in failures {
                report.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} |\n",
                    case.id,
                    case.lang,
                    case.guided,
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
        "| {label} | {} | {} | {} | {} |\n",
        summary.cases, summary.correct, summary.honest_failure, summary.wrong_but_plausible
    )
}

fn run_sql(args: &Args) -> Result<(), String> {
    let cases = load_sql_corpus(&args.sql_corpus)?;
    if cases.is_empty() {
        return Err("the SQL corpus is empty".into());
    }
    let cases = match args.limit {
        Some(limit) if limit < cases.len() => cases[..limit].to_vec(),
        _ => cases,
    };

    let base_url = args.base_url.clone().unwrap_or_default();
    let host = remote::endpoint(&base_url)
        .map(|url| url.host_str().unwrap_or("?").to_owned())
        .map_err(|error| format!("invalid --base-url: {error}"))?;
    let config = RemoteConfig {
        base_url,
        model: args.remote_model.clone().unwrap_or_default(),
        api_key: args
            .api_key
            .clone()
            .or_else(|| std::env::var("SCROBBL_ASK_EVAL_API_KEY").ok())
            .filter(|key| !key.trim().is_empty()),
    };

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| format!("could not start the async runtime: {error}"))?;
    runtime.block_on(run_sql_cases(&config, host, args, &cases))
}

async fn run_sql_cases(
    config: &RemoteConfig,
    host: String,
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
    let evaluation = evaluate_sql(config, &host, args, cases, &data_dir).await;
    let _ = std::fs::remove_dir_all(&data_dir);
    evaluation
}

async fn evaluate_sql(
    config: &RemoteConfig,
    host: &str,
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
    let mut by_guided: BTreeMap<String, SqlCounts> = BTreeMap::new();
    let mut results = Vec::with_capacity(cases.len());
    for (index, case) in cases.iter().enumerate() {
        let observation =
            observe_sql_case(config, case, fixture.projection_path(), args.timeout).await;
        eprintln!(
            "ask-eval: {}/{} {} {}",
            index + 1,
            cases.len(),
            case.id,
            observation.outcome.as_str()
        );
        overall.add(observation.outcome);
        by_language
            .entry(case.lang.clone())
            .or_default()
            .add(observation.outcome);
        by_guided
            .entry(case.guided.clone())
            .or_default()
            .add(observation.outcome);
        results.push(SqlCaseResult {
            id: case.id.clone(),
            lang: case.lang.clone(),
            guided: case.guided.clone(),
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
    let corpus_guided = cases.iter().fold(BTreeMap::new(), |mut acc, case| {
        *acc.entry(case.guided.clone()).or_insert(0) += 1;
        acc
    });

    let result = SqlRunResult {
        schema: 1,
        mode: "sql",
        fixture: SqlFixtureInfo {
            version: fixture::FIXTURE_VERSION,
            today: fixture::TODAY.to_owned(),
        },
        model: SqlModelInfo {
            label: args.label.clone(),
            base_url_host: host.to_owned(),
            model: config.model.clone(),
        },
        corpus: SqlCorpusInfo {
            path: args.sql_corpus.display().to_string(),
            cases: cases.len(),
            by_language: corpus_languages,
            by_guided: corpus_guided,
        },
        overall: overall.summary(),
        by_language: by_language
            .iter()
            .map(|(language, counts)| (language.clone(), counts.summary()))
            .collect(),
        by_guided: by_guided
            .iter()
            .map(|(kind, counts)| (kind.clone(), counts.summary()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const CORPUS: &str = include_str!("../../evals/ask/corpus.jsonl");

    #[test]
    fn limits_accept_the_grammar_names_and_the_numeric_shorthand() {
        assert_eq!(canonical_limit("one"), Some("one"));
        assert_eq!(canonical_limit("1"), Some("one"));
        assert_eq!(canonical_limit("100"), Some("hundred"));
        assert_eq!(canonical_limit("3"), None);
    }

    #[test]
    fn default_slots_are_not_counted_as_active() {
        for slot in SLOTS {
            let value = match slot {
                "target" => "none",
                "name" => "",
                "period" => "all_time",
                "weekday" => "any",
                "group_by" => "none",
                "limit" => "none",
                "order" => "desc",
                _ => unreachable!(),
            };
            assert!(!slot_active(slot, value), "{slot} should be inactive");
        }
        assert!(slot_active("order", "asc"));
        assert!(slot_active("limit", "one"));
    }

    #[test]
    fn names_are_compared_with_ascii_punctuation() {
        assert_eq!(normalize_name("  Björk "), "Björk");
        assert_eq!(
            normalize_name("Don\u{2019}t Stop Me Now"),
            "Don't Stop Me Now"
        );
        assert_eq!(normalize_name("Sigur  Rós"), "Sigur Rós");
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let values = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(percentile(&values, 50.0), 20.0);
        assert_eq!(percentile(&values, 95.0), 40.0);
        assert_eq!(percentile(&[], 50.0), 0.0);
    }

    #[test]
    fn the_versioned_corpus_is_valid_and_balanced() {
        let cases = parse_corpus(CORPUS, "corpus").expect("the corpus must parse");
        assert!(
            (150..=300).contains(&cases.len()),
            "expected 150-300 cases, got {}",
            cases.len()
        );
        let mut ids = BTreeSet::new();
        for case in &cases {
            assert!(ids.insert(&case.id), "duplicate id {}", case.id);
        }
        let english = cases.iter().filter(|case| case.lang == "en").count();
        let italian = cases.iter().filter(|case| case.lang == "it").count();
        assert_eq!(english + italian, cases.len());
        let difference = english.abs_diff(italian);
        assert!(
            difference <= 4,
            "languages are unbalanced: {english}/{italian}"
        );
    }

    #[test]
    fn the_corpus_covers_every_grammar_value() {
        let cases = parse_corpus(CORPUS, "corpus").expect("the corpus must parse");
        let metrics: BTreeSet<&str> = cases
            .iter()
            .map(|case| case.expected.metric.as_str())
            .collect();
        for value in schema::ModelMetric::VALUES {
            assert!(metrics.contains(value), "no case uses metric {value}");
        }
        let targets: BTreeSet<&str> = cases
            .iter()
            .map(|case| case.expected.target.as_str())
            .collect();
        for value in schema::ModelTarget::VALUES {
            assert!(targets.contains(value), "no case uses target {value}");
        }
        let periods: BTreeSet<&str> = cases
            .iter()
            .map(|case| case.expected.period.as_str())
            .collect();
        for value in schema::ModelPeriod::VALUES {
            assert!(periods.contains(value), "no case uses period {value}");
        }
        let weekdays: BTreeSet<&str> = cases
            .iter()
            .map(|case| case.expected.weekday.as_str())
            .collect();
        for value in schema::ModelWeekday::VALUES {
            assert!(weekdays.contains(value), "no case uses weekday {value}");
        }
        let groupings: BTreeSet<&str> = cases
            .iter()
            .map(|case| case.expected.group_by.as_str())
            .collect();
        for value in schema::ModelGroupBy::VALUES {
            assert!(groupings.contains(value), "no case uses group_by {value}");
        }
        let limits: BTreeSet<&str> = cases
            .iter()
            .map(|case| canonical_limit(&case.expected.limit).expect("valid limit"))
            .collect();
        for value in schema::ModelLimit::VALUES {
            assert!(limits.contains(value), "no case uses limit {value}");
        }
        let orders: BTreeSet<&str> = cases
            .iter()
            .map(|case| case.expected.order.as_str())
            .collect();
        assert!(orders.contains("asc"));
        assert!(orders.contains("desc"));
        for tag in ["out_of_scope", "injection", "empty", "gibberish"] {
            assert!(
                cases.iter().any(|case| case.tags.iter().any(|t| t == tag)),
                "no case is tagged {tag}"
            );
        }
        assert!(cases.iter().any(|case| {
            case.tags.iter().any(|tag| tag == "unicode") || !case.expected.name.is_ascii()
        }));
    }

    const SQL_CORPUS: &str = include_str!("../../evals/ask/sql_corpus.jsonl");

    #[test]
    fn the_sql_corpus_is_valid_balanced_and_covers_both_modes() {
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
        for guided in ["expressible", "not_expressible"] {
            assert!(
                cases.iter().any(|case| case.guided == guided),
                "no case is guided {guided}"
            );
        }
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
        // Relative wording would make the expected value depend on the day the
        // evaluation runs; the fixture and the corpus stay fixed in time.
        for case in &cases {
            let question = case.question.to_lowercase();
            for relative in [
                "today",
                "yesterday",
                "this year",
                "last year",
                "quest'anno",
                "l'anno scorso",
                "ultimi",
            ] {
                assert!(
                    !question.contains(relative),
                    "{}: relative wording {relative:?} is not scored",
                    case.id
                );
            }
        }
    }

    #[test]
    fn typed_results_are_compared_by_value_and_type() {
        let expected = ExpectedTable {
            columns: vec!["plays".into(), "note".into()],
            rows: vec![vec![Cell::Integer(10), Cell::Text("ok".into())]],
        };
        let matching = sandbox::SandboxOutput {
            columns: vec!["plays".into(), "note".into()],
            rows: vec![vec![SqlValue::Integer(10), SqlValue::Text("ok".into())]],
            truncated: false,
        };
        assert_eq!(compare_result(&expected, &matching), Ok(()));

        let numeric = sandbox::SandboxOutput {
            columns: vec!["plays".into(), "note".into()],
            rows: vec![vec![SqlValue::Real(10.0), SqlValue::Text("ok".into())]],
            truncated: false,
        };
        assert_eq!(compare_result(&expected, &numeric), Ok(()));

        let wrong_value = sandbox::SandboxOutput {
            columns: vec!["plays".into(), "note".into()],
            rows: vec![vec![SqlValue::Integer(11), SqlValue::Text("ok".into())]],
            truncated: false,
        };
        assert!(compare_result(&expected, &wrong_value).is_err());

        let wrong_rows = sandbox::SandboxOutput {
            columns: vec!["plays".into(), "note".into()],
            rows: Vec::new(),
            truncated: false,
        };
        assert!(compare_result(&expected, &wrong_rows).is_err());

        let truncated = sandbox::SandboxOutput {
            columns: vec!["plays".into(), "note".into()],
            rows: vec![vec![SqlValue::Integer(10), SqlValue::Text("ok".into())]],
            truncated: true,
        };
        assert!(compare_result(&expected, &truncated).is_err());
    }

    #[test]
    fn outcomes_add_up_per_set() {
        let mut counts = SqlCounts::default();
        counts.add(Outcome::Correct);
        counts.add(Outcome::HonestFailure);
        counts.add(Outcome::WrongButPlausible);
        let summary = counts.summary();
        assert_eq!(summary.cases, 3);
        assert_eq!(summary.correct, 1);
        assert_eq!(summary.honest_failure, 1);
        assert_eq!(summary.wrong_but_plausible, 1);
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
