#!/usr/bin/env python3
"""Render the failing cases of #172 run reports as Markdown.

Useful when deciding whether a low exact-match rate is model quality, corpus
ambiguity, or a harness problem. Writes UTF-8 explicitly so Windows consoles do
not mangle names such as 坂本龍一.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def truncate(value: str, limit: int = 240) -> str:
    value = value.replace("\n", " ")
    return value if len(value) <= limit else value[:limit] + "..."


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", nargs="+", type=Path)
    parser.add_argument("--output", type=Path, default=Path("results/failures.md"))
    args = parser.parse_args()

    lines: list[str] = []
    for path in args.reports:
        report = json.loads(path.read_text(encoding="utf-8"))
        failures = [
            case
            for case in report["cases"]
            if not case["schema_valid"] or not case["expected_match"]
        ]
        lines.append(f"## {path.name} - {len(failures)} failures")
        lines.append("")
        if not failures:
            lines.append("None.")
            lines.append("")
            continue
        lines.append("| id | question | problem | output |")
        lines.append("| --- | --- | --- | --- |")
        for case in failures:
            problem = case["error"] or "interpretation mismatch"
            lines.append(
                "| {id} | {question} | {problem} | `{output}` |".format(
                    id=case["id"],
                    question=case["question"].replace("|", "\\|"),
                    problem=problem.replace("|", "\\|").replace("\n", " "),
                    output=truncate(case["output"]).replace("|", "\\|").replace("`", "'"),
                )
            )
        lines.append("")

    args.output.write_text("\n".join(lines), encoding="utf-8")
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
