# Everyday git

Once a [remote repository](remote-repository.md) exists and you have a [role](users-and-roles.md), daily work is normal git. This page maps common tasks onto Rabun Git.

Assume `origin` is:

```text
ssh://git@git.example.com:2222/ada/website.git
```

## Clone (get a working copy)

```bash
git clone ssh://git@git.example.com:2222/ada/website.git
cd website
```

You need at least **read**. The clone includes all branches the server has. Merge-request refs (`refs/rabun/requests/…`) are in the same repo if you fetch them; you do not need them for ordinary editing.

## Browse locally

The forge does not serve a website. On this machine, preview the working copy (or a bare repo you can read) with Zola on loopback. Needs **git** and **Zola 0.23.4+** on PATH. First run fetches the DevLab theme into `~/.cache/rabun-git/themes/`.

```bash
cd website
rgit view
# or: rgit view --open
# or: rgit view /path/to/clone --ref v0.1.0
```

On the forge host, `rgit view ada/website` reads `$RABUN_GIT_ROOT/repos/ada/website.git` if that directory exists (often from `rabun-git shell`). It does not clone over SSH.

The preview binds **127.0.0.1:1111** by default. It is not `rabun-git serve` and not a public git UI.

## See remotes and status

```bash
git remote -v
git status
git log --oneline -5
```

`origin` should point at the forge URL. `git status` tells you if you have uncommitted files.

## Commit locally

Git still stores commits on **this machine** until you push.

```bash
# edit files
git add README.md
git commit -m "docs: explain the setup"
```

Write a Conventional Commits 1.0.0 subject (`feat:`, `fix:`, `docs:`, …). `rgit version hook install` checks that on this machine; a repo with `.rabun/version.toml` `enforce.commits` also checks on push. Details: [Versioning](versioning.md).

To cut a release (bump version files, `CHANGELOG.md`, commit, tag) without pushing:

```bash
rgit version release
```

## Update from the server

```bash
git pull
```

If others merged to `master`/`main`, this fetches and merges (or rebases, if you configured that). Need **read**.

## Branch, then push

Do not commit straight to `master`/`main` unless you are a repo or forge **admin**. Create a branch:

```bash
git checkout master
git pull
git checkout -b feature/contact-form
# edit, commit
git push -u origin feature/contact-form
```

Need **write** to push that branch. `-u` means later `git push` on this branch goes to `origin`.

List branches on the server:

```bash
git branch -r
```

## Why `git push origin master` might fail

`master` and `main` are **protected**. Non-admins get a hook error from the forge. That is expected. Push a feature branch and open a [merge request](merge-requests.md).

Admins can push those branches directly (for example the first import of an existing repo).

## Fetch without merging

```bash
git fetch origin
git log HEAD..origin/master
```

Useful to see incoming commits before you `git pull`.

## Tags

Prefer `rgit version release`, which writes an annotated `vMAJOR.MINOR.PATCH` tag that matches the version files. You can still tag by hand:

```bash
git tag v0.1.0
git push origin v0.1.0
```

Need **write**. If the repo has `.rabun/version.toml` with `enforce.tags`, the name must be SemVer 2.0 with the configured prefix. Tag pushes can start [CI workflows](ci-workflows.md) if a workflow file has `on: tag:`.

## SSH management without git

Same port as git. Examples:

```bash
ssh -p 2222 git@git.example.com repo list
ssh -p 2222 git@git.example.com repo show ada/website
ssh -p 2222 git@git.example.com request list ada/website
ssh -p 2222 git@git.example.com run list ada/website
```

A login SSH session (no command) prints a greeting and the repos you can read.

## If git asks about the host key

First connection to port 2222 shows a fingerprint for the forge host key (`$RABUN_GIT_ROOT/ssh_host_ed25519_key`). Compare with what the operator expects, then type `yes`. This is the same check as the first `ssh git@github.com`.

Next: [Merge requests](merge-requests.md), or [Versioning](versioning.md).
