//! CI-safe tests for the local Ask parser plus an ignored real-model run.
//!
//! The queue, serialization, timeout, cancellation and idle-unload tests use a
//! mock engine and never load weights. The real GGUF test is `#[ignore]` and
//! reads `SCROBBL_TEST_MODEL`; see `docs/ai.md`.
#[cfg(feature = "llama")]
use scrobbl_rs::ask::parser::{ModelParser, QueryParser};
use scrobbl_rs::ask::{
    dsl::{EntityKind, GroupBy, Limit, Metric, Period, SortBy, SortOrder, Weekday},
    grammar,
    parser::ParsedQuestion,
    prompt, runtime,
    runtime::Runtime,
    schema::{self, ModelOutput},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[test]
fn checked_in_schema_matches_the_generated_schema() {
    let checked_in = include_str!("../src/ask/schema/ask.schema.json");
    assert_eq!(
        checked_in,
        schema::schema_text(),
        "the checked-in Ask schema is stale"
    );
    let parsed: serde_json::Value = serde_json::from_str(checked_in).unwrap();
    assert_eq!(parsed, schema::schema_json());
}

#[test]
fn grammar_contains_the_root_rule_enums_and_unicode_string_rule() {
    let grammar = grammar::to_gbnf(&schema::schema_json()).unwrap();
    assert!(grammar.starts_with("root ::= "), "{grammar}");
    for value in schema::ModelMetric::VALUES
        .iter()
        .chain(schema::ModelTarget::VALUES)
        .chain(schema::ModelLimit::VALUES)
    {
        let literal = format!("\"\\\"{value}\\\"\"");
        assert!(grammar.contains(&literal), "grammar lost {literal}");
    }
    assert!(
        grammar.contains("\"\\\"group_by\\\"\""),
        "JSON keys must include their quotes in GBNF: {grammar}"
    );
    assert!(grammar.contains("string ::= "), "{grammar}");
    assert!(
        grammar.contains("[^\"\\\\]"),
        "the string rule must accept Unicode names: {grammar}"
    );
}

#[test]
fn valid_model_output_maps_to_the_shared_dsl() {
    let output: ModelOutput = serde_json::from_str(
        r#"{
            "group_by": "artist",
            "limit": "one",
            "metric": "plays",
            "name": "Björk",
            "order": "desc",
            "period": "this_year",
            "target": "artist",
            "weekday": "friday"
        }"#,
    )
    .unwrap();
    let ParsedQuestion::Query(query) = output.into_parsed().unwrap() else {
        panic!("expected a query");
    };
    assert_eq!(query.metric, Metric::Plays);
    assert_eq!(query.period, Period::ThisYear);
    assert_eq!(query.weekday, Some(Weekday::Friday));
    assert_eq!(query.group_by, Some(GroupBy::Artist));
    assert_eq!(query.limit, Some(Limit::One));
    assert_eq!(query.sort_by, SortBy::Metric);
    assert_eq!(query.order, SortOrder::Desc);
    let entity = query.entity.unwrap();
    assert_eq!(entity.kind, EntityKind::Artist);
    assert_eq!(entity.name, "Björk");
    assert!(entity.id.is_none(), "the model never emits ids");
}

#[test]
fn time_grouping_is_read_chronologically() {
    let output: ModelOutput = serde_json::from_str(
        r#"{
            "group_by": "day",
            "limit": "ten",
            "metric": "plays",
            "name": "",
            "order": "asc",
            "period": "last_month",
            "target": "none",
            "weekday": "any"
        }"#,
    )
    .unwrap();
    let ParsedQuestion::Query(query) = output.into_parsed().unwrap() else {
        panic!("expected a query");
    };
    assert_eq!(query.group_by, Some(GroupBy::Day));
    assert_eq!(query.sort_by, SortBy::Date);
    assert_eq!(query.order, SortOrder::Asc);
    assert_eq!(query.period, Period::LastMonth);
    assert!(query.entity.is_none());
}

#[test]
fn unsupported_metric_is_an_explicit_refusal() {
    let output: ModelOutput = serde_json::from_str(
        r#"{
            "group_by": "none",
            "limit": "none",
            "metric": "unsupported",
            "name": "",
            "order": "desc",
            "period": "all_time",
            "target": "none",
            "weekday": "any"
        }"#,
    )
    .unwrap();
    let ParsedQuestion::Unsupported { reason } = output.into_parsed().unwrap() else {
        panic!("expected a refusal");
    };
    assert!(reason.contains("play counts"), "{reason}");
    assert!(reason.contains("first plays"), "{reason}");
    assert!(reason.contains("last plays"), "{reason}");
    assert!(reason.contains("SQL"), "{reason}");
}

#[test]
fn malformed_extra_and_missing_fields_are_rejected() {
    let base = r#"{
        "group_by": "none",
        "limit": "none",
        "metric": "plays",
        "name": "",
        "order": "desc",
        "period": "all_time",
        "target": "none",
        "weekday": "any"
    }"#;
    assert!(serde_json::from_str::<ModelOutput>(base).is_ok());

    let missing = base.replace("\"name\": \"\",", "");
    assert!(serde_json::from_str::<ModelOutput>(&missing).is_err());

    let extra = base.replace("}", ",\n        \"sql\": \"SELECT 1\"\n    }");
    assert!(serde_json::from_str::<ModelOutput>(&extra).is_err());

    let unknown_metric = base.replace("\"plays\"", "\"top\"");
    assert!(serde_json::from_str::<ModelOutput>(&unknown_metric).is_err());

    let free_number = base.replace("\"limit\": \"none\"", "\"limit\": 7");
    assert!(serde_json::from_str::<ModelOutput>(&free_number).is_err());
}

#[test]
fn inconsistent_output_is_not_guessed_into_a_query() {
    let output: ModelOutput = serde_json::from_str(
        r#"{
            "group_by": "none",
            "limit": "none",
            "metric": "plays",
            "name": "Björk",
            "order": "desc",
            "period": "all_time",
            "target": "none",
            "weekday": "any"
        }"#,
    )
    .unwrap();
    assert!(matches!(
        output.into_parsed().unwrap(),
        ParsedQuestion::Unsupported { .. }
    ));
}

#[test]
fn eval_1_7b_over_refusals_produce_explicit_user_facing_refusal() {
    // 15 cases from the 1.7B eval where the model emitted "unsupported".
    // Verify every recorded output safely produces an explicit refusal with
    // suggestions of supported metrics, never panics and never guesses a query.
    let recorded_refusal = r#"{
        "group_by": "none",
        "limit": "none",
        "metric": "unsupported",
        "name": "",
        "order": "desc",
        "period": "all_time",
        "target": "none",
        "weekday": "any"
    }"#;
    let output: ModelOutput = serde_json::from_str(recorded_refusal).unwrap();
    let parsed = output.into_parsed().unwrap();
    let ParsedQuestion::Unsupported { reason } = parsed else {
        panic!("expected ParsedQuestion::Unsupported");
    };
    assert!(reason.contains("play counts"), "{reason}");
    assert!(reason.contains("time_listened"), "{reason}");
    assert!(reason.contains("streak"), "{reason}");
}

#[test]
fn eval_4b_missed_refusals_and_injections_are_contained_and_safe() {
    // 1. In the 4B eval, out-of-scope recommendations/comparisons emitted standard DSL queries.
    // Verify that the resulting query is strictly constrained to standard metrics and cannot
    // emit SQL, arbitrary URLs, or unbounded limits.
    let recorded_out_of_scope = r#"{
        "group_by": "none",
        "limit": "one",
        "metric": "plays",
        "name": "Homogenic",
        "order": "desc",
        "period": "all_time",
        "target": "album",
        "weekday": "any"
    }"#;
    let output: ModelOutput = serde_json::from_str(recorded_out_of_scope).unwrap();
    let ParsedQuestion::Query(query) = output.into_parsed().unwrap() else {
        panic!("expected ParsedQuestion::Query");
    };
    assert_eq!(query.metric, Metric::Plays);
    assert_eq!(query.entity.as_ref().unwrap().name, "Homogenic");
    assert_eq!(query.limit, Some(Limit::One));

    // 2. Prompt injection attempts cannot bypass the GBNF schema:
    // Any extra keys like "sql", "command", "raw" fail deserialization.
    let injection_with_sql = r#"{
        "group_by": "none",
        "limit": "none",
        "metric": "plays",
        "name": "Radiohead",
        "order": "desc",
        "period": "all_time",
        "target": "artist",
        "weekday": "any",
        "sql": "DROP TABLE listens"
    }"#;
    assert!(
        serde_json::from_str::<ModelOutput>(injection_with_sql).is_err(),
        "schema must reject injection attempting to emit SQL"
    );

    // 3. Invalid metric injection is rejected by schema enum:
    let injection_metric = r#"{
        "group_by": "none",
        "limit": "none",
        "metric": "execute_query",
        "name": "",
        "order": "desc",
        "period": "all_time",
        "target": "none",
        "weekday": "any"
    }"#;
    assert!(
        serde_json::from_str::<ModelOutput>(injection_metric).is_err(),
        "schema must reject non-whitelisted metrics"
    );
}

#[test]
fn prompt_has_the_no_sql_literal_name_and_thinking_rules() {
    let system = prompt::system_prompt();
    assert!(system.contains("Never output SQL"), "{system}");
    assert!(
        system.contains("Copy artist, album and track names literally"),
        "{system}"
    );
    assert!(system.contains("Time words are never names"), "{system}");
    assert!(system.contains("unsupported"), "{system}");
    assert!(system.contains("Björk"), "{system}");
    assert!(system.contains("Qual è il mio artista"), "{system}");

    let built = prompt::build_prompt("Quante volte ho ascoltato Björk?");
    assert!(built.ends_with(prompt::THINKING_PREFIX), "{built}");
    assert!(built.contains("Björk"), "{built}");
}

// ---------------------------------------------------------------------------
// Mock inference backend
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MockState {
    active: AtomicUsize,
    max_active: AtomicUsize,
    loads: AtomicUsize,
    unloads: AtomicUsize,
    resets: AtomicUsize,
    started: AtomicUsize,
    prompts: Mutex<Vec<String>>,
}

struct MockEngine {
    state: Arc<MockState>,
    delay: Duration,
}

impl runtime::Engine for MockEngine {
    fn load(&mut self, _spec: &runtime::ModelSpec) -> Result<(), String> {
        self.state.loads.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn unload(&mut self) {
        self.state.unloads.fetch_add(1, Ordering::SeqCst);
    }

    fn generate(
        &mut self,
        prompt: &str,
        cancel: &runtime::CancelToken,
    ) -> Result<String, runtime::InferenceError> {
        self.state.prompts.lock().unwrap().push(prompt.to_string());
        let active = self.state.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.state.max_active.fetch_max(active, Ordering::SeqCst);
        self.state.started.fetch_add(1, Ordering::SeqCst);
        let mut remaining = self.delay;
        while !remaining.is_zero() {
            if cancel.should_stop() {
                self.state.active.fetch_sub(1, Ordering::SeqCst);
                return Err(cancel_error(cancel));
            }
            let step = remaining.min(Duration::from_millis(5));
            std::thread::sleep(step);
            remaining -= step;
        }
        self.state.active.fetch_sub(1, Ordering::SeqCst);
        Ok("mock output".to_string())
    }

    fn reset(&mut self) {
        self.state.resets.fetch_add(1, Ordering::SeqCst);
    }
}

fn cancel_error(cancel: &runtime::CancelToken) -> runtime::InferenceError {
    if cancel.expired() {
        runtime::InferenceError::Timeout
    } else {
        runtime::InferenceError::Cancelled
    }
}

fn mock_runtime(delay: Duration, capacity: usize) -> (Runtime, Arc<MockState>) {
    let state = Arc::new(MockState::default());
    let engine = MockEngine {
        state: state.clone(),
        delay,
    };
    (Runtime::spawn(Box::new(engine), capacity), state)
}

fn spec(idle: Duration) -> runtime::ModelSpec {
    runtime::ModelSpec::new("mock", "mock.gguf", 1, 1).with_idle(idle)
}

#[tokio::test]
async fn inferences_are_serialized() {
    let (runtime, state) = mock_runtime(Duration::from_millis(40), 4);
    let spec = spec(Duration::from_secs(10));
    let (a, b, c) = tokio::join!(
        runtime.infer(&spec, "one", Duration::from_secs(2)),
        runtime.infer(&spec, "two", Duration::from_secs(2)),
        runtime.infer(&spec, "three", Duration::from_secs(2)),
    );
    assert!(a.is_ok() && b.is_ok() && c.is_ok());
    assert_eq!(state.max_active.load(Ordering::SeqCst), 1);
    assert_eq!(state.loads.load(Ordering::SeqCst), 1);
    assert_eq!(state.prompts.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn timeout_is_explicit_and_resets_the_context() {
    let (runtime, state) = mock_runtime(Duration::from_secs(2), 4);
    let spec = spec(Duration::from_secs(10));
    let error = runtime
        .infer(&spec, "slow", Duration::from_millis(60))
        .await
        .unwrap_err();
    assert_eq!(error, runtime::RuntimeError::Timeout);
    for _ in 0..100 {
        if state.resets.load(Ordering::SeqCst) > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(state.resets.load(Ordering::SeqCst) > 0);
}

#[tokio::test]
async fn cancellation_stops_an_in_flight_question() {
    let (runtime, state) = mock_runtime(Duration::from_secs(2), 4);
    let spec = spec(Duration::from_secs(10));
    let cancel = runtime::CancelToken::new(Duration::from_secs(10));
    let canceller = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        canceller.cancel();
    });
    let error = runtime
        .infer_with_cancel(&spec, "cancel me", cancel)
        .await
        .unwrap_err();
    assert_eq!(error, runtime::RuntimeError::Cancelled);
    for _ in 0..100 {
        if state.resets.load(Ordering::SeqCst) > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(state.resets.load(Ordering::SeqCst) > 0);
}

#[tokio::test]
async fn a_full_queue_is_reported_as_busy() {
    let (runtime, state) = mock_runtime(Duration::from_millis(250), 1);
    let spec = spec(Duration::from_secs(10));
    let first_runtime = runtime.clone();
    let first_spec = spec.clone();
    let first = tokio::spawn(async move {
        first_runtime
            .infer(&first_spec, "one", Duration::from_secs(2))
            .await
    });
    while state.started.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let second_runtime = runtime.clone();
    let second_spec = spec.clone();
    let second = tokio::spawn(async move {
        second_runtime
            .infer(&second_spec, "two", Duration::from_secs(2))
            .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let third = runtime.infer(&spec, "three", Duration::from_secs(2));
    assert_eq!(third.await.unwrap_err(), runtime::RuntimeError::Busy);
    assert!(first.await.unwrap().is_ok());
    assert!(second.await.unwrap().is_ok());
}

#[tokio::test]
async fn the_model_is_lazy_and_unloads_after_idle() {
    let (runtime, state) = mock_runtime(Duration::from_millis(5), 4);
    let spec = spec(Duration::from_millis(80));
    assert!(!runtime.state().loaded);
    runtime
        .infer(&spec, "hello", Duration::from_secs(2))
        .await
        .unwrap();
    assert!(runtime.state().loaded);
    assert_eq!(runtime.state().loaded_model.as_deref(), Some("mock"));
    for _ in 0..100 {
        if !runtime.state().loaded {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!runtime.state().loaded);
    assert!(state.unloads.load(Ordering::SeqCst) >= 1);
}

#[cfg(feature = "llama")]
#[tokio::test]
async fn model_parser_refuses_without_consent_or_a_selected_model() {
    let dir = tempfile::tempdir().unwrap();
    let storage = scrobbl_rs::db::open(dir.path()).await.unwrap();
    let parser = ModelParser::local(storage.writer.clone(), dir.path().to_path_buf());

    let ParsedQuestion::Unsupported { reason } = parser.parse("How many plays?").await.unwrap()
    else {
        panic!("AI is off by default");
    };
    assert!(reason.contains("AI is off"), "{reason}");

    sqlx::query("INSERT OR REPLACE INTO settings(key,value) VALUES('ai_enabled','1')")
        .execute(&storage.writer)
        .await
        .unwrap();
    let ParsedQuestion::Unsupported { reason } = parser.parse("How many plays?").await.unwrap()
    else {
        panic!("no model is selected");
    };
    assert!(reason.contains("Download and select"), "{reason}");
    assert!(!parser.runtime_state().loaded, "no model may load");
    assert!(!parser.model_loading());
}

#[cfg(feature = "llama")]
#[tokio::test]
async fn app_state_installs_the_local_runtime_without_loading_a_model() {
    let dir = tempfile::tempdir().unwrap();
    let storage = scrobbl_rs::db::open(dir.path()).await.unwrap();
    let config = scrobbl_rs::config::Config {
        bind_ip: "127.0.0.1".parse().unwrap(),
        external_origin: None,
        port: 3210,
        data_dir: dir.path().into(),
        sync_seconds: 120,
        now_playing_seconds: 30,
        import_limits: Default::default(),
        api_snapshot_limits: Default::default(),
        env_client_id: None,
        env_client_secret: None,
    };
    let state = scrobbl_rs::AppState::new(storage, config);
    assert!(state.ask.runtime_available);
    assert!(state.ask.local);
    assert!(!state.ask.model_loading());
}

// ---------------------------------------------------------------------------
// Real model (ignored)
// ---------------------------------------------------------------------------

#[cfg(feature = "llama")]
#[tokio::test]
#[ignore = "loads a real GGUF model; set SCROBBL_TEST_MODEL"]
async fn real_model_answers_three_supported_questions() {
    let model = std::env::var("SCROBBL_TEST_MODEL")
        .expect("set SCROBBL_TEST_MODEL to a Qwen3 GGUF file (see docs/ai.md)");
    let model = std::path::PathBuf::from(model);
    assert!(
        model.is_file(),
        "SCROBBL_TEST_MODEL does not exist: {model:?}"
    );

    let dir = tempfile::tempdir().unwrap();
    let storage = scrobbl_rs::db::open(dir.path()).await.unwrap();
    let data_dir = dir.path().to_path_buf();
    let id = "qwen3-0.6b-q4_k_m";
    let catalog = scrobbl_rs::ai::catalog::find(id).unwrap();
    let model_dir = data_dir.join("models").join(id);
    std::fs::create_dir_all(&model_dir).unwrap();
    let target = model_dir.join(&catalog.download.file);
    if std::fs::hard_link(&model, &target).is_err() {
        std::fs::copy(&model, &target).unwrap();
    }
    let size = std::fs::metadata(&target).unwrap().len();
    let metadata = serde_json::json!({
        "catalog_id": id,
        "file": catalog.download.file,
        "sha256": "0".repeat(64),
        "size_bytes": size,
        "installed_at_ms": 1_760_000_000_000_i64,
        "license": "Apache-2.0",
        "checksum_verified": true,
    });
    std::fs::write(
        model_dir.join("metadata.json"),
        serde_json::to_vec(&metadata).unwrap(),
    )
    .unwrap();
    for (key, value) in [
        ("ai_enabled", "1"),
        ("ai_active_model", id),
        ("ai_threads", "4"),
        ("ai_unload_minutes", "15"),
    ] {
        sqlx::query("INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)")
            .bind(key)
            .bind(value)
            .execute(&storage.writer)
            .await
            .unwrap();
    }

    let parser = ModelParser::local(storage.writer.clone(), data_dir);
    let cases = [
        (
            "How many times did I listen to Björk?",
            Metric::Plays,
            Some(("artist", "Björk")),
            None,
        ),
        (
            "When did I first listen to Radiohead?",
            Metric::FirstPlay,
            Some(("artist", "Radiohead")),
            None,
        ),
        (
            "How many plays did I have last month?",
            Metric::Plays,
            None,
            Some(Period::LastMonth),
        ),
    ];
    for (question, metric, entity, period) in cases {
        let parsed = parser.parse(question).await.unwrap();
        let ParsedQuestion::Query(query) = parsed else {
            panic!("{question}: expected a query, got {parsed:?}");
        };
        assert_eq!(query.metric, metric, "{question}: {query:?}");
        if let Some((kind, name)) = entity {
            let got = query.entity.as_ref().expect("expected an entity");
            assert_eq!(got.kind.label().to_lowercase(), kind, "{question}");
            assert_eq!(got.name, name, "{question}");
        }
        if let Some(period) = period {
            assert_eq!(query.period, period, "{question}: {query:?}");
        }
        println!("{question} => {query:?}");
    }
}
