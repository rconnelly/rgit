# User guide

Rgit is Git with etiquette. Run a git forge on a machine you own: SSH remotes, users, merge requests, a small CI runner, and Conventional Commits / SemVer 2.0 by default ([turn that off](versioning.md#disable-etiquette)). `rgit serve` has no HTTP git UI. Browse locally with [`rgit view`](everyday-git.md#browse-locally), or add [Rgit Web](https://docs.rgit.rs/web/) as a companion.

If you know `git clone`, `git commit`, and `git push`, follow the pages in order. Examples use `git.example.com`, user `ada`, repo `ada/website`.

## Contents

1. [What is Rabun Git?](what-it-is.md) — how this compares to GitHub and a plain git remote
2. [Install](install.md) — install the `rabun-git` command
3. [Start the forge](start-the-forge.md) — pack-and-push or manual `init` / `serve`, then first admin and key
4. [Set up a remote repository](remote-repository.md) — create `owner/name`, add `origin`, first push or clone
5. [Users and roles](users-and-roles.md) — add people, register keys, grant and revoke `read` / `write` / `admin`
6. [Everyday git](everyday-git.md) — clone, branches, protected `main` / `master`, local `rgit view`
7. [Merge requests](merge-requests.md) — propose, review, fast-forward merge
8. [CI workflows](ci-workflows.md) — `.rabun/workflows` on push, tag, and request
9. [Versioning](versioning.md) — SemVer, Conventional Commits, changelog, `rgit version`
10. [Command reference](commands.md) — CLI and SSH cheat sheet
11. [Compared to GitHub](compared-to-github.md) — feature and `gh` command gap analysis

Layout, ACL, systemd: [architecture](architecture.md). Ubuntu pack/push: [deploy Ubuntu](deploy-ubuntu.md). Extra disk for `repos/`: [storage volume](storage-volume.md).

## Start here

| Job | Page |
| --- | --- |
| Put a project on the forge | [Remote repository](remote-repository.md) |
| Let teammates in | [Users and roles](users-and-roles.md) |

Clone URL shape used throughout:

```text
ssh://git@git.example.com:2222/ada/website.git
```

Forge commands from this machine (`rgit login` attaches a key after website sign-in; `key copy` is host SSH for the first admin):

```bash
rgit remote add origin git@git.example.com
rgit login --web https://git.example.com
rgit origin repo list
```
