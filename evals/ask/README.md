# Ask eval corpus

`corpus.jsonl` is the versioned question set used to measure the Ask parser.
It is read by the `ask-eval` binary (`src/bin/ask-eval.rs`); the normal test
suite only validates its shape and labels. No model or network is required to
parse it.

## Format

One JSON object per line:

```json
{"id":"en-001","lang":"en","question":"How many times did I listen to Björk?","tags":["plays","unicode"],"expected":{"metric":"plays","target":"artist","name":"Björk","period":"all_time","weekday":"any","group_by":"none","limit":"none","order":"desc"}}
```

| field | meaning |
| --- | --- |
| `id` | stable, unique case id (`en-###` / `it-###`) |
| `lang` | `en` or `it`; every intent has one case per language |
| `question` | the exact user text sent to the model |
| `tags` | optional labels used for failure breakdowns (`injection`, `out_of_scope`, `empty`, `gibberish`, `unicode`, `weekday`, …) |
| `expected` | the interpretation a correct parser must produce |

`expected` mirrors `src/ask/schema.rs::ModelOutput`, the only surface the model
may emit:

| slot | accepted values |
| --- | --- |
| `metric` | `plays`, `time_listened`, `distinct_tracks`, `distinct_artists`, `distinct_albums`, `first_play`, `last_play`, `streak`, `milestones`, `unsupported` |
| `target` | `artist`, `album`, `track`, `none` |
| `name` | literal artist/album/track name copied from the question; `""` when `target` is `none` |
| `period` | `all_time`, `today`, `yesterday`, `this_week`, `last_week`, `this_month`, `last_month`, `this_year`, `last_year` |
| `weekday` | `any`, `monday` … `sunday` |
| `group_by` | `none`, `day`, `week`, `month`, `year`, `hour`, `weekday`, `artist`, `album`, `track` |
| `limit` | `none`, `one`, `five`, `ten`, `twenty_five`, `fifty`, `hundred` (numeric `1/5/10/25/50/100` is accepted as shorthand) |
| `order` | `asc`, `desc` |

Rules used by the labels:

- `target` describes a **named filter**, not the grouping. "Who is my most
  played artist?" has `target: none`, `group_by: artist`.
- A named artist/album/track always carries its literal name, including
  Unicode, digits, apostrophes and punctuation.
- `first_play` / `last_play` use `all_time` unless the question names a period.
- "least played" and "oldest to newest" use `order: asc`; other requests use
  the default `desc`.
- Out-of-scope, adversarial and empty questions expect
  `metric: unsupported` with every other slot at its empty default.

## Metrics computed by the harness

- **intent**: predicted metric equals `expected.metric` (`unsupported` counts
  as an intent).
- **slots**: precision/recall/F1 over `target`, `name`, `period`, `weekday`,
  `group_by`, `limit`, `order`. A slot is counted only when the expected or the
  predicted value is meaningful; defaults such as `period: all_time`,
  `weekday: any` and `group_by: none` are not free hits. Cases expecting
  `unsupported` are excluded from slot scores.
- **refusals**: precision/recall/accuracy of `metric: unsupported`.
- **JSON/schema validity**: output parses as `ModelOutput`.
- **exact match**: every filter slot correct on supported cases; the metric is
  scored separately as intent.
- **latency**: end-to-end p50/p95 and TTFT p50/p95.
- **failure breakdown**: wrong metric/period/name/weekday/refusal, failed
  injection cases and failed empty/gibberish cases.

## Adding cases

1. Add one English and one Italian line (same intent) at the end, with the next
   free id.
2. Use only values from the tables above. The harness rejects anything else.
3. Keep names literal: do not translate, title-case or transliterate them.
4. Run the corpus check and the harness dry parse:

   ```powershell
   cargo test --locked --bin ask-eval
   cargo build --locked --release --bin ask-eval
   ```

5. Re-measure and update `docs/reports/ask-eval.md` and the catalog scores the
   same way a model change would; do not edit labels to match a model.

## Running

```powershell
$env:CARGO_TARGET_DIR='C:\Users\mbena\Documents\scrobbl-rs\target'
cargo run --locked --release --bin ask-eval -- `
  --model C:\models\Qwen3-1.7B-Q4_K_M.gguf `
  --label qwen3-1.7b-q4_k_m `
  --threads 8 `
  --out C:\temp\qwen3-1.7b.json
```

The harness loads the model once, runs two uncounted warm-up questions, then
the whole corpus sequentially with the production prompt, grammar and greedy
sampling. It prints progress to stderr and writes `<out>.json` plus
`<out>.md`. `--limit N` evaluates only the first N cases (the corpus alternates
English/Italian, so a subset stays balanced).
