//! Dedicated inference worker for Ask.
//!
//! One OS thread owns the engine (llama backend, model and any transient
//! context), a bounded queue and the model lifecycle. Inference never runs on a
//! tokio worker: handlers only enqueue a request and await a oneshot channel.
//!
//! The engine abstraction keeps the queue, serialization, timeout,
//! cancellation and idle-unload behaviour testable without a model.
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

/// Queue capacity. One inference runs at a time; a couple of queued questions
/// keep the page responsive without unbounded memory use.
pub const DEFAULT_QUEUE_CAPACITY: usize = 4;

const OUTER_TIMEOUT_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSpec {
    pub id: String,
    pub path: PathBuf,
    pub threads: u32,
    pub unload_minutes: u32,
    /// Idle delay before the model is dropped. Adapters use
    /// `unload_minutes * 60`; tests use a short duration.
    pub idle: Duration,
}

impl ModelSpec {
    pub fn new(
        id: impl Into<String>,
        path: impl Into<PathBuf>,
        threads: u32,
        unload_minutes: u32,
    ) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
            threads,
            unload_minutes,
            idle: Duration::from_secs(u64::from(unload_minutes.max(1)) * 60),
        }
    }

    pub fn with_idle(mut self, idle: Duration) -> Self {
        self.idle = idle;
        self
    }

    fn same_model(&self, other: &Self) -> bool {
        self.id == other.id && self.path == other.path && self.threads == other.threads
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InferenceError {
    Timeout,
    Cancelled,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    Busy,
    Closed,
    Timeout,
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => formatter.write_str("the local model is busy with another question"),
            Self::Closed => formatter.write_str("the local model worker is not running"),
            Self::Timeout => formatter.write_str("the local model did not answer in time"),
            Self::Cancelled => formatter.write_str("the question was cancelled"),
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for RuntimeError {}

/// Cancellation and deadline shared with the inference engine. The engine
/// checks it between sampled tokens, so a timed-out request never leaves a
/// half-decoded context behind.
#[derive(Clone, Debug)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}

impl CancelToken {
    pub fn new(timeout: Duration) -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: Instant::now() + timeout,
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    pub fn should_stop(&self) -> bool {
        self.is_cancelled() || self.expired()
    }

    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    fn error(&self) -> InferenceError {
        if self.is_cancelled() {
            InferenceError::Cancelled
        } else {
            InferenceError::Timeout
        }
    }
}

/// The thing that owns a loaded model. Implementations must not panic on a
/// missing or corrupt file; they return an explicit error instead.
pub trait Engine: Send {
    fn load(&mut self, spec: &ModelSpec) -> Result<(), String>;
    fn unload(&mut self);
    fn generate(&mut self, prompt: &str, cancel: &CancelToken) -> Result<String, InferenceError>;
    /// Called after a timeout, cancellation or failure. A fresh context is
    /// created for every request, so this is a safety net for future engines.
    fn reset(&mut self) {}
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeState {
    pub loaded: bool,
    pub loading: bool,
    pub loaded_model: Option<String>,
    pub last_error: Option<String>,
}

struct Job {
    spec: ModelSpec,
    prompt: String,
    cancel: CancelToken,
    reply: oneshot::Sender<Result<String, InferenceError>>,
}

enum Message {
    Infer(Box<Job>),
    Shutdown,
}

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

struct RuntimeInner {
    sender: SyncSender<Message>,
    state: Arc<Mutex<RuntimeState>>,
    active: Arc<Mutex<Option<CancelToken>>>,
    shutdown: Arc<AtomicBool>,
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(token) = self
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
        {
            token.cancel();
        }
        // Wake an idle worker. A full queue drains eventually and every
        // in-flight generation sees the cancellation above between tokens.
        let _ = self.sender.try_send(Message::Shutdown);
    }
}

impl Runtime {
    pub fn spawn(engine: Box<dyn Engine>, queue_capacity: usize) -> Self {
        let (sender, receiver) = sync_channel(queue_capacity.max(1));
        let state = Arc::new(Mutex::new(RuntimeState::default()));
        let active = Arc::new(Mutex::new(None));
        let shutdown = Arc::new(AtomicBool::new(false));

        let worker_state = state.clone();
        let worker_active = active.clone();
        let worker_shutdown = shutdown.clone();
        let spawn_result = thread::Builder::new()
            .name("ask-runtime".into())
            .spawn(move || {
                worker(
                    engine,
                    receiver,
                    worker_state,
                    worker_active,
                    worker_shutdown,
                );
            });
        if let Err(error) = spawn_result {
            let mut state = state.lock().unwrap_or_else(|poison| poison.into_inner());
            state.last_error = Some(format!("Could not start the local model worker: {error}"));
        }

        Self {
            inner: Arc::new(RuntimeInner {
                sender,
                state,
                active,
                shutdown,
            }),
        }
    }

    pub fn state(&self) -> RuntimeState {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        if let Some(token) = self
            .inner
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
        {
            token.cancel();
        }
        let _ = self.inner.sender.try_send(Message::Shutdown);
    }

    pub async fn infer(
        &self,
        spec: &ModelSpec,
        prompt: &str,
        timeout: Duration,
    ) -> Result<String, RuntimeError> {
        self.infer_with_cancel(spec, prompt, CancelToken::new(timeout))
            .await
    }

    pub async fn infer_with_cancel(
        &self,
        spec: &ModelSpec,
        prompt: &str,
        cancel: CancelToken,
    ) -> Result<String, RuntimeError> {
        if self.inner.shutdown.load(Ordering::SeqCst) {
            return Err(RuntimeError::Closed);
        }
        // The worker enforces the deadline between tokens. This outer guard
        // covers an engine blocked inside one long decode call.
        let guard = cancel.remaining() + OUTER_TIMEOUT_GRACE;
        let cancel_on_timeout = cancel.clone();
        let (reply, response) = oneshot::channel();
        let job = Box::new(Job {
            spec: spec.clone(),
            prompt: prompt.to_string(),
            cancel,
            reply,
        });
        match self.inner.sender.try_send(Message::Infer(job)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(RuntimeError::Busy),
            Err(TrySendError::Disconnected(_)) => return Err(RuntimeError::Closed),
        }

        match tokio::time::timeout(guard, response).await {
            Err(_) => {
                cancel_on_timeout.cancel();
                Err(RuntimeError::Timeout)
            }
            Ok(Err(_)) => Err(RuntimeError::Closed),
            Ok(Ok(result)) => result.map_err(|error| match error {
                InferenceError::Timeout => RuntimeError::Timeout,
                InferenceError::Cancelled => RuntimeError::Cancelled,
                InferenceError::Failed(message) => RuntimeError::Failed(message),
            }),
        }
    }
}

fn worker(
    mut engine: Box<dyn Engine>,
    receiver: Receiver<Message>,
    state: Arc<Mutex<RuntimeState>>,
    active: Arc<Mutex<Option<CancelToken>>>,
    shutdown: Arc<AtomicBool>,
) {
    let mut loaded: Option<ModelSpec> = None;
    let mut idle_deadline: Option<Instant> = None;

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        let message = match idle_deadline {
            Some(deadline) => {
                let wait = deadline.saturating_duration_since(Instant::now());
                match receiver.recv_timeout(wait) {
                    Ok(message) => message,
                    Err(RecvTimeoutError::Timeout) => {
                        engine.unload();
                        loaded = None;
                        idle_deadline = None;
                        let mut state = lock(&state);
                        *state = RuntimeState::default();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            None => match receiver.recv() {
                Ok(message) => message,
                Err(_) => break,
            },
        };

        let Message::Infer(job) = message else {
            break;
        };
        let Job {
            spec,
            prompt,
            cancel,
            reply,
        } = *job;

        if cancel.should_stop() {
            let _ = reply.send(Err(cancel.error()));
            idle_deadline = Some(Instant::now() + spec.idle);
            continue;
        }

        if loaded
            .as_ref()
            .is_none_or(|current| !current.same_model(&spec))
        {
            engine.unload();
            loaded = None;
            {
                let mut state = lock(&state);
                state.loaded = false;
                state.loading = true;
                state.loaded_model = None;
                state.last_error = None;
            }
            match engine.load(&spec) {
                Ok(()) => {
                    loaded = Some(spec.clone());
                    let mut state = lock(&state);
                    state.loaded = true;
                    state.loading = false;
                    state.loaded_model = Some(spec.id.clone());
                }
                Err(error) => {
                    let mut state = lock(&state);
                    state.loading = false;
                    state.loaded = false;
                    state.last_error = Some(error.clone());
                    let _ = reply.send(Err(InferenceError::Failed(error)));
                    continue;
                }
            }
        }

        *lock(&active) = Some(cancel.clone());
        let result = engine.generate(&prompt, &cancel);
        if result.is_err() {
            // A timeout, cancellation or engine failure must never leave a
            // partially decoded sequence available to the next question.
            engine.reset();
        }
        *lock(&active) = None;
        let _ = reply.send(result);
        idle_deadline = Some(Instant::now() + spec.idle);
    }

    engine.unload();
    *lock(&state) = RuntimeState::default();
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

// ---------------------------------------------------------------------------
// llama.cpp engine
// ---------------------------------------------------------------------------

#[cfg(feature = "llama")]
mod llama {
    use super::*;
    use crate::ask::{grammar, schema};
    use llama_cpp_2::{
        LogOptions,
        context::params::LlamaContextParams,
        llama_backend::LlamaBackend,
        llama_batch::LlamaBatch,
        model::{LlamaModel, params::LlamaModelParams},
        sampling::LlamaSampler,
        send_logs_to_tracing,
    };
    use std::{
        num::NonZeroU32,
        sync::{LazyLock, Once},
    };

    pub const CONTEXT_TOKENS: u32 = 2048;
    pub const MAX_NEW_TOKENS: i32 = 256;
    const MAX_OUTPUT_BYTES: usize = 16 * 1024;

    static GRAMMAR: LazyLock<Result<String, String>> =
        LazyLock::new(|| grammar::to_gbnf(&schema::schema_json()));
    static LOGS: Once = Once::new();

    /// CPU-only llama.cpp engine. The backend and model stay loaded between
    /// questions; a context is created per request and dropped on every exit
    /// path, so a timeout cannot leave a partially decoded sequence behind.
    pub struct LlamaEngine {
        backend: Option<LlamaBackend>,
        model: Option<LlamaModel>,
        threads: u32,
        debug: bool,
    }

    impl LlamaEngine {
        pub fn new() -> Self {
            let debug = std::env::var_os("SCROBBL_ASK_DEBUG").is_some();
            Self {
                backend: None,
                model: None,
                threads: 4,
                debug,
            }
        }
    }

    impl Default for LlamaEngine {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Engine for LlamaEngine {
        fn load(&mut self, spec: &ModelSpec) -> Result<(), String> {
            if !spec.path.is_file() {
                return Err("The selected model file is missing.".into());
            }
            if !self.debug {
                LOGS.call_once(|| {
                    send_logs_to_tracing(LogOptions::default().with_logs_enabled(false));
                });
            }
            let backend = LlamaBackend::init()
                .map_err(|error| format!("Could not start the llama backend: {error}"))?;
            let model =
                LlamaModel::load_from_file(&backend, &spec.path, &LlamaModelParams::default())
                    .map_err(|error| format!("Could not load the model file: {error}"))?;
            self.backend = Some(backend);
            self.model = Some(model);
            self.threads = spec.threads.clamp(1, 64);
            Ok(())
        }

        fn unload(&mut self) {
            self.model = None;
            self.backend = None;
        }

        fn generate(
            &mut self,
            prompt: &str,
            cancel: &CancelToken,
        ) -> Result<String, InferenceError> {
            if cancel.should_stop() {
                return Err(cancel.error());
            }
            let model = self
                .model
                .as_ref()
                .ok_or_else(|| InferenceError::Failed("The model is not loaded.".to_string()))?;
            let backend = self.backend.as_ref().ok_or_else(|| {
                InferenceError::Failed("The llama backend is not loaded.".to_string())
            })?;
            let grammar = GRAMMAR
                .as_ref()
                .map_err(|error| InferenceError::Failed(error.clone()))?;

            let mut params = LlamaContextParams::default()
                .with_n_ctx(Some(
                    NonZeroU32::new(CONTEXT_TOKENS).expect("constant context size is non-zero"),
                ))
                .with_n_threads(i32::try_from(self.threads).unwrap_or(4))
                .with_n_threads_batch(i32::try_from(self.threads).unwrap_or(4));
            if self.debug {
                // Timing measurement is off by default; only turn it on when
                // the operator asked for diagnostics.
                params = params.with_no_perf(false);
            }
            let mut context = model.new_context(backend, params).map_err(|error| {
                InferenceError::Failed(format!("Could not create the model context: {error}"))
            })?;

            let vocab = model.vocab();
            let tokens = vocab.tokenize(prompt.as_bytes(), true, true);
            if tokens.is_empty() {
                return Err(InferenceError::Failed(
                    "The model tokenizer produced an empty prompt.".to_string(),
                ));
            }
            if tokens.len() as i32 + MAX_NEW_TOKENS > context.n_ctx() as i32 {
                return Err(InferenceError::Failed(format!(
                    "The question is too long for the model context ({} prompt tokens, {} context tokens).",
                    tokens.len(),
                    context.n_ctx()
                )));
            }

            let mut batch = LlamaBatch::new(tokens.len().max(1), 1);
            let last_index = tokens.len() - 1;
            for (index, token) in tokens.iter().enumerate() {
                batch
                    .add(*token, index as i32, &[0], index == last_index)
                    .map_err(|error| {
                        InferenceError::Failed(format!("Prompt batch error: {error}"))
                    })?;
            }
            if cancel.should_stop() {
                return Err(cancel.error());
            }
            context.decode(&mut batch).map_err(|error| {
                InferenceError::Failed(format!("Prompt decode failed: {error}"))
            })?;

            let mut sampler = LlamaSampler::chain_simple([
                LlamaSampler::grammar(model, grammar, "root").map_err(|error| {
                    InferenceError::Failed(format!("Could not create the JSON grammar: {error}"))
                })?,
                LlamaSampler::greedy(),
            ]);

            let mut output: Vec<u8> = Vec::new();
            let mut generated = 0_i32;
            let mut position = tokens.len() as i32;
            loop {
                if cancel.should_stop() {
                    return Err(cancel.error());
                }
                // `LlamaSampler::sample` already accepts the token internally
                // in the llama.cpp bundled with llama-cpp-2 0.1.161. Calling
                // `accept` again advances the grammar twice and panics
                // (utilityai/llama-cpp-rs#1007).
                let token = sampler.sample(&context, batch.n_tokens() - 1);
                if vocab.is_eog(token) {
                    break;
                }
                let piece = vocab.token_to_piece(token, true, None);
                output.extend_from_slice(&piece);
                if output.len() > MAX_OUTPUT_BYTES {
                    return Err(InferenceError::Failed(
                        "The model produced more output than expected.".to_string(),
                    ));
                }
                generated += 1;
                if generated >= MAX_NEW_TOKENS {
                    break;
                }
                batch.clear();
                batch.add(token, position, &[0], true).map_err(|error| {
                    InferenceError::Failed(format!("Token batch error: {error}"))
                })?;
                position += 1;
                if cancel.should_stop() {
                    return Err(cancel.error());
                }
                context.decode(&mut batch).map_err(|error| {
                    InferenceError::Failed(format!("Token decode failed: {error}"))
                })?;
            }

            if self.debug {
                let timings = context.timings();
                eprintln!(
                    "ask runtime: prompt {} tokens, generated {}, prefill {:.0} ms, decode {:.0} ms",
                    tokens.len(),
                    generated,
                    timings.t_p_eval_ms(),
                    timings.t_eval_ms()
                );
            }

            String::from_utf8(output).map_err(|_| {
                InferenceError::Failed("The model produced invalid UTF-8 output.".to_string())
            })
        }
    }
}

#[cfg(feature = "llama")]
pub use llama::LlamaEngine;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct MockState {
        active: AtomicUsize,
        max_active: AtomicUsize,
        resets: AtomicUsize,
        loads: AtomicUsize,
        unloads: AtomicUsize,
        started: AtomicUsize,
    }

    struct MockEngine {
        state: Arc<MockState>,
        delay: Duration,
    }

    impl Engine for MockEngine {
        fn load(&mut self, _spec: &ModelSpec) -> Result<(), String> {
            self.state.loads.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn unload(&mut self) {
            self.state.unloads.fetch_add(1, Ordering::SeqCst);
        }

        fn generate(
            &mut self,
            _prompt: &str,
            cancel: &CancelToken,
        ) -> Result<String, InferenceError> {
            let active = self.state.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.state.max_active.fetch_max(active, Ordering::SeqCst);
            self.state.started.fetch_add(1, Ordering::SeqCst);
            let mut remaining = self.delay;
            while !remaining.is_zero() {
                if cancel.should_stop() {
                    self.state.active.fetch_sub(1, Ordering::SeqCst);
                    return Err(cancel.error());
                }
                let step = remaining.min(Duration::from_millis(5));
                thread::sleep(step);
                remaining -= step;
            }
            self.state.active.fetch_sub(1, Ordering::SeqCst);
            Ok("{}".to_string())
        }

        fn reset(&mut self) {
            self.state.resets.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn mock(delay: Duration) -> (Runtime, Arc<MockState>) {
        let state = Arc::new(MockState::default());
        let engine = MockEngine {
            state: state.clone(),
            delay,
        };
        (Runtime::spawn(Box::new(engine), 4), state)
    }

    fn spec(idle: Duration) -> ModelSpec {
        ModelSpec::new("mock", "mock.gguf", 1, 1).with_idle(idle)
    }

    #[tokio::test]
    async fn one_inference_runs_at_a_time() {
        let (runtime, state) = mock(Duration::from_millis(40));
        let spec = spec(Duration::from_secs(10));
        let first = runtime.infer(&spec, "one", Duration::from_secs(2));
        let second = runtime.infer(&spec, "two", Duration::from_secs(2));
        let third = runtime.infer(&spec, "three", Duration::from_secs(2));
        let (a, b, c) = tokio::join!(first, second, third);
        assert_eq!(a.unwrap(), "{}");
        assert_eq!(b.unwrap(), "{}");
        assert_eq!(c.unwrap(), "{}");
        assert_eq!(state.max_active.load(Ordering::SeqCst), 1);
        assert_eq!(state.loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn timeout_is_explicit_and_resets_the_engine() {
        let (runtime, state) = mock(Duration::from_secs(2));
        let spec = spec(Duration::from_secs(10));
        let error = runtime
            .infer(&spec, "slow", Duration::from_millis(60))
            .await
            .unwrap_err();
        assert_eq!(error, RuntimeError::Timeout);
        for _ in 0..100 {
            if state.resets.load(Ordering::SeqCst) > 0 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(state.resets.load(Ordering::SeqCst) > 0);
    }

    #[tokio::test]
    async fn cancellation_stops_between_tokens() {
        let (runtime, state) = mock(Duration::from_secs(2));
        let spec = spec(Duration::from_secs(10));
        let cancel = CancelToken::new(Duration::from_secs(10));
        let canceller = cancel.clone();
        let handle = std::thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            canceller.cancel();
        });
        let error = runtime
            .infer_with_cancel(&spec, "cancel me", cancel)
            .await
            .unwrap_err();
        handle.join().unwrap();
        assert_eq!(error, RuntimeError::Cancelled);
        for _ in 0..100 {
            if state.resets.load(Ordering::SeqCst) > 0 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(state.resets.load(Ordering::SeqCst) > 0);
    }

    #[tokio::test]
    async fn a_full_queue_is_reported_as_busy() {
        let state = Arc::new(MockState::default());
        let engine = MockEngine {
            state: state.clone(),
            delay: Duration::from_millis(250),
        };
        let runtime = Runtime::spawn(Box::new(engine), 1);
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
        assert_eq!(third.await.unwrap_err(), RuntimeError::Busy);
        assert_eq!(first.await.unwrap().unwrap(), "{}");
        assert_eq!(second.await.unwrap().unwrap(), "{}");
    }

    #[tokio::test]
    async fn the_model_is_loaded_lazily_and_unloaded_when_idle() {
        let (runtime, state) = mock(Duration::from_millis(5));
        let spec = spec(Duration::from_millis(80));
        assert!(!runtime.state().loaded);
        assert!(!runtime.state().loading);
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
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!runtime.state().loaded);
        assert!(state.unloads.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn cancel_token_reports_the_reason() {
        let cancelled = CancelToken::new(Duration::from_secs(10));
        cancelled.cancel();
        assert_eq!(cancelled.error(), InferenceError::Cancelled);
        let expired = CancelToken::new(Duration::from_millis(0));
        assert_eq!(expired.error(), InferenceError::Timeout);
        assert!(expired.should_stop());
    }
}
