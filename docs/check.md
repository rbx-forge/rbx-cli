# `rbx check` and `rbx status`

`rbx check` runs the check of every tool configured in the directory and returns one exit code. It never prompts, so it is the command for CI. `rbx status` runs the same checks, grouped by env for a person, and always exits `0`.

```sh
rbx check                 # standalone config blocks
rbx check --env all       # every env in rbxplace.toml
rbx check --offline       # nothing that needs the network
```

## Exit codes

| Code | Meaning | What to do |
|---|---|---|
| `0` | every check that ran is clean | nothing |
| `2` | drift somewhere, no failure | run the sync or codegen the summary names, and commit |
| `1` | a check could not answer | read the message |

An error beats drift, drift beats clean. Skipped checks never raise the code. Each tool's own `check` command uses the same codes.

```yaml
- run: rbx check --env all   # fails the job on 1 and on 2
```

## What it runs

A tool is checked when its config file is in the directory.

| File | Check | Network |
|---|---|---|
| `rbxplace.toml` | `env/codegen`: the committed env module matches | no |
| `rbxplace.toml` | `env/root`: each env's start place is still Roblox's (envs named with `--env` that have places) | yes, no credential |
| `rbxshop.toml` | `shop/lockfile`: config against lockfile | no |
| `rbxshop.toml` | `shop/codegen`: the committed modules match | no |
| `rbxshop.toml` | `shop/live`: Roblox against the lockfile, for each env the lockfile has | API key |
| `rbxmeta.toml` | `meta/lockfile`: config against lockfile | no |
| `rbxmeta.toml` | `meta/live`: Roblox against the lockfile, for each env the lockfile has | API key |
| `rbxrtbf.toml` | `rtbf/templates`: every template can match something | no |
| `rbxrtbf.toml` | `rtbf/live`: the file against the published templates | API key |
| `rbxconfig.toml` | `config/live`: the file against the live config | API key |

Only the `live` checks need a key, and they are skipped when there is nothing to compare (no `--env`, no target universe, or nothing synced yet). So a keyless run exits `0` on a clean repository, and `--offline` is fast enough for a pre-commit hook.

!!! warning "Clean means the files agree with each other"
    `shop/lockfile` and `meta/lockfile` never ask Roblox. A resource dropped from both the config and the lockfile reports clean.

### Comparing with Roblox

`shop/live` and `meta/live` compare Roblox with the lockfile, not with the config, so a change not synced yet stays the `lockfile` row's and is not reported twice. A difference is a change made on Roblox since the last `sync` or `pull`: a price edited on the Creator Dashboard, a pass taken off sale, a link added by hand. `rbx shop pull --dry-run` and `rbx meta pull --dry-run` then show it in the context of the config.

```sh
rbx check --env all    # exit 2 when Roblox moved
```

Every run lists what it did not compare, so a clean row is never read as more than it is:

- `shop/live`: `regional_pricing`, which Roblox reports as on whenever managed pricing is, and the icon files (the icon id Roblox holds is compared);
- `meta/live`: the cookie-only fields unless a cookie is given (`--cookie`, `RBX_COOKIE` or `--auto-cookie`; `rbx check` never asks for one), the fields Roblox returns from no endpoint, and the media.

Rows are `tool/check [env]`; without `--env` they read `[default]`.

## Flags

| Flag | Effect |
|---|---|
| `--offline` | Skip checks that need the network |
| `--json` | One JSON document on stdout |
| `--dir <path>` | Look for config files there, `rbxplace.toml` included |

`--places` overrides `--dir` for `rbxplace.toml` alone.

## Example

```
$ rbx check --env all --offline
rbx check
  - env/codegen        no [codegen].output in rbxplace.toml
  ✓ shop/lockfile [dev]   everything in sync
  ! shop/lockfile [prod]  1 to create, 0 to update: run `rbx shop sync`
  ! meta/lockfile [prod]  2 pending changes: run `rbx meta sync`
      name: (unset) → My Game (Live)
      server size: (unset) → 50
  - config/live           --offline: comparing against Roblox needs an API key

! 2 checks found drift (2 clean, 2 skipped). Exit code 2.
```

## `--json`

```json
{
  "schema_version": 1,
  "outcome": "drift",
  "exit_code": 2,
  "totals": { "total": 4, "clean": 0, "drift": 2, "error": 0, "skipped": 2 },
  "checks": [
    {
      "tool": "meta",
      "check": "lockfile",
      "env": "prod",
      "outcome": "drift",
      "summary": "2 pending changes: run `rbx meta sync`",
      "details": ["name: (unset) → My Game (Live)"]
    }
  ]
}
```

| Field | Type | Meaning |
|---|---|---|
| `outcome` | string | `clean`, `drift`, `error` or `skipped` |
| `exit_code` | integer | What `rbx check` returns (also under `status`, which exits `0`) |
| `totals` | object | `total`, `clean`, `drift`, `error`, `skipped` |
| `checks[].tool`, `.check` | string | e.g. `shop`, `lockfile` |
| `checks[].env` | string | Absent on checks that are not per env |
| `checks[].outcome` | string | As above |
| `checks[].summary` | string | The line the human output shows |
| `checks[].details` | string[] | Absent when empty |

Adding a field does not change `schema_version`; changing or removing one does.

Drift as GitHub annotations:

```sh
rbx check --env all --json > check.json || true
jq -r '.checks[] | select(.outcome == "drift")
       | "::warning title=rbx drift::\(.tool)/\(.check) \(.env // "") - \(.summary)"' check.json
exit "$(jq -r '.exit_code' check.json)"
```

## `rbx status`

```sh
rbx status --env all
rbx status --offline
rbx status --json      # same document as check
```

```
$ rbx status --env all --offline
rbx status

  - repository
      - env/codegen  no [codegen].output in rbxplace.toml

  ! dev
      ! meta/lockfile   1 pending change: run `rbx meta sync`

  ✓ prod
      ✓ meta/lockfile   everything in sync

! 1 check out of sync. Re-run the tool's own sync, or `rbx check` for the CI verdict.
rbx status always exits 0; rbx check here would exit 2.
```

Always `0`, even on an unreadable `rbxplace.toml` (shown as an `env/discovery` error), so it is safe under `set -e` or in a shell prompt. For a verdict, read `exit_code` from `--json`, or run `rbx check`.
