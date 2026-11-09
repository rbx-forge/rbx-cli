# rbx config

The experience's live config (what `ConfigService` reads in game), declared per env in `rbxconfig.toml` and published through the Open Cloud Configs API.

```sh
rbx config pull --env dev       # start from what is live
rbx config check --env dev      # diff the file against live
rbx config sync --env dev       # publish the file
```

`rbxconfig.toml` is the whole state: a key missing from the file is removed from live on the next `sync`. Every command but `init` needs `--env` (or `--universe-id`).

## Commands

### rbx config init

Write a commented `rbxconfig.toml`. Refuses if the file exists.

```sh
rbx config init
rbx config --config configs/rbxconfig.toml init
rbx config --repository DataStoresConfig init
```

### rbx config get

`rbx config get [<key>]`

Print the published config, or one key's value.

```sh
rbx config get --env dev
rbx config get "features.new_xp_popup" --env dev
```

#### `--json`

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

| Field | Type | Meaning |
| --- | --- | --- |
| `env` | string | Absent with a bare `--universe-id` |
| `universe_id` | integer | |
| `config_version` | integer | Roblox's `configVersion` |
| `key`, `value` | string, any | The key asked for and its value. Both absent without a key |
| `entries` | object | One entry per key: `type` (`bool`, `number`, `string`, `array`, `object`, `null`) and `value` |

Without a key, the document is the same as `list --json`. An unknown key exits `1`.

### rbx config list

Every published key, with its type and a preview.

```
Live config keys - env: dev (configVersion 14)
  balance.speed_multipliers  [object]  {"tier_1":1.5,"tier_2":2}
  features.new_xp_popup      [bool]    true
```

`--json` prints the document of `get --json` without a key. Nothing published gives an empty `entries` and exit 0.

### rbx config check

Diff `rbxconfig.toml` against live. Read-only. Exits `0` when they match, `2` when they differ, `1` if it could not answer.

### rbx config sync

Publish `rbxconfig.toml` for the env, after showing the diff. Records the published version in `rbxconfig.lock.toml`.

```sh
rbx config sync --env dev --dry-run
rbx config sync --env prod --message "raise the cap" --yes
rbx config sync --env dev --strategy gradual-rollout
```

| Flag | Description |
| --- | --- |
| `--message` / `--no-message` | Publish message, or none |
| `--strategy` | `immediate` (default, about 5 minutes to propagate) or `gradual-rollout` (about 15, server by server) |
| `--dry-run` | Show the diff, publish nothing |
| `--yes` | Skip the confirmation and the message prompt (publishes with no message unless `--message` is given) |
| `--overwrite-newer` | Publish even though someone published since your last sync or pull |

Without a terminal, pass `--yes`, `--message` or `--no-message`.

!!! warning "Someone else's publish is not overwritten silently"
    If live is at another version than the lockfile recorded (a publish from another branch, machine or the Creator Hub, or a rollback), `sync` shows the diff and refuses. `rbx config pull` brings their changes in; `--overwrite-newer` replaces them.

Edits staged as a draft in the Creator Hub are replaced by a publish; `sync` names the keys that draft held. The [limits](#limits) are checked before writing, `--dry-run` included.

### rbx config pull

Write the published config into `rbxconfig.toml` for the env. Local `description` fields are kept, other envs untouched.

| Flag | Description |
| --- | --- |
| `--yes` | Overwrite the file without asking |

### rbx config versions

Revision history, newest first. The live one is tagged `[published]`.

| Flag | Description |
| --- | --- |
| `--count` | How many (default `20`) |
| `--json` | One JSON document on stdout |

#### `--json`

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
      "changed_keys": ["balance.speed_multipliers"],
      "published": true
    }
  ]
}
```

`revision_id` is the full id `rollback` takes. `message` is absent when there was none. `count_reached` is `true` if there may be more.

### rbx config rollback

`rbx config rollback [<revision_id>]`

Publish a past revision again, as a new version. Without an id, a picker lists recent revisions (a terminal is required).

```sh
rbx config rollback --env dev
rbx config rollback --env prod aaaaaaaa-1111-4000-8000-000000000001 --message "revert #123" --yes
```

| Flag | Description |
| --- | --- |
| `--count` | Revisions in the picker (default `10`) |
| `--message` / `-m`, `--no-message` | Publish message, or none |
| `--yes` / `-y` | Skip the confirmation and the message prompt |

### Per-tool flags

These go before the subcommand: `rbx config --repository DataStoresConfig list`.

| Flag | Default | Description |
| --- | --- | --- |
| `--config` | `rbxconfig.toml` | The config file |
| `--repository` | `InExperienceConfig` | Which configs repository. See [Repositories](#repositories) |

## rbxconfig.toml

```toml
[prod.entries."features.new_xp_popup"]
value = false
description = "Disabled in prod until stable"   # local only, never sent

[prod.entries."balance.speed_multipliers"]
value = { tier_1 = 1.5, tier_2 = 2.0 }

[staging.entries."features.new_xp_popup"]
value = true
```

Keys are kept verbatim; in game, `ConfigService:GetConfigAsync():GetValue("features.new_xp_popup")`. An optional `repository = "..."` at the top (before any table) names the repository.

`rbxconfig.lock.toml` records, per env, the published version, when, and the entries. Commit it: `sync` uses it to notice someone else's publish.

## Repositories

The Configs API works the same on eight repositories. `InExperienceConfig` (the default) and `DataStoresConfig` (right-to-be-forgotten templates, see [`rbx rtbf`](rtbf.md)) have documented schemas. `RecommendationServicesConfig`, `ExtendedServicesConfig`, `LeaderboardsConfig`, `ExperienceUserConfig`, `JourneysConfig` and `AntiCheatConfig` are in Roblox's enum and documented nowhere; they work if you know their keys.

| `--repository` | `repository` in the file | Used |
| --- | --- | --- |
| absent | absent | `InExperienceConfig` |
| absent | set | the file's |
| set | absent | the flag's |
| set | different | refused |

`get`, `list` and `versions` do not read the file, so only the flag applies to them.

## Limits

Checked before publishing: 100 entries per repository, 256 characters per key.

## Required API scopes

| Operation | Scope |
| --- | --- |
| `get`, `list`, `check`, `pull`, `versions` | `universe:read` |
| `sync`, `rollback` | `universe:write` |
