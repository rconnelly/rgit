#!/usr/bin/env bash
# Require a SemVer 2.0.0 git tag (`v` prefix) that matches Cargo.toml.
# Usage: ./scripts/check-release-tag.sh [tag]
# tag defaults to GITHUB_REF_NAME.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TAG="${1:-${GITHUB_REF_NAME:-}}"
CARGO_VER="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"

if [[ -z "$CARGO_VER" ]]; then
  echo "could not read version from Cargo.toml" >&2
  exit 1
fi
if [[ -z "$TAG" ]]; then
  echo "usage: $0 vMAJOR.MINOR.PATCH" >&2
  echo "pass a tag or set GITHUB_REF_NAME" >&2
  exit 2
fi

BARE="${TAG#v}"
if [[ "$TAG" == "$BARE" ]]; then
  echo "Release tags must use a v prefix (got ${TAG})." >&2
  echo "SemVer 2.0.0 lives in Cargo.toml; git tags are v plus that version." >&2
  exit 1
fi
if [[ "$BARE" != "$CARGO_VER" ]]; then
  echo "Tag ${TAG} does not match Cargo.toml version ${CARGO_VER} (expected v${CARGO_VER})." >&2
  exit 1
fi

# Pre-release is the SemVer -ident part, not build metadata after +.
CORE="${BARE%%+*}"
PRERELEASE=false
if [[ "$CORE" == *-* ]]; then
  PRERELEASE=true
fi

echo "tag ${TAG} matches Cargo.toml ${CARGO_VER} (SemVer 2.0.0)"
echo "prerelease=${PRERELEASE}"

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    echo "tag=${TAG}"
    echo "version=${CARGO_VER}"
    echo "prerelease=${PRERELEASE}"
  } >>"$GITHUB_OUTPUT"
fi
