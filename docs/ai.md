# Local AI settings, catalog and model downloads

AI is optional and **off by default**. The archive stores only consent and the
selected model id; installed model files are owned by the filesystem. This
document records the catalog data source and the download/error rules that the
API and Activities expose.

## Settings keys

Settings live in the archive `settings` table and are written through the
archive writer after `auth::require_write` (owner session + CSRF).

| key | default | meaning |
| --- | --- | --- |
| `ai_enabled` | `"0"` | explicit consent; nothing is downloaded, loaded or queried while `"0"` |
| `ai_active_model` | `""` | catalog id selected for questions |
| `ai_threads` | `"4"` | inference thread count (1–64) |
| `ai_unload_minutes` | `"15"` | idle unload delay (1–1440 minutes) |
| `ai_model_dir` | `""` | optional absolute model folder; empty means `<data_dir>/models` |

Enabling AI does not download anything. If no model is installed the settings
response says so explicitly. Disabling AI cancels active model downloads so no
further network transfer continues.

`GET /api/v1/ask/status` reports `enabled`, `runtime_available`, `model_ready`
and `model_loading`. `model_ready` is filesystem truth for the selected model;
`model_loading` comes from the live inference worker and is true only while a
load is in progress.

## Curated catalog

The catalog is compiled into the binary and cannot be extended by a client. A
download URL is derived from the pinned repository, revision and file; clients
never supply a URL, and there is no local GGUF import. Every value below was
read from the Hugging Face API (`/api/models/<repo>` and
`/api/models/<repo>/tree/<revision>?recursive=true`) when this slice was
implemented. The SHA-256 values are the Hugging Face LFS object ids.

| id | repo | revision | file | bytes | SHA-256 |
| --- | --- | --- | --- | ---: | --- |
| `qwen3-0.6b-q4_k_m` | `unsloth/Qwen3-0.6B-GGUF` | `50968a4468ef4233ed78cd7c3de230dd1d61a56b` | `Qwen3-0.6B-Q4_K_M.gguf` | 396,705,472 | `ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a` |
| `qwen3-1.7b-q4_k_m` | `ggml-org/Qwen3-1.7B-GGUF` | `daeb8e2d528a760970442092f6bf1e55c3b659eb` | `Qwen3-1.7B-Q4_K_M.gguf` | 1,282,439,264 | `d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5` |
| `qwen3-4b-q4_k_m` | `Qwen/Qwen3-4B-GGUF` | `bc640142c66e1fdd12af0bd68f40445458f3869b` | `Qwen3-4B-Q4_K_M.gguf` | 2,497,280,256 | `7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5` |
| `qwen3-8b-q4_k_m` | `Qwen/Qwen3-8B-GGUF` | `7c41481f57cb95916b40956ab2f0b139b296d974` | `Qwen3-8B-Q4_K_M.gguf` | 5,027,783,488 | `d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785` |
| `qwen3-32b-q4_k_m` | `Qwen/Qwen3-32B-GGUF` | `938a7432affaec9157f883a87164e2646ae17555` | `Qwen3-32B-Q4_K_M.gguf` | 19,762,149,024 | `efd971561896866f0e910cce52761ca77b1b138090c7f15fe284676d57d1f689` |

Why two repositories are not `Qwen/Qwen3-*-GGUF`:

- The official Qwen repositories publish only `Q8_0` for 0.6B and 1.7B. The
  spike (#172) measured the pinned `Q4_K_M` builds from `unsloth` and
  `ggml-org`, so the catalog uses the exact files that produced the documented
  speed figures; parser scores are from the #181 evaluation on the same files.
  Sizes and LFS SHA-256 values were still read from the Hugging Face API at
  implementation time.
- 4B, 8B and 32B are official `Qwen/Qwen3-*-GGUF` Q4_K_M files. Their 4B size
  and checksum match the spike manifest.

No entry is marked `unverified`: every referenced revision and file exists in a
public, non-gated repository, and its size/SHA-256 was verified against the LFS
metadata. If a future value cannot be verified, the entry must use
`status: "unverified"`, be non-selectable and explain the missing value here.

`8B` and the 27B/32B class are present with `status: "gpu_future"` and
`selectable: false`. They are listed for planning only and cannot be downloaded
or selected in v1.

Catalog metadata also records license, minimum/recommended RAM, CPU cores,
optional GPU, 2048-token context, backend, measured speed and parser scores
from the versioned evaluation set (#181): intent accuracy, slot F1, refusal
accuracy, exact-match accuracy and per-language exact match. Entries that were
not measured keep `null` rather than an estimate.

## Filesystem layout

```text
<data_dir>/models/<catalog-id>/
  <file>.gguf
  metadata.json
```

`<data_dir>/models` can be replaced by `ai_model_dir`. The filesystem is the
source of truth: the database stores no installed-model inventory.

`metadata.json` is written atomically (temporary file, fsync, rename, directory
sync) after the checksum passes:

```json
{
  "catalog_id": "qwen3-1.7b-q4_k_m",
  "file": "Qwen3-1.7B-Q4_K_M.gguf",
  "sha256": "…",
  "size_bytes": 1282439264,
  "installed_at_ms": 1760000000000,
  "license": "Apache-2.0",
  "checksum_verified": true
}
```

The catalog endpoint rescans the folder on every request and at startup.
Missing/corrupt metadata, a missing file or a size mismatch becomes an explicit
`error` on the installed entry instead of crashing startup. Orphans not in the
catalog remain visible and can be deleted, but cannot be selected.

## Download job

`POST /api/v1/ai/models/{id}/download` enqueues the durable job
`kind = "model_download"`, visible in Activities. The job parameters contain
only `{ "model_id": "…" }`; the worker resolves the pinned source from the
catalog. The worker:

1. Refuses to start without enough free disk space (`model_disk_full`), and
   checks space again at every progress checkpoint (512 KiB).
2. Downloads to `<file>.part`. If a partial file exists, sends
   `Range: bytes=<len>-` and appends a `206 Partial Content` response. If the
   source ignores Range (`200`), the partial is discarded and the download
   restarts from zero.
3. Exposes byte progress through `jobs.processed`, `jobs.phase` and the
   `job_members` row (`expected_rows` = total bytes), so Activities shows
   `processed / expected`.
4. Verifies the exact total byte size and the SHA-256 over the complete file.
5. Renames the part file to its final name and writes `metadata.json`. The
   archive never sees a partially verified model as installed.

Error rules:

| condition | error code | partial file |
| --- | --- | --- |
| free space below remaining bytes | `model_disk_full` | kept, so a retry can resume after freeing space |
| SHA-256 mismatch or file larger than expected | `model_checksum_mismatch` | removed; retry starts clean |
| user cancellation (`jobs.cancel_requested`) | `model_cancelled` | removed; a new download starts clean |
| transient network/HTTP 429/5xx/stream error | `model_download_retry` (state `waiting`) | kept and resumed with Range |
| source refuses with another HTTP status | `model_source_rejected` | kept at the last checkpoint |
| five failed attempts | `model_download_failed` | kept at the last checkpoint |

Transient failures use the existing durable job pause/backoff mechanism
(`retry_at_ms`). Activities can retry a failed or cancelled download; a retry
resumes from the bytes still on disk unless the partial was removed by the rule
above.

## Runtime feature

`src/ai/mod.rs::runtime_available()` returns `cfg!(feature = "llama")`. The
`llama` feature is a default feature and pulls in
`llama-cpp-2 = "=0.1.161"` with `default-features = false` (CPU-only, no CUDA,
no Metal build flag, no weights in the binary). Build or test with
`--no-default-features` to produce a binary without the runtime: settings,
catalog and downloads stay usable, while Ask and the "test model" action are
reported as unavailable. No command pretends inference works.

The official image compiles the runtime; weights are downloaded at runtime from
the curated catalog and never enter the repository or the image. One model is
loaded at a time. The Docker build stage installs `cmake`, `clang` and
`libclang-dev` to build llama.cpp; the runtime stage installs `libstdc++6` for
the linked C++ runtime. No GPU toolchain is installed.

## Ask parser runtime (#178)

`AppState::new` installs the real parser when the feature is on and the
unavailable parser otherwise. The parser reads `ai_enabled`, `ai_active_model`,
`ai_threads` and `ai_unload_minutes` on every question, resolves the installed
file from the filesystem (checksum-verified metadata, catalog file name) and
never trusts a client-supplied path.

The worker (`src/ask/runtime.rs`) is one dedicated OS thread. It owns the llama
backend and model and a bounded queue (4). Tokio handlers only enqueue and await
a oneshot; inference never runs on a tokio worker. Behaviour:

- Lazy load on the first question; `loading` is visible through the status
  endpoint, and a failed load is an explicit error rather than a panic.
- One inference at a time. The queue is bounded; a full queue returns a clear
  "busy" error.
- Timeout via `SCROBBL_ASK_TIMEOUT_MS` (default `30000`). Cancellation is
  checked between sampled tokens. On timeout the request returns an explicit
  error and the context is dropped/reset; no half-decoded sequence is reused.
- After `ai_unload_minutes` of inactivity the model is dropped. The next
  question re-reads settings and pays the load cost.
- Context 2048 tokens, greedy sampling, at most 256 new tokens. Output is
  accumulated as bytes and decoded once as UTF-8, so a multi-byte character
  split across tokens cannot be corrupted.
- `LlamaSampler::sample` already accepts the token; the adapter never calls
  `accept` afterwards (utilityai/llama-cpp-rs#1007). Timing measurement is off
  by default; set `SCROBBL_ASK_DEBUG=1` to enable it (and llama.cpp logs).
- A missing or corrupt GGUF returns an explicit error; the worker is detached
  with a cancellation signal on shutdown and never panics on filesystem races.

### Model-facing schema and grammar

The model only emits `src/ask/schema.rs::ModelOutput`: `metric`, `target`,
`name`, `period`, `weekday`, `group_by`, `order` and `limit`. Every field is
required and every non-name field is an enum; there are no numeric slots, ids or
SQL. `limit`/`group_by`/`weekday` use explicit `none`/`any` values instead of
missing keys so the grammar stays finite. Rust maps the output to the shared
DSL, resolves the literal name through the existing search and fills optional
filters with `None`.

`src/ask/schema/ask.schema.json` is checked in and generated from the same enum
lists as the Rust deserializer; a unit test fails if it drifts. The JSON-Schema
subset is converted to GBNF by `src/ask/grammar.rs`. Regenerate the checked-in
schema after changing an enum:

```powershell
$env:UPDATE_ASK_SCHEMA='1'; cargo test --lib checked_in_schema_matches_the_generated_one; Remove-Item Env:UPDATE_ASK_SCHEMA
```

The prompt (`src/ask/prompt.rs`) is compact English with EN/IT examples. It
states that names are copied literally, that time words such as "today",
"Fridays" and "last month" never enter `name`, that the model never writes SQL,
and that out-of-scope or injection requests use `metric: "unsupported"`. Qwen3
thinking is disabled by pre-filling
`assistant\n<think>\n\n</think>\n\n` (verified by the #172 spike).

### Real model test

CI never downloads weights or touches the network. The real-model integration
test is ignored and reads a local GGUF path:

```powershell
$env:SCROBBL_TEST_MODEL='C:\models\Qwen3-0.6B-Q4_K_M.gguf'
cargo test --locked --test ask_runtime real_model_answers_three_supported_questions -- --ignored --exact --nocapture
Remove-Item Env:SCROBBL_TEST_MODEL
```

The test installs that file into a temporary archive, asks three supported
questions (including "Björk") and asserts the metric, literal entity and
period. It exercises the real prompt, grammar, worker and adapter; it is not
part of the normal suite.

### Parser evaluation (#181)

`evals/ask/corpus.jsonl` is the versioned eval set: 208 questions (104 English,
104 Italian pairs) covering every metric, period, weekday, grouping, limit and
order, plus Unicode names, ambiguous names, out-of-scope requests, prompt
injection and empty/gibberish input. `evals/ask/README.md` documents the label
format and how to add cases. The corpus contains no archive data.

The `ask-eval` binary loads one GGUF path, reuses the production prompt,
schema, grammar and greedy sampling, and reports intent accuracy, slot
precision/recall/F1, refusal precision/recall/accuracy, JSON validity and
latency (p50/p95/TTFT), overall and per language:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\mbena\Documents\scrobbl-rs\target'
cargo run --locked --release --bin ask-eval -- `
  --model C:\models\Qwen3-1.7B-Q4_K_M.gguf `
  --label qwen3-1.7b-q4_k_m --threads 8 `
  --out C:\temp\qwen3-1.7b.json
```

The corpus shape is checked by a unit test (`cargo test --bin ask-eval`), which
needs no model and no network. Normal CI never builds or runs the harness and
never downloads weights: the `ask-eval` binary is an offline tool, and its
`required-features = ["llama"]` only makes it buildable in the official feature
set. The measured results, methodology, exact commands and target verdict are
in [`docs/reports/ask-eval.md`](reports/ask-eval.md); catalog parser scores are
filled from that run and stay `null` for models that were not measured.

Catalog parser fields mean: `intent` is intent accuracy over all 208 cases,
`slots` is the F1 over active slots, `refusals` is refusal accuracy and
`exact_match` is the fraction of supported cases with every filter slot correct
(the metric is scored by `intent`). `languages[].exact_match` is the same exact
filter match restricted to one language (104 cases each); `languages[].cases`
records that denominator.

The measured #181 intent accuracy is **55.77%** for the 0.6B, **78.85%** for the
1.7B and **92.79%** for the 4B. No model met its target; the 1.7B default is
therefore provisional and its catalog description warns to treat the chips as
part of the answer. The full per-language, slot and refusal results and the
target verdict are in the [evaluation report](reports/ask-eval.md).

## API summary

- `GET /api/v1/ai/settings` — owner session; defaults and effective model folder.
- `POST /api/v1/ai/settings` — owner + CSRF; validates threads, unload minutes,
  absolute model folder and the selected installed model.
- `GET /api/v1/ai/catalog` — catalog merged with installed state, download jobs,
  disk usage and explicit errors.
- `POST /api/v1/ai/models/{id}/download` — requires AI enabled; catalog only.
- `POST /api/v1/ai/models/{id}/cancel` — cancels the active download job.
- `DELETE /api/v1/ai/models/{id}` — body `{ "confirm": true, "force": bool }`;
  refuses without confirmation, refuses the active model unless `force` clears
  the selection, and reports `freed_bytes`.
- `POST /api/v1/ai/models/{id}/select` — installed, verified and selectable only.
- `POST /api/v1/ai/test` — reports runtime/model readiness without loading or
  running inference.
- `GET /api/v1/ask/status` — enabled/runtime/model-ready state for the Ask page.
