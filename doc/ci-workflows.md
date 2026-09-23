# CI workflows

Rabun Git can run shell commands when someone pushes a branch, pushes a tag, or opens/updates a merge request. This is **not** GitHub Actions: there is no `uses:`, no full matrix, and no containers.

By default a job runs on the **forge host** (`sh -c`, same user as `rabun-git serve`). Jobs can also target **builder agents** on Linux, macOS, or Windows (`runs-on:`). Agents poll the forge over SSH on port 2222.

Treat every runner as trusted: your machines, your keys, your workflow files.

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
    runs-on: [linux, macos, windows]
    steps:
      - run: cargo test --locked
```

Omit `runs-on` to run only on the forge host. A list **fans out** to one run per label (not a GitHub Actions matrix).

Supported fields:

| Field | Meaning |
| --- | --- |
| `name` | Label for humans |
| `on.push.branches` | Run on those branch names; omit the list to match every branch |
| `on.tag` | Run on tag pushes (key present and not `false`) |
| `on.request` | Run when a merge request is opened or updated |
| `jobs.<id>.steps[].run` | Command passed to the job shell |
| `jobs.<id>.runs-on` | `linux`, `macos`, `windows`, or a list (one run each) |
| `jobs.<id>.shell` | `sh` (default on unix), `bash`, or `pwsh` (default on windows) |
| `env` | Extra environment for every step (optional) |
| `timeout_minutes` | Per workflow or per job (default **30**) |

Jobs run **one after another** in file order. A failing `run` fails that run. `linux` with no registered linux builder still runs on the forge host. Other labels stay `queued` until an agent claims them.

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

On disk: `$RABUN_GIT_ROOT/runs/<owner>/<name>/<run-id>/` with `status.yaml` (`queued` / `running` / `passed` / `failed`), `job.yaml`, and `log.txt`. Logs are **not** stored in git (size and secrets).

## Builder agents (macOS / Windows / extra Linux)

Register a builder as a forge admin (the name is a forge login; grant it **read** on repos it should clone):

```bash
rgit origin agent register mac --label macos --file ~/.ssh/id_ed25519.pub
rgit origin access grant mac bw/rabun --role read
rgit origin agent list
```

On the builder machine:

```bash
rgit remote add origin git@HOST
rgit agent --labels macos
```

The agent polls `agent next` over SSH, clones `ssh://git@HOST:2222/owner/name.git` at the job SHA, runs each `run:` step, then `agent log` / `agent finish`. Run it under launchd (macOS), Task Scheduler (Windows), or a systemd user unit (Linux). `rgit origin agent --labels macos` is the same poll loop using that remote name.

Labels must match `runs-on` exactly (`linux`, `macos`, `windows`). Windows defaults to `pwsh -NoProfile -Command`; unix defaults to `sh -c`.

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
- Services, containers, reusable workflows, and a full `strategy.matrix` — unsupported (`runs-on: [a, b]` is fan-out only)
- Untrusted forks — this runner is for **your** server, **your** builders, and **your** keys

Next: [Command reference](commands.md).
