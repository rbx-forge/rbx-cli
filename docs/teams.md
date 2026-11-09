# Working in a team

Two people on diverged branches, or a person and a CI job, syncing the same universe. What git shows afterwards is a conflict in a lockfile, and how you resolve it decides whether the next `sync` **updates** a resource or **creates a second one**.

!!! warning "A missing lock entry means create"
    `rbx shop sync` creates every resource that has no entry in `rbxshop.lock.toml`. Roblox cannot delete a pass, badge or product, so a duplicate stays forever: the best repair is `for_sale = false` or `enabled = false`. Before its first create, `sync` stops if the name already exists on Roblox, but a resource renamed since its last sync gets past that check.

## The files that conflict

| File | Conflicts when | Keep | Then run |
| --- | --- | --- | --- |
| `rbxshop.lock.toml` | a resource added or changed on both sides | **every entry from both sides**, never fewer ids than either had | `rbx shop pull --env <name> --dry-run`, then without `--dry-run` |
| `rbxmeta.lock.toml` | the same field changed on both sides | either side whole. Never hand-merge `media.thumbnails`, an ordered array | `rbx meta sync --env <name> --dry-run`, or `rbx meta pull` |
| `rbxconfig.lock.toml` | every sync: `synced_at` and `revision_id` always change | either side | `rbx config sync --env <name> --dry-run` |
| generated modules | always | neither | `rbx shop codegen`, `rbx env codegen` |

`rbx place` has no lockfile. Lockfile keys are sorted, so two people adding different resources usually merge without a conflict.

Add `* text=auto` to the repository's `.gitattributes`: with `core.autocrlf=true` and no attributes, a one-line lockfile change shows up as a whole-file diff.

## Resolving `rbxshop.lock.toml`

```sh
git diff --diff-filter=U rbxshop.lock.toml
# keep every [envs.<env>.passes|badges|products.<key>] table from either side;
# where both have a key, either set of values will do, but keep the `id`
git add rbxshop.lock.toml
rbx shop pull --env <name> --dry-run   # pull also edits rbxshop.toml: read first
rbx shop pull --env <name>
rbx shop check --env <name>
```

`pull` matches remote resources to config keys by the id in the lockfile, and by display name when there is no id. That fallback is the trap: a key whose `name = "..."` differs from the key, once its lock entry is dropped, is re-imported under its display name as a **new** key, and the original key plans a create. So:

- Never resolve by deleting entries.
- `git checkout --ours` or `--theirs` on the whole file is safe only if one side has every id the other has.
- Never delete the lockfile and re-run `sync`. `rbx shop pull` rebuilds it from Roblox; `sync` would try to create everything.

## What a green check means

`shop/lockfile`, `shop/codegen` and `meta/lockfile` compare committed files with each other, not with Roblox: after a merge, a clean row means the config and the lockfile agree, and cannot see a resource dropped from both. `shop/live` and `meta/live` compare the lockfile with Roblox, and report that resource as on Roblox and not in the lockfile. Run `rbx check` with a key after any merge you are unsure of.

## Concurrent syncs

There is no locking: nothing notices that the lockfile or Roblox changed while a command runs.

| Tool | Two runs at once |
| --- | --- |
| `rbx shop sync` | **Unsafe for creates**: both can plan the same create and both succeed. Updates go by id and the last one wins. The lockfile is saved after each resource, so an interrupted run resumes cleanly. A lockfile section recorded for another universe is refused |
| `rbx meta sync` | Last write wins per field, nothing duplicates. Two runs that both change thumbnails are untested: run them one at a time |
| `rbx config sync` | Replaces the whole live config. A publish made since your last sync or pull is refused (see [`--overwrite-newer`](config.md#rbx-config-sync)), but two syncs starting from the same version race, and the second wins. `rbx config rollback` recovers |

## Habits that keep this from happening

- **One writer at a time per env.** The cheapest fix by far.
- **Creates are reviewed, updates are automated.** An update is reversed by editing the TOML and syncing again; a create cannot be reversed and may spend Robux. Put `rbx shop sync --dry-run` in the pull request, let CI run updates, and have a person run the syncs that create.
- **Commit the lockfile in the same commit as the change that caused it**, so it reviews as one change.
- **Never `--env all` from two places at once.** It walks every env with one lockfile in memory.
- **Never hand-edit a lockfile** outside a conflict. A mistyped `id` cannot be recovered.
- **Run `rbx check --offline` on every pull request**, and `rbx check --env all` on a schedule to catch drift from Roblox. See [`rbx check`](check.md).
