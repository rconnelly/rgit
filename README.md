# Rabun Git

[![CI](https://github.com/rconnelly/rgit/actions/workflows/ci.yml/badge.svg)](https://github.com/rconnelly/rgit/actions/workflows/ci.yml)

Self-hosted **git forge CLI**: bare repositories over SSH, CLI merge requests, and a small in-repo YAML workflow runner. No public web UI on the forge host. No GitHub.com.

It's a complete rewrite of MoriGM's rgit. [MoriGM/rgit](https://github.com/MoriGM/rgit)

Browse a local clone (or a bare repo on the host) with `rgit view` — a loopback Zola preview, not HTTP on `serve`.

If you already use `git clone` / `git push`, you can run this on a server you own and point `origin` at it.

```bash
git remote add origin ssh://git@HOST:2222/owner/name.git
```

`rgit` and `rabun-git` are the same program. `rgit` is the short command; crate path, env (`RABUN_GIT_*`), and systemd stay `rabun-git`.

Forge admin from this machine (users, keys, repos — not `git push`):

```bash
rgit remote add origin git@HOST
rgit origin key copy ada --admin
rgit origin repo list
rgit origin key add ada --file ~/.ssh/id_ed25519.pub
```

Versions follow [Semantic Versioning](https://semver.org/). `rgit --version` reports the crate version baked in at build time.

## User guide

The guide is written for people who know basic git (commits, branches, remotes) and are new to this tool. Start with the summary in [`doc/README.md`](doc/README.md), or jump to a section:

| Section | What you will do |
| --- | --- |
| [What is Rabun Git?](doc/what-it-is.md) | Map GitHub-style hosting onto this CLI |
| [Install](doc/install.md) | Build `rabun-git` and link `rgit` |
| [Start the forge](doc/start-the-forge.md) | Pack-and-push or manual `init` / `serve`, then first admin and key |
| [Set up a remote repository](doc/remote-repository.md) | Create `owner/name`, add `origin`, first push or clone |
| [Users and roles](doc/users-and-roles.md) | Create users, attach keys, grant and revoke `read` / `write` / `admin` |
| [Everyday git](doc/everyday-git.md) | Clone, branch, protected `main` / `master`, local `rgit view` |
| [Merge requests](doc/merge-requests.md) | Open, review, and fast-forward merge |
| [CI workflows](doc/ci-workflows.md) | `.rabun/workflows` on push, tag, and request |
| [Command reference](doc/commands.md) | Every CLI and SSH command |
| [Compared to GitHub](doc/compared-to-github.md) | Feature and `gh` command gap analysis |

On-disk layout, ACL internals, systemd: [doc/architecture.md](doc/architecture.md). Ubuntu pack/push: [doc/deploy-ubuntu.md](doc/deploy-ubuntu.md).

## Install

```bash
# From a clone (needs a C compiler and git)
./scripts/install.sh
```

Details: [doc/install.md](doc/install.md).

## Orchestration

Optional `rabun.toml` on the host:

```toml
[[apps]]
name = "git"
description = "Rabun git forge"
command = "rabun-git"
args = ["serve"]
health_url = "http://127.0.0.1:8792/health"
status_file = "/var/lib/rabun-git/status.json"
```

Production is a systemd unit beside `rabun.service`. Settings stay in this forge’s env file, not `/etc/rabun/rabun.env`. Pack this checkout and deploy over SSH (same flow as Burton and Rabun):

```bash
./deploy/ubuntu/push.sh --pack --bootstrap user@HOST
./deploy/ubuntu/push.sh --pack user@HOST
```

`--env` copies a local dotenv file to `/etc/rabun-git/rabun-git.env`. Layout, bootstrap, and Rabun app-manifest updates: [doc/deploy-ubuntu.md](doc/deploy-ubuntu.md).

## Tests

```bash
cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo doc --locked --no-deps --document-private-items
```

Architecture: [doc/architecture.md](doc/architecture.md). Ubuntu pack/push: [doc/deploy-ubuntu.md](doc/deploy-ubuntu.md).
