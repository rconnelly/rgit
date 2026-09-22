# Rabun Git

[![CI](https://github.com/Burton-Workspaces/rabun-git/actions/workflows/ci.yml/badge.svg)](https://github.com/Burton-Workspaces/rabun-git/actions/workflows/ci.yml)

Self-hosted **git forge CLI**: bare repositories over SSH, CLI merge requests, and a small in-repo YAML workflow runner. No web UI. No GitHub.com. No Burton Postgres — this is the private remote the rest of the stack can push to.

Warehouse trees stay written by [rabun-warehouse](https://github.com/Burton-Workspaces/rabun-warehouse). Point a remote here when you want an origin you own:

```bash
git remote add origin ssh://git@HOST:2222/team/warehouse.git
```

Versions follow [Semantic Versioning](https://semver.org/). `rabun-git --version` reports the crate version baked in at build time.

## Install

```bash
# From a clone (needs a C compiler and git)
cargo install --path . --locked
```

## Setup

```bash
rabun-git init
# optional: cp .env.example .env && edit RABUN_GIT_ROOT
rabun-git user add YOURNAME --admin
rabun-git key add YOURNAME --file ~/.ssh/id_ed25519.pub
rabun-git check
rabun-git serve
```

`init` writes `rabun-git.toml`, `.env.example` (env **names**, not tokens), and an empty forge root.

Clone URL:

```text
ssh://git@HOST:2222/owner/name.git
```

SSH username is `git` (identity is the key) or your forge login. Admin SSH on port 22 is unchanged; git listens on **2222** by default.

## Commands

```bash
rabun-git check                 # root, git, SSH bind, admin key
rabun-git status                # companion heartbeat JSON (no keys)
rabun-git serve [--bind HOST:PORT]
rabun-git user add NAME [--admin]
rabun-git key add USER --file KEY.pub
rabun-git repo create owner/name
rabun-git access grant USER owner/name --role write
rabun-git request create owner/name --head BRANCH --title "..."
rabun-git request review owner/name ID --approve --comment lgtm
rabun-git request merge owner/name ID
rabun-git run list owner/name
```

Management commands also work over the git SSH port:

```bash
ssh -p 2222 git@HOST repo list
ssh -p 2222 git@HOST request list owner/name
```

## Merge requests

Push a branch, then open a request (or push a Gerrit-style ref):

```bash
git push origin HEAD:refs/rabun/requests/new/my-branch
rabun-git request list owner/name
rabun-git request merge owner/name 1   # fast-forward only
```

Requests live in the repo as `refs/rabun/requests/<id>/{head,base,meta}`. Non-admins cannot push `master` / `main` directly.

## YAML workflows

Path: `.rabun/workflows/*.yml` at the pushed commit. Not GitHub Actions (`uses:` is unsupported).

```yaml
name: ci
on:
  push:
    branches: [master]
  tag:
  request:
jobs:
  test:
    steps:
      - run: cargo test --locked
```

Supported: `name`, `on.push.branches`, `on.tag`, `on.request`, `jobs.<id>.steps[].run`, optional `env`, `timeout_minutes` (default 30). The runner checks out the commit into a temp worktree on **this server** and runs `sh -c`. Treat it as a trusted single-tenant runner.

Logs: `$RABUN_GIT_ROOT/runs/<owner>/<name>/<id>/`.

## Env

| Variable | Role |
| --- | --- |
| `RABUN_GIT_ROOT` | Data root (default `data/git`) |
| `RABUN_GIT_SSH_BIND` | SSH listen address (default `0.0.0.0:2222`) |
| `RABUN_GIT_HEALTH_BIND` | Loopback `GET /health` (default `127.0.0.1:8792`; empty/`off` disables) |
| `RABUN_GIT_STATUS_FILE` | Companion heartbeat JSON (default `$RABUN_GIT_ROOT/status.json`) |
| `RABUN_GIT_CONFIG` | Path to `rabun-git.toml` |

## Orchestration

Optional `rabun.toml`:

```toml
[[apps]]
name = "git"
description = "Rabun git forge"
command = "rabun-git"
args = ["serve"]
health_url = "http://127.0.0.1:8792/health"
status_file = "/var/lib/rabun-git/status.json"
```

Example systemd unit: [doc/architecture.md](doc/architecture.md). Secrets (if any) stay in `/etc/rabun-git/rabun-git.env`, not `/etc/rabun/rabun.env`.

## Tests

```bash
cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo doc --locked --no-deps --document-private-items
```

Architecture: [doc/architecture.md](doc/architecture.md).
