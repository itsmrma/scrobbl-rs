# Training the Ask SQL model

Ask ships one curated model, **Qwen3.5 2B** (`qwen3.5-2b-q4_k_m`). Out of the box it
reads the projection schema and the rules in the prompt and writes SQLite on its
own. A 2B model does this well for simple lookups, but it struggles with the
projection's specific conventions: multi-artist tracks, editions, `counted`,
relative dates, streaks, and the Spotify export columns. This document describes
how to fine-tune that same model on scrobbl.rs SQL, so one small model answers as
reliably as a much larger one.

The training pipeline is reproducible and synthetic. It never reads an archive.

## 1. What the model must learn

Every question is answered from the [Ask projection](ask-projection.md). The
model must handle every table and column in it. A test
(`every_projection_column_is_taught_by_some_template`) fails if any column of
`projection::TABLES` has no training template.

| Area | Columns | Example questions |
| --- | --- | --- |
| Volume | `listens.counted`, `local_year`, `local_month`, `local_date`, `week_date` | plays per year/month/day, this week, last month, the last *N* days, yesterday |
| Time | `listened_ms`, `duration_kind` | total, per artist/device/year; actual-only time; unknown durations |
| Entities | `artists`, `albums`, `tracks`, `track_artists.ordinal`, `album_artists`, `edition_key`, `duration_ms` | top N with ties, featured artists, collaborations, editions, longest tracks, rank position |
| Calendar | `calendar.*`, `weekday`, `local_hour`, `played_at_local`, `played_at_ms` | streaks, longest gaps, silent days, weekday/hour habits, first/last play |
| Device | `platform`, `device_type`, raw `device` | phone vs computer, Android/iPhone/Xbox/Alexa, top artists per platform |
| Spotify context | `country`, `shuffle`, `skipped`, `offline`, `reason_start`, `reason_end` | listening abroad, shuffle share, skip rate, songs finished, hand-picked artists |
| Provenance | `sources`, `needs_review`, `meta.*`, `week_start` | plays per source, duplicates to review, data freshness |
| Conversation | previous question + SQL | "And in 2024?" follow-ups |
| Refusals | — | weather, writes ("delete my plays"), data the archive lacks (genres, podcasts, BPM) |

The Spotify export `platform` string is unusable for a small model: real exports
mix `ios`, `iOS 14.4 (iPhone12,1)`, `Windows 10 (…)`, `Partner amazon_salmon
Amazon;Echo_Dot;…` and `not_applicable`. The projection groups it into
`platform` (`android`, `ios`, `windows`, `macos`, `linux`, `web`, `xbox`,
`playstation`, `alexa`, `sonos`, `google_cast`, `tv`, `car`, `other`) and
`device_type` (`phone`, `tablet`, `computer`, `web`, `console`, `speaker`, `tv`,
`car`, `other`). Placeholder values become `NULL`
(`projection::classify_platform`). The schema also lists the known
`reason_start`/`reason_end` codes, so a question like "songs I skipped" maps to
real values.

## 2. Dataset

- **Templates:** `evals/ask/train_templates.json` holds 117 templates (114 with SQL, 3 refusal families). Each
  has English and Italian paraphrases, the reference SQL with placeholders, the
  answer kind, columns, and title/computed text in both languages. Some also
  have follow-up turns.
- **Generator:** `src/ask/training.rs` fills the placeholders from fixed pools.
  The pools contain fixture names, public artist names with accents, non-Latin
  scripts and apostrophes (to exercise SQL escaping), years, months, dates,
  hours, platforms, device types, countries, reasons and sources. Each example
  gets a varying "today" between 2025 and 2026. Relative wording such as "this
  month" or "the last 30 days" therefore becomes literal dates computed from
  that day, the same way the runtime prompt states `Today is …`.
- **Exact runtime format:** each record's `prompt` is
  `prompt::build_sql_prompt_with_previous(...)`, the production system prompt
  with the schema, rules and examples, ending with the empty Qwen3.5 thinking
  block. Each `completion` is the JSON object in grammar field order (`title`,
  `computed`, `answer_kind`, `sql`, `columns`) followed by `<|im_end|>`. Training
  and inference therefore use one template, which is the most common cause of a
  fine-tune behaving worse after export.
- **Validated:** `ask-train-data` and the test
  `every_generated_query_runs_in_the_sandbox_with_its_declared_columns` run every
  distinct reference query through `ask::sandbox::execute` on the synthetic
  fixture. Each query must pass every sandbox rule and return exactly the
  declared columns. During development the same queries were also run on a
  projection built from a real ~160k-listen export, through the owner-edited
  SQL route. All templates ran within the sandbox limits.
- **Held out:** questions from `evals/ask/sql_corpus.jsonl` are never generated
  (`evaluation_questions_never_enter_the_training_data`). That corpus stays the
  test set.

Generate it (about 2,000 examples with `--samples 4`; raise it for more variety):

```powershell
cargo run --locked --no-default-features --bin ask-train-data -- `
  --out C:\temp\ask-sft --samples 4 --seed 2026
```

The output is `train.jsonl` (about 95%) and `validation.jsonl` (about 5%, split
by a stable hash of the example id). Each prompt is roughly 4–5k tokens.

## 3. Training recipe

`scripts/ask-training/train.py` (Unsloth + TRL, one CUDA GPU):

```bash
python -m venv .venv && . .venv/bin/activate
pip install -r scripts/ask-training/requirements.txt
python scripts/ask-training/train.py --data /path/ask-sft --out /path/ask-qwen35-2b-sql
```

| Choice | Value | Why |
| --- | --- | --- |
| Base | `unsloth/Qwen3.5-2B` (16-bit) | the curated runtime model; Unsloth advises against 4-bit QLoRA on Qwen3.5 |
| Method | LoRA, r = α = 32, all attention + MLP projections | enough capacity for one narrow skill and a small adapter |
| Loss | completion only (`completion_only_loss=True`) | learn the answer, not the 4k-token schema |
| Schedule | 3 epochs, lr 2e-4 cosine, warm-up 3%, effective batch 16 | small dataset; keep the best checkpoint by validation loss |
| Context | 6,144 tokens | prompt plus answer; the script stops if an example is longer |
| Thinking | always the empty `<think>` block | Ask disables thinking at runtime, so no reasoning traces are needed |

Hardware: about 5 GB VRAM for 2B bf16 LoRA at short context according to Unsloth.
Plan for 12–16 GB at these prompt lengths, for example a single RTX 4080/4090,
A10, L4 or a Colab/Kaggle GPU. Training 2,000 examples × 3 epochs takes well
under an hour on such a card.

The script then exports `q4_k_m` GGUF with `save_pretrained_gguf`, the same
quantisation as the current catalog file. It keeps the LoRA adapter for later
runs.

## 4. Acceptance gate

A fine-tune replaces the base model only if, on the held-out corpus:

1. `ask-eval` (`cargo run --release --bin ask-eval -- --model <gguf>`) scores
   **strictly more correct answers** than base Qwen3.5 2B on the same machine,
   in both English and Italian;
2. no `unsupported` case regresses into SQL, and no answer leaves the
   sandbox's read-only rules;
3. p95 latency stays within 10% of the base model, because the quantisation
   and size are unchanged;
4. `tests/ask_runtime.rs` (`cargo test --test ask_runtime -- --ignored` with
   `SCROBBL_TEST_MODEL`) still passes.

Keep the evaluation report under `docs/reports/` next to the existing
[Ask SQL evaluation report](reports/ask-eval-sql.md).

## 5. Shipping the trained model

1. Publish the GGUF under the project's Hugging Face account, for example
   `itsmrma/scrobbl-ask-qwen3.5-2b-sql-GGUF`, under Apache-2.0 like the base
   model, with a model card that links this document and the evaluation report.
2. Replace the single entry in `src/ai/catalog.rs` with the new repository,
   **pinned revision, file name, exact byte size and LFS SHA-256** read from the
   Hugging Face API. Never guess a checksum. Keep the id stable or add a
   migration of `ai_active_model`.
3. Update the table in [docs/ai.md](ai.md) and the catalog test.

Until then, anyone can try a fine-tune without a release: copy the `.gguf` into
`<data_dir>/models/<folder>/` or set **Settings → AI → Custom local model path**.
`validate_gguf_file` checks it like any custom model.

## 6. Improving it over time

- When Ask answers a question wrongly, write a template for that question shape
  (not the personal value). The sandbox test proves the reference SQL. Then
  regenerate the data and retrain from the base model or the previous adapter.
- Prefer new *shapes* over more samples of an existing shape. Add paraphrases in
  both languages, including informal wording and typos.
- Never train on archive rows, real result values or owner-edited SQL copied
  from an instance. Ask's privacy promise covers the training data too.
