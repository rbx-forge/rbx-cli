# Config files

Each subcommand owns its TOML file; the one file they share is `rbxplace.toml`. No monolithic config: each concern can be adopted, reviewed and left out on its own.

| File | Owned by | Purpose |
| --- | --- | --- |
| `rbxplace.toml` | shared | Maps env names to universe and place ids. Read by every subcommand that takes `--env`; inspect it with [`rbx env`](../env.md) |
| `rbxshop.toml` | [`rbx shop`](../shop.md) | Game passes, badges, developer products. Per-env overlays |
| `rbxmeta.toml` | [`rbx meta`](../meta.md) | Universe and place metadata. Per-env overlays |
| `rbxconfig.toml` | [`rbx config`](../config.md) | In-experience configs. Per-env overlays |
| `rbxrtbf.toml` | [`rbx rtbf`](../rtbf.md) | Right-to-be-forgotten deletion templates. Declared once, published to any env |

Every key in these files is described by a JSON Schema your editor can load: see [Editor support](editor-support.md).

## Lockfiles

Most of these have a lockfile beside them (`<config>.lock.toml`) tracking remote ids, content hashes and what Roblox held at the last `sync` or `pull`. That is how `sync` knows what exists already, and what was changed on Roblox since. `rbxrtbf.toml` has none: the published templates are read back in full.

!!! warning "Commit the lockfiles, and resolve their conflicts with care"
    A missing lock entry means "create". Losing one from `rbxshop.lock.toml` creates a second paid game pass on the next sync, and Roblox has no way to delete it. [Working in a team](../teams.md) is the procedure: which side to keep per file, and what concurrent syncs do per tool.

Lockfiles are tool-owned state rather than input you typed, so their format may change between releases: see [Stability](stability.md).

## `rbxplace.toml` in one screen

```toml
[dev]
universe_id = 9876543210
[dev.places]
main = 123456789012345

[prod]
universe_id = 9876543211
[prod.places]
main = 234567890123456
```

With that in place, every subcommand accepts `--env <name>`:

```sh
rbx shop sync --env prod
rbx place upload --env prod --file game.rbxl
rbx meta sync --env prod
```

Some subcommands also take a plural `--env`: `all` for every env in the file, or a `[groups]` name for a subset:

```sh
rbx shop sync --env all          # every env
rbx meta check --env all         # diff against every env
rbx meta sync --env nonprod      # the envs a group names
rbx place upload --env all --file build.rbxl --yes   # one build to every env
```

Reading the file back rather than editing it is [`rbx env`](../env.md), which also documents groups, `[owner]`, `confirm` and the rest of the format:

```sh
rbx env list                                # every env, its universe id and places
UNIVERSE=$(rbx env get universe-id -e prod) # one bare value, for scripts and CI
```

## Unrecognised keys

An unrecognised key is a warning on stderr, never a failure, so a file written for a newer release still loads on an older binary. The schemas follow the same rule and never close `additionalProperties`.
