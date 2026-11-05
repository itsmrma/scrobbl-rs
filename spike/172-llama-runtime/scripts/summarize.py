#!/usr/bin/env python3
"""Summarize #172 run reports into one Markdown table.

Reads the JSON reports produced by `run-matrix.py` (or by the harness directly)
and prints the `all` summary plus how many outputs contain a thinking block.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def thinking_count(cases: list[dict]) -> int:
    return sum(
        1
        for case in cases
        if " thinking" in case.get("output", "") or "<think>" in case.get("output", "")
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", nargs="+", type=Path)
    parser.add_argument("--output", type=Path, help="write Markdown here instead of stdout")
    args = parser.parse_args()

    lines = [
        "| run | model | chat | schema | exact | e2e p50 | e2e p95 | TTFT p50 | prefill tok/s | decode tok/s | thinking |",
        "| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]

    for path in args.reports:
        report = json.loads(path.read_text(encoding="utf-8"))
        summary = next(
            (entry for entry in report["summary"] if entry["lang"] == "all"), None
        )
        if summary is None:
            continue
        model = Path(report["model"]["path"]).name
        lines.append(
            "| {label} | {model} | {chat} | {schema}/{cases} | {exact}/{cases} | "
            "{p50:.0f} ms | {p95:.0f} ms | {ttft:.0f} ms | {prefill:.0f} | "
            "{decode:.1f} | {thinking} |".format(
                label=report["label"],
                model=model,
                chat=report["config"]["chat"],
                schema=summary["schema_valid"],
                exact=summary["expected_match"],
                cases=summary["cases"],
                p50=summary["total_ms_p50"],
                p95=summary["total_ms_p95"],
                ttft=summary["first_token_ms_p50"],
                prefill=summary["prefill_tokens_per_s_p50"],
                decode=summary["decode_tokens_per_s_p50"],
                thinking=thinking_count(report["cases"]),
            )
        )

    text = "\n".join(lines) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    else:
        print(text, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
