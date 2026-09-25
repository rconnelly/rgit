# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.15.0] - 2026-09-25

### Added

- `--json` on management commands for `rgit-web` (tree, blob, blame, log, refs, diffs, merge requests)
- Web passwords and bearer tokens: `rgit auth login`, `user passwd`, `RABUN_GIT_TOKEN`
- Public repositories (`repo create --public`, `repo visibility`) readable by `--anonymous`
- Git browse commands: `repo tree|blob|blame|log|commit|refs|diff` and `request diff`
- expose JSON git browse, web login, and public repos for rgit-web

## [0.14.0] - 2026-09-23

### Added

- **view:** highlight Browse /tree, copyable clone URL, and cleaner commits

## [0.13.0] - 2026-09-23

### Added

- `rgit version` on this machine: SemVer 2.0 bumps, Conventional Commits 1.0.0 checks, Keep a Changelog updates, and a `chore(release):` commit plus annotated tag
- Forge `hooks/update` opt-in policy via `.rabun/version.toml` (`enforce.commits` / `tags` / `manifests`)
- Version files: `Cargo.toml` / `Cargo.lock`, `package.json` / `package-lock.json`, `pyproject.toml`, `composer.json`, `pubspec.yaml`, `Chart.yaml`, `VERSION` / `version.txt`
- add rgit version for SemVer, changelog, and commit policy

### Changed

- present Conventional Commits and SemVer as default etiquette

## [0.12.1] - 2026-09-23

### Fixed

- GitHub Release tags must match `Cargo.toml` (`v0.12.1`). The `v0.12.1` tag on `0.12.0` failed the SemVer check.
- `rgit view` points at the Zola install docs only (no rsites hint).

## [0.12.0] - 2026-09-23

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
- Workflow `runs-on:` fans out to linux / macos / windows; registered `rgit agent` builders poll over SSH and claim queued jobs (`builders.yaml`)
- `rgit view` renders a local git tree as a loopback Zola + DevLab preview (README, file tree, blobs). Not an HTTP UI on `serve`.
- GitHub Actions release workflow: a SemVer 2.0.0 tag (`vMAJOR.MINOR.PATCH`) publishes Linux archives to GitHub Releases

## [0.1.0]

### Added

- Self-hosted git forge CLI: SSH `upload-pack` / `receive-pack`, users, keys, and path ACL
- CLI merge requests stored as `refs/rabun/requests/<id>/{head,base,meta}` (fast-forward merge)
- In-repo YAML workflows (`.rabun/workflows`) on push, tag, and request
- Companion heartbeat (`rabun.companion/v1`): `status.json`, `rabun-git status`, loopback `GET /health`




[Unreleased]: https://github.com/rconnelly/rgit/compare/v0.15.0...HEAD
[0.15.0]: https://github.com/rconnelly/rgit/releases/tag/v0.15.0
[0.14.0]: https://github.com/rconnelly/rgit/releases/tag/v0.14.0
[0.13.0]: https://github.com/rconnelly/rgit/releases/tag/v0.13.0
[0.12.1]: https://github.com/rconnelly/rgit/releases/tag/v0.12.1
[0.12.0]: https://github.com/rconnelly/rgit/releases/tag/v0.12.0
[0.1.0]: https://github.com/rconnelly/rgit/releases/tag/v0.1.0
