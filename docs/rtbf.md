# rbx rtbf

Declare which data store keys hold a user's data, so Roblox can delete them when a right-to-be-forgotten request arrives.

`rbx rtbf` keeps a local `rbxrtbf.toml` as the canonical source of truth for those declarations and publishes it through the `DataStoresConfig` repository of the Open Cloud Configs API. It targets environments defined in a shared `rbxplace.toml`, shows the difference before publishing, and can check a template against the data stores that actually exist.

**(0.5.0+)** This page describes `main`. Check `rbx --version` against what your `rokit.toml` pins.

## Why this needs a command of its own

When Roblox processes a deletion request for one of your players, it does not know where that player's data lives. A template tells it: a data store name and a key pattern holding a `{UserId}` token, which Roblox substitutes with the requester's id and then deletes what matches.

The risk is not that Roblox refuses a bad template. It is that Roblox **accepts** it. A pattern that matches nothing is stored, reported as configured, and deletes nothing, and you find out when a legal request goes unfulfilled. Roblox's own guidance is to compare your patterns against your live Luau by hand in the Creator Hub, and then to confirm within 30 days that the data went. That guidance is an admission that nothing verifies it for you.

So every rule this tool enforces is one nobody else checks:

- **`{UserId}` is case-sensitive.** `{userId}` is stored happily by Roblox and matches nothing at all. `rbx rtbf` refuses a miscased token locally and names the field it is in.
- **A pattern with no token is a constant.** It matches at most one key, belonging to nobody in particular.
- **An omitted or blank `scope` means `global`.** That is Roblox's rule. A store using a non-default scope, against a template that quietly defaulted to `global`, is another way a pattern matches nothing.
- **A hundred templates per universe** is Roblox's ceiling. A template can cover many keys, so hitting it means widening patterns rather than listing keys.

## Quick start

```sh
# Bootstrap a commented template
rbx rtbf init

# Or pull whatever is already published into a local file
rbx rtbf pull --env prod --api-key YOUR_API_KEY

# Edit rbxrtbf.toml, then read it back as Roblox will see it
rbx rtbf show

# Prove the stores and keys actually exist, then publish
rbx rtbf verify --env prod --api-key YOUR_API_KEY
rbx rtbf sync   --env prod --api-key YOUR_API_KEY
```

`--env` is required on every command that talks to Roblox. It resolves against `rbxplace.toml` (override with the global `--places`), or skip the lookup with `--universe-id <id>`.

## rbxrtbf.toml

Deliberately flat: the whole file is the two template arrays. There is no `[settings]`, no `[experience]` and no `[envs.*]`, because there is nothing to configure about a template beyond the template itself.

```toml
# A key inside a named store. `store` is an exact name, not a pattern.
[[key]]
store = "PlayerInventory"
pattern = "User_{UserId}"

# A non-default scope, itself a pattern.
[[key]]
store = "PlayerSettings"
pattern = "Settings_{UserId}"
scope = "Player_{UserId}"

# An ordered data store, same shape.
[[key]]
store = "PlayerLeaderboard"
pattern = "User_{UserId}"
ordered = true

# A whole store, named by pattern.
[[store]]
pattern = "Player_{UserId}_Save"
```

### Where the id has to be

Roblox's eligibility rule: **the user id must be part of the name or the scope** of the data store or key. So both of these are valid, and this tool accepts both:

```toml
# the id in the key name, the usual shape
[[key]]
store = "PlayerInventory"
pattern = "User_{UserId}"

# the id in the scope, with a constant key name
[[key]]
store = "PlayerData"
pattern = "Data"
scope = "User_{UserId}"
```

A `[[store]]` template has no scope, so its `pattern` is the only place the id can be and the token is required there.

The token is refused wherever it appears **miscased**, whether or not a correct one is also present. `User_{UserId}_{userid}` is the case worth stating: Roblox substitutes the first and leaves the second as literal text, so the key it looks for is `User_123_{userid}`, which matches nothing.

### `[[key]]`

| Field | Required | Meaning |
| --- | --- | --- |
| `store` | yes | The data store holding the key. An **exact name**, not a pattern: Roblox matches the store by name, and only the key and the scope by pattern |
| `pattern` | yes | The key pattern, which must contain `{UserId}` |
| `scope` | no | The scope pattern. Omitted or blank means `global`, which is Roblox's own default |
| `ordered` | no | `true` for an ordered data store rather than a standard one. A bool rather than the API's `STANDARD` / `ORDERED` string, because those are the only two values and a bool cannot be misspelled |

### `[[store]]`

| Field | Required | Meaning |
| --- | --- | --- |
| `pattern` | yes | The data store **name** pattern, which must contain `{UserId}` |

**Standard stores only.** Roblox does not support deleting an entire ordered store, which is why `[[store]]` has no `ordered` field to set. An ordered store's keys are still coverable one pattern at a time with `[[key]]` and `ordered = true`.

### There is no lockfile

For the reason `rbx config` has none: the published config is readable in full, so the remote state is a fetch rather than something that has to be remembered. `rbx rtbf check` compares the file against what Roblox is actually serving.

## Commands

| Command | What it does | Exit codes |
| --- | --- | --- |
| `rbx rtbf init` | Write a commented `rbxrtbf.toml` to start from. Bails if the file exists | `0` written, `1` refused |
| `rbx rtbf show` | Print every declared template with the sample key Roblox would look for, after validating them. Local only, no network. `--json` | `0` valid, `1` a rule is broken |
| `rbx rtbf check` | Compare `rbxrtbf.toml` against the published templates | `0` they match, **`2` they differ**, `1` the check could not answer |
| `rbx rtbf sync` | Publish `rbxrtbf.toml` as the canonical set of templates | `0` published, `1` refused or failed |
| `rbx rtbf pull` | Overwrite `rbxrtbf.toml` with the published templates | `0` written, `1` refused or failed |
| `rbx rtbf verify` | Check every template against the data stores that actually exist. `--json` | `0` every template names something real, **`2` at least one names nothing**, `1` the check could not answer |

Drift and unmatched templates sit on exit code `2` so a CI step can gate on the status alone, and tell "publish this" from "something broke".

### `rbx rtbf show`

The samples are the point. A pattern is read for what it was meant to say; a substituted sample is read for what it will actually match, which is the form you can compare against your Luau. `--user-id` picks the id to substitute (default `1234567890`).

```sh
rbx rtbf show
rbx rtbf show --user-id 1234567890
```

#### `--json`

```json
{
  "schema_version": 1,
  "config_file": "rbxrtbf.toml",
  "max_templates": 100,
  "count": 2,
  "sample_user_id": "1234567890",
  "templates": [
    {
      "kind": "key",
      "store": "PlayerInventory",
      "pattern": "User_{UserId}",
      "scope": "Scope_{UserId}",
      "ordered": false,
      "sample": "PlayerInventory/Scope_1234567890/User_1234567890"
    },
    {
      "kind": "store",
      "pattern": "Player_{UserId}_Save",
      "sample": "Player_1234567890_Save"
    }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with every other `--json` here. `1` today |
| `config_file` | string | The file this was read from, as given or defaulted, so a document captured out of a matrix job still says which one produced it |
| `max_templates` | integer | Roblox's ceiling, carried so a consumer can warn before it is reached rather than hardcoding the number twice |
| `count` | integer | Templates declared, both kinds |
| `sample_user_id` | string | The id substituted into every `sample` |
| `templates[].kind` | string | `key` or `store` |
| `templates[].store` | string | The data store name. **Absent** for a `store` template, whose store *is* the pattern |
| `templates[].pattern` | string | The key pattern, or the store name pattern |
| `templates[].scope` | string | The scope Roblox will match on, **defaulted**: a key template naming none reports `global` rather than being absent, because `global` is the answer and the file's silence is not. **Absent** for a `store` template |
| `templates[].ordered` | boolean | **Absent** for a `store` template: Roblox supports deleting a whole store only for standard stores, so there is no choice to report |
| `templates[].sample` | string | What Roblox will look for once the token is substituted |

Both kinds are one list discriminated by `kind`, rather than the file's two arrays: a consumer filters, and the two-array shape exists in the TOML for a human's sake. A file declaring nothing is `"count": 0` with an empty array, not an absent one.

This is **declared state**, read from the file with no network, so there is no `env` and no `universe_id`. What any universe is actually serving is `rbx check --json`'s question.

### `rbx rtbf sync`

Publishes the whole set: a template absent from the file is removed from live, the same way `rbx config sync` treats a missing key. The file is validated before the network and before the prompt, because a template that would match nothing must not reach a publish that makes it authoritative.

**An empty declaration is refused when the file also names a table this release does not read.** The case it is there for is `[[keys]]`, the plural: it is not a table this file has, so it parses to nothing at all and the file looks full while declaring zero templates. Publishing that replaces the whole published set, and Roblox's only undo is restoring a revision. So a file that declares nothing *and* names any unrecognised root table is refused before the network and before the prompt, with every such table named next to the two the file is made of, `key` and `store`. An empty file naming nothing unrecognised is not refused: declaring nothing is a legitimate state, and publishing it is how you clear the set on purpose.

```sh
rbx rtbf sync --env prod --dry-run    # preview only
rbx rtbf sync --env prod              # prompts for confirmation
rbx rtbf sync --env prod --yes        # skip the prompt
```

| Flag | Description |
| --- | --- |
| `--message` / `--no-message` | Publish message recorded in the revision history, or publish without one. Unlike `rbx config sync` this defaults to a generated message rather than stopping to ask: a template set changes rarely and its content is the interesting part |
| `--dry-run` | Show what would be published and stop |
| `--yes` | Skip the confirmation prompt |

**There is no `--strategy`.** The Configs API offers `Immediate` and a gradual rollout, and this command always publishes with `Immediate`. A template set is not a feature flag: there is no value in a fifteen-minute gradual rollout of "which keys hold a user's data", and a request arriving mid-rollout would be served by whichever half of the fleet answered.

### `rbx rtbf verify`

The question `check` cannot answer. `check` says the file and the published set agree, and both can agree perfectly on a template naming a store you renamed last year.

It applies the same refusal as `sync`, for its own reason: with nothing declared there is nothing to look for, so a declaration a typo emptied would answer `ok: true` with no findings, and that reads as proof the templates are sound rather than as news that they are gone.

```sh
rbx rtbf verify --env prod
rbx rtbf verify --env prod --uncovered   # also list live stores no template covers
```

`--uncovered` is not a failure list. A store holding no user data needs no template, so the output says as much; it is worth reading once.

#### What verify can and cannot see

Two limits, both of them stated in the output rather than papered over:

- **Ordered stores are invisible to it.** Roblox's `Cloud_ListDataStores` covers standard stores only, so a `[[key]]` marked `ordered = true` is reported as **unchecked**, not as missing. Reporting it as missing would be a false alarm, and a check that cries wolf is one people learn to ignore.
- **A store pattern is matched by requiring digits where the token is.** `{UserId}` stands for a user id, so it matches a run of digits and nothing else. That is exact for the common single-token case and deliberately conservative otherwise: a looser wildcard would call `Player_{UserId}_Save` a match for `Player_Settings_Save` and report a template as verified when it is not. A verify that says yes too easily is worse than no verify at all.

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
    {
      "kind": "key",
      "target": "PlayerInventoryV1",
      "verdict": "missing",
      "detail": "no such standard data store in this universe"
    }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | `1` today |
| `config_file` | string | The file the templates were read from |
| `env` | string | The env asked for. **Absent** under a bare `--universe-id`, which is the case where no config named one |
| `universe_id` | string | A string, like every id here: a 64-bit id handed over as a number is one a JSON parser may round |
| `ok` | boolean | False when any template names nothing that exists, which is also the exit-2 condition. Both are present so a consumer that captured stdout does not have to reach for the status |
| `standard_store_count` | integer | Standard data stores the universe holds. Ordered stores are not counted, which is the same limit `unverifiable` reports |
| `findings[].kind` | string | `key` or `store` |
| `findings[].target` | string | The store for a key template, the pattern for a store template |
| `findings[].verdict` | string | `missing`, `unmatched`, or `unverifiable` |
| `findings[].detail` | string | Why, in the same words the human form prints |
| `uncovered` | array of strings | Live stores no template covers. **Absent** unless `--uncovered` asked, because an absent field and an empty array mean different things and a consumer should not have to guess which it got |

**`verdict` has three values rather than being a boolean, and that matters.** `unverifiable` is a limit of Open Cloud, not a broken template, so it is excluded from `ok`. A consumer folding it into a failure would break a build over an ordered store nothing can list.

The document is written **before** the process exits non-zero, so a run that verified and found something still emits it. `.ok` is the field to branch on:

```sh
rbx rtbf verify --env prod --json | jq -e '.ok' > /dev/null || echo "templates need attention"
rbx rtbf verify --env prod --json | jq -r '.findings[] | select(.verdict != "unverifiable") | .target'
```

A run refused before it looks writes **nothing** to stdout: an unreadable file, a template that could match nothing, or a declaration an unrecognised table emptied. There is no document with `ok: false` for those, because nothing was verified; an empty stdout next to a non-zero exit says so, and `jq -e` fails on it either way.

## Environments

The templates are declared **once**, with no per-env overlays, because a key naming scheme is a property of your codebase rather than of an environment: `PlayerInventory` is called that in dev and in prod.

`--env` therefore selects which universe to publish to, not which templates to publish. `--env all`, or an env group, sends the same declaration to several universes, which is the shape you want when the same code runs in each.

```sh
rbx rtbf sync --env all
```

## rbx check

`rbxrtbf.toml` in the working directory is picked up by `rbx check`, as two rows:

```
✓ rtbf/templates  2 templates valid
- rtbf/live       --offline: comparing against Roblox needs an API key
```

`rtbf/templates` is local and always runs, `--offline` included, because every rule it checks is decidable from the file. `rtbf/live` is the comparison against the published set and needs an API key; with no target universe it is skipped rather than failed, so a keyless pre-commit hook still exits 0. A file declaring nothing is clean and says so: an empty declaration is a legitimate state, not drift. The exception is a file that declares nothing *and* names a root table this release does not read, where `rtbf/templates` is an **error** row and `rbx check` exits 1: nothing there disagrees with Roblox, but `[[keys]]` parses to an empty declaration and no tool can say what the file meant to declare. See [docs/check.md](./check.md).

## Required API scopes

| Operation | Scope |
| --- | --- |
| Read the published templates (`check`, `pull`) | `universe:read` |
| Publish templates (`sync`) | `universe:write` |
| List data stores (`verify`) | `universe-datastores.control:list` |

## Relationship to `rbx config`

The templates live in the `DataStoresConfig` repository of the same Configs API `rbx config` drives, under a single entry named `user_data_templates`. So this reaches the same place:

```sh
rbx config --repository DataStoresConfig sync --env prod
```

That invocation works and will keep working. Same API, same repository mechanism, same revision history. What it cannot do is **check** the templates, because its entry model holds an opaque value: to it, `user_data_templates` is a list of tables like any other.

This command exists for the checks that one structurally cannot do. The case of `{UserId}`, the near-miss token in a scope, the defaulted scope, the ordered store that cannot be deleted whole, the template naming a store you no longer have: none of that is visible to a tool that treats the value as opaque, and every one of those mistakes is accepted by Roblox and then silently deletes nothing.
