#!/usr/bin/env bash
# Upsert the git [[apps]] table in Rabun's app manifest.
# Default path is /etc/rabun/rabun.toml (systemd). Missing file is a no-op:
# this forge still runs as rabun-git.service.
#
#   sudo ./register-app.sh
#   ./register-app.sh /path/to/rabun.toml
set -euo pipefail

TOML="${1:-${RABUN_GIT_RABUN_TOML:-/etc/rabun/rabun.toml}}"

if [[ ! -f "$TOML" ]]; then
  echo "rabun.toml not found at ${TOML}; skip app manifest (install rabun, or pass a path)" >&2
  exit 0
fi

if ! command -v python3 >/dev/null; then
  echo "python3 is required to update ${TOML}" >&2
  exit 1
fi

python3 - "$TOML" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")

canonical = (
    "[[apps]]\n"
    'name = "git"\n'
    'description = "Rabun git forge"\n'
    'command = "rabun-git"\n'
    'args = ["serve"]\n'
    'health_url = "http://127.0.0.1:8792/health"\n'
    'status_file = "/var/lib/rabun-git/status.json"\n'
)

apps_re = re.compile(
    r"^\[\[apps\]\][^\n]*(?:\n(?!\[)[^\n]*)*",
    re.MULTILINE,
)


def field(block: str, key: str):
    match = re.search(rf'(?m)^\s*{re.escape(key)}\s*=\s*"([^"]*)"', block)
    return match.group(1) if match else None


def is_git(block: str) -> bool:
    return field(block, "name") == "git" or field(block, "command") == "rabun-git"


matches = list(apps_re.finditer(text))
if not matches:
    body = text if text.endswith("\n") or not text else text + "\n"
    if body.strip() and not body.endswith("\n\n"):
        body += "\n"
    updated = body + canonical
else:
    parts = []
    last = 0
    wrote = False
    for match in matches:
        parts.append(text[last : match.start()])
        block = match.group(0)
        if is_git(block):
            if not wrote:
                replacement = canonical.rstrip("\n")
                if block.endswith("\n"):
                    replacement += "\n"
                parts.append(replacement)
                wrote = True
        else:
            parts.append(block)
        last = match.end()
    parts.append(text[last:])
    updated = "".join(parts)
    if not wrote:
        if updated and not updated.endswith("\n"):
            updated += "\n"
        if updated.strip() and not updated.endswith("\n\n"):
            updated += "\n"
        updated += canonical

if updated == text:
    print(f"git already registered in {path}")
    raise SystemExit(0)

path.write_text(updated, encoding="utf-8")
print(f"updated {path} with [[apps]] name = \"git\"")
PY
