# Command reference

Global flag (all commands):

```bash
rabun-git --config /path/to/rabun-git.toml …
# or: export RABUN_GIT_CONFIG=/path/to/rabun-git.toml
```

`rabun-git --version` prints the crate version.

Over SSH, omit the `rabun-git` prefix and use port **2222**:

```bash
ssh -p 2222 git@git.example.com repo list
```

`init`, `check`, `status`, and `serve` work only on the host, not over SSH.

## Host / operator

| Command | What it does |
| --- | --- |
| `rabun-git init` | Write `rabun-git.toml`, `.env.example`, empty forge root |
| `rabun-git check` | Data root writable, `git` on PATH, SSH bind, admin with a key |
| `rabun-git status` | Companion JSON (`rabun.companion/v1`), no keys |
| `rabun-git serve [--bind HOST:PORT]` | Listen for git + management commands |

## Users and keys

| Command | What it does |
| --- | --- |
| `rabun-git user add NAME [--admin]` | Create user or update forge-admin flag |
| `rabun-git user list` | List logins |
| `rabun-git user remove NAME` | Delete user, keys, and all grants |
| `rabun-git key add USER --file KEY.pub` | Append OpenSSH public keys |
| `rabun-git key list USER` | Fingerprints only |

## Repositories and ACL

| Command | What it does |
| --- | --- |
| `rabun-git repo create owner/name` | Create a bare repo; creator gets repo admin |
| `rabun-git repo list` | Repos the caller can read |
| `rabun-git repo show owner/name` | Path and grants |
| `rabun-git access grant USER owner/name [--role read\|write\|admin]` | Set role (`write` if omitted) |
| `rabun-git access revoke USER owner/name` | Remove that user’s grant |

## Merge requests and CI

| Command | What it does |
| --- | --- |
| `rabun-git request create owner/name --head BRANCH --title "…" [--base BRANCH] [--body "…"]` | Open a request |
| `rabun-git request list owner/name` | List requests |
| `rabun-git request show owner/name ID` | One request |
| `rabun-git request review owner/name ID [--approve\|--reject] [--comment TEXT]` | Review |
| `rabun-git request merge owner/name ID` | Fast-forward the base branch |
| `rabun-git run list owner/name` | CI runs |
| `rabun-git run show owner/name ID` | Status YAML |
| `rabun-git run logs owner/name ID` | Captured log |

## Git URLs and env

Clone / push:

```text
ssh://git@HOST:2222/owner/name.git
```

| Variable | Role |
| --- | --- |
| `RABUN_GIT_ROOT` | Data root (default `data/git`) |
| `RABUN_GIT_SSH_BIND` | SSH listen address (default `0.0.0.0:2222`) |
| `RABUN_GIT_HEALTH_BIND` | Loopback `GET /health` (default `127.0.0.1:8792`; empty/`off` disables) |
| `RABUN_GIT_STATUS_FILE` | Companion JSON (default `$RABUN_GIT_ROOT/status.json`) |
| `RABUN_GIT_CONFIG` | Path to `rabun-git.toml` |

Special push to open a request:

```bash
git push origin HEAD:refs/rabun/requests/new/my-branch
```

## Further reading

- [Architecture and systemd](architecture.md)
- [User guide index](README.md)
