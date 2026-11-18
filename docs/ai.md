# Local AI settings, catalog and local SQL models

AI is optional and **off by default**. Ask operates exclusively with **local GGUF models** running inside scrobbl.rs that translate natural-language questions into read-only SQL queries over the [Ask projection](ask-projection.md). Remote AI servers ("bring your own AI"), external MCP integrations, Guided DSL execution, and UI-control actions are not supported.

You can use curated **Qwen3** models from the built-in catalog by default, or import your own local `.gguf` model on the server.

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
| `ai_unload_minutes` | `"15"` | idle unload delay (`1–1440` minutes) |
| `ai_model_dir` | `""` | optional absolute model folder; empty means `<data_dir>/models` |

Enabling AI does not download anything. Saving a custom path or switching the active catalog model unloads any currently active inference session cleanly so the next question uses the new selection. Disabling AI cancels active model downloads so no further network transfer continues.

`GET /api/v1/ask/status` reports `enabled`, `active_model`, `custom_model_path`, `runtime_available`, `model_ready` and `model_loading`. `model_ready` reflects verified filesystem readiness for the selected model; `model_loading` comes from the live inference worker and is true only while a load is in progress.

## Curated catalog (Qwen3 defaults)

The curated catalog is compiled into the binary (`src/ai/catalog.rs`). A download URL is derived from the pinned repository, revision and file; clients never supply a URL for catalog downloads. Weights are verified by exact byte size and SHA-256 against the Hugging Face LFS object ids.

| id | repo | revision | file | bytes | SHA-256 |
| --- | --- | --- | --- | ---: | --- |
| `qwen3-0.6b-q4_k_m` | `unsloth/Qwen3-0.6B-GGUF` | `50968a4468ef4233ed78cd7c3de230dd1d61a56b` | `Qwen3-0.6B-Q4_K_M.gguf` | 396,705,472 | `ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a` |
| `qwen3-1.7b-q4_k_m` | `ggml-org/Qwen3-1.7B-GGUF` | `daeb8e2d528a760970442092f6bf1e55c3b659eb` | `Qwen3-1.7B-Q4_K_M.gguf` | 1,282,439,264 | `d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5` |
| `qwen3-4b-q4_k_m` | `Qwen/Qwen3-4B-GGUF` | `bc640142c66e1fdd12af0bd68f40445458f3869b` | `Qwen3-4B-Q4_K_M.gguf` | 2,497,280,256 | `7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5` |
| `qwen3-8b-q4_k_m` | `Qwen/Qwen3-8B-GGUF` | `7c41481f57cb95916b40956ab2f0b139b296d974` | `Qwen3-8B-Q4_K_M.gguf` | 5,027,783,488 | `d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785` |
| `qwen3-32b-q4_k_m` | `Qwen/Qwen3-32B-GGUF` | `938a7432affaec9157f883a87164e2646ae17555` | `Qwen3-32B-Q4_K_M.gguf` | 19,762,149,024 | `efd971561896866f0e910cce52761ca77b1b138090c7f15fe284676d57d1f689` |

`8B` and `32B` entries are present with `status: "gpu_future"` and `selectable: false`; they are listed for planning only and cannot be downloaded or selected in this CPU-only release.

All selectable models run SQL generation with a **4096-token** context window and up to **512** generated tokens (`src/ask/runtime.rs`).

## Using custom local `.gguf` models

You can run any GGUF model compatible with `llama.cpp` instead of the default Qwen3 catalog models:

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

## Runtime feature

`src/ai/mod.rs::runtime_available()` returns `cfg!(feature = "llama")`. The default `llama` feature compiles `llama-cpp-2` (CPU-only, `default-features = false`, `GGML_OPENMP=OFF`). Building with `--no-default-features` keeps settings and model management functional while reporting `runtime_available: false` on Ask endpoints.

## Licenses and attribution

`llama.cpp` and `ggml` are MIT-licensed; `llama-cpp-2` / `llama-cpp-sys-2` bindings are `MIT OR Apache-2.0` (see `docs/licenses/upstream/` and `THIRD_PARTY_NOTICES.md`). Curated catalog models are **Apache-2.0** works from the Qwen team and are downloaded only on explicit owner action.

## Ask SQL runtime & grammar

`AppState::new` installs `ModelParser` when `llama` is enabled (`StubParser` in unit/integration tests when constructed with a stub). On each question:

1. `ModelParser` resolves the local `.gguf` path (`ai_custom_model_path` first if set, otherwise verified installed `ai_active_model`).
2. `src/ask/prompt.rs::local_sql_system_prompt` and `build_sql_prompt_with_previous` build the SQL prompt (up to 4096 context tokens, Qwen3 `<think>` block suppressed cleanly).
3. `src/ask/grammar.rs::from_sql_schema` constrains the model's JSON output to `src/ask/schema.rs::SqlModelOutput` (`title`, `computed`, `answer_kind`, `sql`, `columns`).
4. `src/ask/sandbox.rs::execute` runs the SQL against `ask_projection.db` inside a dedicated `tokio::task::spawn_blocking` worker. On repairable errors (`sql_error`, `unauthorized_table`, `unauthorized_column`, etc.), up to `MAX_SQL_REPAIR_ROUNDS` sanitized repair attempts run automatically without ever leaking DB row values into prompts.
5. `src/ask/history_link.rs` enriches returned artist/album/track cells and summary links using `crate::ask::projection::open_read_only`.

For live local GGUF integration testing (ignored in CI):

```powershell
$env:SCROBBL_TEST_MODEL='C:\models\Qwen3-1.7B-Q4_K_M.gguf'
cargo test --locked --test ask_runtime -- --ignored --nocapture
Remove-Item Env:SCROBBL_TEST_MODEL
```

## API summary

- `GET /api/v1/ai/settings` — owner session; current local settings (`enabled`, `active_model`, `custom_model_path`, `threads`, `unload_minutes`, `model_dir`).
- `POST /api/v1/ai/settings` — owner + CSRF; validates `threads`, `unload_minutes`, `model_dir`, `active_model`, and `custom_model_path` (`.gguf` regular file), deletes any legacy deprecated keys, and unloads any loaded session if selection changes.
- `GET /api/v1/ai/catalog` — curated Qwen3 catalog merged with local installed/custom GGUF entries and download job progress.
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
