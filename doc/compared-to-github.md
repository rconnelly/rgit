# Compared to GitHub

rgit covers the git-hosting core: remotes, users and keys, path ACL, fast-forward merge requests, in-repo shell CI, and SemVer 2.0 / Conventional Commits by default. It is not a GitHub clone. There is no website on the forge host, and most GitHub products have no counterpart.

`rgit` and `rabun-git` are the same program. Examples use `rgit` after `rgit remote add origin git@HOST`.

A shorter map lives in [What is Rabun Git?](what-it-is.md).

## Hosted git

| GitHub | `gh` / git | rgit | Gap |
| --- | --- | --- | --- |
| `github.com:owner/name.git` (HTTPS or SSH :22) | `git clone` / `push` | `git clone ssh://git@HOST:2222/owner/name.git` | Different host, port **2222**, SSH only (no HTTPS git) |
| Create repo (UI or API) | `gh repo create` | `rgit origin repo create owner/name` | No org/teams UI; ordinary users create only `theirlogin/…` |
| List your repos | `gh repo list` | `rgit origin repo list` / `--user NAME` | No stars, topics, search, or templates |
| Repo settings, default branch, delete | `gh repo edit` / `delete` | `rgit origin repo show` | No edit, delete, or rename; no visibility toggle (access is ACL only) |
| Git LFS, LFS as a product | git | ordinary git | No LFS server |

## People and access

| GitHub | `gh` | rgit | Gap |
| --- | --- | --- | --- |
| Sign-up, orgs, teams, SSO | `gh auth` | `rgit login` / web invite sign-up (`auth register`); `rgit origin user add` | No orgs, teams, or SSO |
| SSH keys on the account | Settings / `gh ssh-key` | `rgit login` (web handshake), `rgit origin key copy` / `key add` / `key list` | First key via website or host SSH (`key copy`); fingerprints only; no deploy keys, PATs, or fine-grained tokens |
| Collaborators (read/write/admin) | `gh api` / UI | `rgit origin access grant` / `revoke` | Three roles only; no CODEOWNERS; `main` / `master` are always protected |
| Outside collaborators, GitHub Apps | — | — | None |

## Pull requests

| GitHub | `gh` | rgit | Gap |
| --- | --- | --- | --- |
| Open PR | `gh pr create` | `rgit origin request create` or `git push … refs/rabun/requests/new/…` | No draft, assignees, labels, projects, or auto-merge |
| List / show | `gh pr list` / `view` | `request list` / `show` | No diff UI, no file comments |
| Review | `gh pr review` | `request review --approve\|--reject [--comment]` | One comment string; no line comments or requested reviewers |
| Merge | `gh pr merge` | `request merge` | **Fast-forward only**; no squash, rebase, or merge commit |
| Required checks to merge | Branch protection | — | CI can run; merge does not wait on it |

## CI

| GitHub | `gh` | rgit | Gap |
| --- | --- | --- | --- |
| Actions (`.github/workflows`) | `gh run` / `workflow` | `.rabun/workflows/*.yml` + `runs-on` + `rgit agent` | `run:` only; fan-out labels not a full matrix; no `uses:`, containers, secrets store, or marketplace |

## GitHub products with no rgit equivalent

| GitHub | rgit |
| --- | --- |
| Web UI, mobile app, Codespaces | Local `rgit view` (Zola on loopback). No hosted gitweb, mobile app, or Codespaces |
| Issues, Discussions, Projects, Wikis, Pages | None |
| Packages, Releases, Gists | `rgit version release` for SemVer tags, changelog, and manifest bumps on this machine. No forge-hosted release artifacts or packages |
| Dependabot, security advisories, code scanning | None |
| Notifications, webhooks, GitHub Apps, OAuth | Companion heartbeat only (`rgit status` / loopback `/health`) |
| Forks, compare view | No forks; clone the same `owner/name` if you have access |

## Command map (`gh` → `rgit`)

| `gh` | rgit |
| --- | --- |
| `gh auth login` | `rgit login --web https://HOST` (or `rgit origin key copy --admin` without a website) |
| `gh repo create` / `list` / `view` | `repo create` / `list` / `show` |
| `gh repo clone` | `git clone ssh://git@HOST:2222/owner/name.git` |
| `gh repo view` | `rgit view` (local Zola preview; not a hosted page) |
| `gh ssh-key add` | `rgit login` (attaches the generated key) or `rgit origin key copy` / `key add USER --file ~/.ssh/id_ed25519.pub` |
| `gh pr create` / `list` / `view` / `review` / `merge` | `request create` / `list` / `show` / `review` / `merge` |
| `gh run list` / `view` | `run list` / `show` / `logs` |
| `gh api`, `gh issue`, `gh release`, `gh gist`, … | `rgit version release` covers local tagging/changelog; no GitHub Releases product on the forge |

## Host-only commands

These run the forge. They are not GitHub product analogues:

`init`, `check`, `serve`, `shell`, `version`, Ubuntu [pack and push](deploy-ubuntu.md).

## Practical takeaway

rgit is enough if you want private SSH remotes, simple ACL, fast-forward merge requests, Conventional Commits / SemVer 2.0 (or [those gates off](versioning.md#disable-etiquette)), and a few `sh -c` jobs on one box. Anything people do in the GitHub website (except a local `rgit view` of a tree you already have), or with tokens, orgs, issues, or the Actions marketplace, is a gap by design.

Next: [Install](install.md), or the [command reference](commands.md).
