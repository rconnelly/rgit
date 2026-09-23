# Start the forge

These steps run **on the server** that will store repositories. Replace `ada` with your login and `git.example.com` with that machine’s hostname or IP.

On Ubuntu you can pack this checkout and install a systemd unit instead of running `serve` in a terminal: [deploy Ubuntu](deploy-ubuntu.md) (`./deploy/ubuntu/push.sh --pack --bootstrap user@HOST`). The rest of this page is the manual path (`init` in a working directory).

## 1. Create the data directory and config

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

## 2. Create the first admin user

A **forge admin** can create users, grant access anywhere, and push protected branches.

```bash
rabun-git user add ada --admin
```

Logins use the same character rules as repo segments: letters, digits, `.`, `_`, `-`.

## 3. Attach an SSH public key

The forge never stores private keys. Copy the **public** key from the machine you will connect with:

```bash
# on this machine, print the public key:
cat ~/.ssh/id_ed25519.pub
```

On the server:

```bash
rabun-git key add ada --file /path/to/ada.pub
```

If you are setting this up from the same machine:

```bash
rabun-git key add ada --file ~/.ssh/id_ed25519.pub
```

If you do not have a key yet (on this machine):

```bash
ssh-keygen -t ed25519 -C "ada@git.example.com" -f ~/.ssh/id_ed25519
```

Then copy `id_ed25519.pub` to the server and run `key add`. The **first** admin key must be added on the host. After that, extra keys and every other forge command can run from this machine:

```bash
rabun-git remote add origin git@git.example.com
rabun-git origin key add ada --file ~/.ssh/id_ed25519.pub
rabun-git origin repo list
```

## 4. Verify the forge

```bash
rabun-git check
```

You should see the data root, SSH bind (`0.0.0.0:2222` by default), your admin name, and `git: ok`. `check` fails if there is no admin or if no admin has a key.

## 5. Start listening

```bash
rabun-git serve
```

Leave this running. Git clone/push and `ssh -p 2222 …` commands talk to this process.

- Default listen address: `0.0.0.0:2222`
- Override once: `rabun-git serve --bind 0.0.0.0:2222`
- Or set `RABUN_GIT_SSH_BIND` in `.env` / systemd

Open **TCP 2222** on the firewall if clients are not on the same machine. Port **22** (normal SSH login) is unrelated.

On first start, the forge writes an SSH host key at `$RABUN_GIT_ROOT/ssh_host_ed25519_key`. This machine will ask you to trust that host key the first time you connect.

For a systemd unit and pack/push, see [deploy Ubuntu](deploy-ubuntu.md) and [architecture.md](architecture.md#systemd). On that install, run operator commands inside `rabun-git shell` (one sudo, prompt `(rabun-git)`, then `rabun-git user add …` with no prefix).

## 6. Smoke-test SSH from this machine

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
| Connection refused | `rabun-git serve` is running; firewall allows 2222; `--bind` matches the address you are using |
| Wrong port | GitHub uses 22; Rabun Git uses **2222** unless you changed it |

Next: [Set up a remote repository](remote-repository.md).
