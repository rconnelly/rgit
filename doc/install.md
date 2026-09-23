# Install

Install the `rabun-git` command on the **server** (the machine that will host repositories). You can also install it on this machine if you want the same CLI locally; clone and push only need `git` and SSH.

`rgit` and `rabun-git` are the same program. `rgit` is the short command (a symlink). Paths, env (`RABUN_GIT_*`), and systemd stay `rabun-git`. If another `rgit` is already on `PATH` (some recursive-git wrappers use that name), the linker leaves it alone.

## What you need

- [Git](https://git-scm.com/) on `PATH` (`git --version`)
- A [Rust](https://rustup.rs/) toolchain with Cargo (edition 2021, Rust 1.85 or newer)
- A C compiler (the SSH stack links native code)
- An SSH **public** key on each machine that will connect (`~/.ssh/id_ed25519.pub` is typical)

On Debian/Ubuntu you can install the C compiler and git with the repo script:

```bash
./scripts/install-linux-build-deps.sh
```

## Build from a clone

```bash
git clone https://github.com/rconnelly/rgit.git
cd rabun-git
./scripts/install.sh
```

That runs `cargo install --path . --locked` and `./scripts/link-rgit.sh`. It puts `rabun-git` in `~/.cargo/bin` and a `rgit` symlink beside it. Confirm either name:

```bash
rgit --version
rabun-git --version
```

If the command is not found, add Cargo’s bin directory to your `PATH`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## Production Ubuntu

Pack this checkout and copy it onto a server over SSH (the host never talks to GitHub). Same flow as Burton and Rabun:

```bash
./deploy/ubuntu/push.sh --pack --bootstrap user@HOST
./deploy/ubuntu/push.sh --pack user@HOST
```

Layout, systemd, and first admin user: [deploy Ubuntu](deploy-ubuntu.md).

## Check that git works

```bash
git --version
```

Rabun Git shells out to the system `git` binary for every clone, push, and merge. If `git` is missing, `rabun-git check` will fail later.

Next: [Start the forge](start-the-forge.md).
