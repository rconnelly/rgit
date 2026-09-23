# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Beginner user guide under `doc/` (summary, remotes, users and roles, everyday git, merge requests, CI)
- GitHub gap analysis (`doc/compared-to-github.md`): feature tables and `gh` → `rgit` command map
- Pack-and-push from a checkout (`deploy/ubuntu/pack.sh`, `push.sh --pack`) for Ubuntu hosts, including `/etc/rabun-git/rabun-git.env` (`--env`) and Rabun `[[apps]]` housekeeping
- `push.sh` uses SSH publickey only (`--identity` optional) and skips sudo when the remote user is already root or has passwordless sudo
- `pack.sh` refuses a root-owned `dist/release` (or packs under `/tmp`) instead of failing mid-tarball after `sudo pack`
- `rabun-git shell` opens an operator bash as the systemd user so `user` / `repo` / `key` do not need `sudo -u` on each command (prompt `(rabun-git)`)
- Local client: `rgit remote add origin git@HOST` then `rgit origin …` for forge commands (`key add --file` reads a path on this machine; `repo list --user` lists that user’s remotes)
- `rgit origin key copy` registers a public key over host SSH (port 22) so the first forge key does not need `rabun-git shell` on the server (`--admin` creates the user; `--host` / `remote add --host` override `$USER@<forge-host>:22`)
- `rgit` is the official short command: a symlink beside `rabun-git` (Ubuntu install and `scripts/link-rgit.sh`); help text follows argv0; a foreign `rgit` is not overwritten
- `./scripts/install.sh` installs from this checkout (`cargo install --path . --locked`) and links `rgit`

## [0.1.0]

### Added

- Self-hosted git forge CLI: SSH `upload-pack` / `receive-pack`, users, keys, and path ACL
- CLI merge requests stored as `refs/rabun/requests/<id>/{head,base,meta}` (fast-forward merge)
- In-repo YAML workflows (`.rabun/workflows`) on push, tag, and request
- Companion heartbeat (`rabun.companion/v1`): `status.json`, `rabun-git status`, loopback `GET /health`
