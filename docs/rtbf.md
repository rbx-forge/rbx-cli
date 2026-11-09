# rbx rtbf

Declare which data store keys hold a player's data, so Roblox can delete them on a right-to-be-forgotten request. The templates live in `rbxrtbf.toml` and are published to the `DataStoresConfig` repository of the Configs API.

```sh
rbx rtbf init                  # or: rbx rtbf pull --env prod
rbx rtbf show                  # each template, and the key it will match
rbx rtbf verify --env prod     # do those stores exist?
rbx rtbf sync --env prod
```

!!! warning "Roblox accepts templates that match nothing"
    A pattern that matches no key is stored, shown as configured, and deletes nothing. You find out when a request goes unfulfilled. `rbx rtbf` refuses what it can decide locally, and `verify` checks the rest against your live stores.

What is refused locally:

- **A miscased token.** `{UserId}` is case-sensitive; `{userId}` matches nothing. Refused anywhere it appears, even next to a correct one.
- **A pattern with no token.** It would match one key belonging to nobody.
- **More than 100 templates**, Roblox's limit per universe.

An omitted `scope` means `global`, as on Roblox.

## rbxrtbf.toml

```toml
[[key]]
store = "PlayerInventory"        # an exact store name
pattern = "User_{UserId}"

[[key]]
store = "PlayerData"
pattern = "Data"
scope = "User_{UserId}"          # the id can be in the scope instead

[[key]]
store = "PlayerLeaderboard"
pattern = "User_{UserId}"
ordered = true

[[store]]                        # a whole store, by name pattern
pattern = "Player_{UserId}_Save"
```

The user id must appear in the key, the scope or the store name.

| `[[key]]` field | Required | Meaning |
| --- | --- | --- |
| `store` | yes | Store name, exact |
| `pattern` | yes | Key pattern |
| `scope` | no | Scope pattern. Default `global` |
| `ordered` | no | `true` for an ordered data store |

| `[[store]]` field | Required | Meaning |
| --- | --- | --- |
| `pattern` | yes | Store name pattern, with `{UserId}`. Standard stores only: Roblox cannot delete a whole ordered store |

Templates are declared once for every env: `--env` picks which universe to publish to, and `--env all` sends the same set everywhere. There is no lockfile; `check` reads what is published.

## Commands

| Command | Does | Exit codes |
| --- | --- | --- |
| `rbx rtbf init` | Write a commented `rbxrtbf.toml`. Refuses if it exists | `0`, `1` |
| `rbx rtbf show` | Validate, and print each template with a sample key. Offline | `0` valid, `1` invalid |
| `rbx rtbf check` | Compare the file with what is published | `0` same, `2` different, `1` error |
| `rbx rtbf sync` | Publish the file as the whole set | `0`, `1` |
| `rbx rtbf pull` | Overwrite the file with what is published | `0`, `1` |
| `rbx rtbf verify` | Check each template against the stores that exist | `0` all found, `2` some match nothing, `1` error |

`--config <path>` goes before the subcommand: `rbx rtbf --config universes/prod/rbxrtbf.toml check`.

### rbx rtbf show

```sh
rbx rtbf show --user-id 1234567890
```

`--user-id` is the id put into the samples (default `1234567890`).

#### `--json`

```json
{
  "schema_version": 1,
  "config_file": "rbxrtbf.toml",
  "max_templates": 100,
  "count": 2,
  "sample_user_id": "1234567890",
  "templates": [
    { "kind": "key", "store": "PlayerInventory", "pattern": "User_{UserId}", "scope": "global", "ordered": false, "sample": "PlayerInventory/global/User_1234567890" },
    { "kind": "store", "pattern": "Player_{UserId}_Save", "sample": "Player_1234567890_Save" }
  ]
}
```

`kind` is `key` or `store`. `store`, `scope` and `ordered` are absent on store templates; `scope` is filled with `global` when the file omits it.

### rbx rtbf sync

Publish the whole set: a template missing from the file is removed. The file is validated before anything is sent.

```sh
rbx rtbf sync --env prod --dry-run
rbx rtbf sync --env prod --yes
```

| Flag | Description |
| --- | --- |
| `--message` / `--no-message` | Revision message, or none. Default: a generated one |
| `--dry-run` | Show what would be published |
| `--yes` | Skip the confirmation |

Always published immediately, never as a gradual rollout. An empty file is refused if it also contains a table `rbx` does not know (`[[keys]]` for `[[key]]`, typically), since publishing it would erase the set; a genuinely empty file is how you clear it.

### rbx rtbf pull

| Flag | Description |
| --- | --- |
| `--yes` | Overwrite the file without asking |

### rbx rtbf verify

`check` says the file matches what is published; `verify` says the templates name stores that exist.

```sh
rbx rtbf verify --env prod
rbx rtbf verify --env prod --uncovered   # also list stores no template covers
```

| Flag | Description |
| --- | --- |
| `--uncovered` | List live stores no template covers. Not an error: a store without user data needs none |
| `--json` | One JSON document on stdout |

- Ordered stores cannot be listed through Open Cloud, so their templates are reported **unverifiable**, not missing.
- `{UserId}` in a store pattern matches digits only, so `Player_{UserId}_Save` never passes for `Player_Settings_Save`.

#### `--json`

```json
{
  "schema_version": 1,
  "config_file": "rbxrtbf.toml",
  "env": "prod",
  "universe_id": "109876543210987",
  "ok": false,
  "standard_store_count": 4,
  "findings": [
    { "kind": "key", "target": "PlayerInventoryV1", "verdict": "missing", "detail": "no such standard data store in this universe" }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `ok` | `false` when a template matches nothing (same as exit `2`) |
| `findings[].verdict` | `missing`, `unmatched`, or `unverifiable` (not counted against `ok`) |
| `findings[].target` | The store, or the store pattern |
| `uncovered` | Present only with `--uncovered` |

The document is printed even when the exit code is `2`. A run refused before checking prints nothing.

```sh
rbx rtbf verify --env prod --json | jq -e '.ok' > /dev/null || echo "templates need attention"
```

## In rbx check

`rbx check` reports two rows when `rbxrtbf.toml` is present: `rtbf/templates` (local, always runs) and `rtbf/live` (needs an API key, skipped otherwise). See [`rbx check`](check.md).

## Required API scopes

| Operation | Scope |
| --- | --- |
| `check`, `pull` | `universe:read` |
| `sync` | `universe:write` |
| `verify` | `universe-datastores.control:list` |

The same entry (`user_data_templates`) is reachable with `rbx config --repository DataStoresConfig`, which publishes it without any of these checks.
