#!/usr/bin/env bash
# Install a rabun-git binary from a local archive and restart systemd.
# The host does not need GitHub access.
#
# Usage:
#   sudo RABUN_GIT_ARCHIVE=/path/to/rabun-git-<tag>-x86_64-unknown-linux-gnu.tar.gz ./install.sh [tag]
#   sudo RABUN_GIT_ENV_FILE=/path/to/.env ./install.sh
set -euo pipefail

SCRIPT_DIR=""
if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" ]]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

TAG="${1:-${RABUN_GIT_TAG:-}}"
PREFIX="${RABUN_GIT_PREFIX:-/usr/local}"
ARCHIVE_PATH="${RABUN_GIT_ARCHIVE:-}"
ENV_FILE="${RABUN_GIT_ENV_FILE:-}"

if [[ "$(id -u)" -ne 0 ]]; then
  exec sudo --preserve-env=RABUN_GIT_TAG,RABUN_GIT_PREFIX,RABUN_GIT_ARCHIVE,RABUN_GIT_ENV_FILE,RABUN_GIT_RABUN_TOML "$0" "$@"
fi

if [[ -z "$ARCHIVE_PATH" || ! -f "$ARCHIVE_PATH" ]]; then
  echo "set RABUN_GIT_ARCHIVE to a release tarball on this machine (private GitHub repos cannot be fetched from the server)" >&2
  exit 1
fi

install_env_file() {
  local src="$1"
  if [[ -z "$src" || ! -f "$src" ]]; then
    return 0
  fi
  install -d -m 0750 /etc/rabun-git
  install -m 0640 "$src" /etc/rabun-git/rabun-git.env
  if id -u rabun-git >/dev/null 2>&1; then
    chown root:rabun-git /etc/rabun-git/rabun-git.env
  fi
  echo "wrote /etc/rabun-git/rabun-git.env from $(basename "$src")"
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cp "$ARCHIVE_PATH" "$TMP/rabun-git.tar.gz"
SUM="${ARCHIVE_PATH}.sha256"
if [[ -f "$SUM" ]]; then
  expected="$(awk '{print $1}' "$SUM")"
  actual="$(sha256sum "$TMP/rabun-git.tar.gz" | awk '{print $1}')"
  if [[ "$expected" != "$actual" ]]; then
    echo "checksum mismatch for ${ARCHIVE_PATH}" >&2
    exit 1
  fi
fi

tar -xzf "$TMP/rabun-git.tar.gz" -C "$TMP"
BIN="$(find "$TMP" -type f -name rabun-git | head -n1)"
if [[ -z "$BIN" ]]; then
  echo "archive did not contain a rabun-git binary" >&2
  exit 1
fi

install -d "${PREFIX}/bin"
install -m 0755 "$BIN" "${PREFIX}/bin/rabun-git"
"${PREFIX}/bin/rabun-git" --version

install_env_file "$ENV_FILE"

if [[ -n "$SCRIPT_DIR" && -f "${SCRIPT_DIR}/rabun-git.service" ]]; then
  if [[ ! -f /etc/rabun-git/rabun-git.toml && -f "${SCRIPT_DIR}/rabun-git.toml" ]]; then
    install -d -m 0750 /etc/rabun-git
    install -m 0644 "${SCRIPT_DIR}/rabun-git.toml" /etc/rabun-git/rabun-git.toml
  fi
  if [[ ! -f /etc/rabun-git/rabun-git.env && -f "${SCRIPT_DIR}/rabun-git.env.example" ]]; then
    install -d -m 0750 /etc/rabun-git
    install -m 0640 "${SCRIPT_DIR}/rabun-git.env.example" /etc/rabun-git/rabun-git.env
    if id -u rabun-git >/dev/null 2>&1; then
      chown root:rabun-git /etc/rabun-git/rabun-git.env
    fi
  fi
  if id -u rabun-git >/dev/null 2>&1; then
    install -d -m 0755 -o rabun-git -g rabun-git /var/lib/rabun-git
  fi
  cp "${SCRIPT_DIR}/rabun-git.service" /etc/systemd/system/rabun-git.service
  chmod 0644 /etc/systemd/system/rabun-git.service
fi

if [[ -n "$SCRIPT_DIR" && -f "${SCRIPT_DIR}/register-app.sh" ]]; then
  bash "${SCRIPT_DIR}/register-app.sh"
fi

if systemctl cat rabun-git.service >/dev/null 2>&1; then
  systemctl daemon-reload
  systemctl enable --now rabun-git.service
  systemctl restart rabun-git.service
  for _ in $(seq 1 10); do
    if systemctl is-active --quiet rabun-git.service; then
      echo "rabun-git ${TAG:-} is active"
      systemctl --no-pager --full status rabun-git.service | head -n 12 || true
      exit 0
    fi
    sleep 1
  done
  echo "rabun-git installed but the unit did not become active" >&2
  systemctl status rabun-git.service --no-pager >&2 || true
  journalctl -u rabun-git.service -n 40 --no-pager >&2 || true
  exit 1
fi

echo "binary installed; run bootstrap.sh to install the systemd unit"
