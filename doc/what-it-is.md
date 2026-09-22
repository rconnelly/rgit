# What Rabun Git is

Rabun Git is a **git forge you run yourself**. Teammates use ordinary `git clone` / `git push` over SSH. You manage people, keys, and permissions from the command line. There is no website and nothing is sent to GitHub.com.

This guide assumes you already know a little git: commits, branches, and that a **remote** is another copy of a repository you can push to and fetch from.

## The idea in git terms

On your laptop you have a **working copy** (files you edit). GitHub, GitLab, and Rabun Git are all places that store a **remote** copy so other people can get the same history.

| You already know | In Rabun Git |
| --- | --- |
| `git clone git@github.com:org/app.git` | `git clone ssh://git@HOST:2222/org/app.git` |
| GitHub users and SSH keys | `rabun-git user` and `rabun-git key` |
| Repo permissions (read / write / admin) | `rabun-git access grant` |
| Pull requests | Merge **requests** (`rabun-git request` or a special push ref) |
| GitHub Actions | Small YAML files in `.rabun/workflows/` (shell steps only) |

The git protocol is the same. The host, port, and how you log in are different.

## Two machines, two hats

1. **The server** runs `rabun-git serve`. That process listens for git and management commands on **port 2222** (not the usual SSH port 22). Your existing `sshd` on port 22 is left alone.
2. **Laptops** (including yours) use `git` and optionally `ssh -p 2222 …` to talk to that server.

On the server, the `rabun-git` binary you run in a terminal is the **operator**: it can do everything, with no SSH. Over the network, identity is **an SSH public key** that you attached to a forge user.

## Repository names

Every repository is `owner/name`, for example `ada/website` or `team/warehouse`.

- `owner` is a user login or a team-style prefix (`ada`, `team`).
- `name` is the project (`website`).
- Clone URLs may include a leading `/` and a `.git` suffix; both are optional: `/ada/website.git` is the same repo as `ada/website`.

Allowed characters: letters, digits, `.`, `_`, `-`. Names cannot start with `.`.

## What this tool does not do

- No browser UI, issues, wiki, or packages
- No GitHub Actions `uses:` marketplace
- Merge is **fast-forward only** (the branch you merge must already contain the target branch)
- `master` and `main` are **protected**: only a repo or forge **admin** can push them directly. Everyone else opens a merge request.

Next: [Install](install.md).
