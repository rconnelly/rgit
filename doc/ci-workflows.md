# CI workflows

Rabun Git can run shell commands on the **server** when someone pushes a branch, pushes a tag, or opens/updates a merge request. This is **not** GitHub Actions: there is no `uses:`, no matrix, and no containers.

Treat the runner as trusted single-tenant: it runs as the same user as `rabun-git serve`, with `sh -c`, in a temporary worktree of the commit.

## Add a workflow file

In the git project (the copy you clone and commit), create:

```text
.rabun/workflows/ci.yml
```

Commit it and push (or merge via a request) like any other file. The forge reads workflows **at the commit that triggered the event**.

Example:

```yaml
name: ci
on:
  push:
    branches: [master]
  tag:
  request:
jobs:
  test:
    steps:
      - run: cargo test --locked
```

Supported fields:

| Field | Meaning |
| --- | --- |
| `name` | Label for humans |
| `on.push.branches` | Run on those branch names; omit the list to match every branch |
| `on.tag` | Run on tag pushes (key present and not `false`) |
| `on.request` | Run when a merge request is opened or updated |
| `jobs.<id>.steps[].run` | Command passed to `sh -c` |
| `env` | Extra environment for every step (optional) |
| `timeout_minutes` | Per workflow or per job (default **30**) |

Jobs run **one after another** in file order. A failing `run` fails the workflow.

## When it runs

- **push** — `git push` updates `refs/heads/…` and the branch matches `on.push.branches`
- **tag** — `git push` of `refs/tags/…` and `on.tag` is enabled
- **request** — merge request created or updated and `on.request` is enabled

There is no `pull_request` YAML from GitHub; use `request`.

## Inspect runs

Need **read** on the repo.

```bash
rabun-git run list ada/website
rabun-git run show ada/website RUN_ID
rabun-git run logs ada/website RUN_ID
```

Or over SSH:

```bash
ssh -p 2222 git@git.example.com run list ada/website
ssh -p 2222 git@git.example.com run logs ada/website RUN_ID
```

On disk: `$RABUN_GIT_ROOT/runs/<owner>/<name>/<run-id>/` with `status.yaml` (`queued` / `running` / `passed` / `failed`) and `log.txt`. Logs are **not** stored in git (size and secrets).

## A minimal “did it run?” workflow

Useful while you are learning:

```yaml
name: ping
on:
  push:
    branches: [master]
jobs:
  ping:
    steps:
      - run: echo ok && git rev-parse HEAD
```

Push to `master` (as an admin) or merge a request targeting `master`, then `run list`.

## What not to expect

- `uses: actions/checkout@v4` and other GitHub Actions — unsupported
- Services, containers, and reusable workflows — unsupported
- Untrusted forks — this runner is for **your** server and **your** keys

Next: [Command reference](commands.md).
