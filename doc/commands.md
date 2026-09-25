# Command reference

`rgit` and `rabun-git` are the same program. Examples below use `rgit`. Paths, env (`RABUN_GIT_*`), and systemd stay `rabun-git`.

Global flags (all commands):

```bash
rgit --config /path/to/rabun-git.toml …
# or: export RABUN_GIT_CONFIG=/path/to/rabun-git.toml
rgit --identity ~/.ssh/id_ed25519 origin repo list
# or: export RABUN_GIT_SSH_IDENTITY=~/.ssh/id_ed25519
```

`rgit --version` prints the crate version (SemVer 2.0.0 from `Cargo.toml`). Git tags are `v` plus that version.

On this machine, save a forge host once, then use that name as the first word (this is a **forge** alias, not a git remote):

```bash
rgit remote add origin git@git.example.com
rgit origin key copy ada --admin
rgit origin repo list
rgit origin key add ada --file ~/.ssh/id_ed25519.pub
```

Names live in `~/.config/rabun-git/remotes.toml` (`RABUN_GIT_REMOTES` overrides the path). They cannot collide with clap commands (`repo`, `user`, `key`, …).

Over SSH, omit the `rabun-git` prefix and use port **2222**:

```bash
ssh -p 2222 git@git.example.com repo list
```

`init`, `check`, `status`, `view`, `version`, `serve`, `shell`, `remote`, `key copy`, and `rgit agent --labels` (the poll loop) work only on the machine that runs them. `key copy` uses host SSH on port 22 (not git port 2222). The others work over SSH or `rgit origin …`.

On a systemd host (`/etc/rabun-git/rabun-git.env`), mutating commands must run as the `rabun-git` user:

```bash
rabun-git shell
rabun-git user add ada --admin
rabun-git key add ada --file /home/ada/.ssh/id_ed25519.pub
rabun-git repo create ada/website
exit
```

To register the first admin key from this machine (host SSH + sudo, not port 2222):

```bash
rgit origin key copy ada --admin --file ~/.ssh/id_ed25519.pub
```

After that, `rgit origin repo create ada/website` (or `ssh -p 2222 git@HOST …`) needs no sudo.

## Host / operator

| Command | What it does |
| --- | --- |
| `rabun-git init` | Write `rabun-git.toml`, `.env.example`, empty forge root |
| `rabun-git check` | Data root writable, `git` on PATH, SSH bind, admin with a key |
| `rabun-git status` | Companion JSON (`rabun.companion/v1`), no keys |
| `rabun-git serve [--bind HOST:PORT]` | Listen for git + management commands |
| `rabun-git view [PATH\|owner/name] [--ref REF] [--bind 127.0.0.1:1111] [--open]` | Loopback Zola preview of a local git tree (needs `zola` 0.23.4+) |
| `rabun-git version show` | Agreed SemVer and version files in this work tree |
| `rabun-git version check [RANGE]` | Conventional Commits in a range (default: last version tag..HEAD) |
| `rabun-git version bump [auto\|patch\|minor\|major] [--to X.Y.Z] [--dry-run]` | Rewrite version files only |
| `rabun-git version changelog [--from TAG]` | Preview Keep a Changelog notes from commits |
| `rabun-git version release […] [--dry-run] [--no-tag]` | Bump files, CHANGELOG.md, commit, annotated tag |
| `rabun-git version hook install` | Local `commit-msg` hook → `rgit hook commit-msg` |
| `rabun-git shell` | One sudo, then bash as the systemd user (prompt `(rabun-git)`; `exit` to leave) |

## Named remotes (this machine)

| Command | What it does |
| --- | --- |
| `rabun-git remote add NAME URL [--identity FILE] [--host user@HOST]` | Save a forge host (`origin` is the usual name) |
| `rabun-git remote list` | List saved names and URLs |
| `rabun-git remote show NAME` | URL, optional identity, and host SSH for `key copy` |
| `rabun-git remote remove NAME` | Delete a saved name |
| `rabun-git NAME …` | Run a forge command on that host |

URL forms: `HOST`, `user@HOST`, `user@HOST:port`, `ssh://user@HOST:port`. Default SSH user `git`, default port `2222`.

## Users and keys

| Command | What it does |
| --- | --- |
| `rabun-git user add NAME [--admin] [--password …]` | Create user or update forge-admin flag |
| `rabun-git user list` | List logins |
| `rabun-git user remove NAME` | Delete user, keys, grants, and web tokens |
| `rabun-git user passwd NAME --password …` | Set a web password (SSH still uses keys) |
| `rabun-git auth login --user NAME --password …` | Issue a bearer token for `rgit-web` |
| `rabun-git auth whoami` | Current actor (`--token` or operator) |
| `rabun-git auth logout` | Revoke `--token` |
| `rabun-git auth token create [USER]` | Issue a token without a password |
| `rabun-git auth token list [--user NAME]` | List token prefixes |
| `rabun-git auth token revoke TOKEN` | Revoke by secret or prefix |
| `rabun-git key add USER --file KEY.pub` | Append OpenSSH public keys from a local file |
| `rabun-git key add USER --literal 'ssh-ed25519 AAAA…'` | Append a key given on the command line |
| `rabun-git key list USER` | Fingerprints only |
| `rabun-git NAME key copy [USER] [--file KEY.pub] [--admin] [--host user@HOST]` | Copy a public key to the forge over host SSH (port 22); USER defaults to this machine’s username |

## Repositories and ACL

| Command | What it does |
| --- | --- |
| `rabun-git repo create owner/name [--public]` | Create a bare repo; creator gets repo admin |
| `rabun-git repo list` | Repos the caller can read |
| `rabun-git repo list --user NAME` | Repos that user can access (self or forge admin) |
| `rabun-git repo show owner/name` | Path, clone URL, visibility, grants |
| `rabun-git repo tree owner/name [--ref HEAD] [--path DIR]` | Directory listing |
| `rabun-git repo blob owner/name --path FILE [--ref HEAD]` | File contents |
| `rabun-git repo blame owner/name --path FILE [--ref HEAD]` | Line blame |
| `rabun-git repo log owner/name [--ref HEAD] [--path FILE] [--limit N]` | Commit history |
| `rabun-git repo commit owner/name SHA` | One commit |
| `rabun-git repo refs owner/name` | Branches and tags |
| `rabun-git repo diff owner/name --base A --head B` | Unified diff |
| `rabun-git repo visibility owner/name --public\|--private` | Public browse vs ACL-only |
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
| `rabun-git request diff owner/name ID` | Unified diff and commits on the request |
| `rabun-git run list owner/name` | CI runs |
| `rabun-git run show owner/name ID` | Status YAML |
| `rabun-git run logs owner/name ID` | Captured log |
| `rabun-git agent --labels LABEL [--remote origin]` | Poll loop on this machine |
| `rabun-git agent register NAME --label LABEL [--file KEY.pub]` | Register a builder (admin) |
| `rabun-git agent list` | List builders |
| `rabun-git agent next [--label LABEL]` | Claim the oldest matching queued job |
| `rabun-git agent log RUN_ID [--literal TEXT]` | Append to a run log |
| `rabun-git agent finish RUN_ID --status passed\|failed` | Mark a claimed run done |

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
| `RABUN_GIT_REMOTES` | Named remotes file on this machine (default `~/.config/rabun-git/remotes.toml`) |
| `RABUN_GIT_SSH_IDENTITY` | Private key for `rabun-git origin …` |
| `RABUN_GIT_JSON` | Same as `--json` (machine-readable stdout for `rgit-web`) |
| `RABUN_GIT_TOKEN` | Bearer token (`rgit_…`) for web identity |
| `RABUN_GIT_PASSWORD` | Web password for `auth login` / `user passwd` |
| `RABUN_GIT_PUBLIC_HOST` | Host name used in clone URLs |

Special push to open a request:

```bash
git push origin HEAD:refs/rabun/requests/new/my-branch
```

## Further reading

- [Versioning](versioning.md)
- [Architecture and systemd](architecture.md)
- [Ubuntu pack/push](deploy-ubuntu.md)
- [User guide index](README.md)
