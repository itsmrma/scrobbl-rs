#!/usr/bin/env python3
"""Run the #172 measurement matrix on the current machine.

The matrix is deliberately small: the grammar-constrained product path for the
four candidate models, plus the two thinking-mode checks without GBNF. Missing
models are skipped with a warning so the script can run on a partial download.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

MATRIX = [
    {
        "id": "1.7b-nothink",
        "model": "Qwen3-1.7B-Q4_K_M.gguf",
        "chat": "qwen3-nothink",
        "grammar": True,
    },
    {
        "id": "4b-nothink",
        "model": "Qwen3-4B-Q4_K_M.gguf",
        "chat": "qwen3-nothink",
        "grammar": True,
    },
    {
        "id": "0.6b-nothink",
        "model": "Qwen3-0.6B-Q4_K_M.gguf",
        "chat": "qwen3-nothink",
        "grammar": True,
    },
    {
        "id": "qwen25-1.5b",
        "model": "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        "chat": "qwen25",
        "grammar": True,
        # Comparison only: the v1 catalog decision is Qwen3-only.
    },
    {
        "id": "1.7b-think-nogrammar",
        "model": "Qwen3-1.7B-Q4_K_M.gguf",
        "chat": "qwen3-think",
        "grammar": False,
    },
    {
        "id": "1.7b-nothink-nogrammar",
        "model": "Qwen3-1.7B-Q4_K_M.gguf",
        "chat": "qwen3-nothink",
        "grammar": False,
    },
]


def default_binary() -> Path:
    candidates = [
        Path("target/release/scrobbl-ask-spike.exe"),
        Path("target/release/scrobbl-ask-spike"),
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    raise SystemExit(
        "release binary not found; run `cargo build --release` first "
        "or pass --binary"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, help="harness binary")
    parser.add_argument("--models", type=Path, default=Path("models"))
    parser.add_argument("--results", type=Path, default=Path("results"))
    parser.add_argument("--threads", type=int, default=8)
    parser.add_argument("--label", default="local")
    parser.add_argument("--only", action="append", help="run only this matrix id")
    args = parser.parse_args()

    binary = args.binary or default_binary()
    args.results.mkdir(parents=True, exist_ok=True)

    selected = [
        entry for entry in MATRIX if not args.only or entry["id"] in args.only
    ]
    if not selected:
        raise SystemExit("--only did not match any matrix entry")

    failed = 0
    for entry in selected:
        model = args.models / entry["model"]
        if not model.exists():
            print(f"skip {entry['id']}: {model} missing", file=sys.stderr)
            continue

        stem = f"{args.label}-{entry['id']}"
        command = [
            str(binary),
            "--model",
            str(model),
            "--out",
            str(args.results / f"{stem}.json"),
            "--md-out",
            str(args.results / f"{stem}.md"),
            "--label",
            stem,
            "--threads",
            str(args.threads),
            "--chat",
            entry["chat"],
        ]
        if not entry["grammar"]:
            command.append("--no-grammar")

        print(f"\n== {entry['id']} ==", flush=True)
        completed = subprocess.run(command)
        if completed.returncode != 0:
            failed += 1
            print(f"FAILED {entry['id']} ({completed.returncode})", file=sys.stderr)

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
