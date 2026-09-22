# Rabun Git

[![CI](https://github.com/Burton-Workspaces/rabun-git/actions/workflows/ci.yml/badge.svg)](https://github.com/Burton-Workspaces/rabun-git/actions/workflows/ci.yml)

Self-hosted **git forge CLI**: bare repositories over SSH, CLI merge requests, and a small in-repo YAML workflow runner. No web UI. No GitHub.com.

If you already use `git clone` / `git push`, you can run this on a server you own and point `origin` at it.

```bash
git remote add origin ssh://git@HOST:2222/owner/name.git
```

Versions follow [Semantic Versioning](https://semver.org/). `rabun-git --version` reports the crate version baked in at build time.

## User guide

The guide is written for people who know basic git (commits, branches, remotes) and are new to this tool. Start with the summary in [`doc/README.md`](doc/README.md), or jump to a section:

| Section | What you will do |
| --- | --- |
| [What Rabun Git is](doc/what-it-is.md) | Map GitHub-style hosting onto this CLI |
| [Install](doc/install.md) | Build `rabun-git` with Cargo |
| [Start the forge](doc/start-the-forge.md) | `init`, first admin user, SSH key, `serve` |
| [Set up a remote repository](doc/remote-repository.md) | Create `owner/name`, add `origin`, first push or clone |
| [Users and roles](doc/users-and-roles.md) | Create users, attach keys, grant and revoke `read` / `write` / `admin` |
| [Everyday git](doc/everyday-git.md) | Clone, branch, and protected `main` / `master` |
| [Merge requests](doc/merge-requests.md) | Open, review, and fast-forward merge |
| [CI workflows](doc/ci-workflows.md) | `.rabun/workflows` on push, tag, and request |
| [Command reference](doc/commands.md) | Every CLI and SSH command |

On-disk layout, ACL internals, systemd: [doc/architecture.md](doc/architecture.md).

## Install

```bash
# From a clone (needs a C compiler and git)
cargo install --path . --locked
```

Details: [doc/install.md](doc/install.md).

## Tests

```bash
cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo doc --locked --no-deps --document-private-items
```
