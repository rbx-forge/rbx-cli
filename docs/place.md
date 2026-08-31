# rbx place

Upload, download, and rollback Roblox place files via the Open Cloud API.

`rbx place` manages `.rbxl` files across multiple environments (prod, staging, dev) defined in a shared `rbxplace.toml`. It handles Team Create locks gracefully and can require confirmation before writing to sensitive environments.

## Features

- **Multi-environment** - Define prod, staging, dev (and any others) in a single `rbxplace.toml`
- **Upload** - Push a `.rbxl` to one place, every place in an environment, or every environment at once with `--env all`
- **Download** - Fetch the latest or a specific version of a place file
- **Promote** - Copy a place from one environment to another, optionally broadcasting to all target places
- **Rollback** - Revert a place to a previous version with an interactive selector
- **Version history** - List recent versions with published status and timestamps
- **Team Create detection** - Clear error when a place is locked by an active Studio session
- **Confirmation guard** - Per-environment `confirm = true` prompts before write operations, once per run rather than once per environment
- **Fetch** - Auto-populate `rbxplace.toml` from live Roblox universe
- **JSON** - `--json` on `versions`, `places`, `upload`, `promote` and `rollback` writes one document to stdout and nothing else, with documented field names, for `jq` and CI

> Generating the env module lives in [`rbx env gen-module`](./env.md), the command that owns `rbxplace.toml`.

## Quick start

Create a `rbxplace.toml` at the root of your project:

```toml
[prod]
universe_id = 9876543210
confirm = true
places.main = 123456789012345

[staging]
universe_id = 9876543211
places.main = 234567890123456

[dev]
universe_id = 9876543212
places.main = 345678901234567
```

Then upload a build:

```sh
rbx place upload --env staging --file build.rbxl
rbx place upload --env prod --file build.rbxl   # prompts for confirmation
```

## Commands

<details markdown="1">
<summary><code>rbx place upload</code></summary>

Upload a `.rbxl` file to one or all places in an environment. By default, uploads are saved as drafts (not published live).

```sh
rbx place upload --env staging --file build.rbxl           # save as draft
rbx place upload --env prod --place lobby --file build.rbxl --published  # publish live
rbx place upload --env prod --all-places --file build.rbxl
rbx place upload --env all --file build.rbxl --yes          # every env in the file
```

| Flag | Description |
| --- | --- |
| `--env` | Target environment (required). `all` for every env in `rbxplace.toml`, or a `[groups]` name for the envs it lists |
| `--file` | Path to the `.rbxl` file (required) |
| `--place` | Place name to upload to (defaults to the only place if unambiguous) |
| `--all-places` | Upload to every place defined in the environment |
| `--published` | Publish immediately (default: save as draft) |
| `--json` | Write the result to stdout as one JSON document instead of the progress lines |

By default, uploads are saved as drafts. Use `--published` to publish live.

If the target place has an active Team Create session, the upload fails immediately with a clear message rather than returning a generic error.

If the environment has `confirm = true`, a confirmation prompt is shown before uploading.

### `--env all` and groups **(0.5.0+)**

A plural `--env` uploads the same file to every env it names, one after another: `all` walks the file's envs in alphabetical order, a group walks its members in the order they were declared. `--place` and `--all-places` are resolved inside each env, so `--all-places` over `--env all` is every place of every env.

Four things follow from that, and all four are deliberate:

**Everything is resolved before the first byte goes out.** A `--place` name one env does not declare fails the whole run rather than failing it after the first two envs are already written.

**The file is read once.** One buffer is shared by every upload, so `--env all` costs one read of a multi-megabyte `.rbxl`, not one per env.

**One confirmation covers the lot**, and it appears if *any* target env has `confirm = true`. A prompt per env would be reached mid-walk, after writes had already landed somewhere else, which is too late for an answer to mean anything. The prompt names every env it is about to write to. Under `--json`, which cannot prompt, the run is refused before anything is written and stdout stays empty.

**The walk stops where it failed.** The envs before it keep the versions they were given, the env that failed reports why, and the envs after it are never asked. The process exits non-zero.

```
$ rbx place upload --env nonprod --file build.rbxl --yes

env: dev
Uploading build.rbxl (2481.3 KB) → main [dev]
  Universe: 9876543210
  Version type: saved

  main (123456789012345) ... v41

env: staging
Uploading build.rbxl (2481.3 KB) → main [staging]
  Universe: 9876543211
  Version type: saved

  main (234567890123456) ... v174

Upload complete.
```

The `env:` header only appears when there is more than one target, so a single-env run's output is exactly what it has always been.

The other commands here act on one env by construction and refuse a plural selector rather than accepting it and ignoring it: `download` writes to one `--out` path, and `promote` names its two envs itself with `--from` and `--to`.

### `--json`

```sh
rbx place upload --env staging --file build.rbxl --json
```

```json
{
  "schema_version": 1,
  "command": "upload",
  "ok": true,
  "env": "staging",
  "universe_id": "9876543211",
  "published": false,
  "place_id": "234567890123456",
  "version": "173",
  "results": [{ "place": "main", "place_id": "234567890123456", "version": "173" }]
}
```

The fields are shared with `promote` and `rollback` and are described once in [Write documents](#write-documents).

A plural `--env` emits a different envelope, described in [Write documents](#write-documents): the document above, one per env, under a `results` array. A single env emits the document above unchanged, whatever else is in `rbxplace.toml`.

`--json` cannot prompt, so an environment with `confirm = true` needs `--yes`; without it the command fails with a message on stderr naming the flag and writes nothing to stdout.

```sh
VERSION=$(rbx place upload --env staging --file build.rbxl --json | jq -r .version)
```

</details>

<details markdown="1">
<summary><code>rbx place download</code></summary>

Download a place file from Roblox.

```sh
rbx place download --env prod
rbx place download --env prod --version 42 --out backup.rbxl
rbx place download --env staging --published
rbx place download --env staging --saved
```

| Flag | Description |
| --- | --- |
| `--env` | Target environment (required unless `--place-id` is given). One env: `all` and a group name are refused, because every download would land on the one path `--out` names |
| `--place` | Place name (defaults to the only place if unambiguous) |
| `--place-id` | A place id instead of an env and a name. Skips `rbxplace.toml`; global flag |
| `--version` | Specific version number to download (default: latest) |
| `--published` | Download the latest published version specifically |
| `--saved` | Download the latest saved (draft) version specifically |
| `--out` | Output path (default: `<place_id>.rbxl`) |

</details>

<details markdown="1">
<summary><code>rbx place promote</code></summary>

Promote a place from one environment to another. Downloads the source place in-memory and uploads it to the target. Without `--all-places`, the same-named place is targeted in the destination environment.

```sh
rbx place promote --from staging --to prod                         # latest → matching place
rbx place promote --from staging --to prod --all-places            # latest → every place in prod
rbx place promote --from staging --to prod --from-published        # latest published version
rbx place promote --from dev --to staging --version 42 --published # specific version, publish live
rbx place promote --from staging --to prod --log deploy.json       # write traceability log
```

| Flag | Description |
| --- | --- |
| `--from` | Source environment (required) |
| `--to` | Target environment (required) |
| `--place` | Source place name (defaults to the only place if unambiguous) |
| `--all-places` | Upload to every place defined in the target environment |
| `--version` | Specific source version to promote |
| `--from-published` | Promote the latest published version from the source |
| `--from-saved` | Promote the latest saved (draft) version from the source |
| `--published` | Publish immediately on the target (default: save as draft) |
| `--log` | Path to a JSON file for traceability logging (merged, not overwritten) |
| `--json` | Write the result to stdout as one JSON document instead of the progress lines |

If the target environment has `confirm = true`, a confirmation prompt is shown before uploading.

`--env` is not how promote names an env: `--from` and `--to` are, one each. A plural `--env` (`all`, or a group) selects nothing here and is refused rather than silently ignored. Run promote once per pair of envs.

### `--all-places` is a broadcast, not a plural

Without it, promote **maps by name**: the source place is resolved, and the same key is looked up in the target env. `main` goes to `main`, `lobby` goes to `lobby`, and a target env missing that name is an error. That is the default and it is what people usually mean.

`--all-places` does something else. Every place in the target env receives the **same bytes**, downloaded once from the single source place:

```sh
# prod's main, lobby and arena all become copies of staging's main
rbx place promote --from staging --to prod --all-places
```

There is no name matching in that path. It is occasionally what you want (several places that really are the same file) and it is unrecoverable when it is not: each target gets a new version, those version numbers are real, and undoing it is one rollback per place.

So the confirmation says it outright rather than only listing the targets:

```
⚠ Promote staging/main v172 → prod (arena, lobby, main)? This will save as draft.
  Every one of them is overwritten with staging/main, not with its own counterpart.
```

If what you want is "promote every place to its same-named counterpart", that is a different operation and this flag is not it. Run promote once per place, or leave the flag off and let the name mapping do it.

When `--log` is provided, a JSON file is written (or updated) after a successful promote. Only the promoted places are updated in the file; other entries are preserved:

```json
{
  "main": {
    "deployedAt": "2026-05-14T15:30:00+01:00",
    "staging": { "universeId": 9876543210, "placeId": 123456789012345, "version": 172 },
    "production": { "universeId": 9876543211, "placeId": 234567890123456, "version": 27 }
  }
}
```

### `--json`

The same information `--log` files away, on stdout, without a file. `--log` is still honored when both are given; the "Log written" line moves to stderr so the document stays parsable.

```sh
rbx place promote --from staging --to prod --from-published --published --yes --json
```

```json
{
  "schema_version": 1,
  "command": "promote",
  "ok": true,
  "env": "prod",
  "from_env": "staging",
  "universe_id": "9876543211",
  "published": true,
  "source_place": "main",
  "source_place_id": "123456789012345",
  "source_version": "172",
  "place_id": "234567890123456",
  "version": "27",
  "results": [{ "place": "main", "place_id": "234567890123456", "version": "27" }]
}
```

`source_version` is the version that was actually promoted, resolved before anything is downloaded, so `--from-published` and a bare latest both report the number they picked rather than the flag that picked it. See [Write documents](#write-documents) for the rest of the fields.

</details>

<details markdown="1">
<summary><code>rbx place rollback</code></summary>

Roll back a place to a previous version. Without `--version`, shows an interactive selector with recent versions.

```sh
rbx place rollback --env prod               # interactive selector
rbx place rollback --env prod --version 42  # direct rollback
rbx place rollback --env prod --count 20    # show 20 versions in selector
```

| Flag | Description |
| --- | --- |
| `--env` | Target environment (required) |
| `--place` | Place name (defaults to the only place if unambiguous) |
| `--version` | Version to roll back to (skips interactive selector) |
| `--count` | Number of recent versions to show in selector (default: `10`) |
| `--json` | Write the result to stdout as one JSON document instead of the progress lines |

Rollback creates a new version on Roblox (it does not modify history). If the place is locked by Team Create, the operation fails immediately with a clear message.

If the environment has `confirm = true`, a confirmation prompt is shown before rolling back.

### `--json`

```sh
rbx place rollback --env prod --version 37 --yes --json
```

```json
{
  "schema_version": 1,
  "command": "rollback",
  "ok": true,
  "env": "prod",
  "universe_id": "9876543210",
  "published": true,
  "source_version": "37",
  "place_id": "123456789012345",
  "version": "44",
  "results": [{ "place": "main", "place_id": "123456789012345", "version": "44" }]
}
```

Both versions are reported: `source_version` is the one restored, `version` is the new one Roblox created for it. `published` is always `true`, because rolling back republishes live.

`--version` is required under `--json`: the interactive selector is a prompt, and `--json` cannot prompt. Without it the command fails before fetching anything, with a message on stderr naming the flag.

</details>

<details markdown="1">
<summary><code>rbx place versions</code></summary>

List recent versions of a place.

```sh
rbx place versions --env prod
rbx place versions --env staging --count 50
rbx place versions --env prod --filter published
rbx place versions --env prod --filter saved
```

| Flag | Description |
| --- | --- |
| `--env` | Target environment (required unless `--place-id` is given) |
| `--place` | Place name (defaults to the only place if unambiguous) |
| `--count` | Number of versions to show (default: `20`, or `3` when `--filter` is `published`/`saved`) |
| `--filter` | Filter by version type: `all` (default), `published`, or `saved` |
| `--json` | Write the versions to stdout as one JSON document instead of the listing |

### `--json`

One JSON document on stdout, nothing else. Diagnostics, the unknown-key warning in particular, stay on stderr, so the document parses even when `rbxplace.toml` has something wrong with it.

```json
{
  "schema_version": 1,
  "env": "prod",
  "place": "main",
  "place_id": "123456789012345",
  "filter": "all",
  "count": 20,
  "count_reached": false,
  "versions": [
    { "version": "173", "published": true, "create_time": "2024-01-15T14:30:00Z" },
    { "version": "172", "published": false, "create_time": "2024-01-14T09:02:00Z" }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today. Refuse a version you do not understand |
| `env` | string | The environment asked for. **Absent** under a bare `--place-id`, which names a place without an env, the same rule `places` follows under `--universe-id` |
| `place` | string | The `rbxplace.toml` place name, after the `--place` defaulting rule. The place id under `--place-id`, which has no name to give |
| `place_id` | string | The place id, as a string |
| `filter` | string | `all`, `published`, or `saved`: the `--filter` in force |
| `count` | integer | The `--count` in force. A maximum, not a promise |
| `count_reached` | boolean | True when the walk stopped at `--count` rather than running out of versions. Raise `--count` to see the rest |
| `versions` | array of objects | Newest first, the order the listing prints |
| `versions[].version` | string | The version number. `--version` takes it back verbatim |
| `versions[].published` | boolean | Live, as opposed to a saved draft |
| `versions[].create_time` | string | Exactly what Roblox sent, RFC 3339. The listing rewrites this into `2024-01-15 14:30:00 UTC`; that is a rendering, and the document keeps the original |

A place with no versions is an empty `versions` array and exit 0, not an error: a consumer reads a zero off it.

```sh
rbx place versions --env prod --json | jq -r '.versions[] | select(.published) | .version' | head -1
```

</details>

<details markdown="1">
<summary><code>rbx place places</code></summary>

List all places in a universe. Shows which places are configured vs missing from `rbxplace.toml` if using `--env`.

```sh
rbx place places --env prod                    # list places, show which are in toml
rbx place places --universe-id 9876543210     # list places without config
```

| Flag | Description |
| --- | --- |
| `--env` | Environment name (reads universe from toml, shows config status) |
| `--universe-id` | Universe ID override (one-shot listing without toml) |
| `--json` | Write the places to stdout as one JSON document instead of the listing |

### `--json`

```json
{
  "schema_version": 1,
  "env": "prod",
  "universe_id": "9876543210",
  "places": [
    {
      "place_id": "123456789012345",
      "display_name": "Main Place",
      "max_player_count": 50,
      "place": "main",
      "configured": true
    },
    { "place_id": "987654321", "display_name": "Test Arena", "configured": false }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today |
| `env` | string | The environment whose entry named the universe. **Absent** under a bare `--universe-id` |
| `universe_id` | string | The universe listed, as a string |
| `places` | array of objects | One per place Roblox reports, in the order it returned them |
| `places[].place_id` | string | The place id. **Absent** when Roblox returned a path it could not be read out of, which the listing renders as `?` |
| `places[].display_name` | string | The name Roblox shows, which is not the `rbxplace.toml` key |
| `places[].max_player_count` | integer | **Absent** when Roblox did not report one |
| `places[].place` | string | The `rbxplace.toml` key this place is mapped to, the name `--place` takes. **Absent** when the file does not have it |
| `places[].configured` | boolean | Whether the file has this place, the fact the listing marks `NOT in toml`. **Absent**, rather than false, under a bare `--universe-id`: with no config in play the question has no answer |

```sh
rbx place places --env prod --json | jq -r '.places[] | select(.configured | not) | .place_id'
```

</details>

<details markdown="1">
<summary><code>rbx place fetch</code></summary>

Fetch all places from a universe and update `rbxplace.toml`. Existing place keys are preserved where the ID already matches. New places get keys generated from their Roblox display names.

```sh
rbx place fetch --env prod              # dry-run: shows what would be written
rbx place fetch --env prod --write      # writes to rbxplace.toml
rbx place fetch --env prod --universe-id 9876543210 --write  # override universe
```

| Flag | Description |
| --- | --- |
| `--env` | Environment section to update in `rbxplace.toml` (required) |
| `--universe-id` | Universe ID override (uses `rbxplace.toml` value if omitted) |
| `--write` | Write changes to `rbxplace.toml` (default: dry-run) |

</details>

## Write documents

`upload`, `promote`, and `rollback` share one `--json` envelope. It is a receipt: it reports what was written, in the order it was written, with the version number Roblox assigned to each place. An `upload` that named several envs emits one receipt per env, wrapped: see [One document per env](#one-document-per-env-under-env-all).

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today |
| `command` | string | `upload`, `promote`, or `rollback`, so a mixed stream of receipts can be dispatched on |
| `ok` | boolean | False when a target failed. The exit code says the same thing; this is here so a consumer that captured stdout does not have to plumb `$?` through as well |
| `env` | string | The environment written to. For `promote`, the target |
| `from_env` | string | The source environment of a `promote`. **Absent** otherwise |
| `universe_id` | string | The universe written to |
| `published` | boolean | Whether the new versions are live. Always `true` for `rollback` |
| `source_place` / `source_place_id` | string | Where a `promote` read its bytes, after `--place` defaulting. **Absent** otherwise |
| `source_version` | string | The version the new one was made from: the promoted source version, or the version rolled back to. **Absent** for `upload`, whose source is a local file |
| `place_id` / `version` | string | The single-target shortcut: the place written and the version it received. **Absent** under `--all-places`, and absent when nothing was written |
| `results` | array of objects | One entry per place that got a new version, in write order. Empty when the first target failed |
| `results[].place` | string | The `rbxplace.toml` key |
| `results[].place_id` | string | The place id |
| `results[].version` | string | The version Roblox assigned to this write |
| `error` | string | Why the run stopped. **Absent** when `ok` is true. The same text is on stderr, where it is the process's error message |

Three rules are worth stating outright, because scripts depend on them:

**A run that fails partway still emits a document.** `place upload --all-places` can write two places and then hit a Team Create lock on the third. Those two versions exist and cannot be taken back, so `results` reports them, `ok` is false, `error` says what stopped it, and the process still exits non-zero. A deploy log that loses a write that happened is worse than no log.

A failure *before* the first write, on the other hand, writes nothing to stdout at all: an unknown environment, a refused confirmation, a source version that does not exist. Nothing happened, and an empty stdout next to a non-zero exit says so without ambiguity.

**The shape follows the invocation, never the data.** A single-target run fills the `place_id` and `version` shortcuts next to `results`; an `--all-places` run fills `results` only, even against an environment with exactly one place. This is the rule `rbx env get --json` uses for `value`, and it exists so a filter cannot start working by accident and break when a second place is added.

**Ids and version numbers are strings.** They identify an asset rather than count anything, a place id exceeds 2^53, and a consumer that parses them as JSON numbers would round them. Keeping versions in the same form means the output of one command feeds the input of the next without a conversion:

```sh
VERSION=$(rbx place upload --env prod --file build.rbxl --published --yes --json | jq -r .version)
rbx servers list --env prod --version "$VERSION" --json
```

`--json` never prompts. Every write here has a point where it would stop and ask, and under `--json` that question fails instead, with a message on stderr naming the flag that answers it: `--yes` for a `confirm = true` environment, `--version` for the rollback selector.

### One document per env, under `--env all`

**(0.5.0+)** `upload` is the only write here that fans out, and a plural `--env` gives it several receipts to report. They go out under their own envelope rather than as a widened `WriteDocument`: `promote` and `rollback` act on one env by construction, and every consumer already reads `env` and `universe_id` as single values. Widening them would break those readers in order to describe a case they never asked about.

So the rule above holds here too, at one level up: **the shape follows the invocation.** One env emits the receipt itself, unchanged, whatever else `rbxplace.toml` holds. `all` or a group emits this:

```json
{
  "schema_version": 1,
  "command": "upload",
  "ok": true,
  "results": [
    { "schema_version": 1, "command": "upload", "ok": true, "env": "dev", "...": "..." },
    { "schema_version": 1, "command": "upload", "ok": true, "env": "staging", "...": "..." }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | The same version the receipts carry |
| `command` | string | `upload`. Present so this document is dispatched on the same field as the receipts inside it |
| `ok` | boolean | False when any env failed, which is the answer the exit code gives too |
| `results` | array of objects | One receipt per env, in the order they were written: `all` alphabetically, a group in declared order. Each entry is exactly the document described above, so an existing consumer reads any element of this array without changes |

`results` stops where the walk stopped. The envs that landed keep their versions, the env that failed carries its own `ok: false` and `error`, and the envs after it are absent rather than reported as anything: nothing was asked of them.

```sh
rbx place upload --env all --file build.rbxl --published --yes --json \
  | jq -r '.results[] | "\(.env) v\(.version)"'
```

## Configuration

`rbx place` reads `rbxplace.toml` in the working directory (override with the global `--places <path>`). You can configure place IDs manually or use `rbx place fetch` to auto-populate them from a Roblox universe.

```toml
[prod]
universe_id = 9876543210
confirm = true                  # prompt before upload or rollback
places.main   = 123456789012345
places.lobby  = 987654321

[staging]
universe_id = 9876543211
places.main = 234567890123456

[dev]
universe_id = 9876543212
places.main = 345678901234567
```

<details markdown="1">
<summary>Environment fields</summary>

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `universe_id` | `u64` | Yes | Roblox universe ID |
| `env` | `string` | No | Environment type name for code generation (defaults to section name) |
| `confirm` | `bool` | No | Require confirmation before write operations (default: `false`) |
| `places.<name>` | `u64` | No | Place ID mapped to a name |

</details>

The `rbxplace.toml` file is shared with every other subcommand: they all resolve environment names to universe IDs from it.

## Working without `rbxplace.toml`

The global `--place-id` names a place directly, the way `--universe-id` names a universe, and skips the config file. It reaches the reads:

```sh
rbx place versions --place-id 123456789012345
rbx place download --place-id 123456789012345 --out backup.rbxl
```

**The writes refuse it**, with a message saying why:

```
`--place-id` names a place but no env, and `rbx place upload` needs one: the confirm
guard and the --json receipt are both env-scoped. Pass --env <name> ...
```

Two things are genuinely env-scoped and neither survives an id on its own. `confirm = true` is declared on an env, so an env-less write would walk past a guard somebody set on purpose. And the `--json` receipt carries `env` as a documented field, so an env-less write would emit a document missing something consumers were told to expect. Refusing beats either, and beats accepting the flag and ignoring it.

### Required API scopes

| Operation | Scope | Notes |
| --- | --- | --- |
| Upload / Promote (write) | `universe-places:write` | |
| Download / Promote (read) | `legacy-asset:manage` | |
| Version list | `asset:read` | Also used by `--from-published`, `--from-saved`, `--published`, `--saved` |
| Rollback | `asset:write` | |
| List places | `universe:read` | For `places` command |
