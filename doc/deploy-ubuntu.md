# Ubuntu deploy

`rabun-git` is a systemd unit plus a release archive on an **x86_64 (or aarch64) Ubuntu** host. The same machine can already run Rabun: this forge sits beside `rabun.service`, keeps settings in its own env file, and upserts `[[apps]]` in `/etc/rabun/rabun.toml`. The host never talks to GitHub. A laptop (or Actions) packs or downloads the archive and copies it over SSH.

There is no public HTTP git UI and no Caddy virtual host. `serve` listens for git and management commands on **TCP 2222** and writes a loopback companion heartbeat.

## 1. Host

Ubuntu 24.04 LTS. 1 vCPU / 1 GB is enough for hosting repositories; give the box more RAM if `.rabun/workflows` will compile code.

- SSH keys on the host (port 22) so `push.sh` can copy the archive
- **git** at runtime (bootstrap installs it). The host does **not** need `pkg-config` or a compiler unless you run CI workflows that need them
- Firewall: laptops clone on **2222**, not 22

Do **not** `curl | bash` the bootstrap script from GitHub.

## 2. Bootstrap (once)

From a clone on your laptop (`gh auth login` if you have not already). Pack this checkout and install in one step:

```bash
./deploy/ubuntu/push.sh --pack --bootstrap user@HOST
```

`--env` is optional. If you pass it, the file is copied to **`/etc/rabun-git/rabun-git.env`** (0640). That is the correct place for `RABUN_GIT_ROOT` and bind overrides — not `/etc/rabun/rabun.env`.

First-time from a GitHub Release (latest stable if you omit the tag):

```bash
./deploy/ubuntu/push.sh --bootstrap user@HOST
```

`--pack` needs **pkg-config**, a C compiler, git, and Cargo (`./scripts/install-linux-build-deps.sh`). It names the archive with `git describe --tags --always --dirty` unless you pass a tag. Combine `--pack` with `--bootstrap` for a first-time host from this checkout.

After bootstrap, add a forge admin (files under `/var/lib/rabun-git` must stay owned by `rabun-git`):

```bash
sudo -u rabun-git rabun-git user add ada --admin
sudo -u rabun-git rabun-git key add ada --file /path/to/ada.pub
sudo -u rabun-git rabun-git check
```

`rabun-git` loads `/etc/rabun-git/rabun-git.env`, so you do not need to pass `--config` when that file sets `RABUN_GIT_CONFIG`.

Open **TCP 2222** if laptops are not on the same machine. If `ufw` is already active, bootstrap allows 2222. To enable ufw from scratch: `RABUN_GIT_ENABLE_UFW=1 ./deploy/ubuntu/push.sh --pack --bootstrap user@HOST`.

## 3. Later deploys

After bootstrap, pack current `master` (or a dirty tree) and `scp` it:

```bash
./deploy/ubuntu/push.sh --pack user@HOST
```

Two-step (inspect the archive first):

```bash
./deploy/ubuntu/pack.sh
# dist/release/rabun-git-<git-describe>-<triple>.tar.gz
./deploy/ubuntu/push.sh --archive dist/release/rabun-git-<git-describe>-x86_64-unknown-linux-gnu.tar.gz user@HOST
```

GitHub Release deploys are unchanged: omit `--pack` and `push.sh` downloads the latest stable tarball (or a tag you pass). `--pack` and `--archive` cannot be used together.

Refresh the env file without rewriting the example:

```bash
./deploy/ubuntu/push.sh --pack --env .env user@HOST
```

## Layout on the host

| Path | Role |
| --- | --- |
| `/usr/local/bin/rabun-git` | released binary |
| `/etc/rabun-git/rabun-git.env` | `RABUN_GIT_ROOT`, optional bind overrides |
| `/etc/rabun-git/rabun-git.toml` | env var names (not tokens) |
| `/var/lib/rabun-git` | forge root (`users.yaml`, `keys/`, `repos/`, `runs/`) |
| `/var/lib/rabun-git/status.json` | Companion heartbeat (`rabun.companion/v1`) |
| `/etc/systemd/system/rabun-git.service` | `serve` (git SSH `0.0.0.0:2222`, loopback `GET /health` on `127.0.0.1:8792`) |
| `/etc/rabun/rabun.toml` | Rabun app manifest (`[[apps]] name = "git"`) |

The systemd user is `rabun-git`. Git clients still connect as `git@HOST` on port **2222** (russh; not the unix user). Admin SSH on port 22 is unchanged.

Logs: `journalctl -u rabun-git -f`. Heartbeat: `rabun-git status` or `curl -sS http://127.0.0.1:8792/health`. If `rabun-feeds` already uses `8792` on the same host, set `RABUN_GIT_HEALTH_BIND` to another loopback port. Audit sandboxing with `systemd-analyze security rabun-git`. There is still no public HTTP git UI.

Clone URL after DNS points at the host:

```text
ssh://git@HOST:2222/owner/name.git
```

## Housekeeping

Every install:

- Refreshes `rabun-git.service` and restarts the unit
- Upserts Rabun’s app manifest (`name = "git"`, `command = "rabun-git"`, `args = ["serve"]`, `health_url`, `status_file`)
- Leaves warehouse, feeds, and other `[[apps]]` blocks alone
- Leaves `/etc/rabun/rabun.toml` alone when Rabun is not installed
- Does **not** overwrite an existing env file unless you pass `--env`

`rabun list` / `rabun run git` read that manifest after Rabun restarts. This forge still runs on its own unit; do not also `rabun run git` on the same host unless you stop the systemd service.
