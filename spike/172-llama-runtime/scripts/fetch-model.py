#!/usr/bin/env python3
"""Download and verify one pinned GGUF from models.json.

The download resumes with an HTTP Range request, writes to `<file>.part`,
verifies the SHA-256 and only then renames the file into place.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sys
import urllib.error
import urllib.request
from pathlib import Path

CHUNK = 1 << 20
USER_AGENT = "scrobbl-rs-spike/172"


def load_manifest(path: Path) -> list[dict]:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)["models"]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(CHUNK), b""):
            digest.update(chunk)
    return digest.hexdigest()


def human_bytes(value: float) -> str:
    for unit in ("B", "KiB", "MiB", "GiB"):
        if value < 1024 or unit == "GiB":
            return f"{value:.1f} {unit}"
        value /= 1024
    return f"{value:.1f} GiB"


def download(url: str, destination: Path, expected_size: int, expected_sha: str) -> Path:
    part = destination.with_name(destination.name + ".part")
    offset = part.stat().st_size if part.exists() else 0

    free = shutil.disk_usage(destination.parent).free
    remaining = expected_size - offset if offset else expected_size
    if free < remaining:
        raise SystemExit(
            f"not enough free space: need {human_bytes(remaining)}, have {human_bytes(free)}"
        )

    headers = {"User-Agent": USER_AGENT}
    if offset:
        headers["Range"] = f"bytes={offset}-"
        print(f"resuming at {human_bytes(offset)}")

    request = urllib.request.Request(url, headers=headers)
    try:
        response = urllib.request.urlopen(request)
    except urllib.error.HTTPError as error:
        raise SystemExit(f"download failed: HTTP {error.code} {error.reason}") from error

    with response:
        if offset and response.status != 206:
            print("server ignored Range; restarting from zero")
            offset = 0
        mode = "ab" if offset else "wb"
        written = offset
        with part.open(mode) as output:
            while True:
                chunk = response.read(CHUNK)
                if not chunk:
                    break
                output.write(chunk)
                written += len(chunk)
                percent = 100.0 * written / expected_size if expected_size else 0.0
                print(
                    f"\r{human_bytes(written)} / {human_bytes(expected_size)} ({percent:5.1f}%)",
                    end="",
                    flush=True,
                )
    print()

    actual_size = part.stat().st_size
    if expected_size and actual_size != expected_size:
        raise SystemExit(
            f"size mismatch: expected {expected_size} bytes, got {actual_size}; "
            f"left {part} in place for another resume"
        )

    actual_sha = sha256_file(part)
    if actual_sha != expected_sha:
        part.unlink(missing_ok=True)
        raise SystemExit(
            f"sha256 mismatch: expected {expected_sha}, got {actual_sha}; removed {part}"
        )

    os.replace(part, destination)
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--id", help="model id from models.json")
    parser.add_argument("--dir", type=Path, default=Path("models"), help="download directory")
    parser.add_argument(
        "--manifest",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "models.json",
    )
    parser.add_argument("--list", action="store_true", help="list ids and sizes")
    args = parser.parse_args()

    models = load_manifest(args.manifest)
    if args.list:
        for model in models:
            print(
                f"{model['id']:38} {human_bytes(model['bytes']):>10}  "
                f"{model['license']:10}  {model['file']}"
            )
        return 0

    if not args.id:
        parser.error("--id is required unless --list is used")

    model = next((entry for entry in models if entry["id"] == args.id), None)
    if model is None:
        available = ", ".join(entry["id"] for entry in models)
        raise SystemExit(f"unknown model {args.id!r}; available: {available}")

    args.dir.mkdir(parents=True, exist_ok=True)
    destination = args.dir / model["file"]

    if destination.exists():
        if destination.stat().st_size == model["bytes"] and sha256_file(destination) == model["sha256"]:
            print(f"already verified: {destination}")
            return 0
        raise SystemExit(
            f"{destination} exists but its size or sha256 does not match models.json; "
            "move or delete it before retrying"
        )

    url = (
        f"https://huggingface.co/{model['repo']}/resolve/{model['revision']}/"
        f"{model['file']}"
    )
    print(f"{model['label']}")
    print(f"  {url}")
    print(f"  expected {model['sha256']} ({human_bytes(model['bytes'])})")
    download(url, destination, model["bytes"], model["sha256"])
    print(f"verified: {destination}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
