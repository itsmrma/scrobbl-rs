#![allow(clippy::unwrap_used, clippy::disallowed_methods)]
//! CI-safe tests for the local Ask SQL parser plus an ignored real-model run.
//!
//! The queue, serialization, timeout, cancellation and idle-unload tests use a
//! mock engine and never load weights. The real GGUF test is `#[ignore]` and
//! reads `SCROBBL_TEST_MODEL`; see `docs/ai.md`.
#[cfg(feature = "llama")]
use scrobbl_rs::ask::parser::{ModelParser, QueryParser};
use scrobbl_rs::ask::{
    grammar,
    parser::ParsedQuestion,
    prompt, runtime,
    runtime::Runtime,
    schema::{SqlAnswerKind, SqlModelOutput},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[test]
fn sql_grammar_contains_expected_root_and_unicode_string_rule() {
    let grammar = grammar::from_sql_schema().unwrap();
    assert!(grammar.starts_with("root ::= "), "{grammar}");
    for key in [
        "\"\\\"sql\\\"\"",
        "\"\\\"title\\\"\"",
        "\"\\\"computed\\\"\"",
        "\"\\\"answer_kind\\\"\"",
        "\"\\\"columns\\\"\"",
    ] {
        assert!(grammar.contains(key), "grammar missing {key}");
    }
    assert!(grammar.contains("string ::= "), "{grammar}");
    assert!(
        grammar.contains("[^\"\\\\]"),
        "the string rule must accept Unicode strings: {grammar}"
    );
}

#[test]
fn valid_sql_model_output_parses_into_sql_answer_kind_and_rejects_invalid() {
    let output: SqlModelOutput = serde_json::from_str(
        r#"{
            "sql": "SELECT COUNT(DISTINCT l.id) AS plays FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id WHERE l.counted = 1 AND a.name = 'Björk'",
            "title": "Plays of Björk",
            "computed": "Counted distinct plays where artist name is Björk.",
            "answer_kind": "value",
            "columns": [{"name": "plays", "format": "count"}]
        }"#,
    )
    .unwrap();
    assert_eq!(output.answer_kind, SqlAnswerKind::Value);
    assert!(output.sql.contains("Björk"));

    let unsupported: SqlModelOutput = serde_json::from_str(
        r#"{
            "sql": "",
            "title": "Unsupported",
            "computed": "Outside the listening archive schema.",
            "answer_kind": "unsupported",
            "columns": []
        }"#,
    )
    .unwrap();
    assert_eq!(unsupported.answer_kind, SqlAnswerKind::Unsupported);

    assert!(
        serde_json::from_str::<SqlModelOutput>(
            r#"{"sql":"SELECT 1","title":"t","computed":"c","answer_kind":"invalid_kind","columns":[]}"#
        )
        .is_err()
    );
}

#[test]
fn local_sql_prompt_has_schema_counting_rules_and_thinking_prefix() {
    let system = prompt::local_sql_system_prompt("2026-11-16");
    assert!(system.contains("listens("), "{system}");
    assert!(system.contains("counted = 1"), "{system}");
    assert!(system.contains("unsupported"), "{system}");

    let built = prompt::build_sql_prompt("Quante volte ho ascoltato Björk?", "2026-11-16");
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
    for question in [
        "How many times did I listen to Björk?",
        "When did I first listen to Radiohead?",
        "How many plays did I have last month?",
    ] {
        let parsed = parser.parse(question).await.unwrap();
        let ParsedQuestion::Sql(sql) = parsed else {
            panic!("{question}: expected SQL, got {parsed:?}");
        };
        assert!(!sql.sql.trim().is_empty(), "{question}: empty SQL");
        println!("{question} => {}", sql.sql);
    }
}

#[test]
fn sql_prompt_fits_the_context_window_and_uses_sql_grammar() {
    let prompt = prompt::build_sql_prompt("What are my top tracks in 2024?", "2026-11-16");
    let estimated_tokens = prompt.chars().count() / 3;
    assert!(
        estimated_tokens as i32 + runtime::MAX_SQL_NEW_TOKENS <= runtime::SQL_CONTEXT_TOKENS as i32,
        "sql prompt is about {estimated_tokens} tokens"
    );

    // A repair round adds the failed statements and the sanitized errors, and
    // must still fit within the SQL context budget.
    let repair = prompt::build_sql_repair_prompt(
        "What are my top tracks in 2024?",
        None,
        &[
            (
                "SELECT missing_col FROM listens",
                "sql_error (no_such_column): missing_col",
            ),
            (
                "SELECT * FROM nonexistent_table",
                "sql_error (no_such_table): nonexistent_table",
            ),
        ],
        "2026-11-16",
    );
    let repair_tokens = repair.chars().count() / 3;
    assert!(
        repair_tokens as i32 + runtime::MAX_SQL_NEW_TOKENS <= runtime::SQL_CONTEXT_TOKENS as i32,
        "sql repair prompt is about {repair_tokens} tokens"
    );
    assert!(
        repair.contains("SELECT missing_col FROM listens"),
        "{repair}"
    );
    assert!(repair.contains("no_such_column"), "{repair}");

    let grammar = grammar::from_sql_schema().unwrap();
    assert!(grammar.contains("\"\\\"sql\\\"\" ws \":\" ws string"));
}

struct RecordingEngine {
    seen: Arc<Mutex<Vec<(String, String)>>>,
    reply: String,
}

impl runtime::Engine for RecordingEngine {
    fn load(&mut self, spec: &runtime::ModelSpec) -> Result<(), String> {
        self.seen
            .lock()
            .unwrap()
            .push(("LOAD".into(), spec.path.display().to_string()));
        Ok(())
    }

    fn unload(&mut self) {}

    fn generate(
        &mut self,
        prompt: &str,
        _cancel: &runtime::CancelToken,
    ) -> Result<String, runtime::InferenceError> {
        self.seen
            .lock()
            .unwrap()
            .push(("PROMPT".into(), prompt.to_string()));
        Ok(self.reply.clone())
    }
}

#[cfg(feature = "llama")]
#[tokio::test]
async fn local_model_parser_uses_catalog_or_custom_gguf_and_reports_malformed_output() {
    let dir = tempfile::tempdir().unwrap();
    let storage = scrobbl_rs::db::open(dir.path()).await.unwrap();
    let id = "qwen3-1.7b-q4_k_m";
    let catalog = scrobbl_rs::ai::catalog::find(id).unwrap();
    let model_dir = dir.path().join("models").join(id);
    std::fs::create_dir_all(&model_dir).unwrap();
    let target = model_dir.join(&catalog.download.file);
    std::fs::write(&target, b"dummy gguf bytes").unwrap();
    let metadata = serde_json::json!({
        "catalog_id": id,
        "file": catalog.download.file,
        "sha256": "0".repeat(64),
        "size_bytes": 16,
        "installed_at_ms": 1_760_000_000_000_i64,
        "license": "Apache-2.0",
        "checksum_verified": true,
    });
    std::fs::write(
        model_dir.join("metadata.json"),
        serde_json::to_vec(&metadata).unwrap(),
    )
    .unwrap();
    for (key, value) in [("ai_enabled", "1"), ("ai_active_model", id)] {
        sqlx::query("INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)")
            .bind(key)
            .bind(value)
            .execute(&storage.writer)
            .await
            .unwrap();
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let engine = RecordingEngine {
        seen: seen.clone(),
        reply: serde_json::json!({
            "sql": "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1",
            "title": "Plays",
            "computed": "Total counted plays.",
            "answer_kind": "value",
            "columns": [{"name": "plays", "format": "count"}]
        })
        .to_string(),
    };
    let rt = Runtime::spawn(Box::new(engine), 4);
    let parser = ModelParser::new(
        storage.writer.clone(),
        dir.path().to_path_buf(),
        rt,
        Duration::from_secs(5),
    );
    let parsed = parser.parse("How many plays?").await.unwrap();
    assert!(matches!(parsed, ParsedQuestion::Sql(_)), "{parsed:?}");
    {
        let recorded = seen.lock().unwrap();
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].0, "LOAD");
        assert!(recorded[1].1.contains("listens("));
    }

    // Setting ai_custom_model_path points the runtime directly to a valid custom .gguf file.
    let custom_dir = tempfile::tempdir().unwrap();
    let custom_gguf = custom_dir.path().join("custom-sql.gguf");
    let mut gguf_bytes = b"GGUF".to_vec();
    gguf_bytes.resize(128, 7);
    std::fs::write(&custom_gguf, &gguf_bytes).unwrap();
    sqlx::query("INSERT OR REPLACE INTO settings(key,value) VALUES('ai_custom_model_path', ?)")
        .bind(custom_gguf.display().to_string())
        .execute(&storage.writer)
        .await
        .unwrap();
    let parsed_custom = parser.parse("How many plays?").await.unwrap();
    assert!(matches!(parsed_custom, ParsedQuestion::Sql(_)));
    {
        let recorded = seen.lock().unwrap();
        assert!(
            recorded
                .iter()
                .any(|(kind, val)| kind == "LOAD" && val.contains("custom-sql.gguf")),
            "{recorded:?}"
        );
    }

    // Malformed output from the local model returns a clear parse error.
    let bad_seen = Arc::new(Mutex::new(Vec::new()));
    let bad_engine = RecordingEngine {
        seen: bad_seen,
        reply: "not json".to_string(),
    };
    let bad_rt = Runtime::spawn(Box::new(bad_engine), 4);
    let bad_parser = ModelParser::new(
        storage.writer.clone(),
        dir.path().to_path_buf(),
        bad_rt,
        Duration::from_secs(5),
    );
    let err = bad_parser.parse("How many plays?").await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("SQL interpretation"), "{msg}");
}
