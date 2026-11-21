# Local AI settings, catalog and local SQL models

AI is optional and **off by default**. Ask operates exclusively with **local GGUF models** running inside scrobbl.rs that translate natural-language questions into read-only SQL queries over the [Ask projection](ask-projection.md). Remote AI servers ("bring your own AI"), external MCP integrations, Guided DSL execution, and UI-control actions are not supported.

You can use the curated **Qwen3.5 2B** model from the built-in catalog, or import your own local `.gguf` model on the server.

## Activation and privacy

AI is enabled only in **Settings → AI**. Enabling it downloads nothing and loads nothing; the owner must still choose a catalog model (and download it) or configure a custom local `.gguf` model path. Disabling it cancels active downloads, unloads the model and deletes the Ask projection. With AI off there is no model load, no inference and no network request related to AI.

- **Local SQL only:** The model receives only the Ask projection's schema and column meanings (generated from `TABLES`), counting rules, today's date, the question text, and on failure a sanitized SQLite error naming only the error class and identifiers from the model's own SQL.
- **No archive exposure:** The model never receives archive rows, library names, counts, results, prior query values, or credentials. Every SQL query executes read-only inside the [SQL sandbox](ask-projection.md) against `ask_projection.db`, never against `archive.db` or `auth.db`.
- **No external AI calls:** Questions, prompts, and results never leave the server. The only AI-related outbound network request is downloading a pinned curated catalog `.gguf` file when explicitly started by the owner.
- **Editable SQL:** Every answered question keeps its generated SQL visible and editable via **Edit / re-run SQL** (`POST /api/v1/ask/sql`), running through the same read-only sandbox.

For result-based evaluation of SQL questions, see [`evals/ask/README.md`](../evals/ask/README.md) and the [Ask SQL evaluation report](reports/ask-eval-sql.md). For security invariants and linter enforcement around Ask and model handling, see [Agent security and anti-slop guide](agents/security.md).

## Settings keys

Settings live in the archive `settings` table and are written through the archive writer after `auth::require_write` (owner session + CSRF).

| key | default | meaning |
| --- | --- | --- |
| `ai_enabled` | `"0"` | explicit consent; nothing is downloaded, loaded or queried while `"0"` |
| `ai_active_model` | `""` | catalog id (or custom model folder id) selected for questions |
| `ai_custom_model_path` | `""` | optional path to a local `.gguf` file (absolute path or relative to `ai_model_dir`); overrides `ai_active_model` when set |
| `ai_threads` | `"4"` | inference thread count (`1–64`) |
| `ai_gpu_layers` | `"-1"` | GPU offload layers (`-1` = automatic/all layers, `0` = CPU-only, `1–256` = custom layer count; forced to `0` on CPU-only builds, with automatic CPU fallback if GPU load fails at runtime) |
| `ai_unload_minutes` | `"15"` | idle unload delay (`1–1440` minutes) |
| `ai_model_dir` | `""` | optional absolute model folder; empty means `<data_dir>/models` |

Enabling AI does not download anything. Saving a custom path or switching the active catalog model unloads any currently active inference session cleanly so the next question uses the new selection. Disabling AI cancels active model downloads so no further network transfer continues.

`GET /api/v1/ask/status` reports `enabled`, `active_model`, `custom_model_path`, `runtime_available`, `model_ready` and `model_loading`. `model_ready` reflects verified filesystem readiness for the selected model; `model_loading` comes from the live inference worker and is true only while a load is in progress.

## Curated catalog (Qwen3.5 2B Ask SQL)

The curated catalog is compiled into the binary (`src/ai/catalog.rs`) and holds a single model: Qwen3.5 2B fine-tuned on scrobbl.rs Ask SQL ([Training the Ask SQL model](ask-training.md)). A download URL is derived from the pinned repository, revision and file; clients never supply a URL for catalog downloads. Weights are verified by exact byte size and SHA-256 against the Hugging Face LFS object ids.

| id | repo | revision | file | bytes | SHA-256 |
| --- | --- | --- | --- | ---: | --- |
| `qwen3.5-2b-q4_k_m` | `itsmrma/scrobbl-ask-qwen3.5-2b-sql-GGUF` | `aa1c44ceda918948cb6f376cf0d05df7a2a3037e` | `scrobbl-ask-qwen3.5-2b-sql-Q4_K_M.gguf` | 1,274,396,512 | `a62c87f95a0b721bf1ddc6f7bdb5ada18fbe4fe5e33698ed8720b106cfefe50f` |

Earlier releases listed base `unsloth/Qwen3.5-2B-GGUF` (`0.8B`, `2B`, `4B`, `9B`, `27B`). Files of those models already downloaded stay listed under installed models, where they can be deleted.

The model runs SQL generation with an **8192-token** context window and up to **1024** generated tokens (`src/ask/runtime.rs`).

## Using custom local `.gguf` models

You can run any GGUF model compatible with `llama.cpp` instead of the default Qwen3.5 catalog models:

1. **Direct file path (`ai_custom_model_path`)**: In **Settings → AI → Custom local model path (.gguf)**, enter either an absolute path on the server (e.g. `/models/qwen2.5-coder-7b-q4_k_m.gguf`) or a path relative to `ai_model_dir` (e.g. `custom-qwen/model.gguf`).
2. **Managed subfolder in `ai_model_dir`**: Place a subfolder inside `<data_dir>/models/<your-model-id>/` containing a single `.gguf` file. `models::scan` detects valid single-GGUF directories automatically as selectable custom entries.

Every imported `.gguf` file is validated by `src/ai/models.rs::validate_gguf_file` before use:

- It must have a `.gguf` extension and must not be a symlink (`symlink_metadata`).
- It must be a regular file of at least 64 bytes whose magic header starts with `b"GGUF"`.
- Directories under `ai_model_dir` containing neither `metadata.json` nor a valid `.gguf` file are ignored completely rather than exposed or enumerated.
- Deleting a custom folder refuses symlinks, nested subdirectories, or unexpected host files instead of recursively deleting unknown contents.

## Filesystem layout

```text
<data_dir>/models/<catalog-id>/
  <file>.gguf
  metadata.json
```

`<data_dir>/models` can be overridden with `ai_model_dir`. In Docker (`SCROBBL_DATA_DIR=/data`), the default folder is `/data/models`; custom model volumes can be mounted read-only (e.g. `/models:ro`) and referenced via `ai_custom_model_path`.

Catalog `metadata.json` is written atomically after SHA-256 verification succeeds.

## Download job

`POST /api/v1/ai/models/{id}/download` enqueues the durable job `kind = "model_download"`, visible in Activities. The job refuses to start without enough free disk space (`model_disk_full`), supports `Range` resume into `<file>.part`, verifies byte size and SHA-256 (`model_checksum_mismatch` on failure), and promotes `<file>` + `metadata.json` atomically.

## Runtime features (CPU, Vulkan, Metal)

`src/ai/mod.rs::runtime_available()` returns `cfg!(feature = "llama")`. The default `llama` feature compiles `llama-cpp-2` (CPU-only, `default-features = false`, `GGML_OPENMP=OFF`).

- **Vulkan (Linux / Windows)**: compile with `cargo build --release --features vulkan` (or use the `:vulkan` Docker image built from `Dockerfile.vulkan`). On Linux, install `libvulkan-dev` and `glslc` (plus `libvulkan1` / `mesa-vulkan-drivers` at runtime); on Windows, install the LunarG Vulkan SDK (`glslc` on `PATH`). `compiled_backend()` reports `"vulkan"` and the curated Qwen3.5 2B SQL model or any custom `.gguf` offloads layers to GPU according to `gpu_layers` (with automatic CPU fallback if no Vulkan device is available).
- **Metal (macOS Apple Silicon)**: build with `--features metal` (or use the prebuilt `scrobbl-rs-macos-arm64-metal.tar.gz` release archive, since Docker containers on macOS run inside a Linux VM without Metal passthrough). `compiled_backend()` reports `"metal"`, the catalog reports `llama.cpp (Metal)`, `gpu_layers = -1` (default) offloads all layers to Metal, `1..=256` offloads that layer count, and `0` (or any build without `--features metal`) forces `0` layers on GPU.
- **Offline evaluation (`ask-eval`)**: run `cargo run --locked --features metal --bin ask-eval -- --model <path.gguf> --gpu-layers -1` on macOS Apple Silicon (or `--features vulkan` on Linux/Windows) to benchmark the curated Qwen3.5 2B SQL model or any custom `.gguf` on GPU.
- **No runtime (`--no-default-features`)**: keeps settings and model management functional while reporting `runtime_available: false` (`compiled_backend() == "unavailable"`) on Ask endpoints.

## Licenses and attribution

`llama.cpp` and `ggml` are MIT-licensed; `llama-cpp-2` / `llama-cpp-sys-2` bindings are `MIT OR Apache-2.0` (see `docs/licenses/upstream/` and `THIRD_PARTY_NOTICES.md`). Curated catalog models are **Apache-2.0** works from the Qwen team and are downloaded only on explicit owner action.

## Ask SQL runtime & grammar

`AppState::new` installs `ModelParser` when `llama` is enabled (`StubParser` in unit/integration tests when constructed with a stub). On each question:

1. `ModelParser` resolves the local `.gguf` path (`ai_custom_model_path` first if set, otherwise verified installed `ai_active_model`).
2. `src/ask/prompt.rs::local_sql_system_prompt` and `build_sql_prompt_with_previous` build the SQL prompt (up to 8192 context tokens, Qwen3.5 `<think>` block suppressed cleanly).
3. `src/ask/grammar.rs::from_sql_schema` constrains the model's JSON output to `src/ask/schema.rs::SqlModelOutput` (`title`, `computed`, `answer_kind`, `sql`, `columns`).
4. `src/ask/sandbox.rs::execute` runs the SQL against `ask_projection.db` inside a dedicated `tokio::task::spawn_blocking` worker. On repairable errors (`sql_error`, `unauthorized_table`, `unauthorized_column`, etc.), up to `MAX_SQL_REPAIR_ROUNDS` sanitized repair attempts run automatically without ever leaking DB row values into prompts.
5. `src/ask/history_link.rs` enriches returned artist/album/track cells and summary links using `crate::ask::projection::open_read_only`.

For live local GGUF integration testing (ignored in CI):

```powershell
$env:SCROBBL_TEST_MODEL='C:\models\Qwen3.5-2B-Q4_K_M.gguf'
cargo test --locked --test ask_runtime -- --ignored --nocapture
Remove-Item Env:SCROBBL_TEST_MODEL
```

## API summary

- `GET /api/v1/ai/settings` — owner session; current local settings (`enabled`, `active_model`, `custom_model_path`, `threads`, `gpu_layers`, `backend`, `unload_minutes`, `model_dir`).
- `POST /api/v1/ai/settings` — owner + CSRF; validates `threads`, `gpu_layers` (`-1..=256`), `unload_minutes`, `model_dir`, `active_model`, and `custom_model_path` (`.gguf` regular file), deletes any legacy deprecated keys, and unloads any loaded session if selection or `gpu_layers` changes.
- `GET /api/v1/ai/catalog` — curated Qwen3.5 2B SQL catalog (`gpu_optional: true`, dynamic compiled `backend`) merged with local installed/custom GGUF entries, `gpu_layers`, and download job progress.
- `POST /api/v1/ai/models/{id}/download` — requires AI enabled; catalog models only.
- `POST /api/v1/ai/models/{id}/cancel` — cancels active download job.
- `DELETE /api/v1/ai/models/{id}` — body `{ "confirm": true, "force": bool }`.
- `POST /api/v1/ai/models/{id}/select` — selects a verified installed catalog/custom model and unloads prior session.
- `POST /api/v1/ai/test` — reports runtime and selected local `.gguf` readiness without loading or running inference.
- `GET /api/v1/ask/status` — readiness state for the Ask page.
- `POST /api/v1/ask` — body `{ question, previous?, request_id? }`; generates and executes read-only SQL via the local GGUF model.
- `POST /api/v1/ask/sql` — runs owner-edited SQL directly through the same sandbox.
- `POST /api/v1/ask/cancel` — cancels an in-flight Ask inference or SQL query.

Every Ask and AI route requires owner session + CSRF (`auth::require_write` for mutations). Read API bearer keys are always denied (`403`).
