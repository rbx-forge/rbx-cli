# Working in a team

Every other page here describes one person driving one repository. This one is
about two: two humans on diverged branches, or a human and a CI job, both
running a sync against the same universe. What git shows you afterwards is a
conflict in a lockfile, and how you resolve it decides whether the next `sync`
*updates* a resource or *creates a second one*.

Read this before hand-resolving a `rbxshop.lock.toml` conflict. `rbx shop` has
no delete verb, and neither does the Roblox surface it calls: the client exposes
list, get, create and update for passes, badges and products, and nothing else.
A duplicate game pass created by a bad merge cannot be removed by this tool.
The best you get afterwards is `for_sale = false` (passes, products) or
`enabled = false` (badges).

## The files a second person can collide with

| File | Written by | Committed | Where conflicts come from |
| --- | --- | --- | --- |
| `rbxshop.lock.toml` | `shop sync`, `shop pull`, `shop init --from-remote`, `shop rename`, `import` | yes | a resource added or changed on both sides |
| `rbxmeta.lock.toml` | `meta sync`, `meta pull`, `meta init --from-remote`, `import` | yes | the same field patched on both sides, plus the thumbnail array |
| `rbxconfig.lock.toml` | `config sync`, `config pull`, `import` | yes | every sync, unconditionally (see below) |
| `rbxapikey.lock.toml` | the `apikey` verbs | no, it stores secrets | not applicable once you have gitignored it, which `apikey create` refuses to proceed without |
| the codegen folder | `shop codegen`, `shop sync` | usually | never worth merging; regenerate |

`rbx place` has no lockfile at all, so nothing there can conflict.

## Why conflicts are rarer than you would expect

Everything keyed in a lockfile is a `BTreeMap`, so it serializes in sorted key
order: envs, then passes, badges and products within an env. Two people adding
differently named resources write to different regions of the file and git
merges both without asking. Empty maps and unset optional fields are skipped
entirely rather than written as empty tables, so nothing churns just because a
field went unused.

The lockfiles are written by the TOML serializer straight through
`std::fs::write`, with no platform newline translation, so a Windows checkout
and a Linux CI runner produce the same bytes for the same state. That holds
only if git leaves them alone: a checkout with `core.autocrlf=true` and no
`.gitattributes` hands the working tree CRLF, the next write puts LF back, and
a one-line change arrives as a whole-file diff. `* text=auto` in the consuming
repository's `.gitattributes` settles it.

Two things are genuinely not stable across runs, and both are worth knowing
before you read a diff:

- **`rbxconfig.lock.toml` rewrites `synced_at` and `revision_id` on every
  `config sync` and every `config pull`.** `synced_at` is the wall clock at the
  moment of the write, `revision_id` is the `configVersion` Roblox handed back.
  Two branches that each synced the same env will *always* conflict on those two
  lines, whatever else they did. Those lines carry no decision: pick either side
  (the one from the later sync, if you can tell) and move on.
- **`rbxmeta.lock.toml` stores `media.thumbnails` as an ordered array, not a
  keyed table.** Position is meaningful there because it mirrors the display
  order on Roblox. A textual merge of two arrays is positional and there is no
  key to align on, so do not hand-merge it. Take one side whole and let
  `rbx meta pull` restate it.

## The rule that decides everything: a missing lock entry means "create"

`rbx shop` builds its plan by walking the resolved config and looking each key
up in the lockfile's env section. No entry for that key is the entire test for
"this does not exist yet", and the action it produces is `Create`, which is a
bare `POST` to Roblox. Nothing looks for an existing remote resource with the
same name first.

Roblox does not make display names unique. `rbx shop pull` carries a whole
duplicate-name resolution path precisely because two passes called `VIP` in the
same universe are a state Roblox is happy to be in. So a `Create` issued for a
resource that already exists does not fail. It succeeds, and you now own two.

Three consequences:

1. **Never resolve a `rbxshop.lock.toml` conflict by deleting entries.** Every
   entry you drop is one `Create` on the next sync.
2. **`git checkout --ours` / `--theirs` on the whole file is only safe when one
   side is a strict superset of the other.** If each side recorded ids the other
   lacks, taking one side whole throws away real ids.
3. **Deleting the lockfile and re-running `sync` is the worst version of the
   same mistake.** For `rbx meta` and `rbx config` that is merely wasteful
   (both re-send state they already had). For `rbx shop` it recreates every
   pass, badge and product in the config. Badge creation in particular is a
   billable operation on Roblox's side: the create call carries an
   `expectedCost`, `rbx shop sync` exposes it as `--badge-cost` (default `0`),
   and the scope the call needs is named `manage-and-spend-robux`. The command
   that rebuilds a shop lockfile from reality is `rbx shop pull`, not `sync`.

## Recipe: git reports a conflict in `rbxshop.lock.toml`

**Keep the union of resource entries, then let `pull` reconcile the values.**

```sh
# 1. Look at what each side has, entry by entry.
git diff --diff-filter=U rbxshop.lock.toml

# 2. Resolve toward the superset: keep every [envs.<env>.passes.<key>],
#    [envs.<env>.badges.<key>] and [envs.<env>.products.<key>] table that
#    appears on either side. Where both sides have the same key with
#    different field values, either set of values is fine: pull overwrites
#    them from Roblox in the next step. What must survive is the `id`.
git add rbxshop.lock.toml

# 3. Ask Roblox what is actually there. Read it first.
rbx shop pull --env <name> --dry-run
rbx shop pull --env <name>

# 4. Confirm config and lockfile agree again.
rbx shop check --env <name>
```

`rbx shop pull` refetches every pass, badge and product in the universe and
rebuilds that env's lockfile section from the answer. It matches a remote
resource to a config key by **the id already recorded in your lockfile**, and
falls back to the resource's display name for anything it does not recognise.
It also writes what it found back into `rbxshop.toml`, as a base entry or an
`[envs.<name>.*]` overlay. That last part is why step 3 has a `--dry-run` in
front of it: pull is not a lockfile-only operation.

### The trap in step 2, and why "keep the union" is not optional

Suppose the entry you dropped was for a resource whose config key differs from
its display name, which is what setting `name = "..."` does:

```toml
[passes.VIPPass]
name = "VIP Access"
price = 499
```

With no lock entry, pull has no id to match on. It files the remote resource
under its display name, `VIP Access`, and adds *that* as a new config entry.
Your original `VIPPass` key still has no lock entry. The next `rbx shop sync`
therefore creates a second pass, and now `VIP Access` and `VIPPass` are two
paid products in the same universe.

When the config key and the display name happen to be identical, the fallback
lands on the right key and pull does reconstruct the entry correctly. That is
the common case, and it is exactly why the failure is easy to miss in testing
and expensive in production. Do not lean on it.

## Which side to keep, per file

| File | Keep | Then run |
| --- | --- | --- |
| `rbxshop.lock.toml` | the union of resource entries; never fewer ids than either side had | `rbx shop pull --env <name> --dry-run`, then without it |
| `rbxmeta.lock.toml` | either side whole (it records last-applied metadata, not ids you can lose) | `rbx meta sync --env <name> --dry-run` to see what is now considered pending, or `rbx meta pull --env <name>` to take Roblox's word |
| `rbxconfig.lock.toml` | either side; `synced_at` and `revision_id` are bookkeeping | `rbx config sync --env <name> --dry-run`, which prints the live-versus-local diff before writing anything |
| the codegen folder | neither; take either side to clear the marker | `rbx shop codegen`, then `rbx shop codegen --check` and re-stage |

Generated Luau never needs a semantic merge: `rbx shop codegen` and
`rbx env gen-module` rebuild it offline from `rbxshop.toml`, `rbxshop.lock.toml`
and `rbxplace.toml`. See
[Guarding generated files](shop.md#guarding-generated-files).

## `check` is offline for shop and meta, and that changes what green means

Of the checks `rbx check` runs, only `config/live` talks to Roblox. `shop/lockfile`,
`shop/codegen` and `meta/lockfile` compare committed files against committed
files, and `rbx shop check` and `rbx meta check` do the same on their own.

So after a bad merge, a clean `rbx check` means **"the config and the lockfile
agree with each other"**. It does not mean the lockfile agrees with Roblox, and
it cannot: it never asked. A lockfile that lost an entry and a config that still
declares the resource will not report clean (the resource shows as `1 to
create`), but a lockfile that lost *both* the lock entry and the config entry
reports perfectly clean while the resource sits unmanaged in the universe.

The commands that compare against Roblox are `rbx shop pull --dry-run`,
`rbx meta pull --dry-run` and `rbx config check`. Treat the first two as the
audit step after any lockfile merge you were not certain about.

## Is concurrent sync safe?

One statement per tool, from what the code does rather than what would be nice.

### `rbx shop sync`: unsafe for creates, benign for updates

The whole workspace is sequential by design (see
[ARCHITECTURE.md](https://github.com/rbx-forge/rbx-cli/blob/main/ARCHITECTURE.md)), so there is no concurrency *inside* one
run. Two runs against the same universe are a different matter.

- **Two runs that both plan `Create` for the same key produce two remote
  resources.** This is the failure that matters. There is no pre-flight
  existence check, no uniqueness constraint on Roblox's side, and no way to
  delete the loser afterwards.
- **Updates are addressed by the id in the lockfile, so they cannot duplicate
  anything.** Two concurrent updates to the same resource are last-writer-wins
  per call: whichever `update_game_pass` lands second is the state you keep.
- **Icon upload is not a read-modify-write.** The hash is taken from the local
  file and the new asset id comes back in the response, so concurrent uploads
  race on the result, not on a counter. Nothing is corrupted; the later upload
  wins.
- **The lockfile is saved after every single resource**, not once at the end, so
  a run that dies halfway keeps the ids of what it already created. That is
  crash safety, not concurrency safety: it makes an interrupted run resumable,
  it does not stop a second run from creating duplicates.
- **`rbx shop sync` does not refuse when the lockfile's recorded `universe_id`
  disagrees with the env it resolved.** It overwrites the recorded value.
  `rbx shop check` prints the mismatch, and `rbx meta sync` and `rbx config sync`
  refuse outright, so shop is the odd one out here. After a merge that could
  have mixed env sections, run `rbx shop check` before `rbx shop sync`.

### `rbx meta sync`: last-writer-wins, no duplication

Metadata is a set of `PATCH` calls against one universe and one place, so two
concurrent runs interleave field writes and the last call wins per field.
Nothing is created and nothing gets an id, so nothing duplicates. The lockfile
is saved after every successful call, and the visibility ordering rule (public
first, private last) is enforced within a run.

Thumbnails are the exception worth flagging: a sync issues deletes, then
uploads, then a single reorder built from the ids its *own* lockfile now holds.
A concurrent run's uploads are not in that list. Whether Roblox treats a reorder
list that omits existing thumbnails as a reorder or as a truncation is not
something this repository establishes, so **treat concurrent `meta sync` runs
that both touch thumbnails as unverified rather than safe**, and run them one at
a time.

### `rbx config sync`: last-writer-wins over the whole document

`rbx config sync` calls the API's overwrite-and-publish operation with the full
local entry set. It fetches the live config first, but only to print the diff
for your approval: nothing compares the lockfile's `revision_id` against the
live one, and nothing refuses on a mismatch. The `revision_id` in
`rbxconfig.lock.toml` is written and never read back as a precondition.

The practical consequence: **if you sync from a branch that does not have your
colleague's entries, those entries are removed from the live config**, silently,
because the publish replaces the document rather than merging into it.

Recovery exists and is the reason this is a warning rather than a prohibition:
`rbx config versions` lists past revisions and `rbx config rollback` restores
one. Reach for `rbx config pull` first if what you want is to bring the branch
up to date instead.

### `rbx apikey`

`rbxapikey.lock.toml` holds key secrets and belongs in your `.gitignore`
(`rbx apikey create` refuses to create a key whose secret would land in a file
git is not ignoring) so it does not take part in merges at all. It is also the one lockfile
outside the shared version-and-migrate machinery: a version mismatch is refused
with "delete the file and re-run" rather than migrated.

Its own guard against a different kind of concurrency already exists: each
create, update or regenerate records the resolved universes, and a later run
that resolves a different set refuses to proceed. See
[docs/apikey.md](apikey.md).

Concurrent `apikey` runs against the same Roblox account were not analysed for
this page. Treat them as unverified.

## Is there any locking or detection?

No. There is no lock server, no lease, no advisory file lock, and no `If-Match`
style precondition on any write in the reconciling tools. Nothing notices that
the lockfile on disk changed while a command was running, and nothing notices
that Roblox moved under it.

Whether that stays true is a separate question this page does not answer. What
is true today is that the only mitigation the tool offers is the one you apply
yourself, which is what the conventions below are.

## Habits that keep this from happening

**One writer at a time, per env.** The cheapest fix by a distance. Everything
below is a way of making that easier to hold.

Not "CI owns prod, humans own dev", which is the version of this rule that does
not survive contact with `rbx shop`. Creating a pass, a badge or a product is a
`Create` Roblox cannot undo, badge creation spends Robux, and the next habit on
this list says creates are the reviewed step. A reviewed irreversible purchase
is a human act, so the human does sync prod. `rbx meta` says the same from the
other direction: `visibility`, `allow_copying`, `studio_access_to_apis_allowed`
and `server_fill` have no Open Cloud endpoint, so CI cannot write them without
a session cookie on the runner, which [docs/cookie.md](cookie.md) spends a page
arguing against.

The distinction that does hold is **create versus update**. Updates are
addressed by id, are reversible by editing the TOML and syncing again, and are
the right thing to automate: let CI own them, on every env. Creates are none of
those things and belong to a person who read the `--dry-run` output. Arrange
who runs what around that, and when a human does sync prod, make sure nobody
else is doing it that hour.

**Pull before you sync, and commit the lockfile with the change that caused
it.** A lockfile change is not incidental noise to be swept into a later commit:
it is the record of what the sync did. In the same commit as the `rbxshop.toml`
edit, it reviews as one intention. Committed separately, it is a mystery diff.

**Make creates the reviewed step.** `sync` prints `N to create, M to update`
before it does anything, and `--dry-run` prints it without doing anything. An
update is reversible by editing the TOML and syncing again. A create is not.
Put `rbx shop sync --dry-run` in the pull request, not just in the terminal of
whoever runs it.

**Do not run `--env all` from two places at once.** `rbx shop sync --env all`
walks every env in `rbxplace.toml` sequentially, holding one lockfile in memory
across all of them and saving as it goes. Two such runs multiply every
per-env hazard above by the number of envs. `rbx meta check`, `rbx meta sync`,
`rbx meta pull` and `rbx place upload` walk a plural `--env` the same way, and
the same caution applies to each. (`rbx config` still acts on one env per
invocation and does not accept `--env all`.)

**Never hand-edit a lockfile outside a conflict resolution.** They are
tool-owned state. The values a human can safely retype are none of them, and
the `id` fields are the ones a typo makes unrecoverable.

**Let CI run `rbx check` on every pull request, and a `pull --dry-run` on a
schedule.** `rbx check --offline` catches config-versus-lockfile drift with no
credentials and is fast enough for a pre-commit hook. It does not catch
lockfile-versus-Roblox drift, which is what a nightly `rbx shop pull --env all
--dry-run` is for. See [docs/check.md](check.md).

## Related

- [docs/check.md](check.md) is the CI contract and the exit codes
- [docs/shop.md](shop.md) covers the lockfile format and the codegen guard
- [docs/meta.md](meta.md) covers the sync ordering and media hashing
- [docs/config.md](config.md) covers revisions and rollback
- [ARCHITECTURE.md](https://github.com/rbx-forge/rbx-cli/blob/main/ARCHITECTURE.md) explains why every run is sequential
