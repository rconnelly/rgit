#!/usr/bin/env bash
# Create a collision-safe `rgit` symlink beside `rabun-git`.
# Same program; `rgit` is the short command. Does not overwrite a foreign `rgit`.
#
#   ./scripts/link-rgit.sh           # PATH or ~/.cargo/bin
#   ./scripts/link-rgit.sh --dir DIR # DIR/rabun-git -> DIR/rgit
set -euo pipefail

usage() {
  echo "usage: $0 [--dir DIR]" >&2
  exit 2
}

dir=""
if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
fi
if [[ "${1:-}" == "--dir" ]]; then
  dir="${2:-}"
  if [[ -z "$dir" ]]; then
    usage
  fi
elif [[ -n "${1:-}" ]]; then
  usage
fi

if [[ -z "$dir" ]]; then
  if command -v rabun-git >/dev/null 2>&1; then
    dir="$(dirname "$(command -v rabun-git)")"
  elif [[ -x "${HOME}/.cargo/bin/rabun-git" ]]; then
    dir="${HOME}/.cargo/bin"
  else
    echo "rabun-git not found; install with: cargo install --path . --locked" >&2
    exit 1
  fi
fi

long="${dir}/rabun-git"
short="${dir}/rgit"

if [[ ! -x "$long" ]]; then
  echo "rabun-git is not executable: ${long}" >&2
  exit 1
fi

if [[ -L "$short" ]]; then
  target="$(readlink "$short")"
  if [[ "$(basename "$target")" == "rabun-git" ]]; then
    ln -sfn rabun-git "$short"
    echo "rgit -> rabun-git (${short})"
    exit 0
  fi
  echo "refusing to overwrite ${short}: not a symlink to rabun-git (got ${target})" >&2
  exit 1
fi

if [[ -e "$short" ]]; then
  echo "refusing to overwrite ${short}: not a symlink to rabun-git" >&2
  exit 1
fi

ln -sfn rabun-git "$short"
echo "rgit -> rabun-git (${short})"
