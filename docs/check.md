# `rbx check` and `rbx status`

One command that runs every configured tool's check and returns a single exit
code. It is the CI contract: nothing to configure, nothing interactive, and the
exit code is the whole answer.

```sh
rbx check                 # the standalone config blocks
rbx check --env all       # every env in rbxplace.toml
rbx check --offline       # skip anything that needs the network
```

`rbx status` is the same engine with the opposite contract: the
human overview, grouped by environment, that **always exits 0**. See
[`rbx status`](#rbx-status) below.

| | `rbx check` | `rbx status` |
|---|---|---|
| answers | may this build continue | where does this project stand |
| shape | one line per check | one block per environment |
| exit code | `0` / `2` / `1` | always `0` |
| written for | CI | you, at a terminal |

## Exit codes

This is the part scripts depend on, so it is the part to read carefully.

| Code | Meaning | What to do |
|---|---|---|
| `0` | every check that ran came back clean | nothing |
| `2` | at least one check found drift, and none failed | run the sync or codegen the summary names, and commit |
| `1` | at least one check failed | read the message; a check could not answer |

Aggregation is **error beats drift beats clean**. A run that both fails one
check and finds drift in another exits `1`, because "something is broken" and
"something is stale" ask different things of whoever reads the log, and the
broken one is the one that has to be read first.

Skipped checks never raise the exit code. A repo without `rbxshop.toml` is not
a repo with a broken shop, and `--offline` is a deliberate narrowing, not a
failure.

In GitHub Actions:

```yaml
- run: rbx check --env all
  # fails the job on 1 and on 2 alike; use `continue-on-error` plus a
  # conditional on the outcome if drift should be a warning rather than a stop.
```

## What it runs

A tool is checked when its config file is present in the working directory.
There is no `[check]` block to maintain: the list of tools a repo uses is
already on disk.

`--dir` moves that lookup, `rbxplace.toml` included, and an explicit `--places`
overrides it for that one file. It is the same rule `rbx import --dir` writes
by, so `rbx import --dir game` followed by `rbx check --dir game` reads the
files the import just wrote:

| | `rbxplace.toml` read from |
|---|---|
| `rbx check` | `./rbxplace.toml` |
| `rbx check --dir game` | `game/rbxplace.toml` |
| `rbx check --dir game --places shared/envs.toml` | `shared/envs.toml` |

| Config file | Check | Network |
|---|---|---|
| `rbxplace.toml` | `env/gen-module`: the committed env module still matches | no |
| `rbxshop.toml` | `shop/lockfile`: declared passes/badges/products against the lockfile | no |
| `rbxshop.toml` | `shop/codegen`: the committed shop modules still match | no |
| `rbxmeta.toml` | `meta/lockfile`: declared universe/place metadata against the lockfile | no |
| `rbxconfig.toml` | `config/live`: local entries against the live config on Roblox | **yes** |
| `rbxapikey.toml` | `apikey/status`, not yet wired, see below | - |

Only `config/live` needs credentials, so `--offline` is a small cut: everything
else compares committed files against committed files. That makes the offline
mode usable from a pre-commit hook, which is the point of having it.

The key is only demanded once `config/live` has something to compare. With no
`--env`, or with an env `rbxconfig.toml` never declares, the row is skipped and
a keyless run still exits 0 without `--offline`.

Rows are named `tool/check [env]`. Per-env checks produce one row per env; with
no `--env`, each tool falls back to its standalone block and the row is labelled
`[default]`, matching the `[envs.default]` section those tools already write.

## Why this is not a wrapper around the per-tool checks

All five per-tool checks now agree on the contract (`0` clean, `2` drift, `1`
error) so **`rbx check` and `rbx shop check` no longer disagree on exit code
for the same repo**. Either can be trusted in CI; `rbx check` runs all of them
at once, which is the only difference.

It still does not call the per-tool commands, for a reason that is about
stdout rather than exit codes: those commands print as they decide, and under
`--json` stdout belongs to the document: a probe that shelled into them would
emit something `jq` cannot read. So each check rebuilds the comparison from the
same public pieces the command itself uses (the renderers and plan builders,
not a re-description of them) without touching any check body.

## `apikey` is discovered but not checked

`rbx apikey status` classifies key health (expiry, orphan lockfile entries,
missing secrets) and returns success whatever it finds, which the other
per-tool checks no longer do. Reaching that classification from here would mean
either widening
`rbx-apikey` internals or writing a second copy of the rules, and a second copy
of a rule that drifts from the first is the exact failure mode this command
exists to catch.

So the row is reported as skipped, by name, with the command to run by hand:

```
- apikey/status    not yet wired: run `rbx apikey status`
```

Wiring it up properly means giving `apikey status` a structured result to
return. That is worth doing and is not this change.

## Flags

| Flag | Effect |
|---|---|
| `--offline` | skip checks that need network access and credentials |
| `--json` | write the report to stdout as one JSON document |
| `--dir <path>` | look for config files here instead of the working directory, `rbxplace.toml` included |
| `--env <name>` | check one env; `--env all` expands through `rbxplace.toml` |
| `--places <path>` | where `rbxplace.toml` lives, overriding `--dir` for that file |

`rbx check` is non-interactive by construction: it never prompts, so it is safe
to run with no TTY.

## `--json`

```sh
rbx check --env all --json
```

One JSON document on stdout, nothing else. Diagnostics stay on stderr and the
exit code is unchanged, so a consumer can read the document, the stream, or the
status (whichever suits) and get the same answer.

```json
{
  "schema_version": 1,
  "outcome": "drift",
  "exit_code": 2,
  "totals": { "total": 4, "clean": 0, "drift": 2, "error": 0, "skipped": 2 },
  "checks": [
    {
      "tool": "env",
      "check": "gen-module",
      "outcome": "skipped",
      "summary": "no [codegen].output in rbxplace.toml"
    },
    {
      "tool": "meta",
      "check": "lockfile",
      "env": "prod",
      "outcome": "drift",
      "summary": "2 pending changes: run `rbx meta sync`",
      "details": ["name: (unset) → My Game (Live)", "server size: (unset) → 50"]
    }
  ]
}
```

### Fields

These names are the contract. Adding a field is not a breaking change
(consumers are expected to ignore what they do not recognise) but a field
changing meaning or disappearing bumps `schema_version`.

| Field | Type | Meaning |
|---|---|---|
| `schema_version` | integer | Document format. `1` today. Refuse a version you do not understand. |
| `outcome` | string | The aggregate: `clean`, `drift`, `error`, `skipped`. |
| `exit_code` | integer | The exit code `rbx check` returns: `0`, `2`, or `1`. Always agrees with the process under `check`; under `status`, which always exits 0, it is what `check` *would* return. |
| `totals.total` | integer | How many checks ran, including skipped ones. |
| `totals.clean` / `.drift` / `.error` / `.skipped` | integer | Counts by outcome. |
| `checks` | array of objects | One entry per check, in run order. |
| `checks[].tool` | string | `env`, `shop`, `meta`, `config`, `apikey`. |
| `checks[].check` | string | Which check within the tool: `gen-module`, `lockfile`, `codegen`, `live`, `status`. |
| `checks[].env` | string | The env. **Absent** on checks that are not per-env. |
| `checks[].outcome` | string | `clean`, `drift`, `error`, `skipped`. |
| `checks[].summary` | string | One line, the same text the human renderer shows. |
| `checks[].details` | array of strings | Per-change or per-file lines. **Absent** when empty. |

Optional fields are omitted rather than emitted as `null`, so `has("env")` is a
usable test. Every row is an object keyed by name, never a positional array: a
consumer survives a field being added, and does not survive a column shifting.

### In GitHub Actions

```sh
rbx check --env all --json > check.json || true
jq -r '.checks[] | select(.outcome == "drift")
       | "::warning title=rbx drift::\(.tool)/\(.check) \(.env // "") - \(.summary)"' check.json
exit "$(jq -r '.exit_code' check.json)"
```

### Scope

`--json` covers every read in the JSON issue, not just the `check` family:
`check` and `status`, `env list/get`, `servers list/versions/logs`,
`analytics query/metrics`, `ads list/get/status`, `place versions/places` and
the receipts from `place upload/promote/rollback`, `data get/list/revisions/diff`,
`memorystore get/list`, `shop list/show`, `config list/get/versions`,
`ban list/status`, `apikey list/status` and `apikey scopes show`, plus the
receipt from `publish`. Per-command field names are documented alongside each
command.

What every one of them shares is the helper: `rbx_core::output` is the only
place in the tree that serializes to stdout, which is what keeps `--json`
meaning the same thing everywhere: one document, notes and warnings on stderr,
optional fields omitted rather than `null`, and no prompt, ever. Commands that
stop and ask do not carry the flag at all.

## Example

```
$ rbx check --env all --offline
rbx check
  - env/gen-module        no [codegen].output in rbxplace.toml
  ✓ shop/lockfile [dev]   everything in sync
  ! shop/lockfile [prod]  1 to create, 0 to update: run `rbx shop sync`
  ✓ shop/codegen          generated modules match rbxshop.toml
  ! meta/lockfile [prod]  2 pending changes: run `rbx meta sync`
      name: (unset) → My Game (Live)
      server size: (unset) → 50
  - config/live           --offline: comparing against Roblox needs an API key
  - apikey/status         not yet wired: run `rbx apikey status`

! 2 checks found drift (2 clean, 3 skipped). Exit code 2.
```

The checks that compose an existing command (`env/gen-module`, `shop/codegen`)
print their own per-file detail above this summary, since that detail is what
tells you which generated file went stale.

## `rbx status`

The human half. Same discovery, same checks, same rows:
regrouped by environment and stripped of the exit-code contract.

```sh
rbx status                  # the standalone config blocks
rbx status --env all        # every env in rbxplace.toml
rbx status --offline        # the overview you can get with no key and no network
rbx status --json           # the document below, identical in shape to check's
```

```
$ rbx status --env all --offline
rbx status

  - repository
      - env/gen-module  no [codegen].output in rbxplace.toml

  ! dev
      ! meta/lockfile   1 pending change: run `rbx meta sync`
          name: (unset) → My Game (dev)

  ✓ prod
      ✓ meta/lockfile   everything in sync

! 1 check out of sync. Re-run the tool's own sync, or `rbx check` for the CI verdict.
rbx status always exits 0; rbx check here would exit 2.
```

**Always exit 0 is the point.** A status command that fails a script is a check
command with worse output, so `rbx status` is safe under `set -e`, in a shell
prompt, in a `watch`, and in the first line of a `Makefile` target that then
does something else. When you want the verdict, that is what `rbx check` is
for, and the last line says which one it would be. A repository it cannot read
at all is no exception: an unreadable or env-less `rbxplace.toml` prints as an
`env/discovery` error row, and the command still exits 0.

The `repository` block holds the checks that are not per-env (`env/gen-module`
compares a generated file, `apikey/status` answers for the credential).
Environments follow it in alphabetical order, which is the order `--env all`
expands them in.

It reads nothing it does not read for `check`, writes nothing, and is useful
with no API key: `--offline` renders the local half and marks the live rows
skipped rather than refusing to run.

### `rbx status --json`

The same document `rbx check --json` emits, field for field, so a consumer can
read either. One value is worth naming: `exit_code` is what **`rbx check`**
would return for this repository, since `rbx status` itself always exits 0. It
is the field to branch on if you want the verdict without the check's exit
code:

```sh
rbx status --env all --json > status.json   # always exits 0
jq -r '.checks[] | select(.outcome != "clean") | "\(.env // "repo") \(.tool)/\(.check): \(.summary)"' status.json
```

## Related

- `docs/env.md`: `rbx env gen-module --check` and the ignored-key policy
- `docs/ops.md`, which commands touch live state
