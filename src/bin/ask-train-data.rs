#![allow(clippy::disallowed_methods)]
//! Writes the supervised fine-tuning dataset for the local Ask SQL model.
//!
//! Expands `evals/ask/train_templates.json` (see `src/ask/training.rs`), runs
//! every distinct reference query through the read-only Ask sandbox on the
//! fixed synthetic fixture, and writes `train.jsonl` and `validation.jsonl`
//! with `prompt`/`completion` pairs rendered exactly like the runtime prompt.
//! Reads no archive and makes no network request.
//!
//! ```text
//! cargo run --locked --no-default-features --bin ask-train-data -- \
//!   --out C:\temp\ask-sft --samples 4 --seed 2026
//! ```
use scrobbl_rs::ask::{
    fixture,
    sandbox::{self, SandboxLimits},
    training,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::PathBuf,
    process::ExitCode,
};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ask-train-data: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let mut out: Option<PathBuf> = None;
    let mut samples = 4_usize;
    let mut seed = 2026_u64;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(value()?)),
            "--samples" => samples = value()?.parse().map_err(|_| "--samples expects a number")?,
            "--seed" => seed = value()?.parse().map_err(|_| "--seed expects a number")?,
            "-h" | "--help" => {
                eprintln!("usage: ask-train-data --out <dir> [--samples 4] [--seed 2026]");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let out = out.ok_or("--out <dir> is required")?;
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;

    let examples = training::generate(samples, seed)?;
    eprintln!(
        "ask-train-data: {} examples, validating SQL…",
        examples.len()
    );

    // A fresh private folder for the synthetic fixture archive; removed below.
    let work = std::env::temp_dir().join(format!("scrobbl-ask-train-{}", std::process::id()));
    if work.exists() {
        return Err(format!("{} already exists", work.display()));
    }
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let fixture = fixture::build(&work)
        .await
        .map_err(|e| format!("could not build the fixture: {e}"))?;
    let mut checked = BTreeSet::new();
    for example in &examples {
        if example.sql.is_empty() || !checked.insert(example.sql.clone()) {
            continue;
        }
        let output = sandbox::execute(
            fixture.projection_path(),
            &example.sql,
            SandboxLimits::default(),
        )
        .map_err(|r| {
            format!(
                "{}: sandbox rejected the SQL: {}",
                example.id, r.model_error
            )
        })?;
        let declared: Vec<&str> = example.columns.iter().map(|(n, _)| n.as_str()).collect();
        if output.columns != declared {
            return Err(format!(
                "{}: columns {:?} differ from the declared {:?}",
                example.id, output.columns, declared
            ));
        }
    }

    drop(fixture);
    let _ = std::fs::remove_dir_all(&work);

    let mut files = BTreeMap::new();
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    for example in &examples {
        let split = if example.is_validation() {
            "validation"
        } else {
            "train"
        };
        let file = match files.entry(split) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let path = out.join(format!("{split}.jsonl"));
                let file =
                    std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                entry.insert((std::io::BufWriter::new(file), 0_usize))
            }
        };
        serde_json::to_writer(&mut file.0, &example.record()).map_err(|e| e.to_string())?;
        file.0.write_all(b"\n").map_err(|e| e.to_string())?;
        file.1 += 1;
        for tag in &example.tags {
            *tags.entry(tag.clone()).or_default() += 1;
        }
    }
    for (split, (mut writer, count)) in files {
        writer.flush().map_err(|e| e.to_string())?;
        eprintln!("ask-train-data: {count} {split} examples");
    }
    eprintln!(
        "ask-train-data: {} distinct queries ran in the sandbox; tags: {}",
        checked.len(),
        tags.iter()
            .map(|(tag, n)| format!("{tag}={n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}
