# Merge requests

A merge request is “please fast-forward `master` (or `main`) to this branch.” There is no web form. Requests are stored **in the git repo** as refs, so they clone with the project.

You need **write** to open or comment. You need repo **admin** (or forge admin) to merge. Review `--approve` does not merge by itself.

Replace `ada/website` and host names as in earlier pages.

## Before you open one

1. [Create the repo and `origin`](remote-repository.md).
2. Push a **feature branch** (not `master`/`main` unless you are an admin):

```bash
git checkout -b feature/contact-form
git add -A
git commit -m "Add contact form"
git push -u origin feature/contact-form
```

The branch name must already exist on the forge (`git push` above).

## Option A — CLI (`request create`)

On the server:

```bash
rabun-git request create ada/website --head feature/contact-form --title "Add contact form"
```

Optional `--base master` (default is the bare repo’s HEAD branch) and `--body "…"` for a longer description.

From this machine:

```bash
ssh -p 2222 git@git.example.com request create ada/website --head feature/contact-form --title "Add contact form"
```

The command prints an id (`created request #1`).

`--head` can be a branch name or a commit SHA that is already in the repo.

## Option B — Gerrit-style push (no extra round trip)

Still on the feature branch:

```bash
git push origin HEAD:refs/rabun/requests/new/feature/contact-form
```

After `receive-pack`, the forge allocates the next id and writes `refs/rabun/requests/<id>/{head,base,meta}`. Need **write**. You cannot push into `refs/rabun/requests/123/…` yourself; only `…/new/<branch>` is allowed.

## List and show

```bash
rabun-git request list ada/website
rabun-git request show ada/website 1
```

Or:

```bash
ssh -p 2222 git@git.example.com request list ada/website
ssh -p 2222 git@git.example.com request show ada/website 1
```

Meta includes title, author, state (`open` / `merged` / `rejected`), base branch, and reviews.

## Review

Anyone with **write** can review. Merge still needs **admin**.

```bash
rabun-git request review ada/website 1 --approve --comment lgtm
rabun-git request review ada/website 1 --reject --comment "needs tests"
rabun-git request review ada/website 1 --comment "nit: rename the helper"
```

`--approve` and `--reject` cannot be combined. `--comment` is optional with approve/reject.

## Merge (fast-forward only)

```bash
rabun-git request merge ada/website 1
```

This moves the **base** branch (usually `master` or `main`) to the request head **only if** that head is a fast-forward: every commit on the base is already in the feature branch.

If someone else landed commits on `master` first:

```bash
git checkout feature/contact-form
git fetch origin
git merge origin/master    # or: git rebase origin/master
git push
```

Then merge the request again. There is no `--no-ff` merge in v1.

After a successful merge, the request state is `merged`. Protected-branch rules still apply: the merge is performed by the forge as an admin action, not as a raw push from a writer.

## Where requests live

Inside the bare repo:

- `refs/rabun/requests/<id>/head` — proposed tip
- `refs/rabun/requests/<id>/base` — target branch commit when opened/updated
- `refs/rabun/requests/<id>/meta` — YAML (title, author, body, state, reviews)

You can inspect with `git ls-remote ssh://git@git.example.com:2222/ada/website.git` and look for `refs/rabun/requests/`.

Opening or updating a request can start [CI](ci-workflows.md) if a workflow has `on: request:`.

Next: [CI workflows](ci-workflows.md).
