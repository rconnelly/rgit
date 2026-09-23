# Set up a remote repository

This is the usual first goal: a repository on the forge that other machines can clone and push to.

In git, a **remote** is a nickname for that server copy. Most people call it `origin`. After these steps, `git push origin` sends commits to Rabun Git the same way it would send them to GitHub.

Replace:

- `git.example.com` — server hostname or IP
- `ada` — forge user who owns the project
- `website` — repository name

## Who can create a repository

```bash
rabun-git repo create owner/name
```

| Who | What they can create |
| --- | --- |
| Operator (`rabun-git` on the server) | Any `owner/name` |
| Forge admin (over SSH or locally) | Any `owner/name` |
| Ordinary user over SSH | Only `theirlogin/…` (for example `linus/notes`) |

The creator is granted **admin** on that repository automatically.

You can run create on the server:

```bash
rabun-git repo create ada/website
```

Or from this machine, after `rgit remote add origin git@git.example.com`:

```bash
rabun-git origin repo create ada/website
# same as: ssh -p 2222 git@git.example.com repo create ada/website
```

Confirm:

```bash
rabun-git origin repo list
rabun-git origin repo show ada/website
```

`repo show` prints the on-disk path and who has access.

## Clone URL

```text
ssh://git@git.example.com:2222/ada/website.git
```

The `:2222` is required unless you set `Port 2222` in `~/.ssh/config` (see [Start the forge](start-the-forge.md#optional-shorter-git-urls)).

SSH user is `git` or your forge login. The forge maps the **key** to a user; do not put the repo owner in the SSH username the way some hosts do.

## Path A — existing local git repo (add a remote)

You already have commits on this machine and want the forge to be `origin`.

On the **server**, create the empty repo if you have not:

```bash
rabun-git repo create ada/website
```

On **this machine**:

```bash
cd website
git remote -v
```

If there is already an `origin` (for example GitHub), either rename it or add a second remote:

```bash
# keep GitHub, add Rabun Git under a different name
git remote add rabun ssh://git@git.example.com:2222/ada/website.git
git push -u rabun master
```

Or make Rabun Git the primary remote:

```bash
git remote add origin ssh://git@git.example.com:2222/ada/website.git
git push -u origin master
```

If your default branch is `main`:

```bash
git push -u origin main
```

`-u` sets upstream so later `git push` / `git pull` know which remote branch to use.

**First push of `master` or `main`:** the creator (repo admin) and forge admins may push those branches directly. Other users cannot; they push a feature branch and open a [merge request](merge-requests.md).

## Path B — brand new project

No git repo yet. On this machine:

```bash
mkdir website
cd website
git init -b master
echo "# Website" > README.md
git add README.md
git commit -m "Initial commit"
git remote add origin ssh://git@git.example.com:2222/ada/website.git
git push -u origin master
```

Create `ada/website` on the forge **before** the push (`repo create` above). Pushing to a name that does not exist fails; Rabun Git does not auto-create repos on first push.

## Path C — another machine (clone)

Someone else (or you, on another machine) already pushed. They need [read (or higher) access](users-and-roles.md) and their SSH key registered.

```bash
git clone ssh://git@git.example.com:2222/ada/website.git
cd website
```

That creates a local `origin` pointing at the forge. Work as usual:

```bash
git pull
# edit files
git add -A
git commit -m "Describe the change"
git push
```

If you are not a repo admin, push a **branch** instead of `master`/`main`:

```bash
git checkout -b feature/contact-form
git push -u origin feature/contact-form
```

Then open a merge request (see [Merge requests](merge-requests.md)).

## Point a tool or another clone at the forge

Same URL everywhere:

```bash
git remote add origin ssh://git@git.example.com:2222/ada/website.git
```

Example from the README for a warehouse tree you already commit locally:

```bash
git remote add origin ssh://git@git.example.com:2222/team/warehouse.git
```

Create `team/warehouse` first (`rabun-git repo create team/warehouse`) and [grant write](users-and-roles.md) to whoever should push.

## See what the server stored

On the server, bare repos live under `$RABUN_GIT_ROOT/repos/owner/name.git`. You do not need to `cd` there for daily git; use clone URLs from this machine.

```bash
rabun-git repo list
ssh -p 2222 git@git.example.com repo list
```

## Common mistakes

| What happened | Why |
| --- | --- |
| `repository must be owner/name` | Use `ada/website`, not `website` |
| `repository … already exists` | `repo create` is one-shot; add a remote and push instead |
| `users may only create repos under their own name` | Log in as that user, or have an admin create `team/…` |
| `Could not resolve hostname` / connection refused | Missing `:2222`, or `serve` is not running |
| Push to `master` rejected | You are not a repo/forge admin — use a feature branch + merge request |
| `repository ada/website not found` on push | `repo create` was not run, or the owner/name does not match the URL |

Next: [Users and roles](users-and-roles.md) if other people need access, or [Everyday git](everyday-git.md) to work on the repo.
