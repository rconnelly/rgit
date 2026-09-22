#!/usr/bin/env bash
# Install OS packages needed to compile rabun-git on Linux (C compiler for
# native crypto crates; git is required at runtime and in tests).
#
#   ./scripts/install-linux-build-deps.sh          # install if missing
#   ./scripts/install-linux-build-deps.sh --check  # exit 1 if missing (no install)
set -euo pipefail

CHECK=0
if [[ "${1:-}" == "--check" ]]; then
  CHECK=1
elif [[ "${1:-}" != "" ]]; then
  echo "usage: $0 [--check]" >&2
  exit 2
fi

cc_ok() {
  command -v cc >/dev/null 2>&1 || command -v gcc >/dev/null 2>&1
}

missing=()
cc_ok || missing+=("a C compiler (build-essential)")
command -v git >/dev/null 2>&1 || missing+=("git")
command -v pkg-config >/dev/null 2>&1 || missing+=("pkg-config")

if [[ ${#missing[@]} -eq 0 ]]; then
  echo "Linux build deps OK (cc, git, pkg-config)"
  exit 0
fi

echo "missing Linux build deps: ${missing[*]}" >&2

if [[ "$CHECK" -eq 1 ]]; then
  echo "install with: ./scripts/install-linux-build-deps.sh" >&2
  exit 1
fi

if command -v apt-get >/dev/null 2>&1; then
  if [[ "$(id -u)" -ne 0 ]]; then
    if command -v sudo >/dev/null 2>&1; then
      exec sudo DEBIAN_FRONTEND=noninteractive "$0" "$@"
    fi
    echo "run as root, or: sudo apt-get install -y build-essential git pkg-config" >&2
    exit 1
  fi
  export DEBIAN_FRONTEND=noninteractive
  apt-get update
  apt-get install -y --no-install-recommends build-essential git pkg-config
  echo "installed build-essential git pkg-config"
  exit 0
fi

if command -v dnf >/dev/null 2>&1; then
  if [[ "$(id -u)" -ne 0 ]]; then
    echo "run as root, or: sudo dnf install -y gcc git pkgconf-pkg-config" >&2
    exit 1
  fi
  dnf install -y gcc git pkgconf-pkg-config
  exit 0
fi

echo "unknown distro; install a C compiler, git, and pkg-config" >&2
exit 1
