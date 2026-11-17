# Ask SQL evaluation (free questions)

Status: **not measured in this environment**. No OpenAI-compatible endpoint or
API key was available, so the harness was built and validated but no reference
remote model was run. The tables below are deliberately left as *not measured*;
no numbers are estimated or copied from another model.

The guided-questions evaluation is unchanged and is reported separately in
[Ask parser evaluation](ask-eval.md).

## What is scored

Free questions are scored **by result**, not by SQL text. The `ask-eval` binary
builds the fixed synthetic fixture from `src/ask/fixture.rs` (version 1), asks
one OpenAI-compatible server for a query, runs it through the shipped
`ask::sandbox`, and compares the typed rows with the expected values in
`evals/ask/sql_corpus.jsonl`.

Each case lands in exactly one bucket:

| outcome | meaning |
| --- | --- |
| **correct** | the query ran and returned exactly the expected typed result: same column count, same row count, same values in the same order (integer and real compare numerically, text exactly, `NULL` only to `NULL`). A truncated result is not correct. |
| **honest failure** | no plausible answer was claimed: the model refused (`answer_kind = "unsupported"`), the reply did not match the JSON schema, the request failed, or the sandbox rejected the query after the two repair rounds. The rule code is recorded. |
| **wrong but plausible** | the query ran and returned a result, but the result differs from the expected answer. This is the dangerous bucket: it looks like an answer. |

Repair rounds mirror production: a rejected or failing query is sent back with
its own SQL and the sanitized error, up to two times. A query that runs and
returns the wrong values is **not** repaired, because the application cannot
know the answer is wrong.

The model is asked at temperature 0 with the shipped SQL prompt
(`ask::prompt::remote_sql_system_prompt`), the schema generated from
`projection::TABLES`, the fixed "today" `2025-02-01`, and the question. It never
receives fixture values or results. Every question names a fixed year or date,
so the prompt's "today" cannot change an expected value.

## Corpus

`evals/ask/sql_corpus.jsonl` has **26 cases: 13 English and 13 Italian**.

| set | cases | notes |
| --- | ---: | --- |
| Guided vocabulary can express | 10 | plays, total listening time, distinct songs, first play, longest streak |
| Guided vocabulary cannot express | 16 | top-3 with ties, plays by device, same-day repeats, comparisons, discovery and return, after-midnight days, countries, shuffle plus device, offline, first-to-last gap |

Every case names a fixed period or date. The reference SQL for each case is
checked against the expected values on every `cargo test`, so the corpus cannot
silently drift from the fixture. The corpus is synthetic and contains no
personal data.

What the corpus does **not** cover: prompt injection and sandbox attacks (they
have their own versioned corpus and CI suite in `tests/sandbox_adversarial.rs`),
charts and GUI rendering (Playwright suites), follow-up questions, and questions
whose answer depends on the day the evaluation runs.

## Reference remote model

**Not measured in this environment.** The evaluator needs an
OpenAI-compatible server and, for hosted providers, an API key; neither was
configured here, and the harness deliberately never falls back to a guessed or
local endpoint.

Exact command (PowerShell, from the repository root, with the shared target
directory used by the other evaluations):

```powershell
$env:CARGO_TARGET_DIR='C:\Users\mbena\Documents\scrobbl-rs\target'
$env:SCROBBL_ASK_EVAL_BASE_URL='https://api.example.com/v1'
$env:SCROBBL_ASK_EVAL_MODEL='<model-name>'
$env:SCROBBL_ASK_EVAL_API_KEY='<key>'   # omit for a local server

cargo run --locked --release --bin ask-eval -- --sql `
  --base-url $env:SCROBBL_ASK_EVAL_BASE_URL `
  --remote-model $env:SCROBBL_ASK_EVAL_MODEL `
  --label '<model-label>' `
  --sql-corpus evals/ask/sql_corpus.jsonl `
  --out C:\Users\mbena\AppData\Local\Temp\opencode\ask-eval-runs\<model-label>-sql.json
```

The harness reads the API key from `SCROBBL_ASK_EVAL_API_KEY` when `--api-key`
is omitted, so the key does not land in shell history. It writes
`<model-label>-sql.json` and `<model-label>-sql.md`; the raw run artifacts stay
outside the repository. Only the aggregate results belong in this report, and
they must be pasted from a real run with the exact model name, server host and
date.

## Results

### Reference remote model

Not measured. No endpoint or key was available in this environment.

| set | cases | correct | honest failure | wrong but plausible |
| --- | ---: | ---: | ---: | ---: |
| all | 26 | not measured | not measured | not measured |
| en | 13 | not measured | not measured | not measured |
| it | 13 | not measured | not measured | not measured |
| guided expressible | 10 | not measured | not measured | not measured |
| guided not expressible | 16 | not measured | not measured | not measured |

### Local catalog models

**Not measured, and not planned for this report.** The recommendation for every
downloaded local catalog model is guided questions. The measured guided accuracy
from #181 is 55.8% intent for 0.6B, 78.9% for 1.7B and 92.8% for 4B (see
[Ask parser evaluation](ask-eval.md)); small local models are expected to write
worse SQL than a larger hosted model, and a wrong query returns a plausible
number. Measuring them would require loading each GGUF and running the corpus,
and the product decision does not depend on the result: guided stays the local
default and remains the recommendation. The catalog therefore stores no SQL
accuracy field and shows none.

If a future change measures a local model, use exactly the same fixture,
corpus, prompt, sandbox and command, replace `--base-url` with a local
OpenAI-compatible server, and add a row here with the model file and date. Do not
reuse a number for another model.

## Reproducibility checks (model-free, run in CI)

```powershell
cargo test --locked --bin ask-eval
```

That suite validates the SQL corpus shape, unique ids, language balance, tagged
coverage, and re-runs **every** `reference_sql` against the fixed fixture to
prove that the checked-in expected values still hold. It needs no model, no
network and no API key. The typed comparison, the outcome counters and the
fixture itself have model-free tests too.

## Limitations

- 26 cases on one small fixture measure whether a model can answer the kinds of
  question guided questions cannot, not a production accuracy rate.
- One run at temperature 0 can still vary between providers and model versions.
  Record the exact model name and date.
- The fixture is synthetic and fixed; it cannot represent every archive.
- The harness compares values, not presentation. It does not check the
  "computed" sentence, the title, `answer_kind` or chart choice, so a correct
  number shown with a misleading sentence is still scored correct.
- Reports are per model and language only; no per-case pass rate is published
  for a model that was not actually run.

## Related documents

- [Local AI settings and answer modes](../ai.md#answer-modes)
- [Ask projection and SQL sandbox](../ask-projection.md)
- [Ask eval corpus format](../../evals/ask/README.md)
