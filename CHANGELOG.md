# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Beginner user guide under `doc/` (summary, remotes, users and roles, everyday git, merge requests, CI)
- Laptop pack-and-push from a checkout (`deploy/ubuntu/pack.sh`, `push.sh --pack`) for Ubuntu hosts, including `/etc/rabun-git/rabun-git.env` (`--env`) and Rabun `[[apps]]` housekeeping

## [0.1.0]

### Added

- Self-hosted git forge CLI: SSH `upload-pack` / `receive-pack`, users, keys, and path ACL
- CLI merge requests stored as `refs/rabun/requests/<id>/{head,base,meta}` (fast-forward merge)
- In-repo YAML workflows (`.rabun/workflows`) on push, tag, and request
- Companion heartbeat (`rabun.companion/v1`): `status.json`, `rabun-git status`, loopback `GET /health`
