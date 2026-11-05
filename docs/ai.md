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
and `model_loading`. `model_loading` stays false in this ticket; model loading
belongs to the inference worker.

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
  speed and exact-match figures. Sizes and LFS SHA-256 values were still read
  from the Hugging Face API at implementation time.
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
optional GPU, 2048-token context, backend, spike speed and language exact-match
scores. Parser scores remain `null` until the parser evaluation in #181.

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
`llama` feature is declared in `Cargo.toml` but has no dependency in this
ticket; #178 adds `llama-cpp-2` behind it. Without the feature the settings,
catalog and downloads remain usable, while Ask and the "test model" action are
reported as unavailable. No command pretends inference works.

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
