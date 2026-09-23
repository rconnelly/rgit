#!/usr/bin/env bash
# Copy a release onto a bare-metal Ubuntu host over SSH. The server never talks to GitHub.
# Run this from a clone on a machine that can see the private repo (laptop or Actions).
#
#   gh auth login   # once, locally
#   ./deploy/ubuntu/push.sh --bootstrap user@host
#   ./deploy/ubuntu/push.sh --bootstrap --env .env user@host
#   ./deploy/ubuntu/push.sh user@host
#   ./deploy/ubuntu/push.sh --pack user@host
# Omit the tag to install the latest stable GitHub Release.
# --pack builds from this checkout (quick patches / integration testing).
# SSH is publickey-only (no sshd password prompt). Use a sudoer that already
# has your key in authorized_keys — not root unless that account has the key.
set -euo pipefail

BOOTSTRAP=0
ARCHIVE=""
ENV_FILE=""
PACK=0
IDENTITY="${RABUN_GIT_SSH_IDENTITY:-}"
SSH_PORT="${RABUN_GIT_SSH_PORT:-22}"
REPO="${RABUN_GIT_REPO:-Burton-Workspaces/rabun-git}"

usage() {
  echo "usage: $0 [--bootstrap] [--archive FILE] [--env FILE] [--pack] [--identity FILE] [--port N] user@host [tag]" >&2
  echo "tag defaults to the latest stable GitHub Release when omitted" >&2
  echo "--pack builds a release archive from this checkout (Linux + cargo)" >&2
  echo "--archive FILE installs that tarball instead of a GitHub Release" >&2
  echo "--env FILE copies a dotenv file to /etc/rabun-git/rabun-git.env" >&2
  echo "  (forge paths and binds; not /etc/rabun/rabun.env)" >&2
  echo "--identity FILE is the SSH private key (also RABUN_GIT_SSH_IDENTITY)" >&2
  echo "SSH uses publickey only; root@HOST fails unless that key is authorized" >&2
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bootstrap) BOOTSTRAP=1; shift ;;
    --archive) ARCHIVE="${2:-}"; shift 2 ;;
    --env) ENV_FILE="${2:-}"; shift 2 ;;
    --pack) PACK=1; shift ;;
    --identity) IDENTITY="${2:-}"; shift 2 ;;
    --port) SSH_PORT="${2:-}"; shift 2 ;;
    -h | --help) usage ;;
    --) shift; break ;;
    -*) usage ;;
    *) break ;;
  esac
done

TARGET_HOST="${1:-}"
TAG="${2:-}"
if [[ -z "$TARGET_HOST" ]]; then
  usage
fi
if [[ -n "$ENV_FILE" && ! -f "$ENV_FILE" ]]; then
  echo "--env file is missing: ${ENV_FILE}" >&2
  exit 1
fi
if [[ -n "$IDENTITY" && ! -f "$IDENTITY" ]]; then
  echo "--identity file is missing: ${IDENTITY}" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# -o Port= works for both ssh (-p) and scp (-P). scp -p means preserve times,
# so passing -p 22 makes scp try to copy a local file named "22".
# publickey only: never fall back to sshd password auth (root@ often offers it).
SSH_OPTS=(
  -o Port="$SSH_PORT"
  -o ServerAliveInterval=15
  -o PreferredAuthentications=publickey
  -o PubkeyAuthentication=yes
)
if [[ -n "$IDENTITY" ]]; then
  SSH_OPTS+=(-i "$IDENTITY" -o IdentitiesOnly=yes)
fi
if [[ -n "${RABUN_GIT_SSH_KNOWN_HOSTS:-}" ]]; then
  SSH_OPTS+=(-o StrictHostKeyChecking=yes -o UserKnownHostsFile="$RABUN_GIT_SSH_KNOWN_HOSTS")
else
  SSH_OPTS+=(-o StrictHostKeyChecking=accept-new)
fi

ssh_publickey_hint() {
  echo "SSH publickey failed for ${TARGET_HOST}." >&2
  echo "This script does not prompt for an SSH password. Use a sudoer that already has your key in authorized_keys, not root@ unless that key is installed." >&2
}

remote() {
  local st=0
  ssh "${SSH_OPTS[@]}" "$TARGET_HOST" "$@" || st=$?
  if [[ "$st" -eq 255 ]]; then
    ssh_publickey_hint
  fi
  return "$st"
}

# Privilege the install. SSH login is already publickey; any password here is sudo.
# Skip sudo when the remote user is root. Prefer passwordless sudo; otherwise a TTY.
remote_privileged() {
  local cmd="$1"
  if remote 'test "$(id -u)" -eq 0'; then
    remote "$cmd"
    return
  fi
  if ssh "${SSH_OPTS[@]}" "$TARGET_HOST" "sudo -n true" >/dev/null 2>&1; then
    remote "sudo $cmd"
    return
  fi
  if [[ -t 0 ]]; then
    echo "SSH used your key for ${TARGET_HOST}; sudo needs your password to install" >&2
    ssh -t "${SSH_OPTS[@]}" "$TARGET_HOST" "sudo $cmd"
    return
  fi
  echo "sudo on ${TARGET_HOST} needs a password and this session has no TTY." >&2
  echo "Re-run from a terminal, configure NOPASSWD, or connect as a keyed root user." >&2
  exit 1
}

target_triple() {
  case "$(remote uname -m)" in
    x86_64 | amd64) echo "x86_64-unknown-linux-gnu" ;;
    aarch64 | arm64) echo "aarch64-unknown-linux-gnu" ;;
    *)
      echo "unsupported server architecture: $(remote uname -m)" >&2
      exit 1
      ;;
  esac
}

latest_stable_tag() {
  gh release view --repo "$REPO" --json tagName --jq .tagName
}

pack_checkout() {
  local pack_script pack_log packed_tag remote_triple
  pack_script="${SCRIPT_DIR}/pack.sh"
  if [[ ! -f "$pack_script" ]]; then
    echo "missing ${pack_script}" >&2
    exit 1
  fi
  remote_triple="$(target_triple)"
  case "$(uname -m)" in
    x86_64 | amd64) local_triple="x86_64-unknown-linux-gnu" ;;
    aarch64 | arm64) local_triple="aarch64-unknown-linux-gnu" ;;
    *)
      echo "unsupported local architecture: $(uname -m)" >&2
      exit 1
      ;;
  esac
  if [[ "$local_triple" != "$remote_triple" ]]; then
    echo "pack host is ${local_triple} but ${TARGET_HOST} is ${remote_triple}; pack on a matching Linux machine" >&2
    exit 1
  fi
  pack_log="$(mktemp)"
  if ! bash "$pack_script" ${TAG:+"$TAG"} | tee "$pack_log"; then
    rm -f "$pack_log"
    exit 1
  fi
  ARCHIVE="$(sed -n 's/^packed //p' "$pack_log" | tail -n1)"
  packed_tag="$(sed -n 's/^tag //p' "$pack_log" | tail -n1)"
  rm -f "$pack_log"
  if [[ -z "$ARCHIVE" || ! -f "$ARCHIVE" ]]; then
    echo "pack.sh did not produce an archive" >&2
    exit 1
  fi
  if [[ -z "$TAG" && -n "$packed_tag" ]]; then
    TAG="$packed_tag"
  fi
}

if [[ "$PACK" -eq 1 ]]; then
  if [[ -n "$ARCHIVE" ]]; then
    echo "use --pack or --archive, not both" >&2
    exit 1
  fi
  pack_checkout
fi

if [[ -z "$ARCHIVE" ]]; then
  if ! command -v gh >/dev/null; then
    echo "install GitHub CLI (gh) or pass --archive / --pack" >&2
    exit 1
  fi
  if [[ -z "$TAG" ]]; then
    TAG="$(latest_stable_tag)"
    echo "using latest stable release ${TAG}"
  fi
  TRIPLE="$(target_triple)"
  STAGE="$(mktemp -d)"
  trap 'rm -rf "$STAGE"' EXIT
  echo "downloading rabun-git-${TAG}-${TRIPLE}.tar.gz from ${REPO}"
  gh release download "$TAG" --repo "$REPO" --dir "$STAGE" \
    --pattern "rabun-git-${TAG}-${TRIPLE}.tar.gz*"
  ARCHIVE="$(find "$STAGE" -name "rabun-git-${TAG}-${TRIPLE}.tar.gz" | head -n1)"
  if [[ -z "$ARCHIVE" ]]; then
    echo "release ${TAG} has no ${TRIPLE} tarball" >&2
    exit 1
  fi
fi

REMOTE_DIR="/tmp/rabun-git-push-$$"
remote "mkdir -p $(printf '%q' "$REMOTE_DIR")/deploy"
scp "${SSH_OPTS[@]}" -q "$ARCHIVE" "${TARGET_HOST}:${REMOTE_DIR}/rabun-git.tar.gz"
if [[ -f "${ARCHIVE}.sha256" ]]; then
  scp "${SSH_OPTS[@]}" -q "${ARCHIVE}.sha256" "${TARGET_HOST}:${REMOTE_DIR}/rabun-git.tar.gz.sha256"
fi
scp "${SSH_OPTS[@]}" -q -r "${SCRIPT_DIR}/." "${TARGET_HOST}:${REMOTE_DIR}/deploy/"
REMOTE_ENV=""
if [[ -n "$ENV_FILE" ]]; then
  scp "${SSH_OPTS[@]}" -q "$ENV_FILE" "${TARGET_HOST}:${REMOTE_DIR}/rabun-git.env"
  REMOTE_ENV="${REMOTE_DIR}/rabun-git.env"
fi
remote "chmod +x $(printf '%q' "$REMOTE_DIR")/deploy/bootstrap.sh $(printf '%q' "$REMOTE_DIR")/deploy/install.sh $(printf '%q' "$REMOTE_DIR")/deploy/register-app.sh"

REMOTE_ARCHIVE="${REMOTE_DIR}/rabun-git.tar.gz"
if [[ "$BOOTSTRAP" -eq 1 ]]; then
  echo "bootstrapping ${TARGET_HOST}"
  remote_privileged "env RABUN_GIT_ARCHIVE=$(printf '%q' "$REMOTE_ARCHIVE") RABUN_GIT_ENV_FILE=$(printf '%q' "$REMOTE_ENV") RABUN_GIT_ENABLE_UFW=$(printf '%q' "${RABUN_GIT_ENABLE_UFW:-}") bash $(printf '%q' "$REMOTE_DIR")/deploy/bootstrap.sh $(printf '%q' "${TAG:-}")"
else
  echo "installing ${TAG:-archive} on ${TARGET_HOST}"
  remote_privileged "env RABUN_GIT_ARCHIVE=$(printf '%q' "$REMOTE_ARCHIVE") RABUN_GIT_ENV_FILE=$(printf '%q' "$REMOTE_ENV") bash $(printf '%q' "$REMOTE_DIR")/deploy/install.sh $(printf '%q' "${TAG:-}")"
fi
remote "rm -rf $(printf '%q' "$REMOTE_DIR")"
echo "done"
