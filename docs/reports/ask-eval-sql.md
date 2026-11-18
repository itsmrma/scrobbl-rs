# Ask SQL evaluation (local GGUF models)

Status: **not measured in this environment**. No local `.gguf` model weights are
bundled in CI or measured in this run; the tables below stay marked as
*not measured* until an operator runs `ask-eval --model <file.gguf>` on real
hardware. No numbers are estimated or invented.

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
  --model C:\models\Qwen3-1.7B-Q4_K_M.gguf `
  --label qwen3-1.7b-q4_k_m `
  --threads 8 `
  --sql-corpus evals/ask/sql_corpus.jsonl `
  --out C:\temp\qwen3-1.7b-sql.json
```

Both curated Qwen3 models (`0.6B`, `1.7B`, `4B`) and any custom local `.gguf`
model can be benchmarked using the same command.

## Results

Not measured in this environment.

| model | cases | correct | honest failure | wrong but plausible |
| --- | ---: | ---: | ---: | ---: |
| Curated Qwen3 / custom local GGUF | 26 | not measured | not measured | not measured |

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
