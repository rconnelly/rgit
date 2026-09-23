# Versioning

Rabun Git can bump versions, write `CHANGELOG.md`, and enforce [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/) plus [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html) on the clone and on the forge.

Policy lives in the repo as `.rabun/version.toml`, next to [CI workflows](ci-workflows.md). A release is one commit that updates every version file and the changelog; the annotated tag name must match those files.

These commands run on **this machine** (like `rgit view`). They are not available over SSH.

## Commands

```bash
rgit version show
rgit version check [RANGE]
rgit version bump [auto|patch|minor|major] [--to X.Y.Z] [--dry-run]
rgit version changelog [--from TAG]
rgit version release [auto|patch|minor|major] [--to X.Y.Z] [--dry-run] [--no-tag]
rgit version hook install
```

| Command | What it does |
| --- | --- |
| `show` | Print the agreed version and the files that hold it |
| `check` | Require Conventional Commits in a range (default: last `v*` tag..`HEAD`) |
| `bump` | Rewrite version files only |
| `changelog` | Preview Keep a Changelog notes from commits |
| `release` | Bump files, rewrite `CHANGELOG.md`, `git add`, `chore(release): X.Y.Z`, annotated tag |
| `hook install` | Write `.git/hooks/commit-msg` so local commits are checked immediately |

`release` does not push and does not publish to crates.io or npm. Push the tag yourself when you are ready.

`auto` (the default for `bump` and `release`) looks at commits since the last version tag:

| Commits | SemVer bump |
| --- | --- |
| `feat:` | MINOR |
| `fix:` or `perf:` | PATCH |
| `!` after the type, or a `BREAKING CHANGE:` footer | MAJOR (`0.x` stays MINOR so you do not jump to 1.0.0 by accident) |
| `chore(release):`, merges, and other types | no bump |

Pass `patch`, `minor`, `major`, or `--to 1.2.3` to skip inference.

## Version files

`rgit version` detects these in the work tree root and keeps them in agreement:

| Kind | Files |
| --- | --- |
| Cargo | `Cargo.toml` (`[package].version` or `[workspace.package].version`), matching stanza in `Cargo.lock` |
| npm | `package.json`, top-level `version` in `package-lock.json` |
| Python | `pyproject.toml` (`[project].version` or `[tool.poetry].version`) |
| PHP | `composer.json` |
| Dart | `pubspec.yaml` |
| Helm | `Chart.yaml` (`version`, not `appVersion`) |
| Generic | `VERSION` or `version.txt` |

If two source files disagree, `show` / `bump` / `release` stop with an error.

## Policy file

Commit this to enable forge-side checks and to name the changelog:

```toml
[version]
tag_prefix = "v"
changelog = "CHANGELOG.md"

[version.enforce]
commits = true
tags = true
manifests = true
```

Without the file, local `show` / `bump` / `release` still work. The forge does **not** reject ordinary commit messages or tags until you push a tree that contains this file with `enforce` flags on.

| Flag | Gate |
| --- | --- |
| `commits` | New commits on a branch push must be Conventional Commits (merges and `Revert ` / `fixup!` are skipped) |
| `tags` | Tag names must be `{tag_prefix}` plus SemVer 2.0 (`v1.2.3`) |
| `manifests` | The tagged commit's version files must equal that SemVer (no `v` in the files) |

`git commit --no-verify` skips the local hook. It cannot skip the forge `hooks/update` check.

## Local hook vs forge gate

```bash
rgit version hook install
```

writes `.git/hooks/commit-msg` so `git commit` fails fast on this machine.

On the forge, the existing `hooks/update` script already calls `rgit hook update`. After you upgrade the forge binary, any repo that has committed `.rabun/version.toml` is gated on the next push. You do not re-install server hooks.

## Changelog

`release` uses [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). It keeps `## [Unreleased]`, inserts `## [X.Y.Z] - YYYY-MM-DD`, maps `feat` → Added, `fix` → Fixed, and similar types to Changed / Removed / Security, and refreshes compare/tag footer links when it can see a GitHub-style repository URL (`origin` or `Cargo.toml` `repository`).

If `CHANGELOG.md` is missing, it is created.

## This project

This repository ships `.rabun/version.toml`. Cut a rabun-git release with:

```bash
rgit version release
git push origin HEAD --tags
```

Next: [Command reference](commands.md), or [Everyday git](everyday-git.md).
