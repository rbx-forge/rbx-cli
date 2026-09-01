# Quick start

Clone to a deployed staging place, on one page.

Every other page here explains one command in depth. This one does the opposite: it walks the whole path once, with the files written out, and links to the depth where you would want it rather than before. If you already have an experience on Roblox, skip to [Adopt what exists](#2b-adopt-what-exists).

What you need first: a Roblox account, and [Rokit](https://github.com/rojo-rbx/rokit) to pin the tools.

## 1. Pin the tools

Everything below is version-pinned in the repository, so a new machine and a CI runner get the same binaries.

```toml title="rokit.toml"
[tools]
rbx = "rbx-forge/rbx-cli@0.6.0"
rojo = "rojo-rbx/rojo@7.7.0"
```

```sh
rokit install
```

`rbx` and `rojo` do different halves of the job and neither replaces the other. Rojo turns a directory of `.luau` files into a `.rbxl`. `rbx` puts that file on Roblox, and manages everything about the experience that is not the file: metadata, live config, passes, keys.

## 2a. Create the experience

Skip this if the universe already exists.

```sh
rbx init create-universe
```

Run it bare and it asks for what it needs: a name, and the env name to record it under. Answer `staging` for the env when it asks, and it writes `rbxplace.toml` for you.

A second place inside the same universe, when you want one:

```sh
rbx init create-place --universe-id 987654321 --name "Lobby" --place lobby
```

Depth: [`rbx init`](init.md), including creating the group first if you do not have one.

## 2b. Adopt what exists

The other way in. One command reads a live universe and writes every config and lockfile from it:

```sh
rbx import --universe-id 123456789 --env staging
```

Run it once per environment. Nothing is invented: what lands in the files is what Roblox is already serving.

```sh
rbx import --universe-id 123456789 --env staging --dry-run
```

Depth: [`rbx import`](import.md).

## 3. Read back what you have

```sh
rbx env list
```

`rbxplace.toml` is the file every other command resolves `--env` through, and this is the command that proves it parses. It is committed, holds no secret, and is the one file worth reading by hand once:

```toml title="rbxplace.toml"
[staging]
universe_id = 9876543211
places.main = 234567890123456

[prod]
universe_id = 9876543210
confirm = true
places.main = 123456789012345
```

`confirm = true` on `prod` is the whole safety model in one line: every write to that env stops and asks. Set it before you need it.

Depth: [`rbx env`](env.md), which also generates a Luau module so the game can read its own env at runtime.

## 4. Get a key that works

Open Cloud keys are declared in a committed file and created from it, so the scopes a key holds are reviewable rather than clicked together on a dashboard.

```toml title="rbxapikey.toml"
[settings]
default_envs = ["staging"]
default_expiration_months = 3
default_secret_file = ".secrets/{name}.env"

[keys.deploy]
description = "Uploads place files from CI."
scopes = ["universe-places:write"]
```

```sh
rbx apikey create deploy
```

The secret is written to `.secrets/deploy.env`, which must be gitignored. See [the gitignore](#7-what-not-to-commit) below.

Then prove it works, which is a real authenticated read rather than a syntax check:

```sh
rbx doctor --env staging
```

Depth: [`rbx apikey`](apikey.md) for the key split worth using (a deploy key that cannot ban anyone, a read-only key for development), and [`rbx doctor`](doctor.md) for what it probes.

## 5. Ship a build

```sh
rojo build --output build.rbxl
rbx place upload --env staging --file build.rbxl
```

That saves a draft. Publishing it live is a separate flag, deliberately:

```sh
rbx place upload --env staging --file build.rbxl --published
```

An upload that reports a version and changed nothing is telling you the truth about a file Roblox already had: see `created` on [the place page](place.md).

Depth: [`rbx place`](place.md), including promoting a build between envs and rolling one back.

## 6. Put it in CI

One workflow, and the two commands that matter are the last two.

```yaml title=".github/workflows/deploy.yml"
name: Deploy
on:
  push:
    branches: [main]

jobs:
  staging:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - uses: CompeyDev/setup-rokit@v0.1.2
      - run: rojo build --output build.rbxl
      - run: rbx check --env staging
        env:
          RBX_API_KEY: ${{ secrets.RBX_API_KEY }}
      - run: rbx place upload --env staging --file build.rbxl --yes
        env:
          RBX_API_KEY: ${{ secrets.RBX_API_KEY }}
```

`rbx check` runs every configured tool's check in one pass and exits `2` on drift, so a pipeline can tell "publish this" from "something broke" on the status alone.

Promoting to production is deliberately not on this trigger. It is a command somebody runs, or a job behind a manual approval:

```sh
rbx place promote --from staging --to prod
```

Depth: [`rbx check`](check.md) for the exit codes, and [Live operations](ops.md) for the safety model once you start acting on a running game.

## 7. What not to commit

```gitignore title=".gitignore"
# Open Cloud secrets. Each of these files is a bare API key.
.secrets/

# Local copies data commands write before they overwrite anything.
.rbx/backups/

build.rbxl
```

The lockfiles beside your configs (`rbxshop.lock.toml` and friends) **are** committed, and they matter: a missing lock entry means "create", so losing one creates a second paid game pass that this tool cannot delete. [Working in a team](teams.md) is the procedure for resolving a conflict in one, and it is worth reading before you need it.

## Where to go next

You now have a repository that builds, checks and deploys. The pages worth reading in order of when they start mattering:

- [`rbx meta`](meta.md) and [`rbx config`](config.md), for everything about the experience that is not the place file.
- [`rbx shop`](shop.md), for passes, badges and developer products with typed Luau codegen.
- [`rbx rtbf`](rtbf.md), before somebody sends a deletion request rather than after.
- [Live operations](ops.md), the entry point to acting on a running game.
