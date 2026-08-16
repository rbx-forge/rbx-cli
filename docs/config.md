# rbx config

Manage Roblox in-experience live configs via the Open Cloud Configs API.

`rbx config` keeps a local `rbxconfig.toml` as the canonical source of truth for your in-experience tunables and syncs it to Roblox. It targets environments defined in a shared `rbxplace.toml`, shows diffs before publishing, and supports gradual rollout.

## Features

- **Declarative config** - All tunables in a single `rbxconfig.toml`, organized per environment
- **Diff preview** - `check` and `sync --dry-run` show exactly what changes before any publish
- **Full sync** - `rbxconfig.toml` is the canonical state: missing keys are removed from live
- **Pull** - Mirror the live config back into `rbxconfig.toml`, preserving local descriptions
- **Revision history** - `versions` and `rollback` to inspect and revert past publishes
- **Gradual rollout** - Optional `GradualRollout` deployment strategy (~15 min propagation)
- **Multi-environment** - Targets environments defined in `rbxplace.toml` (shared with `rbx place`)
- **JSON** - `--json` on `get`, `list` and `versions` writes one document to stdout and nothing else, with documented field names, for `jq` and CI

## Quick start

```sh
# Bootstrap a template
rbx config init

# Or pull the existing live config into a local file
rbx config pull --env dev --api-key YOUR_API_KEY

# Edit rbxconfig.toml, then preview + publish
rbx config check --env dev --api-key YOUR_API_KEY
rbx config sync  --env dev --api-key YOUR_API_KEY
```

`--env` is required on all commands except `init`. It is resolved against `rbxplace.toml` (override with the global `--places`).

## Commands

<details>
<summary><code>rbx config init</code></summary>

Write a commented template `rbxconfig.toml` in the current directory. Bails if the file already exists. Override the target path with `--config <path>`, which belongs to `rbx config` itself and so goes **before** the subcommand.

```sh
rbx config init
rbx config --config configs/rbxconfig.toml init
```

</details>

<details>
<summary><code>rbx config get [&lt;key&gt;]</code></summary>

Print the live published config. If a key is provided, prints only that key's value.

```sh
rbx config get --env dev
rbx config get "features.new_xp_popup" --env dev
```

### `--json`

One JSON document on stdout, nothing else. Diagnostics stay on stderr, so the document parses whatever else the run had to say.

This document is a snapshot of the **published** config: what Roblox is serving right now. It does not read `rbxconfig.toml`, and says so by having no `config_file` field. Whether the local file agrees with live is a different question, and `rbx config check` is what answers it — under `rbx check --json` that row is `config/live` and it carries `outcome`, `summary` and `details`. None of those three words appears here, so a filter written for one cannot half-read the other.

```sh
rbx config get "ops.teleport_place_id" --env dev --json
```

```json
{
  "schema_version": 1,
  "env": "dev",
  "universe_id": 9876543210,
  "config_version": 14,
  "key": "ops.teleport_place_id",
  "value": 12345,
  "entries": {
    "ops.teleport_place_id": { "type": "number", "value": 12345 }
  }
}
```

Without a key, the whole published config comes back in the same envelope — the identical document `rbx config list --json` emits, so one filter reads both.

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today. Refuse a version you do not understand |
| `env` | string | The env named on the command line. **Absent** under a bare `--universe-id`, where the human form prints a `<universe-id>` placeholder that is a label and not an env name |
| `universe_id` | integer | The universe this snapshot is from |
| `config_version` | integer | Roblox's `configVersion` for this snapshot, in snake case like every other field |
| `key` | string | The key asked for. **Absent** when none was |
| `value` | any | That key's value, raw, exactly what the bare form prints. **Absent** whenever `key` is |
| `entries` | object | Keyed by config key: one entry when `key` is set, all of them otherwise. Always present |
| `entries.<key>.type` | string | `bool`, `number`, `string`, `array`, `object`, `null` — the words the listing prints in its type column |
| `entries.<key>.value` | any | The published value |

There is no `totals` object. `rbx check --json` has one and it counts outcomes; one here would count keys under the same name. `.entries | length` is the count, and it cannot be misread.

Which of `value` and `entries` you get is decided by the invocation, never by the data: a keyless read omits `value` even against a config holding exactly one key, so a filter cannot start working by accident and break when a second key is published. An unknown key stays an error (exit 1), never a document with a null value — "not published" and "published as nothing" are different facts.

```sh
PLACE=$(rbx config get "ops.teleport_place_id" --env dev --json | jq -r .value)
rbx config get --env dev --json | jq -r '.entries | to_entries[] | select(.value.type == "bool") | .key'
```

</details>

<details>
<summary><code>rbx config list</code></summary>

List all published config keys with their type and a compact value preview.

```sh
rbx config list --env dev
```

Output example:

```
Live config keys - env: dev (configVersion 14)
  balance.speed_multipliers  [object]  {"tier_1":1.5,"tier_2":2}
  features.new_xp_popup      [bool]    true
  ops.teleport_place_id      [number]  12345
```

### `--json`

The same snapshot as one JSON document on stdout, nothing else — and it is the *same document* `rbx config get --json` emits without a key, envelope and all, so one filter reads both. See the field table under `rbx config get` for what each field means.

```sh
rbx config list --env dev --json
```

```json
{
  "schema_version": 1,
  "env": "dev",
  "universe_id": 9876543210,
  "config_version": 14,
  "entries": {
    "balance.speed_multipliers": { "type": "object", "value": { "tier_1": 1.5, "tier_2": 2 } },
    "features.new_xp_popup": { "type": "bool", "value": true },
    "ops.teleport_place_id": { "type": "number", "value": 12345 }
  }
}
```

A universe with nothing published yet is an empty `entries` object and exit 0, not a missing document: `.entries | length == 0` has to be answerable.

```sh
rbx config list --env dev --json | jq -r '.entries | keys[]'
rbx config list --env prod --json | jq -r .config_version
```

</details>

<details>
<summary><code>rbx config check</code></summary>

Show the diff between local `rbxconfig.toml` and the live published config. Read-only - no draft, no publish, no confirmation prompt.

Exit codes: `0` local matches live, `2` entries differ, `1` the check could not answer. Drift sits on its own code so a CI step can gate on the status alone.

```sh
rbx config check --env dev
```

</details>

<details>
<summary><code>rbx config sync</code></summary>

Push `rbxconfig.toml` as the canonical state for the target env. Uses `PUT /draft:overwrite` so keys absent from the file are removed from live. Always shows the diff first. Writes the published `configVersion` and entries to `rbxconfig.lock.toml` on success.

```sh
rbx config sync --env dev --dry-run    # preview only
rbx config sync --env dev              # prompts for confirmation
rbx config sync --env dev --yes        # skip confirmation
rbx config sync --env dev --strategy gradual-rollout
```

| Flag | Description |
| --- | --- |
| `--message` / `--no-message` | Publish message, or publish without one |

**`--yes` answers the message question too.** `sync` asks twice on a terminal: once to confirm, once for a publish message. `--yes` means "do not ask me anything", so it covers both and publishes with an empty message — pass `--message` alongside it if the message matters.

Off a terminal with none of the three, the run refuses and names them. It used to reach the prompt anyway and fail with `not a terminal`, which is a fact about the stream rather than about the flag that fixes it.
| `--strategy` | `immediate` (default) or `gradual-rollout` |
| `--dry-run` | Show diff without publishing |
| `--yes` | Skip confirmation prompt |

</details>

<details>
<summary><code>rbx config pull</code></summary>

Fetch the live published config and write it to `rbxconfig.toml` under the target env. Preserves any local `description` annotations on keys that still exist. Other envs in the file are left untouched. The published `configVersion` and timestamp are recorded in `rbxconfig.lock.toml`.

```sh
rbx config pull --env dev
rbx config pull --env dev --yes       # overwrite without confirmation
rbx config --config staging.toml pull --env dev   # write to a different file
```

| Flag | Description |
| --- | --- |
| `--yes` | Overwrite without confirmation if file exists |

</details>

<details>
<summary><code>rbx config versions</code></summary>

List the revision history for the target env's universe. The current revision is tagged `[published]`.

```sh
rbx config versions --env dev
rbx config versions --env dev --count 50
```

| Flag | Description |
| --- | --- |
| `--count` | Number of revisions to show (default: `20`) |
| `--json` | Write the revisions to stdout as one JSON document |

### `--json`

One JSON document on stdout, nothing else. The progress line the human form prints is not part of the history, so under `--json` it is simply not printed.

```sh
rbx config versions --env dev --json
```

```json
{
  "schema_version": 1,
  "env": "dev",
  "universe_id": 9876543210,
  "count": 20,
  "count_reached": false,
  "revisions": [
    {
      "revision_id": "aaaaaaaa-1111-4000-8000-000000000001",
      "version": 14,
      "time": "2026-08-15T09:30:00Z",
      "message": "raise the cap",
      "changed_keys": ["balance.speed_multipliers", "ops.teleport_place_id"],
      "published": true
    },
    {
      "revision_id": "bbbbbbbb-2222-4000-8000-000000000002",
      "version": 13,
      "time": "2026-08-14T09:30:00Z",
      "changed_keys": [],
      "published": false
    }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today |
| `env` | string | The env named on the command line. **Absent** under a bare `--universe-id` |
| `universe_id` | integer | The universe whose history this is |
| `count` | integer | The `--count` in force for this run |
| `count_reached` | boolean | True when the run stopped because it hit `--count` rather than because it ran out of revisions. Raise `--count` to see further back |
| `revisions` | array of objects | Newest first, the order Roblox returns and the listing prints |
| `revisions[].revision_id` | string | Full id, not the eight-character prefix the listing shows. This is what `rbx config rollback` takes |
| `revisions[].version` | integer | The `configVersion` this publish produced |
| `revisions[].time` | string | The timestamp Roblox sent, untouched ISO. The listing rewrites it to be read; a consumer wants it back |
| `revisions[].message` | string | The publish message. **Absent** when there was none, which is not the same fact as an empty one — the listing renders both as `(no message)` |
| `revisions[].changed_keys` | array of strings | The keys this revision changed, sorted so the same history renders the same bytes twice running. The listing prints only the count, which is `length`. Always an array, empty included |
| `revisions[].published` | boolean | True for the revision currently serving players — the one the listing tags `[published]`. Stated rather than inferred from the position, so a consumer that sorted the array still knows |

This is history, not state: it shares nothing with the `get`/`list` document beyond the envelope, and nothing at all with the `config/live` row of `rbx check --json`, which is a verdict rather than a record.

```sh
rbx config versions --env prod --json | jq -r '.revisions[] | select(.published) | .revision_id'
rbx config versions --env prod --count 50 --json | jq -r '.revisions[].changed_keys[]' | sort | uniq -c
```

</details>

<details>
<summary><code>rbx config rollback [&lt;revision_id&gt;]</code></summary>

Roll back to a previous revision. Restores the chosen revision into the draft and publishes it as a new version. If `revision_id` is omitted, an interactive picker lists recent revisions (current tagged `[published]`).

```sh
rbx config rollback --env dev                   # interactive picker
rbx config rollback --env dev <revision_id>     # direct
rbx config rollback --env dev --count 30        # picker with more entries
```

| Flag | Description |
| --- | --- |
| `--count` | Number of revisions to show in the picker (default: `10`) |

</details>

### Per-tool flags

| Flag | Default | Description |
| --- | --- | --- |
| `--config` | `rbxconfig.toml` | Path to the local config file |
| `--universe-id` | _none_ | Bypass `rbxplace.toml` lookup. `--env` is still required to name the section in `rbxconfig.toml` for commands that read/write it |

## Configuration

### rbxplace.toml

`rbx config` resolves environment names to universe IDs from `rbxplace.toml` (shared with `rbx place`):

```toml
[prod]
universe_id = 9876543210
places.main = 123456789012345

[staging]
universe_id = 9876543211
places.main = 234567890123456
```

Pass a different path via the global `--places`, or skip the lookup entirely with `--universe-id <id>`. Each env section may set `confirm = true` to force interactive confirmation before any write to that env.

### rbxconfig.toml

The local source of truth for your tunables, organized per environment. Every entry under `[<env>.entries."key"]` is synced as a config key. Scalars become scalar values; tables become JSON objects. Each entry has a required `value` and an optional `description` (local-only - not sent to Roblox).

```toml
[prod.entries."features.new_xp_popup"]
value = false
description = "Disabled in prod until stable"

[prod.entries."ops.teleport_place_id"]
value = 12345

[prod.entries."balance.speed_multipliers"]
value = { tier_1 = 1.5, tier_2 = 2.0 }

[staging.entries."features.new_xp_popup"]
value = true
description = "Testing new popup - remove in v2"
```

Dotted key names (e.g. `"features.new_xp_popup"`) are preserved verbatim as the Roblox config key. In-game, read them with:

```lua
ConfigService:GetConfigAsync():GetValue("features.new_xp_popup")
```

### rbxconfig.lock.toml

Written automatically by `pull` and `sync`, next to `rbxconfig.toml`. Records, per environment, the last published `revision_id` (the `v{N}` configVersion), the `synced_at` timestamp, and a snapshot of the entries that were pushed or pulled. Informational only - not sent to Roblox. Commit it if you want to track sync history alongside your config.

```toml
version = 1

[envs.prod]
revision_id = "v14"
synced_at = "2024-01-15T14:30:00Z"

[envs.prod.entries]
"features.new_xp_popup" = false
"ops.teleport_place_id" = 12345
"balance.speed_multipliers" = { tier_1 = 1.5, tier_2 = 2.0 }
```

### Required API scopes

| Operation | Scope |
| --- | --- |
| Read live config (`get`, `list`, `check`, `pull`, `sync` diff) | `universe:read` |
| Write config (`sync`, `rollback`) | `universe:write` |

## Deployment strategies

| Strategy | Flag | Propagation |
| --- | --- | --- |
| Immediate | `--strategy immediate` | ~5 minutes |
| Gradual rollout | `--strategy gradual-rollout` | ~15 minutes |

Gradual rollout incrementally applies the config across servers, reducing the blast radius of a bad config push.
