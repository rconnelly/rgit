# rabun-git architecture

How the forge is put together (layout, ACL, systemd). For a beginner walkthrough with examples, start at the [user guide](README.md).

Self-hosted git forge CLI. Burton and Rabun do not import this crate. Warehouse workers remain the writers of warehouse trees; this process is a **git remote** they may push to.

```
This machine                     Your server
  git clone/push  --SSH:2222-->  rabun-git serve (russh)
  ssh git@host request …  ---->  same binary, management commands
                                 bare repos under RABUN_GIT_ROOT/repos/
                                 users.yaml, keys/, access.yaml
                                 refs/rabun/requests/*
                                 .rabun/workflows runner
```

SSH is the only public network surface (default `0.0.0.0:2222`; russh username `git`). Loopback `GET /health` is companion heartbeat only (`127.0.0.1:8792`). There is no HTTP git UI on `serve`. On this machine, `rgit view` can render a local tree with Zola on loopback. The systemd user is `rabun-git`; admin SSH on port 22 is unchanged.

## Layout

`$RABUN_GIT_ROOT/` (default `data/git`):

- `users.yaml` — login + forge admin flag
- `keys/<user>.pub` — OpenSSH public keys
- `access.yaml` — `owner/name` → user → `read` \| `write` \| `admin`
- `repos/<owner>/<name>.git/` — bare repositories
- `runs/<owner>/<name>/<run-id>/` — `status.yaml` + `job.yaml` + `log.txt`
- `builders.yaml` — registered workflow agents and labels
- `ssh_host_ed25519_key` — generated on first `serve`
- `status.json` — `rabun.companion/v1`

## ACL

- Forge admin (`users.yaml`) bypasses per-repo ACL.
- Local CLI (`rabun-git` on the host) is the **operator** and has full access.
- SSH identity is the key. Username `git` maps to the key's owner (GitHub-style).
- `read`: fetch, list requests/runs.
- `write`: push non-protected branches, open/review requests.
- `admin` (repo or forge): push `master`/`main`, merge, grant access.
- `hooks/update` rejects protected-branch updates for non-admins. Env: `RABUN_GIT_USER`, `RABUN_GIT_REPO`, `RABUN_GIT_ROOT`, `RABUN_GIT_BIN`.

## Merge requests

Stored in git so they clone with the repo:

- `refs/rabun/requests/<id>/head`
- `refs/rabun/requests/<id>/base`
- `refs/rabun/requests/<id>/meta` — YAML blob (title, author, state, reviews)

`git push origin HEAD:refs/rabun/requests/new/<branch>` allocates the next id after receive-pack. Merge is fast-forward only in v1.

## Workflows

`.rabun/workflows/*.yml` at the triggering commit. Subset: `on.push.branches`, `on.tag`, `on.request`, `jobs.*.steps[].run`, `jobs.*.runs-on`, `jobs.*.shell`, `env`, `timeout_minutes`. No `uses:`, full matrix, or containers. Empty `runs-on` (or `linux` with no linux builder) runs on the forge host. Other labels are queued in `runs/` until a registered agent claims them over SSH (`builders.yaml`).

## systemd

Pack this checkout and copy it onto Ubuntu over SSH (same flow as Burton and Rabun): [deploy Ubuntu](deploy-ubuntu.md). The unit shipped in `deploy/ubuntu/rabun-git.service` runs as user `rabun-git` with forge data under `/var/lib/rabun-git`. Git clients still use `ssh://git@HOST:2222/…`.

```ini
[Unit]
Description=Rabun git forge
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=rabun-git
Group=rabun-git
WorkingDirectory=/var/lib/rabun-git
Environment=RABUN_GIT_CONFIG=/etc/rabun-git/rabun-git.toml
Environment=RABUN_GIT_ROOT=/var/lib/rabun-git
EnvironmentFile=-/etc/rabun-git/rabun-git.env
ExecStart=/usr/local/bin/rabun-git serve
Restart=on-failure
```

Optional companion `rabun.toml` on the host (`register-app.sh` upserts this on install):

```toml
[[apps]]
name = "git"
description = "Rabun git forge"
command = "rabun-git"
args = ["serve"]
health_url = "http://127.0.0.1:8792/health"
status_file = "/var/lib/rabun-git/status.json"
```

Settings stay in `/etc/rabun-git/rabun-git.env`, not `/etc/rabun/rabun.env`.

## Privacy

No telemetry and no outbound forge calls. Host keys and user keys stay on disk you own. Companion JSON never includes key material.
