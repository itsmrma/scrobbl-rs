#!/usr/bin/env python3
"""Measure the #172 distribution budget: image delta and binary size.

The budgets from the ticket are checked in MiB (25 image delta, 30 binary),
which is stricter than decimal MB. Over-budget runs exit with status 1 so the
spike cannot silently claim success.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

MIB = 1024 * 1024
IMAGE_BUDGET = 25 * MIB
BINARY_BUDGET = 30 * MIB


def docker(*args: str, capture: bool = True) -> str:
    completed = subprocess.run(
        ["docker", *args],
        check=True,
        text=True,
        stdout=subprocess.PIPE if capture else subprocess.DEVNULL,
    )
    if not capture:
        return ""
    return completed.stdout.strip()


def image_size(reference: str) -> int:
    return int(docker("image", "inspect", "--format", "{{.Size}}", reference))


def extract_binary(reference: str, path_in_image: str, destination: Path) -> None:
    container = f"scrobbl-spike-size-{os.getpid()}"
    docker("create", "--name", container, reference, capture=True)
    try:
        docker("cp", f"{container}:{path_in_image}", str(destination))
    finally:
        docker("rm", "--force", container)


def human(value: int | None) -> str:
    if value is None:
        return "n/a"
    return f"{value / MIB:.1f} MiB"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", help="current scrobbl-rs image reference")
    parser.add_argument("--spike", required=True, help="spike image reference")
    parser.add_argument("--binary", type=Path, help="binary already copied out of the image")
    parser.add_argument(
        "--path-in-image",
        default="/usr/local/bin/scrobbl-ask-spike",
        help="binary path used when --binary is omitted",
    )
    parser.add_argument("--json-out", type=Path)
    args = parser.parse_args()

    baseline = image_size(args.baseline) if args.baseline else None
    spike = image_size(args.spike)

    binary_size: int | None = None
    if args.binary:
        binary_size = args.binary.stat().st_size
    else:
        with_destination = Path(f"scrobbl-ask-spike-size-{os.getpid()}")
        try:
            extract_binary(args.spike, args.path_in_image, with_destination)
            binary_size = with_destination.stat().st_size
        finally:
            with_destination.unlink(missing_ok=True)

    delta = spike - baseline if baseline is not None else None
    image_ok = delta is None or delta <= IMAGE_BUDGET
    binary_ok = binary_size <= BINARY_BUDGET

    print(f"baseline image : {human(baseline)}")
    print(f"spike image    : {human(spike)}")
    print(f"image delta    : {human(delta)}  (budget {human(IMAGE_BUDGET)})")
    print(f"spike binary   : {human(binary_size)}  (budget {human(BINARY_BUDGET)})")
    print(f"image budget   : {'PASS' if image_ok else 'FAIL'}")
    print(f"binary budget  : {'PASS' if binary_ok else 'FAIL'}")

    if args.json_out:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(
            json.dumps(
                {
                    "baseline_image_bytes": baseline,
                    "spike_image_bytes": spike,
                    "image_delta_bytes": delta,
                    "binary_bytes": binary_size,
                    "image_budget_bytes": IMAGE_BUDGET,
                    "binary_budget_bytes": BINARY_BUDGET,
                    "image_ok": image_ok,
                    "binary_ok": binary_ok,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    return 0 if image_ok and binary_ok else 1


if __name__ == "__main__":
    sys.exit(main())
