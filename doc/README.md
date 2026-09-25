# User guide

Rgit is Git with etiquette. Rabun Git is a git forge you run on a machine you own: SSH remotes, users and roles, merge requests, a small CI runner, and Conventional Commits / SemVer 2.0 by default ([how to turn that off](versioning.md#disable-etiquette)). There is no website on the forge host. To read a tree in a browser, use [`rgit view`](everyday-git.md#browse-locally) on this machine.

If you know `git clone`, `git commit`, and `git push`, start at the top and follow the pages in order. Each page has copy-paste examples (`git.example.com`, user `ada`, repo `ada/website`).

## Contents

1. [What is Rabun Git?](what-it-is.md) — how this compares to GitHub and a plain git remote
2. [Install](install.md) — build the `rabun-git` command
3. [Start the forge](start-the-forge.md) — pack-and-push or manual `init` / `serve`, then first admin and key
4. [Set up a remote repository](remote-repository.md) — create `owner/name`, add `origin`, first push or clone
5. [Users and roles](users-and-roles.md) — add people, register keys, grant and revoke `read` / `write` / `admin`
6. [Everyday git](everyday-git.md) — clone, branches, protected `main` / `master`, local `rgit view`
7. [Merge requests](merge-requests.md) — propose, review, fast-forward merge
8. [CI workflows](ci-workflows.md) — `.rabun/workflows` on push, tag, and request
9. [Versioning](versioning.md) — SemVer, Conventional Commits, changelog, `rgit version`
10. [Command reference](commands.md) — CLI and SSH cheat sheet
11. [Compared to GitHub](compared-to-github.md) — feature and `gh` command gap analysis

Operators who need on-disk layout, ACL internals, or systemd: [architecture](architecture.md). Ubuntu pack/push: [deploy Ubuntu](deploy-ubuntu.md). Extra disk for `repos/`: [storage volume](storage-volume.md).

## Two jobs you will do

| Job | Start here |
| --- | --- |
| Put an existing or new project on the forge | [Remote repository](remote-repository.md) (after the server is up) |
| Let teammates in and control who can push | [Users and roles](users-and-roles.md) |

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
