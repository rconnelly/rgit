# Start the forge

Replace `ada` with your login and `git.example.com` with the hostname or IP of the machine that will store repositories.

There are two ways to get the forge listening. Both still need a **first admin user** and that user’s **first SSH public key** on the host.

| | Pack and push (Ubuntu) | Manual |
| --- | --- | --- |
| When | Ubuntu host; you have this checkout on this machine | Any host; you run commands on the server yourself |
| Command | `./deploy/ubuntu/push.sh --pack --bootstrap user@HOST` | `rabun-git init` then `serve` in a directory |
| Details | [deploy Ubuntu](deploy-ubuntu.md) | Rest of this page, Path B |

What each path covers:

| Step | Pack and push | Manual |
| --- | --- | --- |
| Install the `rabun-git` binary | yes (`/usr/local/bin`) | you install first ([install](install.md)) |
| Data directory and config | yes (`/var/lib/rabun-git`, `/etc/rabun-git/`) | you run `init` |
| First admin user | **no** — you add after | you run `user add` |
| First SSH public key | **no** — you add after | you run `key add` |
| `rabun-git check` | optional after the key | you run `check` |
| Start `serve` on port 2222 | yes (systemd, stays up) | you run `serve` in a terminal |
| Open TCP 2222 | yes if `ufw` is already active | you open the firewall |

Layout, systemd, and later deploys: [deploy Ubuntu](deploy-ubuntu.md).

## Path A — pack and push (automatic install)

From this checkout (needs a C compiler and Cargo to pack; SSH to the host is publickey only):

```bash
./deploy/ubuntu/push.sh --pack --bootstrap user@HOST
```

That copies the binary, writes config, creates `/var/lib/rabun-git`, enables `rabun-git.service`, and starts `serve`. It does **not** create a forge user or register a key.

On the **server**, add the first admin and key (files under `/var/lib/rabun-git` must stay owned by `rabun-git`):

```bash
rabun-git shell
rabun-git user add ada --admin
rabun-git key add ada --file /path/to/ada.pub
rabun-git check
exit
```

`rabun-git shell` is one sudo, then bash as the systemd user (prompt `(rabun-git)`). How to get `ada.pub` is under [First admin and SSH key](#first-admin-and-ssh-key) below.

Then [smoke-test SSH](#smoke-test-ssh-from-this-machine).

## Path B — manual (`init` and `serve`)

These steps run **on the server**. Install `rabun-git` first ([install](install.md)).

### 1. Create the data directory and config

Pick a working directory (any folder is fine) and initialize:

```bash
mkdir -p /var/lib/rabun-git
cd /var/lib/rabun-git
rabun-git init
```

`init` writes:

- `rabun-git.toml` — which env var holds the data root, and the SSH listen address
- `.env.example` — env **names** only (copy to `.env` if you want to change paths)
- `data/git/` — empty forge root (`users.yaml`, `access.yaml`, `keys/`, `repos/`, `runs/`)

To keep data somewhere else:

```bash
cp .env.example .env
# edit .env:
#   RABUN_GIT_ROOT=/var/lib/rabun-git/data
```

`RABUN_GIT_ROOT` is the directory that will contain bare repos, keys, and ACL files. Default is `data/git` under the current directory.

### 2. First admin and SSH key

Follow [First admin and SSH key](#first-admin-and-ssh-key). On this path, run `user add` / `key add` in the same directory as `init` (or after `cd` / env so `RABUN_GIT_ROOT` is set). No `rabun-git shell` unless you are on a systemd host.

### 3. Verify the forge

```bash
rabun-git check
```

You should see the data root, SSH bind (`0.0.0.0:2222` by default), your admin name, and `git: ok`. `check` fails if there is no admin or if no admin has a key.

### 4. Start listening

```bash
rabun-git serve
```

Leave this running. Git clone/push and `ssh -p 2222 …` commands talk to this process.

- Default listen address: `0.0.0.0:2222`
- Override once: `rabun-git serve --bind 0.0.0.0:2222`
- Or set `RABUN_GIT_SSH_BIND` in `.env` / systemd

Open **TCP 2222** on the firewall if clients are not on the same machine. Port **22** (normal SSH login) is unrelated.

On first start, the forge writes an SSH host key at `$RABUN_GIT_ROOT/ssh_host_ed25519_key`. This machine will ask you to trust that host key the first time you connect.

## First admin and SSH key

A **forge admin** can create users, grant access anywhere, and push protected branches. The forge never stores private keys.

```bash
rabun-git user add ada --admin
```

Logins use the same character rules as repo segments: letters, digits, `.`, `_`, `-`.

On this machine, print the public key you will connect with:

```bash
cat ~/.ssh/id_ed25519.pub
```

If you do not have a key yet:

```bash
ssh-keygen -t ed25519 -C "ada@git.example.com" -f ~/.ssh/id_ed25519
```

Copy that **`.pub`** line to the server (not the private key). On the server:

```bash
rabun-git key add ada --file /path/to/ada.pub
```

If you are setting this up from the same machine as the forge:

```bash
rabun-git key add ada --file ~/.ssh/id_ed25519.pub
```

The **first** admin key must be added on the host. After that, extra keys and every other forge command can run from this machine:

```bash
rgit remote add origin git@git.example.com
rgit origin key add ada --file ~/.ssh/id_ed25519.pub
rgit origin repo list
```

## Smoke-test SSH from this machine

```bash
ssh -p 2222 git@git.example.com
```

You should get a short greeting (`Hi ada, this is rabun-git.`) and a repo list (empty so far). Identity is the key, not a password. The SSH username may be `git` (GitHub-style) or your forge login (`ada`).

If that fails, see [Troubleshooting](#troubleshooting) below.

## Optional: shorter git URLs

On each client machine, add to `~/.ssh/config`:

```sshconfig
Host git.example.com
  HostName git.example.com
  Port 2222
  User git
  IdentityFile ~/.ssh/id_ed25519
```

Then `ssh git.example.com` and `git clone git.example.com:ada/website.git` use port 2222 automatically. The rest of this guide still writes the full `ssh://git@HOST:2222/…` URL so it works without that file.

## Troubleshooting

| Symptom | What to try |
| --- | --- |
| `rabun-git: command not found` | `export PATH="$HOME/.cargo/bin:$PATH"` |
| `git is not on PATH` | Install git; confirm `git --version` |
| `no admin user` | `rabun-git user add YOURNAME --admin` |
| `admin user(s) have no SSH keys` | `rabun-git key add YOURNAME --file KEY.pub` — must be a **.pub** file |
| SSH `Permission denied (publickey)` | Same private key as the `.pub` you registered; `ssh -p 2222 -i ~/.ssh/id_ed25519 git@HOST` |
| Connection refused | `serve` is running (`rabun-git serve` or `systemctl status rabun-git`); firewall allows 2222; `--bind` matches the address you are using |
| Wrong port | GitHub uses 22; Rabun Git uses **2222** unless you changed it |

Next: [Set up a remote repository](remote-repository.md).
