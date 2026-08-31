# rbx data

Read, overwrite, copy and recover one data store entry.

Deliberately narrow. Browsing, diffing revisions visually and restoring by clicking are better done in [DataStoria](https://github.com/this-fifo/DataStoria)'s editor than in a terminal, and this does not try to replace it. What it covers is the scriptable half, plus the two things a single-universe GUI cannot do at all: copying between environments, and running from a script.

See [ops.md](../ops.md) for install, keys and the safety model.

Needs `universe-datastores.objects:read,update,create,list`, plus `universe-datastores.control:create,list` (the `control` scopes govern the **store**, `objects` govern its **entries**, and creating an entry in a store that does not exist yet needs both), plus `universe-datastores.versions:list,read` for `revisions` and `restore`, plus `universe-datastores.control:snapshot` for `snapshot`.

## Why overwriting, not deleting

Measured against a live universe, and the two are not symmetric:

| | after `set` / `reset` | after `DELETE` |
| --- | --- | --- |
| a normal read | the new value | nothing, 404 |
| the entry in a listing | present | present with `--show-deleted` |
| the previous value | **gone at once** | **readable for 30 days** |

Deleting is the obvious way to reset a player and the wrong one: the game then reads nothing, which behaves however your wrapper decides, and the old value stays readable for a month anyway.

The surprising half is the last row. Writing an entry four times leaves only the fourth: `listRevisions` returns one row and an earlier revision by id answers `404 Entry not found at revision`, even though the revision counter did increment.

**So by default an overwrite is unrecoverable through the API, and the backup file this writes before every write is not a convenience.**

There is one way to change that in advance, and it has to be in advance: see [snapshots](#snapshots).

## Resetting a player

`--template <path>` names the profile to write; without it, `reset` reads `playerdata.template.json` from the working directory. There is no built-in default profile, because a fresh profile is a fact about your game and inventing one would write a shape the game does not read.

```sh
# reads playerdata.template.json by default
rbx data --datastore PlayerData reset Player_156 --env prod

# or point at the one your game actually ships
rbx data --datastore PlayerData reset Player_156 --template assets/new-profile.json --env prod

# for real
rbx data --datastore PlayerData reset Player_156 --env prod --apply
```

The dry run prints the current value beside the new one. On `--apply` the current value is written to `.rbx/backups/prod/Player_156-20260815T091500Z.json` before anything is sent, then you are prompted.

### Where the backups go

```
.rbx/backups/<env>/<entry>-<UTC timestamp>.json
```

Beside `rbxplace.toml`, not in the working directory: the copy belongs to the project, so it is in the same place whether you ran the command from the repository root or from three directories down. One subdirectory per env, because the same entry key in staging and in prod are two different player profiles.

The timestamp is not decoration. A fixed name like `<entry>.backup.json` means resetting the same player twice overwrites the copy of the value the first reset destroyed, and that older file is exactly the one worth having. Two writes inside the same second get a `-2` suffix rather than replacing each other.

**Gitignore `.rbx/`.** These files are real player data. Add it next to your other ignores:

```gitignore
.rbx/
```

`--keep <n>` bounds the pile, defaulting to 10:

```sh
# keep the last 3 copies of this entry, delete older ones after the write
rbx data --datastore PlayerData reset Player_156 --env prod --keep 3 --apply
```

Retention counts the file it just wrote, applies to that entry only (one env's directory holds every key, and `--keep` on one player never evicts another's) and touches nothing it did not write. It is refused with `--backup` and `--no-backup`, which leave it nothing to do, and `--keep 0` is refused too: "no backup at all" is `--no-backup`, said plainly.

Nothing prunes on a schedule or in the background. A directory only shrinks during a write to that same entry, so a key you stop touching keeps its history until you delete it yourself.

### Skipping the local copy

`--backup <path>` writes it exactly there instead, creating no directory around it and pruning nothing near it. `--no-backup` means there is none:

```sh
rbx data --datastore PlayerData set Player_156 --value '{}' --no-backup --apply --yes
```

Two situations justify it. After [`data snapshot`](#snapshots), Roblox keeps the replaced value as a revision for 30 days, so the local copy is redundant for the next write to each key. And in a container with a read-only working directory there is nowhere to put it, without this flag the command fails before it sends anything, because the copy is written first on purpose.

Outside those, it throws away the only way back. The command says so on every run rather than only in this page, and the two flags cannot be combined: one names where the copy goes, the other says there is none.

## Removing an entry **(0.6.0+)**

`RemoveAsync`, from outside the game.

```sh
rbx data --datastore PlayerData delete Player_156 --env prod          # dry run
rbx data --datastore PlayerData delete Player_156 --env prod --apply
```

Despite the name it is the **gentler** of the two ways to start a player over. A normal read then answers nothing, so a game that builds a fresh profile when it finds none builds one, from its own template rather than from a copy of that template you have to keep in step. And the value survives: the entry stays in a listing with `--show-deleted`, and its last value stays readable through `data revisions` for thirty days. `set` and `reset` destroy it the moment they land.

The local copy is written first anyway, because thirty days is a deadline and a file is not. A key that does not exist is reported and nothing is sent.

Needs `universe-datastores.objects:delete`.

**One ordering matters.** A live session holding that profile in memory writes it back when it ends, undoing this. Delete while nobody is in the experience, or end the session from inside the game first. Resetting yourself mid-playtest is the in-game job, not this one.

## Reading and writing

```sh
rbx data --datastore PlayerData get Player_156 --env prod
rbx data --datastore PlayerData get Player_156 --env prod --out profile.json

rbx data --datastore PlayerData set Player_156 --value '{"coins":0}' --env prod --apply
rbx data --datastore PlayerData set Player_156 --file profile.json --env prod --apply

# atomic, unlike read-then-write: two grants at once both land
rbx data --datastore PlayerData increment Coins_156 --by 500 --env prod --apply
```

An overwrite **keeps the entry's `users` and `attributes`** unless you pass `--drop-metadata`. `users` is the association Roblox uses to answer a player's data request, and sending only `value` would sever it silently.

## Finding stores **(0.6.0+)**

The command to run when you do not yet know what to put in `--datastore`.

```sh
rbx data stores --env prod
rbx data stores --show-deleted --env prod
```

Experience-wide, so it takes neither `--datastore` nor `--scope`. Needs `universe-datastores.control:list`.

A store exists **from its first write**, not from the first `GetDataStore`, so a name that is absent here is a store the game has never written to. That also explains the names you did not choose: a game running in Studio writes wherever its own wrapper points, so a `-studio` twin of the live store is normal, and a wrapper library keeps its bookkeeping in a store of its own next to the data it manages.

## Finding keys

```sh
rbx data --datastore PlayerData list --env prod
rbx data --datastore PlayerData list --prefix Player_ --env prod
rbx data --datastore PlayerData list --show-deleted --env prod
```

Every subcommand also takes `--scope`, the data store scope, defaulting to `global`. Leave it alone unless your game writes with an explicit scope: a key written under one scope is invisible from another, so a wrong `--scope` reads as "entry not found" rather than as an error.

## Snapshots

The one thing that makes an overwrite survivable, and it only works if you do it **before** the write.

```sh
rbx data snapshot --env prod            # dry run
rbx data snapshot --env prod --apply
```

After a snapshot, the next write to *every* key in the experience keeps the value it replaced as a revision, guaranteed readable for 30 days. That is exactly the guarantee the table above says you do not normally get. It covers one overwrite per key: the second write of the day replaces the first without keeping it.

Roblox allows **one snapshot per experience per UTC day**. A second call the same day is not an error: it reports the standing snapshot's time and changes nothing.

That cap is why this takes `--apply` even though a snapshot can only ever add recoverability. Spending the day's snapshot early is not free: one taken at 09:00 protects the values as of 09:00, so a key written at 10:00 and again at 17:00 keeps the 09:00 value, not the 10:00 one. Take it immediately before the risky write, not at the start of the day out of habit.

Experience-wide, so it takes neither `--datastore` nor `--scope`. Needs `universe-datastores.control:snapshot`.

## Recovering

```sh
rbx data --datastore PlayerData revisions Player_156 --env prod
rbx data --datastore PlayerData revisions Player_156 --revision <id> --env prod
rbx data --datastore PlayerData restore Player_156 --revision <id> --env prod --apply
```

Expect fewer revisions than you wrote, for the reason above. This is mostly useful after a delete, where the value from before survives, or after a snapshot, which is what puts a revision there to find. To undo an overwrite with neither, use the backup file: `ls .rbx/backups/<env>/` lists them newest last, and putting one back is `set --file`:

```sh
rbx data --datastore PlayerData set Player_156 \
  --file .rbx/backups/prod/Player_156-20260815T091500Z.json --env prod --apply
```

## Copying between environments

The one a single-universe tool cannot do: pull a profile from production into staging to reproduce a bug on real data, or onto a test account.

```sh
rbx data --datastore PlayerData copy Player_156 --from prod --to staging --apply
rbx data --datastore PlayerData copy Player_156 --from prod --to dev --to-entry Player_999 --apply
```

Source and destination are both named explicitly and neither falls back to `--env`, so nothing is copied because a flag was forgotten. The **destination's** `users` is kept, not the source's: attaching one player's association to another player's key is not a copy anybody means to make.

## Comparing

```sh
rbx data --datastore PlayerData diff Player_156 --revisions <a>,<b> --env prod --open
rbx data --datastore PlayerData diff Player_156 --between prod,staging --open
```

Both sides are written to files and handed to a diff tool: `$RBX_DIFF_TOOL` if set, else `code --diff`, else `git diff --no-index`. Without `--open` the two paths are printed for you to open however you like.

That is DataStoria's best screen, obtained without writing a diff viewer, and it works for people who do not use VS Code.


## Ordered data stores

`rbx data ordered` is the leaderboard resource: a different Open Cloud resource from everything above, not a mode of it. Values are integers, ordering happens on Roblox's side, and there is **no revision history at all**.

That last point is why nothing here writes a backup file. The backups the rest of this page insists on exist because an overwrite to a standard store destroys a JSON document that only a local copy can bring back. An ordered entry is one integer, and there is nothing to reconstruct.

`--datastore` names the store, the same flag as above, and `--scope` applies.

```sh
# The top ten
rbx data ordered list --datastore Highscores --env prod

# The top 100, ascending, only scores between 1000 and 5000
rbx data ordered list --datastore Highscores --limit 100 --asc --min 1000 --max 5000

rbx data ordered get Player_156 --datastore Highscores
rbx data ordered set Player_156 4200 --datastore Highscores
rbx data ordered increment Player_156 -50 --datastore Highscores
rbx data ordered delete Player_156 --datastore Highscores
```

| Verb | What it does |
| --- | --- |
| `list` | The leaderboard. Descending by default: "the top players" is the reason the resource exists, so ascending is the case that takes a flag |
| `get <entry>` | One value. A key nobody has written prints a note, not an error |
| `set <entry> <value>` | Exact value, creating the entry when absent. `--no-create` refuses instead |
| `increment <entry> <amount>` | Atomic add. Negative subtracts |
| `delete <entry>` | Removes the entry. Deleting one that is not there is a no-op, not a failure |

`--limit`, `--min` and `--max` are all applied by Roblox, not after the fact: a listing sorted or filtered locally would give the top of page one rather than the top of the store. `--min`/`--max` become one `filter` expression, which is the only comparison grammar the endpoint accepts.

**Reach for `increment` over `set` whenever more than one writer touches a key.** A read-then-set from two places loses one of the two updates; the increment endpoint does not.

`set`, `increment` and `delete` ask before writing, and `set` and `delete` name the current value in the prompt: there is no revision history to look it up in afterwards. `-y` / `--yes` skips the prompt.

**What is deliberately missing**: `snapshot`, `revisions`, `restore`, `diff`. Roblox offers none of them on this resource, and a command answering "not supported" for four of its verbs would be worse than not having them.

Scopes: `universe.ordered-data-store.scope.entry:read` for `list` and `get`, `:write` for the rest.

## Machine-readable output

`--json` on the four reads (`get`, `list`, `revisions` and `diff`) writes one JSON document to stdout and nothing else. Everything that is not the result (the revision line, the key count, "No entry", the unknown-key warning from `rbxplace.toml`) goes to stderr, so `jq` reads the pipe and a human still reads the terminal.

The four writes do not take it. `set`, `reset`, `restore`, `copy`, `increment` and `snapshot` all stop and ask before they act, and a format that owns stdout cannot stop and ask: the prompt would land in the document, or in a pipeline where nobody can answer it. So the flag is not there to be refused at runtime: it does not exist on those subcommands at all.

### The stored value is nested, not escaped

A player profile is already JSON, and it goes into the document as JSON, under a `value` key:

```json
{
  "schema_version": 1,
  "datastore": "PlayerData",
  "scope": "global",
  "entry": "Player_156",
  "found": true,
  "deleted": false,
  "revision_id": "08DEF1A1B5E3ADA9.0000000002.01",
  "value": {
    "coins": 500,
    "level": 12,
    "inventory": ["hat", "sword"]
  }
}
```

So `jq .value.coins` works, and a profile stored as `500` reads back as the number `500` rather than the string `"500"`.

The worry about nesting arbitrary data is a worry about *spreading* it. Nothing of ours lives inside `value`, so a profile with a `schema_version` key of its own is `.value.schema_version` and collides with nothing. Escaping the value into a string would have bought back a namespace that was never at risk, at the cost of `jq -r .value | jq .coins` and of every stored number becoming quoted.

### What these documents do not say

They read real player data, so they say no more than the human form already says out loud.

**`users` and `attributes` are not in any of them.** `users` is the `users/156` association Roblox answers a player's data request from. It is on every entry this command fetches, `data get` has never printed it, and a second player identifier landing in whatever collects your CI output is not a field anybody asked for. `path`, `etag` and `createTime` are absent for the duller version of the same reason: unprinted today, so unpromised today.

**`diff` carries paths, not values.** Both sides are already written to files; putting two profiles through the pipe as well would say more than the human form ever has.

### `data get --json`

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today. Refuse a version you do not understand |
| `datastore` / `scope` | string | The store this was read from, as `--datastore` and `--scope` named it |
| `entry` | string | The key that was asked for |
| `found` | boolean | False when there is no such key. Exit code is 0 either way, the same non-event the human form prints as "No entry" |
| `deleted` | boolean | True when the entry is soft-deleted and still readable. Roblox purges it thirty days after the delete |
| `revision_id` | string | The revision the value came from. **Absent** when Roblox did not say, and when there is no entry |
| `value` | any | The stored value, nested. **Absent** under `--out` and when there is no entry. A present `null` is a real answer: a stored `null` and an entry with no value cannot be told apart, and the game cannot tell either |
| `out` | string | Where `--out` wrote the value. **Absent** without `--out` |

### `data stores --json` **(0.6.0+)**

```json
{
  "schema_version": 1,
  "show_deleted": false,
  "limit": 100,
  "count": 2,
  "limit_reached": false,
  "stores": [
    { "id": "PlayerData-v1", "create_time": "2026-08-27T16:41:02Z", "deleted": false },
    { "id": "Tickets-prod", "create_time": "2026-08-27T16:41:01Z", "deleted": false }
  ]
}
```

No `datastore` or `scope` key: this is the document you read before you have either. `id` is what every other subcommand takes as `--datastore`. `create_time` is **absent** when the response omitted it. `deleted` is only ever true with `--show-deleted`, since nothing else returns a soft-deleted store.

### `data set --json`, and `reset`, `restore`, `delete` **(0.6.0+)**

```json
{
  "schema_version": 1,
  "datastore": "PlayerData",
  "scope": "global",
  "entry": "Player_156",
  "action": "set",
  "applied": true,
  "existed": true,
  "revision_id": "08DF077D....01",
  "backup": ".rbx/backups/prod/Player_156-20260831T163538Z.json"
}
```

**Requires `--yes`.** `--json` refuses to prompt and every write asks for a confirmation, so the pair would either draw a prompt into a pipe or quietly skip a confirmation. Clap refuses the combination rather than either.

`action` is the verb you asked for, not the one they share internally: `set`, `reset`, `restore`, `copy` or `delete`. `applied` is false for a dry run, which is a success that changed nothing, and telling those apart from the exit code alone is impossible. `existed` says whether the key was there before, so `set` reports whether it created one and `delete` reports whether it found anything to remove. `revision_id` is **absent** on a dry run and on a delete, and it is the field this document exists for: it is what `data revisions --revision` takes. `backup` is **absent** under `--no-backup` and when there was no previous value to copy.

### `data list --json`

```json
{
  "schema_version": 1,
  "datastore": "PlayerData",
  "scope": "global",
  "prefix": "Player_",
  "show_deleted": false,
  "limit": 100,
  "limit_reached": false,
  "count": 2,
  "entries": [{ "id": "Player_156" }, { "id": "Player_881" }]
}
```

`prefix` is **absent** when the listing was unfiltered, which is not the same as a prefix of `""`. `limit_reached` says the run stopped at `--limit` rather than at the end of the store, so a script knows to raise it instead of concluding the store is small. A prefix that matches nothing is an empty `entries` array and exit 0, never silence.

Entries are objects rather than bare strings, so `.entries[].id` keeps working the day a listing carries a second field.

### `data revisions --json`

Two documents, and `--revision` is what picks between them. Without it, the list:

```json
{
  "schema_version": 1,
  "datastore": "PlayerData",
  "scope": "global",
  "entry": "Player_156",
  "count": 2,
  "revisions": [
    {
      "revision_id": "08DEF1A1B5E3ADA9.0000000002.01",
      "create_time": "2026-08-15T09:15:00.1234567Z",
      "state": "DELETED",
      "deleted": true
    },
    {
      "revision_id": "08DEF1A1B5E3ADA9.0000000001.01",
      "create_time": "2026-08-14T11:02:33.0000000Z",
      "state": "ACTIVE",
      "deleted": false
    }
  ]
}
```

`create_time` keeps Roblox's full precision, where the table shortens it to the second. `deleted` is derived from `state` so a consumer does not keep its own list of spellings.

With `--revision <id>`, that revision's value instead, under the same `value` rule as `get`:

```json
{
  "schema_version": 1,
  "datastore": "PlayerData",
  "scope": "global",
  "entry": "Player_156",
  "revision_id": "08DEF1A1B5E3ADA9.0000000001.01",
  "value": { "coins": 500 }
}
```

### `data diff --json`

```json
{
  "schema_version": 1,
  "datastore": "PlayerData",
  "scope": "global",
  "entry": "Player_156",
  "left": {
    "label": "prod-Player_156",
    "path": "/tmp/prod-Player_156.json",
    "env": "prod"
  },
  "right": {
    "label": "staging-Player_156",
    "path": "/tmp/staging-Player_156.json",
    "env": "staging"
  }
}
```

Each side carries `revision` under `--revisions` and `env` under `--between`, exactly one of the two, so a consumer reads which comparison it got rather than parsing `label` apart.

`--json` and `--open` are refused together. `--open` hands stdout to `git diff --no-index` and the terminal to `code --diff`, either of which would write somebody else's output into the document.

```sh
# hand the pair to your own tool
rbx data --datastore PlayerData diff Player_156 --between prod,staging --json \
  | jq -r '.left.path, .right.path' | xargs delta
```

## Exporting

`get --out` writes one value. For history, `list` the keys and `get` each: entry listings only carry ids, so reading values is one call per entry and this makes no attempt to hide that cost.

```sh
rbx data --datastore PlayerData list --prefix Player_ --limit 500 --env prod --json \
  | jq -r '.entries[].id' \
  | while read -r key; do
      rbx data --datastore PlayerData get "$key" --env prod --json > "backup-$key.json"
    done
```
