#!/usr/bin/env python3
"""Collect distribution notices and corresponding sources from the locked build.

Run after `npm ci`, `cargo vendor --locked vendor > .cargo/config.toml` and build.
Only explicit source roots enter the bundle; private data and build caches never do.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parent.parent
SOURCE_ROOTS = (
    "Cargo.toml", "Cargo.lock", "package.json", "package-lock.json", "LICENSE",
    "REUSE.toml", "README.md", "THIRD_PARTY_NOTICES.md", "GLOSSARY.md", "AGENTS.md",
    "Dockerfile", ".dockerignore", "compose.yaml", ".env.example", ".gitignore",
    "vite.config.mts", "playwright.config.cjs", "frontend", "static", "src",
    "migrations", "release-migrations", "private-migrations", "tests", "scripts",
    "docs", ".github", ".cargo", "vendor", "node_modules",
)
NOTICE_NAMES = ("license", "licence", "copying", "notice", "copyright", "authors")


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True)


def collect(directory, destination):
    files = []
    for file in sorted(directory.rglob("*")):
        if not file.is_file() or file.is_symlink():
            continue
        relative = file.relative_to(directory)
        # Preserve native subcomponent notices, including LICENSES/*.txt.
        if any(any(name in part.lower() for name in NOTICE_NAMES) for part in relative.parts):
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(file, target)
            files.append(str(relative))
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/distribution")
    parser.add_argument("--target", default="x86_64-unknown-linux-gnu")
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    notices = out / "THIRD_PARTY_LICENSES"
    notices.mkdir(exist_ok=True)
    metadata = json.loads(command("cargo", "metadata", "--locked", "--offline", "--format-version", "1", "--filter-platform", args.target))
    # Include build dependencies as well as runtime dependencies. Preserve the exact target/feature tree.
    tree = command("cargo", "tree", "--locked", "--offline", "--target", args.target, "--edges", "normal,build", "--format", "{p}", "--prefix", "none")
    (out / "cargo-tree.txt").write_text(tree)
    selected = {tuple(line.split()[:2]) for line in tree.splitlines()}
    manifest = {"revision": args.revision, "target": args.target, "rustc": command("rustc", "--version").strip(), "cargo": [], "npm": []}
    features = {node["id"]: node["features"] for node in metadata["resolve"]["nodes"]}
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["name"] == "scrobbl-rs":
            continue
        directory = Path(package["manifest_path"]).parent
        destination = notices / "cargo" / (package["name"] + "-" + package["version"])
        files = collect(directory, destination)
        if package["name"] in ("ts-rs", "ts-rs-macros"):
            destination.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / "docs/licenses/upstream/ts-rs-MIT.txt", destination / "LICENSE")
            files.append("LICENSE")
        if not package["license"] or not files:
            raise RuntimeError(f"Missing license metadata or notices: {package['name']} {package['version']}")
        manifest["cargo"].append({"name": package["name"], "version": package["version"], "license": package["license"], "repository": package["repository"], "target_build": (package["name"], "v" + package["version"]) in selected, "features": features.get(package["id"], []), "notices": files})
    lock = json.loads((ROOT / "package-lock.json").read_text())
    for location, package in sorted(lock["packages"].items()):
        if not location or not (ROOT / location).is_dir():
            continue  # Optional packages for other operating systems are absent from this build.
        installed = json.loads((ROOT / location / "package.json").read_text())
        destination = notices / "npm" / location.removeprefix("node_modules/")
        files = collect(ROOT / location, destination)
        if installed["name"].startswith("@rolldown/binding-"):
            # The native binding package omits notices; its same-version parent ships them.
            parent = json.loads((ROOT / "node_modules/rolldown/package.json").read_text())
            if parent["version"] != installed["version"]:
                raise RuntimeError("Rolldown binding/parent version mismatch")
            files += collect(ROOT / "node_modules/rolldown", destination)
        if not installed.get("license") or not files:
            raise RuntimeError(f"Missing license metadata or notices: {location}")
        manifest["npm"].append({"name": installed["name"], "version": installed["version"], "license": installed["license"], "build_tool": package.get("dev", False), "notices": files})
    # Rust's standard library contains its own third-party components.
    rust_docs = Path(command("rustc", "--print", "sysroot").strip()) / "share/doc/rust"
    if not (rust_docs / "COPYRIGHT-library.html").is_file():
        raise RuntimeError("Install rust-docs to include the standard library's notices")
    shutil.copytree(rust_docs / "licenses", notices / "rust/licenses", dirs_exist_ok=True)
    shutil.copyfile(rust_docs / "COPYRIGHT-library.html", notices / "rust/COPYRIGHT-library.html")
    (out / "distribution.json").write_text(json.dumps(manifest, indent=2) + "\n")
    for name in ("LICENSE", "THIRD_PARTY_NOTICES.md"):
        shutil.copyfile(ROOT / name, out / name)
    archive = out / "source.tar.gz"
    with tarfile.open(archive, "w:gz") as tar:
        for name in SOURCE_ROOTS:
            path = ROOT / name
            if path.exists():
                tar.add(path, arcname="scrobbl-rs/" + name, filter=lambda info: None if Path(info.name).name in (".DS_Store", "__pycache__") else info)
        tar.add(out / "distribution.json", arcname="scrobbl-rs/distribution.json")
        tar.add(notices, arcname="scrobbl-rs/THIRD_PARTY_LICENSES")
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (out / "source.tar.gz.sha256").write_text(digest + "  source.tar.gz\n")
    print(f"Packaged {len(manifest['cargo'])} Cargo and {len(manifest['npm'])} npm packages; source SHA-256 {digest}")


if __name__ == "__main__":
    main()
