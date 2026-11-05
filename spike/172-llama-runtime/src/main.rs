//! Throwaway measurement harness for issue #172.
//!
//! It loads one GGUF model, constrains generation with a GBNF grammar generated
//! from `schema/ask.schema.json`, runs the fixed corpus and writes a JSON
//! report plus a Markdown summary. It is not part of the scrobbl-rs build.

mod corpus;
mod grammar;
mod prompt;
mod schema;
mod stats;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Read as _;
use std::num::{NonZeroU32, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::{LogOptions, send_logs_to_tracing};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::prompt::ChatMode;

const LLAMA_CPP_2_VERSION: &str = "0.1.161";

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

/// Set `SCROBBL_SPIKE_TRACE=1` to see which llama.cpp call is running.
fn trace(message: impl std::fmt::Display) {
    if std::env::var_os("SCROBBL_SPIKE_TRACE").is_some() {
        eprintln!("trace: {message}");
    }
}

fn run() -> Result<()> {
    let args = Args::parse()?;

    // llama.cpp logs every graph tensor at INFO on this build; route them to
    // `tracing` (no subscriber) so a 30-case run stays readable. Set
    // SCROBBL_SPIKE_LLAMA_LOGS=1 when debugging a runtime failure.
    if std::env::var_os("SCROBBL_SPIKE_LLAMA_LOGS").is_none() {
        send_logs_to_tracing(LogOptions::default().with_logs_enabled(false));
    }

    let schema_text = fs::read_to_string(&args.schema)
        .with_context(|| format!("reading schema {}", args.schema.display()))?;
    let schema: Value = serde_json::from_str(&schema_text)
        .with_context(|| format!("parsing schema {}", args.schema.display()))?;
    let gbnf = grammar::to_gbnf(&schema).context("generating GBNF from the trial schema")?;
    if let Some(path) = &args.emit_grammar {
        write_file(path, gbnf.as_bytes())?;
        eprintln!("wrote grammar {}", path.display());
    }

    let cases = corpus::load(&args.corpus)?;
    if cases.is_empty() {
        bail!("corpus {} is empty", args.corpus.display());
    }

    let compact_schema = serde_json::to_string(&schema)?;
    let system = prompt::system_prompt(&compact_schema);

    let backend = LlamaBackend::init()?;

    let model_params = LlamaModelParams::default();
    let load_start = Instant::now();
    trace(format!("loading model {}", args.model.display()));
    let model = LlamaModel::load_from_file(&backend, &args.model, &model_params)
        .with_context(|| format!("loading model {}", args.model.display()))?;
    let load_ms = millis(load_start.elapsed());
    trace(format!("model loaded in {load_ms:.0} ms"));

    let context_start = Instant::now();
    let n_ctx = NonZeroU32::new(args.ctx).context("--ctx must be greater than zero")?;
    let context_params = LlamaContextParams::default()
        .with_n_ctx(Some(n_ctx))
        .with_n_threads(args.threads)
        .with_n_threads_batch(args.threads)
        // llama_context_default_params() disables perf measurement.
        .with_no_perf(false);
    trace("creating context");
    let mut context = model
        .new_context(&backend, context_params)
        .context("creating the llama context")?;
    let context_create_ms = millis(context_start.elapsed());
    trace(format!("context created in {context_create_ms:.0} ms"));

    // Warm up the tokenizer/CPU path; the report keeps these runs out of the
    // latency sample but still clears the KV cache so each case starts clean.
    for case in cases.iter().take(args.warmup.min(cases.len())) {
        let prompt = prompt::build(args.chat, &system, &case.question);
        if let Err(error) = generate(
            &model,
            &mut context,
            &prompt,
            &gbnf,
            args.max_tokens,
            args.no_grammar,
        ) {
            eprintln!("warmup case {} failed: {error:#}", case.id);
        }
    }

    let mut results = Vec::with_capacity(cases.len());
    for case in &cases {
        let prompt = prompt::build(args.chat, &system, &case.question);
        let result = match generate(
            &model,
            &mut context,
            &prompt,
            &gbnf,
            args.max_tokens,
            args.no_grammar,
        ) {
            Ok(generation) => evaluate(case, &schema, generation),
            Err(error) => CaseResult {
                id: case.id.clone(),
                lang: case.lang.clone(),
                question: case.question.clone(),
                output: String::new(),
                json_valid: false,
                schema_valid: false,
                expected_match: false,
                error: Some(format!("{error:#}")),
                timings: None,
            },
        };
        results.push(result);
    }

    let model_bytes = fs::metadata(&args.model)
        .with_context(|| format!("reading metadata for {}", args.model.display()))?
        .len();
    let model_sha256 = if args.no_sha256 {
        None
    } else {
        Some(sha256_file(&args.model)?)
    };

    let report = RunReport {
        label: args.label.clone(),
        generated_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        host: host_info(),
        model: ModelInfo {
            path: args.model.display().to_string(),
            bytes: model_bytes,
            sha256: model_sha256,
        },
        runtime: RuntimeInfo {
            llama_cpp_2: LLAMA_CPP_2_VERSION.to_string(),
            harness: env!("CARGO_PKG_VERSION").to_string(),
            load_ms,
            context_create_ms,
            n_ctx: context.n_ctx(),
        },
        config: ConfigInfo {
            chat: args.chat.as_str().to_string(),
            threads: args.threads,
            max_tokens: args.max_tokens,
            warmup: args.warmup,
            grammar_sha256: sha256_bytes(gbnf.as_bytes()),
            system_prompt_sha256: sha256_bytes(system.as_bytes()),
        },
        cases: results,
        summary: Vec::new(),
    };

    let report = RunReport {
        summary: summarize_all(&report.cases),
        ..report
    };

    write_file(&args.out, serde_json::to_vec_pretty(&report)?.as_slice())?;
    if let Some(path) = &args.md_out {
        write_file(path, render_markdown(&report).as_bytes())?;
    }

    println!(
        "wrote {} (model load {load_ms:.0} ms, context {context_create_ms:.0} ms)",
        args.out.display()
    );
    for summary in &report.summary {
        println!(
            "{}: schema {}/{} exact {}/{} e2e p50 {:.0} ms p95 {:.0} ms decode {:.1} tok/s",
            summary.lang,
            summary.schema_valid,
            summary.cases,
            summary.expected_match,
            summary.cases,
            summary.total_ms_p50,
            summary.total_ms_p95,
            summary.decode_tokens_per_s_p50,
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

struct Generation {
    output: String,
    timings: CaseTimings,
}

fn generate(
    model: &LlamaModel,
    context: &mut LlamaContext<'_>,
    prompt_text: &str,
    gbnf: &str,
    max_tokens: i32,
    no_grammar: bool,
) -> Result<Generation> {
    let vocab = model.vocab();
    trace("tokenizing prompt");
    let tokens = vocab.tokenize(prompt_text.as_bytes(), true, true);
    if tokens.is_empty() {
        bail!("tokenizer produced an empty prompt");
    }
    trace(format!("prompt is {} tokens", tokens.len()));

    let n_ctx = context.n_ctx() as i32;
    if tokens.len() as i32 + max_tokens > n_ctx {
        bail!(
            "prompt is {} tokens, max_tokens is {max_tokens}, n_ctx is {n_ctx}",
            tokens.len()
        );
    }

    trace("clearing KV cache");
    context
        .clear_kv_cache_seq(None, None, None)
        .context("clearing the KV cache")?;
    context.reset_timings();

    let mut batch = LlamaBatch::new(tokens.len().max(1), 1);
    let last_index = tokens.len() - 1;
    for (index, token) in tokens.iter().enumerate() {
        batch.add(*token, index as i32, &[0], index == last_index)?;
    }

    let start = Instant::now();
    trace("prefill decode");
    context.decode(&mut batch).context("prompt decode")?;
    trace("prefill done");
    let prefill_wall = start.elapsed();

    trace("creating grammar sampler");
    let mut sampler = if no_grammar {
        LlamaSampler::chain_simple([LlamaSampler::greedy()])
    } else {
        LlamaSampler::chain_simple([
            LlamaSampler::grammar(model, gbnf, "root").context("creating the grammar sampler")?,
            LlamaSampler::greedy(),
        ])
    };
    trace("grammar ready");

    let mut output: Vec<u8> = Vec::new();
    let mut generated = 0i32;
    let mut first_token_ms = None;
    let mut n_cur = tokens.len() as i32;

    loop {
        trace(format!("sampling token at position {n_cur}"));
        // `LlamaSampler::sample` already accepts the token internally in the
        // llama.cpp bundled with llama-cpp-2 0.1.161. Calling `accept` again
        // advances the grammar twice and empties its stack (#1007).
        let token = sampler.sample(context, batch.n_tokens() - 1);
        trace(format!("sampled token {}", token.0));

        if generated == 0 {
            first_token_ms = Some(millis(start.elapsed()));
        }

        if vocab.is_eog(token) {
            trace("sampled EOG");
            break;
        }

        let piece = vocab.token_to_piece(token, true, None);
        output.extend_from_slice(&piece);
        generated += 1;

        if generated >= max_tokens {
            break;
        }

        batch.clear();
        batch.add(token, n_cur, &[0], true)?;
        n_cur += 1;
        trace("decode next token");
        context.decode(&mut batch).context("token decode")?;
    }

    let total = start.elapsed();
    let timings = context.timings();

    let mut prefill_ms = timings.t_p_eval_ms();
    let mut n_p_eval = timings.n_p_eval().max(0) as f64;
    if prefill_ms <= 0.0 {
        prefill_ms = millis(prefill_wall);
        n_p_eval = tokens.len() as f64;
    }
    let decode_ms = timings.t_eval_ms();
    let n_eval = timings.n_eval().max(0) as f64;

    Ok(Generation {
        output: String::from_utf8_lossy(&output).into_owned(),
        timings: CaseTimings {
            prompt_tokens: tokens.len(),
            generated_tokens: generated.max(0) as usize,
            prefill_ms,
            first_token_ms: first_token_ms.unwrap_or_else(|| millis(prefill_wall)),
            decode_ms,
            total_ms: millis(total),
            prefill_tokens_per_s: tokens_per_second(n_p_eval, prefill_ms),
            decode_tokens_per_s: tokens_per_second(n_eval, decode_ms),
        },
    })
}

fn tokens_per_second(tokens: f64, millis: f64) -> f64 {
    if millis <= 0.0 {
        0.0
    } else {
        tokens / (millis / 1000.0)
    }
}

fn evaluate(case: &corpus::Case, schema: &Value, generation: Generation) -> CaseResult {
    let trimmed = generation.output.trim();
    let parsed: std::result::Result<Value, _> = serde_json::from_str(trimmed);

    let (json_valid, value, error) = match parsed {
        Ok(value) => (true, Some(value), None),
        Err(error) => (false, None, Some(format!("invalid JSON: {error}"))),
    };

    let (schema_valid, error) = match (&value, error) {
        (Some(value), _) => match schema::validate(value, schema) {
            Ok(()) => (true, None),
            Err(error) => (false, Some(format!("schema: {error:#}"))),
        },
        (None, error) => (false, error),
    };

    let expected_match = schema_valid
        && value
            .as_ref()
            .is_some_and(|value| case.expected.matches(value));

    CaseResult {
        id: case.id.clone(),
        lang: case.lang.clone(),
        question: case.question.clone(),
        output: generation.output,
        json_valid,
        schema_valid,
        expected_match,
        error,
        timings: Some(generation.timings),
    }
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct RunReport {
    label: String,
    generated_at_unix: u64,
    host: HostInfo,
    model: ModelInfo,
    runtime: RuntimeInfo,
    config: ConfigInfo,
    cases: Vec<CaseResult>,
    summary: Vec<Summary>,
}

#[derive(Serialize)]
struct HostInfo {
    os: String,
    arch: String,
    cpu: Option<String>,
    logical_cores: usize,
    peak_rss_kb: Option<u64>,
}

#[derive(Serialize)]
struct ModelInfo {
    path: String,
    bytes: u64,
    sha256: Option<String>,
}

#[derive(Serialize)]
struct RuntimeInfo {
    llama_cpp_2: String,
    harness: String,
    load_ms: f64,
    context_create_ms: f64,
    n_ctx: u32,
}

#[derive(Serialize)]
struct ConfigInfo {
    chat: String,
    threads: i32,
    max_tokens: i32,
    warmup: usize,
    grammar_sha256: String,
    system_prompt_sha256: String,
}

#[derive(Serialize, Clone)]
struct CaseResult {
    id: String,
    lang: String,
    question: String,
    output: String,
    json_valid: bool,
    schema_valid: bool,
    expected_match: bool,
    error: Option<String>,
    timings: Option<CaseTimings>,
}

#[derive(Serialize, Clone, Copy)]
struct CaseTimings {
    prompt_tokens: usize,
    generated_tokens: usize,
    prefill_ms: f64,
    first_token_ms: f64,
    decode_ms: f64,
    total_ms: f64,
    prefill_tokens_per_s: f64,
    decode_tokens_per_s: f64,
}

#[derive(Serialize)]
struct Summary {
    lang: String,
    cases: usize,
    json_valid: usize,
    schema_valid: usize,
    expected_match: usize,
    validity_rate: f64,
    exact_match_rate: f64,
    total_ms_p50: f64,
    total_ms_p95: f64,
    first_token_ms_p50: f64,
    first_token_ms_p95: f64,
    prefill_tokens_per_s_p50: f64,
    decode_tokens_per_s_p50: f64,
}

fn summarize_all(cases: &[CaseResult]) -> Vec<Summary> {
    let mut summaries = vec![summarize("all", cases)];
    for lang in ["en", "it"] {
        let subset: Vec<CaseResult> = cases
            .iter()
            .filter(|case| case.lang == lang)
            .cloned()
            .collect();
        if !subset.is_empty() {
            summaries.push(summarize(lang, &subset));
        }
    }
    summaries
}

fn summarize(lang: &str, cases: &[CaseResult]) -> Summary {
    let total = cases.len();
    let count =
        |predicate: fn(&CaseResult) -> bool| cases.iter().filter(|case| predicate(case)).count();
    let json_valid = count(|case| case.json_valid);
    let schema_valid = count(|case| case.schema_valid);
    let expected_match = count(|case| case.expected_match);

    let totals = values(cases, |timings| timings.total_ms);
    let first_tokens = values(cases, |timings| timings.first_token_ms);
    let prefills = values(cases, |timings| timings.prefill_tokens_per_s);
    let decodes = values(cases, |timings| timings.decode_tokens_per_s);

    let rate = |n: usize| {
        if total == 0 {
            0.0
        } else {
            n as f64 / total as f64
        }
    };

    Summary {
        lang: lang.to_string(),
        cases: total,
        json_valid,
        schema_valid,
        expected_match,
        validity_rate: rate(schema_valid),
        exact_match_rate: rate(expected_match),
        total_ms_p50: stats::percentile(&totals, 50.0).unwrap_or(0.0),
        total_ms_p95: stats::percentile(&totals, 95.0).unwrap_or(0.0),
        first_token_ms_p50: stats::percentile(&first_tokens, 50.0).unwrap_or(0.0),
        first_token_ms_p95: stats::percentile(&first_tokens, 95.0).unwrap_or(0.0),
        prefill_tokens_per_s_p50: stats::percentile(&prefills, 50.0).unwrap_or(0.0),
        decode_tokens_per_s_p50: stats::percentile(&decodes, 50.0).unwrap_or(0.0),
    }
}

fn values(cases: &[CaseResult], pick: impl Fn(&CaseTimings) -> f64) -> Vec<f64> {
    cases
        .iter()
        .filter_map(|case| case.timings.as_ref().map(&pick))
        .collect()
}

fn render_markdown(report: &RunReport) -> String {
    let mut out = String::new();
    writeln!(out, "# Spike #172 run — {}", report.label).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "- generated at (unix): {}", report.generated_at_unix).unwrap();
    writeln!(
        out,
        "- host: {} / {} / {} / {} logical cores / peak RSS {}",
        report.host.os,
        report.host.arch,
        report.host.cpu.as_deref().unwrap_or("unknown CPU"),
        report.host.logical_cores,
        report
            .host
            .peak_rss_kb
            .map(|kb| format!("{kb} KiB"))
            .unwrap_or_else(|| "n/a".to_string())
    )
    .unwrap();
    writeln!(
        out,
        "- model: `{}` ({} bytes, sha256 {})",
        report.model.path,
        report.model.bytes,
        report.model.sha256.as_deref().unwrap_or("not computed")
    )
    .unwrap();
    writeln!(
        out,
        "- runtime: llama-cpp-2 {} / harness {} / load {:.0} ms / context {:.0} ms / n_ctx {}",
        report.runtime.llama_cpp_2,
        report.runtime.harness,
        report.runtime.load_ms,
        report.runtime.context_create_ms,
        report.runtime.n_ctx
    )
    .unwrap();
    writeln!(
        out,
        "- config: chat `{}`, threads {}, max_tokens {}, warmup {}, grammar sha256 {}, prompt sha256 {}",
        report.config.chat,
        report.config.threads,
        report.config.max_tokens,
        report.config.warmup,
        report.config.grammar_sha256,
        report.config.system_prompt_sha256
    )
    .unwrap();
    writeln!(out).unwrap();

    writeln!(
        out,
        "| language | cases | JSON valid | schema valid | exact match | e2e p50 (ms) | e2e p95 (ms) | first token p50 (ms) | first token p95 (ms) | prefill tok/s p50 | decode tok/s p50 |"
    )
    .unwrap();
    writeln!(
        out,
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
    )
    .unwrap();
    for summary in &report.summary {
        writeln!(
            out,
            "| {} | {} | {}/{} | {}/{} | {}/{} | {:.0} | {:.0} | {:.0} | {:.0} | {:.1} | {:.1} |",
            summary.lang,
            summary.cases,
            summary.json_valid,
            summary.cases,
            summary.schema_valid,
            summary.cases,
            summary.expected_match,
            summary.cases,
            summary.total_ms_p50,
            summary.total_ms_p95,
            summary.first_token_ms_p50,
            summary.first_token_ms_p95,
            summary.prefill_tokens_per_s_p50,
            summary.decode_tokens_per_s_p50,
        )
        .unwrap();
    }
    writeln!(out).unwrap();

    let failures: Vec<&CaseResult> = report
        .cases
        .iter()
        .filter(|case| !case.schema_valid || !case.expected_match)
        .collect();
    writeln!(out, "## Failures ({})", failures.len()).unwrap();
    writeln!(out).unwrap();
    if failures.is_empty() {
        writeln!(out, "None.").unwrap();
    } else {
        writeln!(out, "| id | lang | question | problem | output |").unwrap();
        writeln!(out, "| --- | --- | --- | --- | --- |").unwrap();
        for case in failures {
            let problem = case.error.as_deref().unwrap_or("interpretation mismatch");
            writeln!(
                out,
                "| {} | {} | {} | {} | `{}` |",
                case.id,
                case.lang,
                escape_markdown(&case.question),
                escape_markdown(problem),
                escape_markdown(&truncate(&case.output, 180)),
            )
            .unwrap();
        }
    }

    out
}

fn escape_markdown(value: &str) -> String {
    value
        .replace('|', "\\|")
        .replace('`', "'")
        .replace(['\r', '\n'], " ")
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

// ---------------------------------------------------------------------------
// Host / file helpers
// ---------------------------------------------------------------------------

fn host_info() -> HostInfo {
    HostInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu: cpu_brand(),
        logical_cores: std::thread::available_parallelism()
            .map(NonZeroUsize::get)
            .unwrap_or(0),
        peak_rss_kb: peak_rss_kb(),
    }
}

fn cpu_brand() -> Option<String> {
    if cfg!(target_os = "linux") {
        let text = fs::read_to_string("/proc/cpuinfo").ok()?;
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("model name") {
                return value
                    .split_once(':')
                    .map(|(_, value)| value.trim().to_string());
            }
        }
        None
    } else if cfg!(target_os = "macos") {
        let output = Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    }
}

fn peak_rss_kb() -> Option<u64> {
    let text = fs::read_to_string("/proc/self/status").ok()?;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("VmHWM:") {
            return value.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        fs::File::open(path).with_context(|| format!("opening {} for hashing", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_digest(hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_digest(hasher.finalize())
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

struct Args {
    model: PathBuf,
    corpus: PathBuf,
    schema: PathBuf,
    out: PathBuf,
    md_out: Option<PathBuf>,
    emit_grammar: Option<PathBuf>,
    ctx: u32,
    threads: i32,
    max_tokens: i32,
    warmup: usize,
    chat: ChatMode,
    label: String,
    no_sha256: bool,
    no_grammar: bool,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut raw = HashMap::new();
        let mut iterator = std::env::args().skip(1);
        while let Some(flag) = iterator.next() {
            if flag == "--help" || flag == "-h" {
                print_help();
                std::process::exit(0);
            }
            let Some(name) = flag.strip_prefix("--") else {
                bail!("unexpected argument {flag:?}");
            };
            if name == "no-sha256" {
                raw.insert(name.to_string(), "true".to_string());
                continue;
            }
            if name == "no-grammar" {
                raw.insert(name.to_string(), "true".to_string());
                continue;
            }
            let value = iterator
                .next()
                .with_context(|| format!("missing value for --{name}"))?;
            raw.insert(name.to_string(), value);
        }

        let logical = std::thread::available_parallelism()
            .map(NonZeroUsize::get)
            .unwrap_or(1);
        // One thread per physical core is the safer default on the NAS
        // (i5-10500: 6 cores / 12 threads) and on Apple hybrid CPUs.
        let default_threads = (logical / 2).max(1) as i32;

        let model = PathBuf::from(required(&raw, "model")?);
        let out = PathBuf::from(required(&raw, "out")?);

        Ok(Self {
            model,
            corpus: PathBuf::from(
                raw.get("corpus")
                    .cloned()
                    .unwrap_or_else(|| "corpus/questions.jsonl".to_string()),
            ),
            schema: PathBuf::from(
                raw.get("schema")
                    .cloned()
                    .unwrap_or_else(|| "schema/ask.schema.json".to_string()),
            ),
            out,
            md_out: raw.get("md-out").map(PathBuf::from),
            emit_grammar: raw.get("emit-grammar").map(PathBuf::from),
            ctx: parse_number(&raw, "ctx")?.unwrap_or(2048),
            threads: parse_number(&raw, "threads")?.unwrap_or(default_threads),
            max_tokens: parse_number(&raw, "max-tokens")?.unwrap_or(128),
            warmup: parse_number::<usize>(&raw, "warmup")?.unwrap_or(3),
            chat: ChatMode::parse(
                raw.get("chat")
                    .map(String::as_str)
                    .unwrap_or("qwen3-nothink"),
            )?,
            label: raw
                .get("label")
                .cloned()
                .unwrap_or_else(|| "local".to_string()),
            no_sha256: raw.contains_key("no-sha256"),
            no_grammar: raw.contains_key("no-grammar"),
        })
    }
}

fn required(raw: &HashMap<String, String>, key: &str) -> Result<String> {
    raw.get(key)
        .cloned()
        .with_context(|| format!("missing required --{key}"))
}

fn parse_number<T>(raw: &HashMap<String, String>, key: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    raw.get(key)
        .map(|value| {
            value
                .parse::<T>()
                .with_context(|| format!("--{key} must be a number, got {value:?}"))
        })
        .transpose()
}

fn print_help() {
    println!(
        "scrobbl-ask-spike — issue #172 feasibility harness\n\
         \n\
         required:\n\
           --model <path.gguf>     model to load\n\
           --out <path.json>       JSON report to write\n\
         optional:\n\
           --corpus <path>         default corpus/questions.jsonl\n\
           --schema <path>         default schema/ask.schema.json\n\
           --md-out <path.md>      Markdown summary to write\n\
           --emit-grammar <path>   write the generated GBNF\n\
           --ctx <n>               default 2048\n\
           --threads <n>           default logical cores / 2\n\
           --max-tokens <n>        default 128\n\
           --warmup <n>            default 3\n\
           --chat <mode>           qwen3-nothink (default), qwen3-think, qwen25, plain\n\
           --label <text>          label in the report, default local\n\
           --no-sha256             skip hashing the model file\n\
           --no-grammar            diagnostic only: sample without GBNF\n"
    );
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn pinned_version_matches_manifest() {
        assert!(
            include_str!("../Cargo.toml").contains("=0.1.161"),
            "update LLAMA_CPP_2_VERSION when bumping llama-cpp-2"
        );
    }

    #[test]
    fn shipped_schema_generates_a_grammar() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../schema/ask.schema.json")).unwrap();
        let grammar = crate::grammar::to_gbnf(&schema).unwrap();
        let quoted = |value: &str| format!("\"\\\"{value}\\\"\"");
        for expected in [
            "intent",
            "count_plays",
            "target",
            "artist",
            "weekday",
            "monday",
            "limit",
        ] {
            let expected = quoted(expected);
            assert!(grammar.contains(&expected), "grammar lost {expected}");
        }
        assert!(grammar.contains("string ::= "), "grammar lost string rule");
    }

    #[test]
    fn tokens_per_second_handles_zero_and_positive_durations() {
        assert_eq!(super::tokens_per_second(10.0, 0.0), 0.0);
        assert_eq!(super::tokens_per_second(10.0, 500.0), 20.0);
    }

    #[test]
    fn corpus_expected_values_fit_the_shipped_schema() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../schema/ask.schema.json")).unwrap();
        let cases = crate::corpus::parse(include_str!("../corpus/questions.jsonl")).unwrap();

        for case in cases {
            if case.expected.intent == "unsupported" {
                continue;
            }
            let value = json!({
                "intent": case.expected.intent.as_str(),
                "target": case.expected.target.as_deref().unwrap_or(""),
                "name": case.expected.name.as_deref().unwrap_or(""),
                "period": case.expected.period.as_deref().unwrap_or(""),
                "weekday": case.expected.weekday.as_deref().unwrap_or(""),
                "limit": case.expected.limit.as_deref().unwrap_or(""),
            });
            if let Err(error) = crate::schema::validate(&value, &schema) {
                panic!("{}: {error:#}", case.id);
            }
        }
    }
}
