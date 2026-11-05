# Ask parser evaluation (#181)

Measured baseline for the local Ask parser on the versioned question set. All
numbers below come from real GGUF files on CPU; nothing is estimated, and labels
were not edited to match a model.

## Verdict

| Model | Target (intent / slot F1 / refusal) | Intent | Slot F1 | Refusal recall | Refusal accuracy | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| 0.6B (`qwen3-0.6b-q4_k_m`) | informational only | 55.77% | 0.611 | 52.38% | 58.17% | **n/a** |
| 1.7B (`qwen3-1.7b-q4_k_m`) | >=95% / >=90% / >=98% | 78.85% | 0.838 | 78.57% | 88.46% | **FAIL** |
| 4B (`qwen3-4b-q4_k_m`) | >=98% / >=95% / 100% | 92.79% | 0.965 | 78.57% | 94.23% | **FAIL** |

**No measured model meets its target, so the default is not confirmed.** The 1.7B default misses all three of its targets (78.85% intent, 0.838 slot F1, 88.46% refusal accuracy). The 4B is materially better (92.79% intent, 0.965 slot F1, 94.23% refusal accuracy) and meets the slot target, but it misses the 98% intent and 100% refusal targets: it accepts some out-of-scope and empty questions. Promoting 4B would not satisfy the spec and would roughly double the download and latency, so the 1.7B tier stays provisional pending prompt/model work or a deliberate revision of the targets. The feature stays opt-in and the catalog carries the measured scores and warnings.

## Methodology

### Corpus

`evals/ask/corpus.jsonl` is versioned in the repository and contains no archive
data. It has **208 labelled cases: 104 English and 104 Italian**, built as
intent pairs so that every question exists in both languages.

| Group | Cases | Notes |
| --- | ---: | --- |
| Supported questions | 166 | Every metric, period, weekday, grouping, limit and order value |
| Out of scope | 20 | Weather, poetry, recommendations, comparisons, "why", multi-hop sequence |
| Prompt injection | 14 | SQL, schema, system prompt and raw-row requests |
| Empty | 4 | Empty and whitespace-only strings |
| Gibberish | 4 | Random tokens and punctuation |

The corpus also covers **38 Unicode cases** (for example Björk, Sigur Rós,
Motörhead, `Don't Stop Me Now`), **6 ambiguous cases** where the same name can be
an artist or an album, apostrophes, digits and punctuation. Every expected label is a value
of the production model-facing schema in `src/ask/schema.rs`; the harness rejects
unknown values. A model-free unit test validates the corpus shape, unique ids,
language balance and value coverage.

### Production path

`ask-eval` (new `src/bin/ask-eval.rs`, `required-features = ["llama"]`) loads one
GGUF file and reuses the shipped path end to end:

- `ask::prompt::build_prompt` (the same compact English prompt with EN/IT
  examples; Qwen3 thinking disabled by the pre-filled empty thinking block);
- the `ModelOutput` JSON schema and the GBNF grammar that constrains generation;
- greedy sampling, context 2048 tokens, maximum 256 new tokens;
- one model loaded for the whole corpus, 8 inference threads.

The model only emits DSL JSON. In production Rust validates it and computes
every number from the archive; the harness stops after validation, because this
ticket measures interpretation, not execution.

### Metrics

- **Intent accuracy**: predicted `metric` equals the expected metric (a refusal
  is the `unsupported` intent, so it participates in this score).
- **Slot precision / recall / F1** over `target`, `name`, `period`, `weekday`,
  `group_by`, `limit` and `order`. A slot is counted only when the expected or
  the predicted value is meaningful; defaults (`period: all_time`,
  `weekday: any`, `group_by: none`, `order: desc`) are not free hits. Cases
  expecting a refusal are excluded from slot scores.
- **Refusal precision / recall / accuracy** for `metric: unsupported`.
- **JSON/schema validity**: output parses into `ModelOutput`.
- **Exact match**: every filter slot correct on supported cases (denominator
  166). The metric is scored by intent, so a wrong metric cannot be hidden by an
  exact filter interpretation.
- **Latency**: wall-clock seconds per question, including prompt decode; TTFT is
  time to the first sampled token. p50/p95 use the nearest-rank method. Two
  uncounted warm-up questions run first; model load and cold start are not part
  of these numbers. Latency aggregates only schema-valid generations.

### Hardware and build

Windows 11 desktop, AMD Ryzen 7 5700X (8 cores / 16 threads), 8 inference
threads, CPU only, no GPU. Release build without LTO; the distributed profile
also strips symbols, which does not change runtime speed. The same machine and
thread count as the #172 spike, so the numbers are comparable.

### Models

All three files were already present locally; nothing was downloaded for this
evaluation. The SHA-256 of each local file matches the pinned catalog value.

| Catalog id | File | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `qwen3-0.6b-q4_k_m` | `Qwen3-0.6B-Q4_K_M.gguf` | 396,705,472 | `ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a` |
| `qwen3-1.7b-q4_k_m` | `Qwen3-1.7B-Q4_K_M.gguf` | 1,282,439,264 | `d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5` |
| `qwen3-4b-q4_k_m` | `Qwen3-4B-Q4_K_M.gguf` | 2,497,280,256 | `7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5` |

The optional Qwen2.5-1.5B fallback was not run: its file is not present locally
and this task explicitly forbids re-downloading models. It has no catalog entry
and no score; no Qwen3 number is reused for it.

### Commands

Run from the repository root with the shared target directory:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\mbena\Documents\scrobbl-rs\target'
cargo build --locked --release --bin ask-eval

cargo run --locked --release --bin ask-eval -- `
  --model C:\Users\mbena\AppData\Local\Temp\opencode\ask-eval-models\Qwen3-0.6B-Q4_K_M.gguf `
  --label qwen3-0.6b-q4_k_m --threads 8 --warmups 2 `
  --corpus evals/ask/corpus.jsonl `
  --out C:\Users\mbena\AppData\Local\Temp\opencode\ask-eval-runs\qwen3-0.6b-q4_k_m.json

cargo run --locked --release --bin ask-eval -- `
  --model C:\Users\mbena\AppData\Local\Temp\opencode\ask-eval-models\Qwen3-1.7B-Q4_K_M.gguf `
  --label qwen3-1.7b-q4_k_m --threads 8 --warmups 2 `
  --corpus evals/ask/corpus.jsonl `
  --out C:\Users\mbena\AppData\Local\Temp\opencode\ask-eval-runs\qwen3-1.7b-q4_k_m.json

cargo run --locked --release --bin ask-eval -- `
  --model C:\Users\mbena\AppData\Local\Temp\opencode\ask-eval-models\Qwen3-4B-Q4_K_M.gguf `
  --label qwen3-4b-q4_k_m --threads 8 --warmups 2 `
  --corpus evals/ask/corpus.jsonl `
  --out C:\Users\mbena\AppData\Local\Temp\opencode\ask-eval-runs\qwen3-4b-q4_k_m.json
```

The harness writes `<label>.json` (machine-readable summary) and `<label>.md`
(same table) next to the requested output. The raw run artifacts live outside
the repository; the aggregate results are reproduced below.

### Reproducibility checks

- The three SHA-256 values above were recomputed from the local files and match
  `src/ai/catalog.rs`.
- `cargo clippy --locked --all-targets -- -D warnings` passes.
- `cargo test --locked --bin ask-eval` validates the corpus without a model.
- Normal CI never builds or runs `ask-eval`, never downloads a model and never
  touches the network; the harness is opt-in and offline.

## Results

### Overall

| Model | Cases | JSON valid | Intent | Slot P | Slot R | Slot F1 | Refusal P | Refusal R | Refusal acc | Exact | p50 | p95 | TTFT p50 | TTFT p95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0.6B (`qwen3-0.6b-q4_k_m`) | 208 | 208/208 | 55.77% | 0.698 | 0.544 | 0.611 | 0.247 | 0.524 | 0.582 | 0.404 | 3004 ms | 3254 ms | 1410 ms | 1547 ms |
| 1.7B (`qwen3-1.7b-q4_k_m`) | 208 | 208/208 | 78.85% | 0.876 | 0.802 | 0.838 | 0.688 | 0.786 | 0.885 | 0.657 | 5942 ms | 6557 ms | 3617 ms | 3802 ms |
| 4B (`qwen3-4b-q4_k_m`) | 208 | 208/208 | 92.79% | 0.957 | 0.974 | 0.965 | 0.917 | 0.786 | 0.942 | 0.885 | 13130 ms | 13990 ms | 9029 ms | 9257 ms |

### Per language

| Model | Lang | Cases | JSON valid | Intent | Slot P | Slot R | Slot F1 | Refusal P | Refusal R | Refusal acc | Exact | p50 | p95 | TTFT p50 | TTFT p95 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0.6B | en | 104 | 104/104 | 51.92% | 0.741 | 0.564 | 0.640 | 0.250 | 0.571 | 0.567 | 0.446 | 2999 ms | 3202 ms | 1402 ms | 1523 ms |
| 0.6B | it | 104 | 104/104 | 59.62% | 0.657 | 0.523 | 0.583 | 0.244 | 0.476 | 0.596 | 0.361 | 3011 ms | 3256 ms | 1416 ms | 1554 ms |
| 1.7B | en | 104 | 104/104 | 83.65% | 0.894 | 0.837 | 0.865 | 0.654 | 0.809 | 0.875 | 0.711 | 5965 ms | 6470 ms | 3616 ms | 3802 ms |
| 1.7B | it | 104 | 104/104 | 74.04% | 0.857 | 0.767 | 0.810 | 0.727 | 0.762 | 0.894 | 0.602 | 5920 ms | 6557 ms | 3618 ms | 3796 ms |
| 4B | en | 104 | 104/104 | 95.19% | 0.955 | 0.988 | 0.972 | 1.000 | 0.762 | 0.952 | 0.904 | 13066 ms | 13945 ms | 8992 ms | 9232 ms |
| 4B | it | 104 | 104/104 | 90.38% | 0.959 | 0.959 | 0.959 | 0.850 | 0.809 | 0.933 | 0.868 | 13185 ms | 13990 ms | 9052 ms | 9259 ms |

### Failures by category

A case counts once per tag it carries, so tag counts can overlap. `out_of_scope`,
`injection`, `empty` and `gibberish` failures are missed refusals; metric,
period and name failures are wrong interpretations.

| Category tag | 0.6B failed | 1.7B failed | 4B failed |
| --- | ---: | ---: | ---: |
| `ambiguous` | 0 | 0 | 0 |
| `distinct_albums` | 6 | 1 | 6 |
| `distinct_artists` | 6 | 1 | 4 |
| `distinct_tracks` | 6 | 3 | 0 |
| `empty` | 4 | 0 | 4 |
| `explicit_order` | 10 | 9 | 2 |
| `first_play` | 3 | 0 | 0 |
| `gibberish` | 2 | 0 | 0 |
| `grouping` | 54 | 39 | 6 |
| `hour` | 2 | 2 | 1 |
| `injection` | 6 | 1 | 0 |
| `last_play` | 3 | 0 | 0 |
| `milestones` | 4 | 0 | 0 |
| `out_of_scope` | 8 | 8 | 5 |
| `plays` | 12 | 9 | 1 |
| `streak` | 8 | 6 | 2 |
| `time_listened` | 12 | 3 | 2 |
| `topn` | 38 | 24 | 3 |
| `unicode` | 14 | 8 | 1 |
| `weekday` | 18 | 6 | 2 |

| Model | Wrong metric | Wrong period | Wrong name | Wrong weekday | Refusal errors | Injection missed | Empty/gibberish missed | Failed cases |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0.6B | 72 | 56 | 30 | 8 | 87 | 6 | 6 | 134 |
| 1.7B | 35 | 13 | 12 | 0 | 24 | 1 | 0 | 71 |
| 4B | 6 | 2 | 0 | 0 | 12 | 0 | 4 | 30 |

- **0.6B**: 134 failed cases; the per-tag table above is the useful breakdown and the raw JSON keeps the full list.
- **1.7B**: 71 failed cases; the per-tag table above is the useful breakdown and the raw JSON keeps the full list.
- **4B**: `en-022`, `it-027`, `en-029`, `en-031`, `it-031`, `en-032`, `it-032`, `en-035`, `it-035`, `en-037`, `it-037`, `en-038`, `it-038`, `it-050`, `it-053`, `en-058`, `it-068`, `it-069`, `it-070`, `it-078`, `it-079`, `en-082`, `en-084`, `it-084`, `en-086`, `it-086`, `en-097`, `it-097`, `en-098`, `it-098`

### Latency interpretation

The 4B model is roughly twice as slow as the 1.7B default and about four times
as slow as the 0.6B light model. TTFT is a large share of the total because the
prompt is decoded on CPU for every question; the 2048-token context is not the
bottleneck with these short prompts. These are warm, single-model, desktop
numbers. They do not cover the first load after idle unload, concurrent users or
the NAS target from issue #31, which remains a separate physical verification.

## Default model decision

The catalog default stays `qwen3-1.7b-q4_k_m` only if it meets the 1.7B targets;
otherwise the default must move to the model that meets its own tier target.
The measured decision is recorded in [Verdict](#verdict) above. No model met
its target, so the tier was deliberately **not** swapped: promoting the 4B would
not satisfy its target and would double download size and latency. The 1.7B
entry stays the provisional default, with the measured scores and a warning in
its catalog description; revising the default is a follow-up decision, not a
silent score edit.

## What #182 needs to know

- Use the 1.7B desktop p50/p95 above for the Ask latency budget; do not claim the
  NAS threshold. It was not measured here.
- The harness measures the parser in isolation, not the HTTP round trip, the
  worker queue, database execution or the UI. End-to-end latency is higher and
  belongs to #182.
- Refusal failures above are the concrete adversarial/out-of-scope cases to
  reproduce through `POST /api/v1/ask`; the `unsupported` path must stay explicit
  and user-facing.
- The model is loaded once for the whole corpus. Cold-start and unload/reload
  behaviour is not measured by this harness.
- Qwen2.5-1.5B remains unmeasured; any comparison must run the same corpus and
  commands above, not reuse Qwen3 numbers.

