# Users and roles

Rabun Git has no sign-up page. An admin creates a **user**, attaches that person’s **SSH public key**, then **grants a role** on each repository they should see.

This page is a set of workflows. Run `rabun-git …` on the server (operator, full access) or the same commands over SSH if you are a forge admin:

```bash
ssh -p 2222 git@git.example.com user list
```

Examples use:

- `ada` — forge admin (already created in [Start the forge](start-the-forge.md))
- `linus` — a teammate
- `ada/website` — a repository from [Set up a remote repository](remote-repository.md)

## What a “user” is

A user is a login in `users.yaml` plus zero or more OpenSSH public keys in `keys/<login>.pub`. Git never asks for a password. When someone runs `ssh -p 2222 git@HOST` or `git push`, the forge looks up **which user owns that key**.

The SSH username can be:

- `git` (usual), or
- the forge login (`linus`)

If the username is anything else, authentication is rejected even if the key is registered.

Forge-wide **admin** (`--admin`) is a flag on the user, not a per-repo role. Forge admins skip per-repo ACL and can manage users and keys.

## Roles on a repository

Grant these with `rabun-git access grant`. Each higher role includes the one below it.

| Role | Clone / fetch | Push feature branches and tags | Open and review merge requests | Push `master` / `main` | Merge requests | Grant/revoke access on that repo |
| --- | --- | --- | --- | --- | --- | --- |
| `read` | yes | no | no | no | no | no |
| `write` | yes | yes | yes | no | no | no |
| `admin` | yes | yes | yes | yes | yes | yes |

Also:

- **Forge admin** — everything above on every repo, plus `user` / `key` management.
- **Operator** — `rabun-git` run on the server; same power as a forge admin, no SSH.
- Creating a repo grants **admin** on that repo to the creator.

`master` and `main` are protected. Writers push other branches and use [merge requests](merge-requests.md).

---

## Workflow: add a teammate who can push

Goal: `linus` can clone `ada/website`, push feature branches, and open merge requests. He cannot push `master` directly.

### 1. Create the user (not a forge admin)

On the server, as operator or forge admin:

```bash
rabun-git user add linus
rabun-git user list
```

`user add` with the same name again updates the admin flag (last write wins).

### 2. Register their public key

On **Linus’s laptop**:

```bash
ssh-keygen -t ed25519 -C "linus@git.example.com" -f ~/.ssh/id_ed25519
cat ~/.ssh/id_ed25519.pub
```

Copy that one line to the server (file `linus.pub`) and **do not** copy the private key (`id_ed25519` without `.pub`).

The **first** key for a new user must be added by an operator or forge admin (Linus cannot SSH in yet). After that, he can append extra keys himself.

```bash
rabun-git key add linus --file /path/to/linus.pub
rabun-git key list linus
```

`key list` prints fingerprints, not the key material. Only a forge admin can add keys for someone else.

### 3. Grant write on the repository

```bash
rabun-git access grant linus ada/website --role write
rabun-git repo show ada/website
```

`--role write` is the default if you omit `--role`.

Linus checks from his laptop:

```bash
ssh -p 2222 git@git.example.com repo list
git clone ssh://git@git.example.com:2222/ada/website.git
```

He should see `ada/website` and the clone should succeed.

### 4. He pushes a branch (not `master`)

```bash
cd website
git checkout -b feature/readme
echo "Hello from linus" >> README.md
git add README.md
git commit -m "Mention linus"
git push -u origin feature/readme
```

A direct `git push origin master` should be **rejected** until someone with repo `admin` (or a forge admin) merges a request.

---

## Workflow: read-only access (clone only)

A designer or CI mirror that must not push:

```bash
rabun-git user add pat
rabun-git key add pat --file /path/to/pat.pub
rabun-git access grant pat ada/website --role read
```

`pat` can `git clone` and `git fetch`. Push and merge requests fail with a need-`write` error.

---

## Workflow: make someone a repository admin

They need to merge requests and push `main`/`master`, but should not create forge users.

```bash
rabun-git access grant linus ada/website --role admin
```

Granting a new role **replaces** the previous role on that repo (write becomes admin). They can now:

```bash
ssh -p 2222 git@git.example.com request merge ada/website 1
ssh -p 2222 git@git.example.com access grant pat ada/website --role read
```

---

## Workflow: make someone a forge admin

They can create users, add keys for anyone, and bypass repo ACL.

```bash
rabun-git user add linus --admin
```

Same command as creating a user; existing `linus` is updated. To take forge admin away without deleting the account:

```bash
rabun-git user add linus
```

(omit `--admin`). Their per-repo roles in `access.yaml` still apply.

---

## Workflow: let a user create their own repos

Ordinary users may `repo create` only under **their** login:

```bash
ssh -p 2222 git@git.example.com repo create linus/notes
```

Linus becomes repo admin on `linus/notes`. Ada (forge admin) can still create `team/shared` for a group and grant people `write` or `admin` on it.

```bash
rabun-git repo create team/shared
rabun-git access grant ada team/shared --role admin
rabun-git access grant linus team/shared --role write
```

The operator/forge admin can create any `owner/name`. The `owner` segment does not have to be an existing user; it is just a path prefix (`team`, `ada`, `acme`).

---

## Workflow: revoke access or remove a person

Stop Linus from using `ada/website` (account stays; other repos unchanged):

```bash
rabun-git access revoke linus ada/website
rabun-git repo show ada/website
```

Remove the account, keys, and **all** repo grants:

```bash
rabun-git user remove linus
```

They can no longer authenticate. Repositories they owned remain on disk; grant someone else `admin` if the team still needs them.

---

## Who is allowed to run these commands

| Command | Operator on the server | Forge admin over SSH | Repo admin over SSH | The user themselves |
| --- | --- | --- | --- | --- |
| `user add` / `user list` / `user remove` | yes | yes | no | no |
| `key add` / `key list` for themselves | yes | yes | yes | yes |
| `key add` / `key list` for someone else | yes | yes | no | no |
| `access grant` / `revoke` on a repo | yes | yes | yes (that repo) | no |
| `repo create` `theirname/…` | yes | yes | if they are that user | yes |
| `repo create` `other/…` | yes | yes | no | no |

Commands that never work over SSH: `init`, `check`, `status`, `serve`. Use those on the host.

## Where this is stored

Under `$RABUN_GIT_ROOT`:

- `users.yaml` — login + forge admin flag
- `keys/<user>.pub` — public keys
- `access.yaml` — `owner/name` → user → `read` \| `write` \| `admin`

You can read those files; prefer `rabun-git user` / `key` / `access` so they stay valid YAML.

Next: [Everyday git](everyday-git.md).
