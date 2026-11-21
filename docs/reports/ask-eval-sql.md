# Ask SQL evaluation (local GGUF models)

Status: **measured for CPU Qwen3.5 tiers (`0.8B`, `2B`, `4B`)** via `ask-eval` on 8 CPU threads with warm system-prefix KV/SSM cache (`--threads 8`, `context_tokens = 8192`, greedy decoding, `max_new_tokens = 1024`). Larger `gpu_future` tiers (`9B`, `27B`) remain *not measured*. No numbers are estimated or invented.

## What is scored

Ask is scored **by typed result**, not by SQL text comparison. The offline
`ask-eval` binary builds the fixed synthetic fixture from `src/ask/fixture.rs`
(version 1), runs a local `.gguf` model via `ask::runtime::Session` with the
production SQL GBNF grammar (`ask::grammar::from_sql_schema`), executes the
generated query through `ask::sandbox::execute`, and compares the typed rows
against `evals/ask/sql_corpus.jsonl`.

Each case lands in exactly one bucket:

| outcome | meaning |
| --- | --- |
| **correct** | the query ran and returned exactly the expected typed result: same column count, same row count, same values in the same order (`integer` and `real` compare numerically, text exactly, `NULL` only to `NULL`). A truncated result is not correct. |
| **honest failure** | no plausible answer was claimed: the model refused (`answer_kind = "unsupported"`), the reply did not match the JSON schema, inference failed, or the sandbox rejected the query after the two repair rounds. The rule code is recorded. |
| **wrong but plausible** | the query ran and returned a result, but the result differs from the expected answer. |

Repair rounds mirror production: a rejected or failing query is sent back with
its own SQL and the sanitized error, up to two times. A query that runs and
returns wrong values is **not** repaired, because the application cannot know
from values alone that the answer is wrong.

The model runs deterministically (temperature 0) with the shipped local SQL
prompt (`ask::prompt::local_sql_system_prompt`), the schema generated from
`projection::TABLES`, the fixed "today" date `2025-02-01`, and the question. It
never receives fixture values or query results.

## Corpus

`evals/ask/sql_corpus.jsonl` has **26 cases: 13 English and 13 Italian**.
Every case names a fixed year or date range, and its `reference_sql` is
re-executed on every `cargo test` run so expected outputs cannot drift from the
fixture.

What the corpus does **not** cover: prompt injection and sandbox attacks (they
have their own adversarial test suite in `tests/sandbox_adversarial.rs`), chart
and GUI rendering (Playwright suites), follow-up questions, and relative date
wording.

## Running local model evaluations

Exact command (PowerShell, from the repository root):

```powershell
$env:CARGO_TARGET_DIR='C:\Users\mbena\Documents\scrobbl-rs\target'

cargo run --locked --release --bin ask-eval -- `
  --model C:\models\Qwen3.5-2B-Q4_K_M.gguf `
  --label qwen3.5-2b-q4_k_m `
  --threads 8 `
  --corpus evals/ask/sql_corpus.jsonl `
  --out C:\temp\qwen3.5-2b-sql.json
```

Both curated Qwen3.5 models (`0.8B`, `2B`, `4B`) and any custom local `.gguf`
model can be benchmarked using the same command.

## Results

Measured locally on an 8-core desktop CPU (`--threads 8`, 2 warmup turns before measurement, warm system-prefix state cache, fixture version 1, today `2025-02-01`):

| model | cases | correct | honest failure | wrong but plausible | EN | IT | p50 | p95 | TTFT p50 | TTFT p95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `qwen3.5-0.8b-q4_k_m` | 26 | 14 | 1 | 11 | 8/13 | 6/13 | 12832 ms | 54073 ms | 171 ms | 190 ms |
| `qwen3.5-2b-q4_k_m` | 26 | 16 | 4 | 6 | 10/13 | 6/13 | 17366 ms | 70378 ms | 239 ms | 272 ms |
| `qwen3.5-4b-q4_k_m` | 26 | 26 | 0 | 0 | 13/13 | 13/13 | 22741 ms | 37934 ms | 529 ms | 627 ms |
| `qwen3.5-9b-q4_k_m` (`gpu_future`) | 26 | not measured | not measured | not measured | — | — | — | — | — | — |
| `qwen3.5-27b-q4_k_m` (`gpu_future`) | 26 | not measured | not measured | not measured | — | — | — | — | — | — |

## Other ~2B model families (2026-11-19)

Same corpus, prompt, grammar, 8 threads and machine (Ryzen 7 5700X, 8 cores),
all `Q4_K_M` GGUF files with hashes verified against Hugging Face. Qwen3.5-2B was
re-run in the same session as a control and reproduced the row above.

| model | cases | correct | honest failure | wrong but plausible | EN | IT | p50 | p95 | TTFT p50 | TTFT p95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `qwen3.5-2b-q4_k_m` (control) | 26 | 16 | 4 | 6 | 10/13 | 6/13 | 16724 ms | 68746 ms | 230 ms | 305 ms |
| `gemma4-e2b-q4_k_m` | 26 | 15 | 1 | 10 | 7/13 | 8/13 | 17854 ms | 45769 ms | 418 ms | 499 ms |
| `lfm3-1.7b-q4_k_m` | 26 | 12 | 4 | 10 | 7/13 | 5/13 | 9549 ms | 75803 ms | 213 ms | 289 ms |

- No Qwen3.6 model near 2B exists (only 27B and 35B-A3B), so it was not measured.
- Gemma 4 E2B has 4.65 B effective parameters (2.88 GiB), not 2 B.
- Neither alternative beats Qwen3.5-2B, and both return more wrong but plausible
  answers (10 vs 6). None closes the gap to Qwen3.5-4B (26/26).
- `llama-cpp-2 0.1.161` (the pinned, latest crates.io release) cannot load LFM3
  (`null result from llama cpp`). The LFM3 row was measured with a throwaway
  build against `llama-cpp-rs` `main` (`0.1.162`, unreleased); the repository
  was not changed.
- LFM3 is published under the `lfm1.0` license, not Apache-2.0. Check it before
  adding it to the curated catalog.

## Engine speed: llama.cpp vs mistral.rs (2026-11-19)

Raw engine throughput on the same GGUF files, CPU only, 8 threads, 512-token
prompt, 128 generated tokens, 3 iterations. llama.cpp is `llama-bench` build
`b12549`; mistral.rs is a CPU build of `master` (`fb4cdc3`) with default
features. This measures the engine only: no GBNF grammar, no system-prefix
cache, not the production `Session` path.

| model | llama.cpp prefill | llama.cpp decode | mistral.rs prefill | mistral.rs decode |
| --- | ---: | ---: | ---: | ---: |
| Qwen3.5-2B | 338 t/s | 19.8 t/s | 201 t/s | 17.4 t/s |
| Gemma 4 E2B | 626 t/s | 15.8 t/s | 566 t/s | 12.1 t/s |
| LFM3-1.7B | 331 t/s | 26.2 t/s | 304 t/s | 21.6 t/s |

- llama.cpp is faster on every model: about 1.7x on Qwen3.5 prefill, 9% to 14%
  on the others, and 14% to 30% on decode. The published "up to 1.8x faster on
  CPU" claim for mistral.rs did not reproduce on this hardware.
- mistral.rs `v0.9.5` (release) cannot load the Gemma 4 text-only GGUF (it needs
  the multimodal projector) nor LFM3. LFM3 only works on `master`.
- Limiting mistral.rs to 8 threads (`RAYON_NUM_THREADS=8`) changed nothing;
  llama.cpp with 16 threads reached 370 t/s prefill and 18.5 t/s decode on
  Qwen3.5-2B.

## Reproducibility checks (model-free, run in CI)

```powershell
cargo test --locked --bin ask-eval
```

That suite validates the corpus shape, unique ids, language balance, and re-runs
**every** `reference_sql` against the fixed fixture to prove that checked-in
expected values match the schema and counting rules. It needs no `.gguf` weights
and no network access.

## Related documents

- [Local AI settings and custom `.gguf` models](../ai.md)
- [Ask projection and SQL sandbox](../ask-projection.md)
- [Ask eval corpus format](../../evals/ask/README.md)
- [Agent security and anti-slop guide](../agents/security.md)
