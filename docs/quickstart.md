# Quick start

From nothing to a staging deploy running in CI. You need a Roblox account, and [mise](https://mise.jdx.dev) or [Rokit](https://github.com/rojo-rbx/rokit).

## 1. Pin the tools

=== "mise"

    ```sh
    mise use --pin github:rbx-forge/rbx-deploy github:rojo-rbx/rojo
    ```

=== "Rokit"

    ```sh
    rokit add rbx-forge/rbx-deploy --alias rbx
    rokit add rojo-rbx/rojo
    ```

Rojo builds the `.rbxl` from your source; `rbx` ships it and manages everything else about the experience.

## 2. Create or adopt the experience

A new one: run it bare, and answer `staging` when asked for the env name.

```sh
rbx init create-universe
```

One that exists already:

```sh
rbx import --universe-id 9876543211 --env staging
```

Either way you get `rbxplace.toml`. Check it with `rbx env list`:

```toml title="rbxplace.toml"
[staging]
universe_id = 9876543211
places.main = 234567890123456

[prod]
universe_id = 9876543210
confirm = true             # every write to prod asks first
places.main = 123456789012345
```

## 3. Get a key

In the Creator Hub, **Open Cloud > API Keys**: a key restricted to the staging universe with `universe-places:write`. Pass it through the environment, not a flag:

```sh
export RBX_API_KEY="..."
```

## 4. Ship a build

```sh
rojo build --output build.rbxl
rbx place upload --env staging --file build.rbxl              # a draft
rbx place upload --env staging --file build.rbxl --published  # live
```

## 5. Put it in CI

```yaml title=".github/workflows/deploy.yml"
name: Deploy
on:
  push:
    branches: [main]

jobs:
  staging:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: jdx/mise-action@v5          # or CompeyDev/setup-rokit@v0.2.1
      - run: rojo build --output build.rbxl
      - run: rbx check --env staging --offline
      - run: rbx place upload --env staging --file build.rbxl --yes
        env:
          RBX_API_KEY: ${{ secrets.RBX_API_KEY }}
```

`rbx check --offline` needs no key, and exits `2` when the repository disagrees with itself: a config not synced, a generated module out of date.

Whether Roblox still matches the repository is a separate question, asked on a schedule so a change made on the Creator Dashboard does not block a deploy. Give it a second key holding read scopes only ([Scopes by command](reference/scopes.md)):

```yaml title=".github/workflows/drift.yml"
name: Drift
on:
  schedule:
    - cron: "0 6 * * *"
  workflow_dispatch:

jobs:
  roblox:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: jdx/mise-action@v5
      - run: rbx check --env all       # exit 2 when Roblox moved
        env:
          RBX_API_KEY: ${{ secrets.RBX_READ_KEY }}
```

It compares what Roblox has with the lockfiles, in the `shop/live` and `meta/live` rows: see [Comparing with Roblox](check.md#comparing-with-roblox). The read key needs the scopes listed for `check` on [Scopes by command](reference/scopes.md).

Promote to production by hand, or behind a manual approval:

```sh
rbx place promote --from staging --to prod
```

## 6. Commit the lockfiles

Ignore `build.rbxl`, and any file holding a key. Commit `rbxplace.toml` and every `*.lock.toml`.

!!! warning "A lost lock entry creates a duplicate"
    `rbx shop sync` creates whatever has no lock entry, and Roblox cannot delete passes or products. Read [Working in a team](teams.md) before resolving a conflict in one.

## Next

[`rbx meta`](meta.md) and [`rbx config`](config.md) for the rest of the experience, [`rbx shop`](shop.md) for passes, badges and products, [`rbx rtbf`](rtbf.md) before the first deletion request.
