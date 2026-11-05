#!/usr/bin/env python3
"""Measure the #180 distribution budget: runtime image delta and binary size.

The ticket budgets are 25 MiB for the image increment and 30 MiB for the
binary. The official image also ships the corresponding-source/notices bundle
(`/usr/share/scrobbl-rs`), which is a fixed GPL distribution obligation and not
part of the AI runtime addition. The gated "runtime image delta" is therefore

    (official image size - debian baseline size) - source/notices bundle size

while the raw delta and the bundle size are printed for transparency. The
binary is measured from the image (stripped release profile) or from `--binary`
for a local cross-check without Docker. Over-budget runs exit with status 1.

This script has no Docker dependency until an image is actually measured; the
pure `evaluate` function is unit-tested without a container runtime.
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
DEFAULT_BASELINE = "debian:bookworm-slim"
DEFAULT_BINARY_PATH = "/usr/local/bin/scrobbl-rs"
DEFAULT_DISTRIBUTION_PATH = "/usr/share/scrobbl-rs"


def human(value: int | None) -> str:
    if value is None:
        return "not measured"
    return f"{value / MIB:.1f} MiB"


def evaluate(
    *,
    image_bytes: int | None,
    baseline_bytes: int | None,
    distribution_bytes: int,
    binary_bytes: int | None,
) -> dict:
    """Apply the budget rules without touching Docker or the filesystem."""
    if image_bytes is not None and baseline_bytes is None:
        raise ValueError("a baseline image is required when measuring an image")
    if distribution_bytes < 0:
        raise ValueError("distribution size cannot be negative")

    raw_delta = None
    runtime_delta = None
    image_ok = None
    if image_bytes is not None:
        raw_delta = image_bytes - baseline_bytes
        runtime_delta = raw_delta - distribution_bytes
        image_ok = runtime_delta <= IMAGE_BUDGET

    binary_ok = None if binary_bytes is None else binary_bytes <= BINARY_BUDGET
    ok = all(check is not False for check in (image_ok, binary_ok))
    return {
        "image_bytes": image_bytes,
        "baseline_image_bytes": baseline_bytes,
        "raw_image_delta_bytes": raw_delta,
        "distribution_bytes": distribution_bytes,
        "runtime_image_delta_bytes": runtime_delta,
        "image_budget_bytes": IMAGE_BUDGET,
        "binary_bytes": binary_bytes,
        "binary_budget_bytes": BINARY_BUDGET,
        "image_ok": image_ok,
        "binary_ok": binary_ok,
        "ok": ok,
    }


def docker(*arguments: str) -> str:
    return subprocess.check_output(["docker", *arguments], text=True).strip()


def image_size(reference: str) -> int:
    return int(docker("image", "inspect", "--format", "{{.Size}}", reference))


def ensure_image(reference: str) -> int:
    try:
        return image_size(reference)
    except subprocess.CalledProcessError:
        docker("pull", reference)
        return image_size(reference)


def extract_binary(reference: str, path_in_image: str, destination: Path) -> None:
    container = f"scrobbl-size-{os.getpid()}"
    docker("create", "--name", container, reference)
    try:
        docker("cp", f"{container}:{path_in_image}", str(destination))
    finally:
        subprocess.run(["docker", "rm", "--force", container], check=False,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def distribution_bytes(reference: str, path_in_image: str) -> int:
    output = docker("run", "--rm", "--entrypoint", "du", reference, "-sb", path_in_image)
    return int(output.split()[0])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", help="official image reference to measure")
    parser.add_argument(
        "--baseline",
        default=DEFAULT_BASELINE,
        help=f"baseline image reference (default: {DEFAULT_BASELINE})",
    )
    parser.add_argument(
        "--path-in-image",
        default=DEFAULT_BINARY_PATH,
        help=f"binary path inside the image (default: {DEFAULT_BINARY_PATH})",
    )
    parser.add_argument(
        "--distribution-path",
        default=DEFAULT_DISTRIBUTION_PATH,
        help="corresponding-source/notices bundle excluded from the runtime delta "
        f"(default: {DEFAULT_DISTRIBUTION_PATH})",
    )
    parser.add_argument(
        "--binary",
        type=Path,
        help="measure this binary instead of extracting it from --image",
    )
    parser.add_argument(
        "--json-out",
        type=Path,
        help="write the measurement and budget verdict as JSON",
    )
    args = parser.parse_args()

    if args.image is None and args.binary is None:
        parser.error("pass --image, --binary, or both")

    image = None
    baseline = None
    distribution = 0
    if args.image:
        baseline = ensure_image(args.baseline)
        image = image_size(args.image)
        distribution = distribution_bytes(args.image, args.distribution_path)

    binary = None
    if args.binary:
        binary = args.binary.stat().st_size
    elif args.image:
        extracted = Path(f"scrobbl-size-{os.getpid()}")
        try:
            extract_binary(args.image, args.path_in_image, extracted)
            binary = extracted.stat().st_size
        finally:
            extracted.unlink(missing_ok=True)

    result = evaluate(
        image_bytes=image,
        baseline_bytes=baseline,
        distribution_bytes=distribution,
        binary_bytes=binary,
    )

    if image is not None:
        print(f"baseline image      : {human(baseline)}")
        print(f"official image      : {human(image)}")
        print(f"raw image delta     : {human(result['raw_image_delta_bytes'])}")
        print(f"source/notices      : {human(distribution)}")
        print(
            "runtime image delta : "
            f"{human(result['runtime_image_delta_bytes'])}  "
            f"(budget {human(IMAGE_BUDGET)})"
        )
        print(f"image budget        : {'PASS' if result['image_ok'] else 'FAIL'}")
    print(f"binary              : {human(binary)}  (budget {human(BINARY_BUDGET)})")
    print(f"binary budget       : {'PASS' if result['binary_ok'] else 'FAIL'}")

    if args.json_out:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")

    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
