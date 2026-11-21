//! Dedicated inference worker for Ask.
//!
//! One OS thread owns the engine (`llama.cpp` backend, loaded GGUF model, and
//! transient context), a bounded queue, and the model lifecycle. Inference
//! never runs on a Tokio worker: handlers only enqueue a request and await a
//! oneshot channel.
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
    pub gpu_layers: i32,
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
            gpu_layers: crate::ai::DEFAULT_GPU_LAYERS,
            unload_minutes,
            idle: Duration::from_secs(u64::from(unload_minutes.max(1)) * 60),
        }
    }

    pub fn with_gpu_layers(mut self, gpu_layers: i32) -> Self {
        self.gpu_layers = gpu_layers;
        self
    }

    pub fn with_idle(mut self, idle: Duration) -> Self {
        self.idle = idle;
        self
    }

    fn same_model(&self, other: &Self) -> bool {
        self.id == other.id
            && self.path == other.path
            && self.threads == other.threads
            && self.gpu_layers == other.gpu_layers
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

/// One generation with wall-clock timings.
///
/// The production worker only needs [`Generation::text`], while `ask-eval`
/// records latency and throughput without reimplementing the decode loop.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Generation {
    pub text: String,
    /// Prompt tokens after tokenization.
    pub prompt_tokens: i32,
    /// Tokens emitted, excluding the end-of-generation token.
    pub generated_tokens: i32,
    /// Time until the first sampled token, prompt decode included.
    pub ttft_ms: f64,
    /// Decode time after the first token.
    pub decode_ms: f64,
    /// Wall-clock time for the whole call.
    pub total_ms: f64,
}

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

/// Owns a loaded local GGUF model. Implementations must never panic on a
/// missing or corrupt file; they return an explicit error instead.
pub trait Engine: Send {
    fn load(&mut self, spec: &ModelSpec) -> Result<(), String>;
    fn unload(&mut self);
    fn generate(&mut self, prompt: &str, cancel: &CancelToken) -> Result<String, InferenceError>;
    /// One generation with timings.
    fn generate_timed(
        &mut self,
        prompt: &str,
        cancel: &CancelToken,
    ) -> Result<Generation, InferenceError> {
        self.generate(prompt, cancel).map(|text| Generation {
            text,
            ..Generation::default()
        })
    }
    /// Called after a timeout, cancellation or failure.
    fn reset(&mut self) {}
}

/// Installed when the build has no local inference runtime.
#[cfg(not(feature = "llama"))]
pub struct UnavailableEngine;

#[cfg(not(feature = "llama"))]
impl Engine for UnavailableEngine {
    fn load(&mut self, _spec: &ModelSpec) -> Result<(), String> {
        Err("This build does not include the local AI runtime.".into())
    }
    fn unload(&mut self) {}
    fn generate(&mut self, _prompt: &str, _cancel: &CancelToken) -> Result<String, InferenceError> {
        Err(InferenceError::Failed(
            "This build does not include the local AI runtime.".into(),
        ))
    }
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
    Unload,
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

    pub fn unload(&self) {
        let _ = self.inner.sender.try_send(Message::Unload);
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
        struct CancelOnDrop<'a>(&'a CancelToken, bool);
        impl Drop for CancelOnDrop<'_> {
            fn drop(&mut self) {
                if !self.1 {
                    self.0.cancel();
                }
            }
        }
        let mut drop_guard = CancelOnDrop(&cancel_on_timeout, false);
        let (reply, response) = oneshot::channel();
        let job = Box::new(Job {
            spec: spec.clone(),
            prompt: prompt.to_string(),
            cancel,
            reply,
        });
        match self.inner.sender.try_send(Message::Infer(job)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                drop_guard.1 = true;
                return Err(RuntimeError::Busy);
            }
            Err(TrySendError::Disconnected(_)) => {
                drop_guard.1 = true;
                return Err(RuntimeError::Closed);
            }
        }

        let outcome = tokio::time::timeout(guard, response).await;
        drop_guard.1 = true;
        match outcome {
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
                        *lock(&state) = RuntimeState::default();
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

        let job = match message {
            Message::Shutdown => break,
            Message::Unload => {
                engine.unload();
                loaded = None;
                idle_deadline = None;
                *lock(&state) = RuntimeState::default();
                continue;
            }
            Message::Infer(job) => job,
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
            let load_result = match engine.load(&spec) {
                Ok(()) => Ok(()),
                Err(gpu_error) if spec.gpu_layers != 0 => {
                    eprintln!(
                        "ask runtime: GPU model load failed for '{}' (gpu_layers={}): {}; falling back to CPU-only (0 GPU layers)",
                        spec.id, spec.gpu_layers, gpu_error
                    );
                    let mut cpu_spec = spec.clone();
                    cpu_spec.gpu_layers = 0;
                    engine.load(&cpu_spec).map_err(|cpu_error| {
                        if cpu_error == gpu_error {
                            cpu_error
                        } else {
                            format!("{gpu_error} (CPU fallback also failed: {cpu_error})")
                        }
                    })
                }
                Err(error) => Err(error),
            };
            match load_result {
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
    use crate::ask::grammar;
    use llama_cpp_2::{
        LogOptions,
        context::{
            LlamaContext,
            params::LlamaContextParams,
            session::{LlamaStateSeqFlags, SeqState},
        },
        llama_backend::LlamaBackend,
        llama_batch::LlamaBatch,
        model::{LlamaModel, params::LlamaModelParams},
        sampling::LlamaSampler,
        send_logs_to_tracing,
        token::LlamaToken,
    };
    use std::{
        num::NonZeroU32,
        sync::{LazyLock, Once},
    };

    pub const SQL_CONTEXT_TOKENS: u32 = 8192;
    pub const MAX_SQL_NEW_TOKENS: i32 = 1024;
    const MAX_OUTPUT_BYTES: usize = 32 * 1024;
    const PREFILL_BATCH_TOKENS: usize = 512;

    static SQL_GRAMMAR: LazyLock<Result<String, String>> = LazyLock::new(grammar::from_sql_schema);
    static LOGS: Once = Once::new();

    struct PrefixCache {
        prefix_str: String,
        prefix_tokens: Vec<LlamaToken>,
        state: SeqState,
    }

    fn decode_token_slice(
        context: &mut LlamaContext<'_>,
        batch: &mut LlamaBatch<'_>,
        tokens: &[LlamaToken],
        start_pos: usize,
        mark_last_logits: bool,
        cancel: &CancelToken,
    ) -> Result<(), InferenceError> {
        let last_offset = tokens.len().saturating_sub(1);
        for chunk_start in (0..tokens.len()).step_by(PREFILL_BATCH_TOKENS) {
            if cancel.should_stop() {
                return Err(cancel.error());
            }
            batch.clear();
            let chunk_end = (chunk_start + PREFILL_BATCH_TOKENS).min(tokens.len());
            for (offset, token) in tokens[chunk_start..chunk_end].iter().enumerate() {
                let rel_index = chunk_start + offset;
                let pos = (start_pos + rel_index) as i32;
                let logits = mark_last_logits && rel_index == last_offset;
                batch.add(*token, pos, &[0], logits).map_err(|error| {
                    InferenceError::Failed(format!("Prompt batch error: {error}"))
                })?;
            }
            context.decode(batch).map_err(|error| {
                InferenceError::Failed(format!("Prompt decode failed: {error}"))
            })?;
        }
        Ok(())
    }

    /// CPU-only llama.cpp engine. The backend, model, and evaluated system
    /// prompt prefix state stay cached between questions; a fresh context is
    /// created per request and dropped on every exit path, so a timeout cannot
    /// leave a partially decoded sequence behind.
    pub struct LlamaEngine {
        backend: Option<LlamaBackend>,
        model: Option<LlamaModel>,
        prefix_cache: Option<PrefixCache>,
        threads: u32,
        debug: bool,
    }

    impl LlamaEngine {
        pub fn new() -> Self {
            let debug = std::env::var_os("SCROBBL_ASK_DEBUG").is_some();
            Self {
                backend: None,
                model: None,
                prefix_cache: None,
                threads: 4,
                debug,
            }
        }

        pub fn generate_timed_sql(
            &mut self,
            prompt: &str,
            cancel: &CancelToken,
        ) -> Result<Generation, InferenceError> {
            let started = Instant::now();
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
            let grammar = SQL_GRAMMAR
                .as_ref()
                .map_err(|error| InferenceError::Failed(error.clone()))?;

            let mut params = LlamaContextParams::default()
                .with_n_ctx(Some(
                    NonZeroU32::new(SQL_CONTEXT_TOKENS).unwrap_or(NonZeroU32::MIN),
                ))
                .with_n_batch(SQL_CONTEXT_TOKENS)
                .with_n_ubatch(PREFILL_BATCH_TOKENS as u32)
                .with_n_threads(i32::try_from(self.threads).unwrap_or(4))
                .with_n_threads_batch(i32::try_from(self.threads).unwrap_or(4));
            if self.debug {
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
            if tokens.len() as i32 + MAX_SQL_NEW_TOKENS > context.n_ctx() as i32 {
                return Err(InferenceError::Failed(format!(
                    "The question is too long for the model context ({} prompt tokens, {} context tokens).",
                    tokens.len(),
                    context.n_ctx()
                )));
            }

            let mut batch = LlamaBatch::new(PREFILL_BATCH_TOKENS.max(1), 1);
            let system_prefix = prompt
                .starts_with("<|im_start|>system\n")
                .then(|| {
                    prompt
                        .find("<|im_end|>\n")
                        .map(|end| &prompt[..end + "<|im_end|>\n".len()])
                })
                .flatten();

            let mut decode_start = 0_usize;
            if let Some(prefix_str) = system_prefix {
                if let Some(cached) = self.prefix_cache.as_ref().filter(|c| {
                    c.prefix_str == prefix_str
                        && tokens.len() > c.prefix_tokens.len()
                        && tokens.starts_with(&c.prefix_tokens)
                }) {
                    if context.state_seq_set(&cached.state, 0).is_ok() {
                        decode_start = cached.prefix_tokens.len();
                    } else {
                        context.clear_kv_cache();
                        self.prefix_cache = None;
                    }
                }
                if decode_start == 0 {
                    let prefix_tokens = vocab.tokenize(prefix_str.as_bytes(), true, true);
                    if !prefix_tokens.is_empty()
                        && prefix_tokens.len() < tokens.len()
                        && tokens.starts_with(&prefix_tokens)
                    {
                        decode_token_slice(
                            &mut context,
                            &mut batch,
                            &prefix_tokens,
                            0,
                            false,
                            cancel,
                        )?;
                        if let Ok(state) = context.state_seq_get(0, LlamaStateSeqFlags::empty()) {
                            self.prefix_cache = Some(PrefixCache {
                                prefix_str: prefix_str.to_string(),
                                prefix_tokens: prefix_tokens.clone(),
                                state,
                            });
                        }
                        decode_start = prefix_tokens.len();
                    }
                }
            }

            decode_token_slice(
                &mut context,
                &mut batch,
                &tokens[decode_start..],
                decode_start,
                true,
                cancel,
            )?;

            let mut sampler = LlamaSampler::chain_simple([
                LlamaSampler::grammar(model, grammar, "root").map_err(|error| {
                    InferenceError::Failed(format!("Could not create the JSON grammar: {error}"))
                })?,
                LlamaSampler::greedy(),
            ]);

            let mut output: Vec<u8> = Vec::new();
            let mut generated = 0_i32;
            let mut position = tokens.len() as i32;
            let mut first_token_at: Option<Instant> = None;
            loop {
                if cancel.should_stop() {
                    return Err(cancel.error());
                }
                let token = sampler.sample(&context, batch.n_tokens() - 1);
                if first_token_at.is_none() {
                    first_token_at = Some(Instant::now());
                }
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
                if generated >= MAX_SQL_NEW_TOKENS {
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

            let text = String::from_utf8(output).map_err(|_| {
                InferenceError::Failed("The model produced invalid UTF-8 output.".to_string())
            })?;
            let total_ms = started.elapsed().as_secs_f64() * 1000.0;
            let ttft_ms = first_token_at
                .map(|at| at.saturating_duration_since(started).as_secs_f64() * 1000.0)
                .unwrap_or(total_ms);
            Ok(Generation {
                text,
                prompt_tokens: i32::try_from(tokens.len()).unwrap_or(i32::MAX),
                generated_tokens: generated,
                ttft_ms,
                decode_ms: (total_ms - ttft_ms).max(0.0),
                total_ms,
            })
        }
    }

    impl Default for LlamaEngine {
        fn default() -> Self {
            Self::new()
        }
    }

    pub(crate) fn model_params_for_gpu_layers(configured_gpu_layers: i32) -> LlamaModelParams {
        let effective = crate::ai::effective_gpu_layers(configured_gpu_layers);
        let params = LlamaModelParams::default();
        if effective < 0 {
            params.with_n_gpu_layers(u32::MAX)
        } else {
            params.with_n_gpu_layers(u32::try_from(effective).unwrap_or(0))
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
            let model = LlamaModel::load_from_file(
                &backend,
                &spec.path,
                &model_params_for_gpu_layers(spec.gpu_layers),
            )
            .map_err(|error| format!("Could not load the model file: {error}"))?;
            self.backend = Some(backend);
            self.model = Some(model);
            self.threads = spec.threads.clamp(1, 64);
            Ok(())
        }

        fn unload(&mut self) {
            self.prefix_cache = None;
            self.model = None;
            self.backend = None;
        }

        fn reset(&mut self) {
            self.prefix_cache = None;
        }

        fn generate(
            &mut self,
            prompt: &str,
            cancel: &CancelToken,
        ) -> Result<String, InferenceError> {
            self.generate_timed(prompt, cancel)
                .map(|generation| generation.text)
        }

        fn generate_timed(
            &mut self,
            prompt: &str,
            cancel: &CancelToken,
        ) -> Result<Generation, InferenceError> {
            self.generate_timed_sql(prompt, cancel)
        }
    }
}

#[cfg(feature = "llama")]
pub use llama::{LlamaEngine, MAX_SQL_NEW_TOKENS, SQL_CONTEXT_TOKENS};

/// A loaded engine reused for many prompts. Offline tools (`ask-eval`)
/// use it to measure one model across a corpus without reloading the weights;
/// production keeps the dedicated worker and its queue.
#[cfg(feature = "llama")]
pub struct Session {
    engine: LlamaEngine,
    spec: ModelSpec,
    timeout: Duration,
    loaded: bool,
}

#[cfg(feature = "llama")]
impl Session {
    /// Load the model immediately. A missing or corrupt file is an explicit
    /// error, never a panic.
    pub fn open(spec: ModelSpec, timeout: Duration) -> Result<Self, RuntimeError> {
        let mut engine = LlamaEngine::new();
        engine.load(&spec).map_err(RuntimeError::Failed)?;
        Ok(Self {
            engine,
            spec,
            timeout,
            loaded: true,
        })
    }

    /// Run one prompt against the loaded model with the production SQL prompt,
    /// grammar and greedy sampling.
    pub fn run(&mut self, prompt: &str) -> Result<Generation, RuntimeError> {
        if !self.loaded {
            self.engine.load(&self.spec).map_err(RuntimeError::Failed)?;
            self.loaded = true;
        }
        let cancel = CancelToken::new(self.timeout);
        let result = self.engine.generate_timed(prompt, &cancel);
        if result.is_err() {
            self.engine.reset();
        }
        result.map_err(|error| match error {
            InferenceError::Timeout => RuntimeError::Timeout,
            InferenceError::Cancelled => RuntimeError::Cancelled,
            InferenceError::Failed(message) => RuntimeError::Failed(message),
        })
    }
}

#[cfg(feature = "llama")]
impl Drop for Session {
    fn drop(&mut self) {
        self.engine.unload();
    }
}

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
            let deadline = Instant::now() + self.delay;
            loop {
                if cancel.should_stop() {
                    self.state.active.fetch_sub(1, Ordering::SeqCst);
                    return Err(cancel.error());
                }
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    break;
                };
                if remaining.is_zero() {
                    break;
                }
                thread::sleep(remaining.min(Duration::from_millis(5)));
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
                .infer(&first_spec, "one", Duration::from_secs(10))
                .await
        });
        while state.started.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let second_runtime = runtime.clone();
        let second_spec = spec.clone();
        let second = tokio::spawn(async move {
            second_runtime
                .infer(&second_spec, "two", Duration::from_secs(10))
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let third = runtime.infer(&spec, "three", Duration::from_secs(10));
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

    #[tokio::test]
    async fn dropping_infer_future_cancels_the_token_and_resets_engine() {
        let (runtime, state) = mock(Duration::from_secs(5));
        let spec = spec(Duration::from_secs(10));
        let cancel = CancelToken::new(Duration::from_secs(10));
        let handle = tokio::spawn({
            let runtime = runtime.clone();
            let spec = spec.clone();
            let cancel = cancel.clone();
            async move {
                let _ = runtime.infer_with_cancel(&spec, "drop me", cancel).await;
            }
        });
        for _ in 0..100 {
            if state.started.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(state.started.load(Ordering::SeqCst), 1);
        handle.abort();
        let _ = handle.await;
        for _ in 0..100 {
            if state.resets.load(Ordering::SeqCst) > 0 && state.active.load(Ordering::SeqCst) == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            cancel.is_cancelled(),
            "dropping infer future must cancel token"
        );
        assert!(state.resets.load(Ordering::SeqCst) > 0);
        assert_eq!(state.active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn gpu_load_failure_falls_back_to_cpu_and_completes_inference() {
        struct GpuFallbackEngine {
            seen_gpu_layers: Arc<Mutex<Vec<i32>>>,
        }
        impl Engine for GpuFallbackEngine {
            fn load(&mut self, spec: &ModelSpec) -> Result<(), String> {
                self.seen_gpu_layers.lock().unwrap().push(spec.gpu_layers);
                if spec.gpu_layers != 0 {
                    Err("no GPU device available".into())
                } else {
                    Ok(())
                }
            }
            fn unload(&mut self) {}
            fn generate(
                &mut self,
                _prompt: &str,
                _cancel: &CancelToken,
            ) -> Result<String, InferenceError> {
                Ok("{}".to_string())
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let runtime = Runtime::spawn(
            Box::new(GpuFallbackEngine {
                seen_gpu_layers: seen.clone(),
            }),
            4,
        );
        let gpu_spec = spec(Duration::from_secs(10)).with_gpu_layers(-1);
        let first = runtime
            .infer(&gpu_spec, "one", Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(first, "{}");
        assert_eq!(*seen.lock().unwrap(), vec![-1, 0]);

        // Subsequent inference with the same spec must reuse the fallen-back model
        // without retrying the failing GPU load.
        let second = runtime
            .infer(&gpu_spec, "two", Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(second, "{}");
        assert_eq!(*seen.lock().unwrap(), vec![-1, 0]);

        // Changing gpu_layers reloads the model.
        let partial_spec = spec(Duration::from_secs(10)).with_gpu_layers(16);
        let third = runtime
            .infer(&partial_spec, "three", Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(third, "{}");
        assert_eq!(*seen.lock().unwrap(), vec![-1, 0, 16, 0]);
    }

    #[cfg(feature = "llama")]
    #[test]
    fn model_params_respect_compiled_gpu_feature_and_zero_override() {
        let auto = super::llama::model_params_for_gpu_layers(-1);
        let partial = super::llama::model_params_for_gpu_layers(16);
        let cpu = super::llama::model_params_for_gpu_layers(0);
        assert_eq!(cpu.n_gpu_layers(), 0);
        if crate::ai::gpu_offload_supported() {
            assert_eq!(auto.n_gpu_layers(), i32::MAX);
            assert_eq!(partial.n_gpu_layers(), 16);
        } else {
            assert_eq!(auto.n_gpu_layers(), 0);
            assert_eq!(partial.n_gpu_layers(), 0);
        }
    }
}
