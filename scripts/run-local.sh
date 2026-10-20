#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if command -v cargo >/dev/null 2>&1; then
  scrobbl_cargo=cargo
elif [ -x "$HOME/.cargo/bin/cargo" ]; then
  scrobbl_cargo="$HOME/.cargo/bin/cargo"
else
  echo 'Rust is required. Install it from https://rustup.rs and run this script again.' >&2
  exit 1
fi
exec "$scrobbl_cargo" run --locked "$@"
