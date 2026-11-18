# Ask SQL evaluation corpus

`sql_corpus.jsonl` is the versioned question set used to score **Ask local SQL
generation by result** on the fixed synthetic fixture from `src/ask/fixture.rs`.
It is read by the offline `ask-eval` binary (`src/bin/ask-eval.rs`); normal CI
validates its shape, labels, and every `reference_sql` query without requiring a
model or network access.

## SQL corpus format

One JSON object per line:

```json
{"id":"sql-en-001","lang":"en","question":"How many times did I listen to Björk?","tags":["plays","artist"],"reference_sql":"SELECT COUNT(DISTINCT l.id) AS plays FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id WHERE l.counted = 1 AND a.name = 'Björk'","expected":{"columns":["plays"],"rows":[[10]]}}
```

| field | meaning |
| --- | --- |
| `id` | stable, unique case id (`sql-en-###` / `sql-it-###`) |
| `lang` | `en` or `it` |
| `question` | the exact user text sent to the local model |
| `tags` | labels used for coverage and the failure breakdown (`device`, `country`, `shuffle`, `offline`, `streak`, `ranking`, `gap`, `first_play`, `combined_conditions`, `hour`, ...) |
| `reference_sql` | the query that defines the expected answer; it is re-run on every test against the fixture |
| `expected` | typed result: `columns` plus `rows` of JSON scalars (`null`, integer/real number, string) |

Rules:

- Values are fixed and synthetic. They come from `src/ask/fixture.rs`, version 1;
  never copy a personal value or a result from a real archive.
- Do not use relative wording ("today", "last year", "ultimi 3 mesi"). Each
  question names a fixed year or date so the expected value does not depend on
  when the evaluation runs.
- Row order is part of the answer: a ranking query must order deterministically,
  including tie-breakers, and the reference SQL shows the intended order.
- Every expected value must be re-derivable by `reference_sql`. The test
  `every_reference_query_returns_its_expected_result_on_the_fixed_fixture`
  fails if a literal and its query disagree.
- If a fixture row changes, bump `fixture::FIXTURE_VERSION` and re-check every
  expected value.

## Running offline against any local GGUF model

Evaluate either a curated Qwen3.5 file or any imported local `.gguf` file:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\mbena\Documents\scrobbl-rs\target'

cargo run --locked --release --bin ask-eval -- `
  --model C:\models\Qwen3.5-2B-Q4_K_M.gguf `
  --label qwen3.5-2b-q4_k_m `
  --threads 8 `
  --out C:\temp\qwen3.5-2b-sql.json
```

The harness loads the local GGUF file once via `src/ask/runtime.rs::Session`,
runs the production `local_sql_system_prompt` and SQL GBNF grammar
(`grammar::from_sql_schema`), executes the generated SQL through
`ask::sandbox::execute` (with repair rounds), and writes `<out>.json` and
`<out>.md` outside the repository. Use `--limit N` to evaluate a balanced
English/Italian subset. Report aggregate measurements in
[`docs/reports/ask-eval-sql.md`](../../docs/reports/ask-eval-sql.md).
