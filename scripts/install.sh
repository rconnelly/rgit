#!/usr/bin/env bash
# Install `rabun-git` from this checkout and link `rgit` beside it.
#
#   ./scripts/install.sh
#
# Runs:
#   cargo install --path . --locked
#   ./scripts/link-rgit.sh
set -euo pipefail

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  echo "usage: $0" >&2
  echo "runs: cargo install --path . --locked && ./scripts/link-rgit.sh" >&2
  exit 2
fi
if [[ -n "${1:-}" ]]; then
  echo "usage: $0" >&2
  exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required; install Rust from https://rustup.rs/" >&2
  exit 1
fi

if [[ -x "$ROOT/scripts/install-linux-build-deps.sh" ]]; then
  "$ROOT/scripts/install-linux-build-deps.sh" --check
fi

cargo install --path . --locked
"$ROOT/scripts/link-rgit.sh"
if command -v rgit >/dev/null 2>&1; then
  rgit --version
else
  rabun-git --version
fi
