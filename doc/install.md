# Install

Install the `rabun-git` command on the **server** (the machine that will host repositories). You can also install it on a laptop if you want the same CLI locally; laptops only need `git` and SSH to clone and push.

## What you need

- [Git](https://git-scm.com/) on `PATH` (`git --version`)
- A [Rust](https://rustup.rs/) toolchain with Cargo (edition 2021, Rust 1.85 or newer)
- A C compiler (the SSH stack links native code)
- An SSH **public** key on each laptop that will connect (`~/.ssh/id_ed25519.pub` is typical)

On Debian/Ubuntu you can install the C compiler and git with the repo script:

```bash
./scripts/install-linux-build-deps.sh
```

## Build from a clone

```bash
git clone https://github.com/Burton-Workspaces/rabun-git.git
cd rabun-git
cargo install --path . --locked
```

That puts `rabun-git` in `~/.cargo/bin`. Confirm:

```bash
rabun-git --version
```

If the command is not found, add Cargo’s bin directory to your `PATH`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## Check that git works

```bash
git --version
```

Rabun Git shells out to the system `git` binary for every clone, push, and merge. If `git` is missing, `rabun-git check` will fail later.

Next: [Start the forge](start-the-forge.md).
