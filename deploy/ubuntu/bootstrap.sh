#!/usr/bin/env bash
# First-time Ubuntu host setup for rabun-git.
# Must run from a copy of deploy/ubuntu on the server (scp or git clone).
# Does not fetch the private GitHub repo. Safe on a host that already runs Rabun:
# it writes a systemd unit beside rabun.service and upserts [[apps]] in rabun.toml.
#
#   sudo RABUN_GIT_ARCHIVE=/path/to/rabun-git-<tag>-<target>.tar.gz ./bootstrap.sh
#   sudo RABUN_GIT_ENV_FILE=/path/to/.env ./bootstrap.sh
# Tag is optional; pass the SemVer from the archive when you have it.
set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
  echo "run as root" >&2
  exit 1
fi

TAG="${1:-}"
ARCHIVE_PATH="${RABUN_GIT_ARCHIVE:-}"
ENV_FILE="${RABUN_GIT_ENV_FILE:-}"
ENABLE_UFW="${RABUN_GIT_ENABLE_UFW:-}"

SCRIPT_DIR=""
if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" ]]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi
if [[ -z "$SCRIPT_DIR" ]]; then
  echo "bootstrap must be run from a file path (do not pipe from curl on a private repo)." >&2
  echo "From a machine that can see GitHub: ./deploy/ubuntu/push.sh --bootstrap user@HOST TAG" >&2
  exit 1
fi

need() {
  local name="$1"
  if [[ ! -f "${SCRIPT_DIR}/${name}" ]]; then
    echo "missing ${SCRIPT_DIR}/${name}" >&2
    exit 1
  fi
}

need rabun-git.toml
need rabun-git.env.example
need rabun-git.service
need install.sh
need register-app.sh

export DEBIAN_FRONTEND=noninteractive
# A shared droplet already has extra apt sources. If one is unhealthy, continue.
apt-get update || echo "warning: apt-get update failed (often a third-party repo); continuing from existing package lists" >&2
apt-get install -y --no-install-recommends ca-certificates curl tar git

id -u rabun-git >/dev/null 2>&1 || useradd --system --home /var/lib/rabun-git --shell /usr/sbin/nologin rabun-git
install -d -m 0755 -o rabun-git -g rabun-git /var/lib/rabun-git
install -d -m 0750 /etc/rabun-git

if [[ ! -f /etc/rabun-git/rabun-git.toml ]]; then
  cp "${SCRIPT_DIR}/rabun-git.toml" /etc/rabun-git/rabun-git.toml
  chmod 0644 /etc/rabun-git/rabun-git.toml
fi
if [[ -n "$ENV_FILE" && -f "$ENV_FILE" ]]; then
  install -m 0640 "$ENV_FILE" /etc/rabun-git/rabun-git.env
  chown root:rabun-git /etc/rabun-git/rabun-git.env
  echo "wrote /etc/rabun-git/rabun-git.env from $(basename "$ENV_FILE")"
elif [[ ! -f /etc/rabun-git/rabun-git.env ]]; then
  cp "${SCRIPT_DIR}/rabun-git.env.example" /etc/rabun-git/rabun-git.env
  chmod 0640 /etc/rabun-git/rabun-git.env
  chown root:rabun-git /etc/rabun-git/rabun-git.env
fi

cp "${SCRIPT_DIR}/rabun-git.service" /etc/systemd/system/rabun-git.service
chmod 0644 /etc/systemd/system/rabun-git.service

bash "${SCRIPT_DIR}/register-app.sh"

systemctl daemon-reload
systemctl enable rabun-git.service

if command -v ufw >/dev/null 2>&1; then
  if ufw status 2>/dev/null | grep -q "Status: active"; then
    ufw allow 2222/tcp comment "rabun-git SSH" || true
    echo "allowed TCP 2222 in ufw (git SSH; port 22 admin SSH is unchanged)"
  elif [[ "$ENABLE_UFW" == "1" || "$ENABLE_UFW" == "true" ]]; then
    ufw allow OpenSSH || ufw allow 22/tcp
    ufw allow 2222/tcp comment "rabun-git SSH"
    ufw --force enable
    echo "enabled ufw with OpenSSH and TCP 2222"
  fi
fi

if [[ -z "$ARCHIVE_PATH" ]]; then
  echo "bootstrap finished without a binary; copy a release tarball and run:" >&2
  echo "  sudo RABUN_GIT_ARCHIVE=/path/to/rabun-git-<tag>-<target>.tar.gz ${SCRIPT_DIR}/install.sh ${TAG:-vX.Y.Z}" >&2
  exit 0
fi

RABUN_GIT_ARCHIVE="$ARCHIVE_PATH" RABUN_GIT_ENV_FILE="$ENV_FILE" bash "${SCRIPT_DIR}/install.sh" ${TAG:+"$TAG"}

echo
echo "bootstrap complete."
echo "  binary: /usr/local/bin/rabun-git --version"
echo "  unit: systemctl status rabun-git"
echo "  env file: /etc/rabun-git/rabun-git.env"
echo "  forge root: /var/lib/rabun-git"
echo "  git SSH: 0.0.0.0:2222 (open TCP 2222 on the firewall)"
echo "  admin user: sudo -u rabun-git rabun-git user add NAME --admin"
echo "  admin key:  sudo -u rabun-git rabun-git key add NAME --file KEY.pub"
if [[ -f /etc/rabun/rabun.toml ]]; then
  echo "  rabun apps: /etc/rabun/rabun.toml ([[apps]] name = \"git\")"
fi
