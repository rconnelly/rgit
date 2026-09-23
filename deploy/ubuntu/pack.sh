#!/usr/bin/env bash
# Build a Linux production archive from this checkout: release binary, LICENSE, README.
# Run on Linux (this machine or Actions). Same tarball shape as GitHub Releases.
#
#   ./deploy/ubuntu/pack.sh [tag]
#   RABUN_GIT_PACK_DIR=dist/release ./deploy/ubuntu/pack.sh
#   ./deploy/ubuntu/push.sh --pack user@host
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

usage() {
  echo "usage: $0 [tag]" >&2
  echo "tag defaults to git describe (or Cargo.toml version)" >&2
  exit 2
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
fi

TAG="${1:-${RABUN_GIT_TAG:-}}"
if [[ -z "$TAG" ]]; then
  if [[ -d .git ]] && command -v git >/dev/null; then
    TAG="$(git describe --tags --always --dirty 2>/dev/null || true)"
  fi
fi
if [[ -z "$TAG" ]]; then
  TAG="v$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
fi
if [[ -z "$TAG" ]]; then
  echo "could not determine a tag; pass one explicitly" >&2
  exit 1
fi

REVISION="${RABUN_GIT_REVISION:-${GITHUB_SHA:-}}"
if [[ -z "$REVISION" && -d .git ]] && command -v git >/dev/null; then
  REVISION="$(git rev-parse HEAD 2>/dev/null || true)"
fi

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64 | amd64) TRIPLE="x86_64-unknown-linux-gnu" ;;
  aarch64 | arm64) TRIPLE="aarch64-unknown-linux-gnu" ;;
  *)
    echo "unsupported architecture: ${ARCH} (pack on Linux)" >&2
    exit 1
    ;;
esac

if [[ "$(uname -s)" != Linux ]]; then
  echo "pack the release on Linux so the binary matches the server (got $(uname -s))" >&2
  exit 1
fi

if [[ -x "$ROOT/scripts/install-linux-build-deps.sh" ]]; then
  "$ROOT/scripts/install-linux-build-deps.sh" --check
fi

if ! command -v cargo >/dev/null; then
  echo "cargo is required to pack a release" >&2
  exit 1
fi

if command -v rustc >/dev/null; then
  HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
  if [[ -n "$HOST_TRIPLE" && "$HOST_TRIPLE" != "$TRIPLE" ]]; then
    echo "cross-compile from ${HOST_TRIPLE} to ${TRIPLE} is not supported; pack on the server architecture" >&2
    exit 1
  fi
fi

ensure_writable_dir() {
  local dir="$1"
  mkdir -p "$dir" || return 1
  [[ -w "$dir" ]] || return 1
  local probe="$dir/.rabun-git-pack-write-test"
  : >"$probe" || return 1
  rm -f "$probe"
}

OUT_DIR="${RABUN_GIT_PACK_DIR:-dist/release}"
if ! ensure_writable_dir "$OUT_DIR"; then
  if [[ -n "${RABUN_GIT_PACK_DIR:-}" ]]; then
    echo "cannot write to RABUN_GIT_PACK_DIR=${OUT_DIR}" >&2
    exit 1
  fi
  echo "warning: ${OUT_DIR} is not writable (often owned by root after a sudo pack)." >&2
  echo "warning: sudo chown -R \"\$USER:\$USER\" dist" >&2
  OUT_DIR="${TMPDIR:-/tmp}/rabun-git-pack"
  echo "warning: packing in ${OUT_DIR} instead" >&2
  if ! ensure_writable_dir "$OUT_DIR"; then
    echo "cannot write to ${OUT_DIR}" >&2
    exit 1
  fi
fi
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
ARCHIVE="${OUT_DIR}/rabun-git-${TAG}-${TRIPLE}.tar.gz"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

echo "building release binary (${TRIPLE})"
cargo build --release --locked
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  TARGET_DIR="$CARGO_TARGET_DIR"
  if [[ "$TARGET_DIR" != /* ]]; then
    TARGET_DIR="${ROOT}/${TARGET_DIR}"
  fi
else
  TARGET_DIR="${ROOT}/target"
fi
BIN="${TARGET_DIR}/release/rabun-git"
if [[ ! -x "$BIN" ]]; then
  echo "release build did not produce ${BIN}" >&2
  exit 1
fi
"$BIN" --version

install -m 0755 "$BIN" "$STAGE/rabun-git"
cp LICENSE README.md "$STAGE/"

SHORT_REV="${REVISION:0:7}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
cat >"$STAGE/BUILD" <<EOF
TAG=${TAG}
VERSION=${VERSION}
REVISION=${REVISION}
REVISION_SHORT=${SHORT_REV}
TRIPLE=${TRIPLE}
PROFILE=release
EOF

echo "creating ${ARCHIVE}"
TMP_ARCHIVE="$(mktemp "${TMPDIR:-/tmp}/rabun-git-pack.XXXXXX.tar.gz")"
tar -C "$STAGE" -czf "$TMP_ARCHIVE" .
mv "$TMP_ARCHIVE" "$ARCHIVE"
(cd "$OUT_DIR" && sha256sum "$(basename "$ARCHIVE")" >"$(basename "$ARCHIVE").sha256")

echo "packed ${ARCHIVE}"
echo "checksum ${ARCHIVE}.sha256"
echo "tag ${TAG}"
