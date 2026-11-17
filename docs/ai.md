# Local AI settings, catalog and model downloads

AI is optional and **off by default**. The archive stores only consent and the
selected model id; installed model files are owned by the filesystem. This
document records the catalog data source and the download/error rules that the
API and Activities expose.

## Activation and privacy

AI is enabled only in **Settings → AI**. Enabling it downloads nothing and loads
nothing; the owner must still choose a catalog model and start its download.
Disabling it cancels active downloads, unloads the model and deletes the Ask
projection. With AI off there is no model load, no inference and no network
request related to AI.

The model never sees archive rows. What it receives depends on the answer mode:

- **Guided questions.** The Rust prompt contains only the question, the
  supported DSL vocabulary and examples; the model emits DSL JSON constrained
  by a grammar. Rust executes that DSL against the canonical consultation
  selection and computes every number.
- **Free questions (SQL).** The model receives the Ask projection's schema and
  column meanings (generated from `TABLES`), the counting rules, today's date,
  the question, and on failure a sanitized SQLite error that names only the
  error class and identifiers from the model's own SQL. It never receives
  archive values, names, counts, results, prior results or credentials. Its SQL
  runs read-only against a separate projection file inside the
  [SQL sandbox](ask-projection.md), never against the archive.

By default there is no cloud AI call and no telemetry. The only additional
outbound request is the model file download from the pinned catalog URL,
started explicitly by the owner. The one exception is the opt-in
[bring your own AI](#bring-your-own-ai) engine, which sends the question text
(and, in free mode, the schema and sanitized errors) to a server the owner
names. Questions and results stay in the instance. Model files are treated as
untrusted input, are verified by SHA-256 before use and cannot inject SQL.

## Answer modes

Ask answers in one of two modes, chosen per engine in **Settings → AI**. Guided
questions are the default for the downloaded local model; free questions (SQL)
are the default for "bring your own AI". The choice is stored per engine, so
switching engines restores that engine's mode, and the Ask page shows the active
mode with a link to change it.

| | Guided questions | Free questions (SQL) |
| --- | --- | --- |
| What it answers | the metrics, filters, periods and groupings in the frozen vocabulary | anything in the Ask projection: devices, countries, shuffle, skips, offline, combined conditions, comparisons, ties, first/last play, gaps and streaks |
| When the model misunderstands | refuses, or the chips show the wrong interpretation and can be edited before or after asking | may return a wrong but plausible result; the "computed" sentence and the SQL show what it did, and the SQL can be edited and re-run |
| Who computes the numbers | tested Rust executors | SQLite, on the model's query, inside the read-only sandbox |
| Privacy | the model never sees archive values | the same, plus the SQL sandbox: only the schema, question and sanitized errors leave, and the projection is a separate file with no settings or credentials |
| Recommended for | every downloaded local catalog model (0.6B–4B) | your own AI server or a hosted API with a larger model |

Guided questions are **frozen** after the rolling 90/180-day periods of
[#201](https://github.com/itsmrma/scrobbl-rs/issues/201): new metrics, filters
and periods are not added. A question the vocabulary cannot express is answered
by switching to free questions, not by extending guided questions.

The recommendations are fixed, not measured per model: every local catalog
entry reports `recommendation: "guided"`, the "bring your own AI" option
recommends free questions, and the catalog stores no SQL accuracy field.
Measured guided accuracy (#181) is in the
[Ask parser evaluation](reports/ask-eval.md); the SQL evaluation that scores
free answers by result, including what was **not** measured, is in the
[Ask SQL evaluation](reports/ask-eval-sql.md). The projection data model and
every sandbox layer and rejection rule are documented in
[Ask projection and SQL sandbox](ask-projection.md).


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
| `ai_provider` | `"local"` | `local` (downloaded model) or `remote` (own AI server or API) |
| `ai_remote_url` | `""` | OpenAI-compatible base URL, e.g. `http://192.168.1.20:11434/v1` |
| `ai_remote_model` | `""` | model name sent to that server |
| `ai_remote_key` | `""` | optional API key; stored like the Spotify client secret and never returned (`remote_key_set` only) |
| `ai_answer_mode_local` | `"guided"` | answer mode for the downloaded local engine: `guided` or `sql` |
| `ai_answer_mode_remote` | `"sql"` | answer mode for the remote engine: `guided` or `sql` |

Enabling AI does not download anything. If no model is installed the settings
response says so explicitly. Disabling AI cancels active model downloads so no
further network transfer continues.

`GET /api/v1/ask/status` reports `enabled`, `runtime_available`, `model_ready`
and `model_loading`. `model_ready` is filesystem truth for the selected model;
`model_loading` comes from the live inference worker and is true only while a
load is in progress.

## Bring your own AI

Instead of downloading a model, the owner can point Ask at an AI server they
already run or pay for. Choose **My own AI server or API** under Settings → AI,
then enter the address, the model name and, if needed, an API key. Any server
that implements the OpenAI `POST {base}/chat/completions` protocol works:
Ollama, LM Studio, llama.cpp `llama-server`, vLLM, or a hosted API (OpenAI,
OpenRouter, Groq, Together, ...). Tools that are not themselves an
OpenAI-compatible server (for example the OpenCode CLI) cannot be used directly;
use the provider endpoint behind them.

What changes and what does not:

- The server receives the system prompt and the question text. In guided mode
  that is the Ask vocabulary and examples; in free mode it is the projection
  schema, the counting rules and the question. It never receives archive rows,
  names from the library, numbers or results. Guided replies are validated and
  every number is computed by Rust; free replies are SQL that runs read-only in
  the [SQL sandbox](ask-projection.md). Only sanitized errors, never values, go
  back for a repair round.
- The request asks for schema-constrained JSON (`response_format: json_schema`).
  A server that answers 400/422 is retried with `json_object`, then without
  `response_format`; the JSON is extracted from the reply even when it comes
  inside a Markdown fence or after a reasoning block.
- With the remote engine nothing is downloaded or loaded and the build does not
  need the llama runtime. Ask works in a `--no-default-features` build.
- The address must be `http://` or `https://`, may be on the private network and
  may not contain credentials. The key is sent only as a bearer token to that
  address and is never returned by the API, logged or echoed in errors.
- **Test connection** sends one sample question and reports the result.

### What a stronger model can do

The remote engine uses a richer prompt and schema than the local one. The model
still only emits typed vocabulary; it never reads the archive or any result.

- **Chained questions.** The reply is `{"steps": [...]}` with up to 4 steps. A
  step may set `ref_step` and `ref_row` to build on one row of an earlier step:
  the artist, album or track when that step grouped by artist, album or track,
  or the dates, weekday or hour when it grouped by day, week, month, year,
  weekday or hour. Rust executes the steps in order, reads the row itself and
  fills the entity or range. A step also inherits the earlier step's filters
  that it leaves unset ("that month" keeps meaning that artist). Example: "in
  the month I listened to Ado the most, what was my top track?" is two steps.
  A step that cannot bind (the earlier one was not grouped, or has no such row)
  fails with an explanation and the earlier steps stay visible.
- **Precise questions.** Besides the usual fields a step can carry an inclusive
  `from`/`to` date range (`YYYY-MM-DD`, validated), an `hour` (0-23, local
  time) and a `source` (`spotify_import`, `spotify_api`, `listenbrainz`). The
  range replaces the period and the year and appears as editable chips. The
  system prompt states today's date (UTC) so the model can turn relative
  ranges such as "the last two weeks" into dates; nothing else about the
  archive is added.
- **Rolling periods.** Both engines can choose `last_90_days` ("last 3
  months") and `last_180_days` ("last 6 months") besides 7, 30 and 365 days.
  Before these existed, "last 3 months" could only be mapped to "last month"
  ([#201](https://github.com/itsmrma/scrobbl-rs/issues/201)).
- **Follow-ups.** After an answer, "Continue from the previous answer" (on by
  default with a remote AI) sends the previous question and its interpretation
  chips, so "and last year?" or "what about Radiohead?" work. Results are never
  sent; the question is clipped to 500 characters and at most 20 chips are
  sent.
- Every answer, including each step, links to the listens it was computed from.
  Editing a chip re-runs that single step.

A reply that is a single flat interpretation (what the local model emits) is
still accepted. The local model keeps its smaller grammar and does not chain.
- Quality depends on the model. Small models may misread a question; the
  interpretation chips stay editable. A hosted provider sees every question you
  ask, so enabling it is an explicit, saved consent.

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
optional GPU, the guided-mode 3072-token context, backend, measured speed and
parser scores from the versioned evaluation set (#181): intent accuracy, slot
F1, refusal accuracy, exact-match accuracy and per-language exact match. Entries
that were not measured keep `null` rather than an estimate. The catalog's
`context_tokens` figure describes guided questions only; free questions (SQL)
run with a 4096-token context. Every entry also carries a
fixed `recommendation`: `guided` for every downloadable local model. The
"bring your own AI" option recommends `sql`; `AiCatalog.remote_recommendation`
reports that fixed value. No measured SQL accuracy is stored or shown.

## Model choice and memory

The settings page shows the catalog size, minimum and recommended RAM, CPU
cores, measured speed and parser accuracy before a download starts. The v1
CPU-only guidance is:

| Tier | Catalog id | Weights on disk | Typical role |
| --- | --- | ---: | --- |
| Light | `qwen3-0.6b-q4_k_m` | ~378 MiB | Fastest; 55.8% measured intent, so use the chips |
| Default (provisional) | `qwen3-1.7b-q4_k_m` | ~1.2 GiB | 78.9% measured intent; #181 did not confirm this default |
| Quality | `qwen3-4b-q4_k_m` | ~2.3 GiB | Best measured accuracy (92.8%) but still below the 4B target |

The #181 evaluation did not confirm the default: no model met its target, and
the 4B is the closest but roughly twice as slow. Keep the feature opt-in, treat
the interpretation chips as part of the answer, and read the
[evaluation report](reports/ask-eval.md) before recommending a model.

8B and 32B entries remain `gpu_future` and are not selectable in v1. Guided
questions use a 3072-token context; free questions (SQL) use 4096 tokens. One
model is loaded at a time either way, so the loaded model
needs its file size plus roughly 1–2 GiB of working memory; use the catalog's
minimum/recommended RAM values as the authoritative figures. The downloader
refuses to start without enough free disk space for the remaining bytes and
rechecks at every checkpoint.

## Filesystem layout

```text
<data_dir>/models/<catalog-id>/
  <file>.gguf
  metadata.json
```

`<data_dir>/models` can be replaced by `ai_model_dir`. The filesystem is the
source of truth: the database stores no installed-model inventory.

In the official image `SCROBBL_DATA_DIR=/data`, so the default model directory
is **`/data/models`**. There is no separate `SCROBBL_AI_MODEL_DIR` variable; an
operator can instead set the absolute `ai_model_dir` setting (for example a
different mounted folder). Keep models on the same persistent `/data` volume:
the container root filesystem is read-only and the app never writes model files
(partial downloads, metadata or temporary files) outside the configured model
directory. An external `ai_model_dir` must be a separately writable mount. See
[Docker installation](docker.md).

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
loaded at a time. The Docker build stage installs `cmake`, `clang`,
`libclang-dev` and `python3` to build llama.cpp from the vendored crate sources;
the runtime stage installs `libstdc++6` for the linked C++ runtime. Because
`llama-cpp-2` is built with `default-features = false`, its optional OpenMP
feature is off, `GGML_OPENMP=OFF`, and **no `libgomp1`** is needed. No CUDA,
Metal, Vulkan, BLAS or GPU toolchain is installed. The image is built for
linux/amd64 (published and smoke-tested) and linux/arm64 (built on trusted
main/tag pushes; see [Docker installation](docker.md)).

The release profile strips symbols, which keeps the distributed binary inside
the 30 MiB budget without enabling LTO (LTO would add link time for no measured
size need). The Docker workflow also measures the runtime image increment
against `debian:bookworm-slim`; the corresponding-source/notices bundle is
reported separately because it is a fixed GPL distribution obligation, not part
of the AI runtime addition. Budgets: **runtime image delta ≤ 30 MiB**, **binary
≤ 30 MiB**. CI records both in `ci-logs/image-size.json` and fails the build if
either is exceeded.

## Licenses and attribution

llama.cpp and ggml are MIT-licensed and are compiled from the pinned
`llama-cpp-sys-2` sources. The `llama-cpp-2`/`llama-cpp-sys-2` Rust bindings are
MIT OR Apache-2.0 (Copyright (c) Dial AI); both texts are kept in
`docs/licenses/upstream/` and copied into the image notices. The full
attribution and the exact upstream revision are in
[THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

Catalog models are **Apache-2.0** works from the Qwen team (Alibaba Group). The
catalog response shows the license name and URL before download, and the
installed `metadata.json` records it. Weights are downloaded by the owner from
the pinned upstream repository; they are never distributed in the Git
repository, the container image or the source archive, and they remain under
their upstream license.

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
- The SQL sandbox has its own wall-clock budget, `SCROBBL_ASK_SQL_TIMEOUT_MS`
  (default `3000`, any positive whole number of milliseconds, no upper bound).
  It is read once when Ask is installed, so it applies to every sandbox run:
  model SQL, each repair round and owner-edited `POST /api/v1/ask/sql`. The
  instruction budget stays fixed at 5,000,000. See
  [Ask projection and SQL sandbox](ask-projection.md#5-time-and-cancellation).
- After `ai_unload_minutes` of inactivity the model is dropped. The next
  question re-reads settings and pays the load cost.
- `LlamaSampler::sample` already accepts the token; the adapter never calls
  `accept` afterwards (utilityai/llama-cpp-rs#1007). Timing measurement is off
  by default; set `SCROBBL_ASK_DEBUG=1` to enable it (and llama.cpp logs).
- Guided questions use a 3072-token context and at most 256 new tokens. Free
  questions (SQL) use a 4096-token context and at most 512 new tokens; the SQL
  grammar needs the extra room for a full statement. Output is accumulated as
  bytes and decoded once as UTF-8, so a multi-byte character split across
  tokens cannot be corrupted.
- A missing or corrupt GGUF returns an explicit error; the worker is detached
  with a cancellation signal on shutdown and never panics on filesystem races.

### Model-facing schema and grammar

The model only emits `src/ask/schema.rs::ModelOutput` (schema v2): `artist`,
`group_by`, `limit`, `metric`, `month`, `name`, `order`, `period`, `target`,
`weekday` and `year`. Every field is required in the grammar; `artist`, `month`
and `year` default when a recorded v1 output omits them. Strings are literal
names copied from the question; `year` is `0` or a four-digit year that Rust
bounds to 1900–2200; every other field is an enum with explicit `none`/`any`
values, so the grammar stays finite. There are no ids or SQL.

Metrics: `plays`, `time_listened`, `distinct_tracks`, `distinct_artists`,
`distinct_albums`, `first_play`, `last_play`, `streak`, `milestones`, `rank`
(position of the entity among its kind by plays, ties share a position),
`share` (entity plays over all plays in the same selection) and
`listening_days` (distinct calendar days with a counted play). Periods add
`last_7_days`, `last_30_days` and `last_365_days`; a named year or month narrows
the selection directly.

Name resolution:

- `artist` narrows a named track or album ("Show by Ado"); with no named track
  or album it is the subject ("my top songs by Ado").
- A track name resolves to the song across its editions: tracks with the same
  folded title and the same credited artists (single, album, deluxe) count
  together, so "first time I listened to Show" finds the earliest edition.
- The page context (artist, album or track page) answers questions that name
  nothing, and narrows a named track or album to the page's artist or album.
  It never replaces what the question names: before 0.1 alpha, asking about
  the song "Show" on Ado's page answered about Ado. A name that is not in the
  page's scope is still looked up in the whole library.
- Ambiguity remains explicit: candidates are offered, and a chosen artist
  candidate fills the scope while a chosen song fills the entity.

`src/ask/schema/ask.schema.json` is checked in and generated from the same enum
lists as the Rust deserializer; a unit test fails if it drifts. The JSON-Schema
subset is converted to GBNF by `src/ask/grammar.rs`. Regenerate the checked-in
schema after changing an enum:

```powershell
$env:UPDATE_ASK_SCHEMA='1'; cargo test --lib checked_in_schema_matches_the_generated_one; Remove-Item Env:UPDATE_ASK_SCHEMA
```

The prompt (`src/ask/prompt.rs`) is compact English with EN/IT examples and
embeds the schema without descriptions to stay well inside the guided
3072-token context; a test bounds its size. It
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

`evals/ask/corpus.jsonl` is the versioned eval set: 220 questions (110 English,
110 Italian pairs) covering every metric, period, weekday, grouping, limit and
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
needs no model and no network. Normal CI never runs an evaluation, never loads a
model and never downloads weights; it only compiles the harness (under the
default `llama` feature) and runs the model-free corpus checks. The `ask-eval`
binary is an offline tool, and its `required-features = ["llama"]` only makes it
buildable in the official feature set. The measured results, methodology, exact commands and target verdict are
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

Free questions have their own evaluation: it scores SQL answers **by typed
result** on a fixed synthetic fixture and reports correct answers, honest
failures and wrong-but-plausible answers. The reference remote model was not
measured in this environment, and local catalog models are not measured; both
facts are stated in the [Ask SQL evaluation](reports/ask-eval-sql.md). The corpus
format is in [`evals/ask/README.md`](../evals/ask/README.md).

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
- `POST /api/v1/ask` — asks a question; body `{question, context?, previous?,
  mode?, request_id?}`. `mode` (`guided` or `sql`) applies to that request only
  and is never persisted; an unknown value is a `400`, and omitting it keeps the
  stored mode for the selected engine.
- `POST /api/v1/ask/sql` — runs owner-edited SQL through the same sandbox.
- `POST /api/v1/ask/cancel` — requests cancellation of an in-flight Ask; body
  `{request_id?}`, where omitting it cancels every active question.
- Every Ask route requires the owner session and CSRF. A read API key is denied
  (`403`), even alongside an owner cookie.
- `POST /api/v1/ai/settings` also accepts `provider`, `remote_url`,
  `remote_model` and `remote_key` (omit it to keep the stored key, send `""` to
  clear it). `POST /api/v1/ai/test` contacts the remote server when
  `provider` is `remote`.
