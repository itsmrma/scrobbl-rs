# Ask SQL fine-tune evaluation (`Qwen3.5 2B` LoRA, Colab 2026-11-21)

This report records the offline evaluation of the first `Qwen3.5 2B` LoRA fine-tune produced for [#248](https://github.com/itsmrma/scrobbl-rs/issues/248) following [Training the Ask SQL model](../ask-training.md).

## Artifact & conversion

- **Base model:** `unsloth/Qwen3.5-2B` (16-bit bf16)
- **Dataset:** `scripts/ask-training/ask-sft.zip` (`ask-train-data --samples 4 --seed 2026`, 1,940 train / 100 validation examples)
- **Adapter:** Colab L4 bf16 LoRA (`r = alpha = 32`, 3 epochs, `train.py --skip-gguf`)
- **GGUF conversion:** Merged in `bfloat16` on CPU, exported to F16 with `llama.cpp` at commit `0fb8f1577` (`3ba1ce0e2~1`, `--no-nextn`, 320 tensors), and quantized with `llama-quantize` (`Q4_K_M`).
  - Note: `llama.cpp` `>= b12343` (`3ba1ce0e2`, PR `#32102`) merges `attn_qkv`+`attn_gate` and `ssm_beta`+`ssm_alpha` during conversion. `llama-cpp-2 0.1.161` (the pinned crates.io release in `Cargo.toml`) requires separate `ssm_beta` and `ssm_alpha` tensors, while newer `llama.cpp` builds (`b12359` / `1.0.0`) remain backward-compatible with the unfused layout. Converting pre-`3ba1ce0e2` keeps the `.gguf` compatible with both.
- **Output file:** `scrobbl-ask-qwen3.5-2b-sql-Q4_K_M.gguf` (`1,274,396,512` bytes, SHA-256 `a62c87f95a0b721bf1ddc6f7bdb5ada18fbe4fe5e33698ed8720b106cfefe50f`)

## Held-out SQL corpus (`evals/ask/sql_corpus.jsonl`)

Measured back-to-back on the same 10-core Apple Silicon machine (`--threads 8`, 2 warm-up turns, `context_tokens = 8192`, greedy decoding, `max_new_tokens = 1024`, fixture version 2, today `2025-02-01`):

| model | set | cases | correct | honest failure | wrong but plausible | p50 | p95 | TTFT p50 | TTFT p95 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `qwen3.5-2b-q4_k_m` (base control) | all | 26 | 15 | 4 | 7 | 5251 ms | 21788 ms | 128 ms | 171 ms |
| `qwen3.5-2b-q4_k_m` (base control) | en | 13 | 10 | 1 | 2 | 5420 ms | 21732 ms | 134 ms | 171 ms |
| `qwen3.5-2b-q4_k_m` (base control) | it | 13 | 5 | 3 | 5 | 5251 ms | 49242 ms | 122 ms | 182 ms |
| `scrobbl-ask-qwen3.5-2b-sql-q4_k_m` | all | 26 | **22** | **0** | **4** | 4366 ms | 9024 ms | 105 ms | 130 ms |
| `scrobbl-ask-qwen3.5-2b-sql-q4_k_m` | en | 13 | 10 | 0 | 3 | 4366 ms | 14758 ms | 105 ms | 130 ms |
| `scrobbl-ask-qwen3.5-2b-sql-q4_k_m` | it | 13 | **12** | **0** | **1** | 4802 ms | 8608 ms | 108 ms | 132 ms |

### Remaining failures on the fine-tuned model

All 4 remaining failures are `wrong_but_plausible` (0 sandbox rejections, 0 malformed JSON/schema failures, 0 false refusals):

| case | language | detail |
| --- | --- | --- |
| `sql-en-002` | en | `cell (1, listened_ms) expected 3380000, got 16` |
| `sql-en-004` | en | `cell (1, first_play) expected "2024-01-02", got "2024-01-02 15:00:00"` |
| `sql-it-004` | it | `cell (1, first_play) expected "2024-01-02", got null` |
| `sql-en-010` | en | `cell (1, artists) expected 1, got 0` |

Compared with the base model, the fine-tune fixed `sql-it-002` (known-duration listening time), `sql-it-005` (Italian streak), `sql-en-006` and `sql-it-006` (top-N with ties), `sql-it-007` (`device_type` vs `platform`), `sql-it-010` (Italian discovery), `sql-it-011` (after midnight), `sql-en-013` (shuffle + platform + artist), and `sql-it-013` (days between first and last play). Two English cases (`sql-en-004`, `sql-en-010`) regressed relative to base, leaving English at `10/13` vs `10/13`.

## Unsupported refusal probe & runtime integration

Six out-of-scope prompts (weather, delete plays, music genre — 3 EN + 3 IT) were run through `ask-eval` with the same runtime prompt and GBNF grammar:

- **Fine-tuned model:** `6/6` returned `answer_kind = "unsupported"` (`p50 = 1076 ms`, `p95 = 1406 ms`).
- **Base model:** `4/6` returned `unsupported`; both music-genre questions (`sql-en-903`, `sql-it-903`) hallucinated SQL instead of refusing.

`cargo test --locked --test ask_runtime -- --ignored` (`real_model_answers_three_supported_questions`) passed with `SCROBBL_TEST_MODEL` pointing to the fine-tuned GGUF (`1 passed; 0 failed`).

## Acceptance gate verdict

From [Training the Ask SQL model](../ask-training.md#4-acceptance-gate):

| criterion | result | status |
| --- | --- | --- |
| 1. Strictly more correct than base Qwen3.5 2B in **both** EN and IT | Overall `22/26` vs `15/26`; IT `12/13` vs `5/13` (**+7**); EN `10/13` vs `10/13` (**tie**) | **Not met (EN tie)** |
| 2. No `unsupported` regression; stays inside read-only sandbox | `6/6` `unsupported` refused (base `4/6`), `0` sandbox violations | **Pass** |
| 3. p95 latency within 10% of base | `9024 ms` vs `21788 ms` (`-58.6%`) | **Pass** |
| 4. `tests/ask_runtime.rs` (`--ignored`) passes | `real_model_answers_three_supported_questions` passed | **Pass** |

Because criterion 1 requires strictly more correct answers in **both** English and Italian, `src/ai/catalog.rs` is left pinned to base `qwen3.5-2b-q4_k_m` for now, and the GGUF is not promoted as the default catalog download yet. To clear the gate in the next training iteration, add English template coverage / paraphrases for:

1. Total `SUM(listened_ms)` on counted plays with known duration (`sql-en-002`, where it returned `COUNT` instead of `SUM`).
2. `MIN(local_date)` vs `MIN(played_at_local)` for "When did I first listen to ..." (`sql-en-004` / `sql-it-004`).
3. Discovery cohort queries ("first played in year X and still played in year Y", `sql-en-010`).
