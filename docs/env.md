# rbx env

Read `rbxplace.toml`: the shared file that maps env names to universe and place ids.

Every other subcommand resolves `--env` against this file. `rbx env` is the read side of it: it answers "what id will `--env prod` actually target?" without opening the TOML by hand, and without calling Roblox. It is fully offline: no API key, no cookie.

## Features

- **List** - See every env, its universe id, and its places, rendered in the file's own TOML shape
- **Get** - Print one bare value to stdout, ready for `$(...)` capture in scripts and CI
- **Set** - Write the settings nothing on Roblox can produce: `[owner]`, `[codegen]`, `[groups]`, and the per-env `codegen` / `confirm`
- **Gen-module** - Export the whole map as a Luau/Lua/JSON/TypeScript module for your game code, with `--check` to prove the committed copy was never hand-edited
- **JSON** - `--json` on `list` and `get` writes one document to stdout and nothing else, with documented field names, for `jq` and CI
- **Completions** - The env and place names in this file are what `--env <TAB>` and `--place <TAB>` offer, in bash, zsh, fish and PowerShell
- **Same resolution as everything else** - Delegates to the shared resolver, so `rbx env get place-id --env prod` prints the exact id `rbx place upload --env prod` would write to
- **Offline** - Reads the local file only

## Quick start

```sh
rbx env list                            # everything in rbxplace.toml
rbx env get universe-id --env prod      # 9876543211
```

## Commands

<details markdown="1">
<summary><code>rbx env list</code></summary>

Show the envs defined in `rbxplace.toml`. Pass the global `--env <name>` to show a single one.

```sh
rbx env list
rbx env list --env prod       # just this env
rbx env list --names          # env names only, one per line
rbx env list --place-names    # place names only, one per line
rbx env list --json           # one JSON document on stdout
```

| Flag | Description |
| --- | --- |
| `--names` | Print env names only, one per line, no colors, for scripts and completion helpers |
| `--place-names` | Print place names only, one per line. Every env's, deduplicated, unless `--env` narrows it |
| `--json` | Write the envs to stdout as one JSON document. Rejected together with either name listing |
| `--env <name>` | Show only this env (global flag). `--env all` is the same as omitting it |
| `--places <path>` | Path to `rbxplace.toml` (global flag, default `rbxplace.toml`) |

Output mirrors the file so it maps back onto what you'd edit:

```
rbxplace.toml
owner = group 1234567

[dev]
universe_id  = 9876543210
places.lobby = 987654321
places.main  = 123456789012345

[prod]  confirm
universe_id = 9876543211
owner       = user 42
places.main = 234567890123456
```

`confirm` next to the env header means that env has `confirm = true` and will prompt before write operations. A per-env `owner` line appears only when `[<env>.owner]` overrides the top-level `[owner]`.

### `--json`

One JSON document on stdout, nothing else. Diagnostics (the unknown-key warning in particular) stay on stderr, so the document parses even when the file has something wrong with it.

```json
{
  "schema_version": 1,
  "places_file": "rbxplace.toml",
  "owner": { "type": "group", "id": 1234567 },
  "envs": [
    {
      "name": "dev",
      "universe_id": 9876543210,
      "confirm": false,
      "codegen": true,
      "places": { "lobby": 987654321, "main": 123456789012345 }
    },
    {
      "name": "prod",
      "universe_id": 9876543211,
      "env": "production",
      "owner": { "type": "user", "id": 42 },
      "confirm": true,
      "codegen": true,
      "places": { "main": 234567890123456 }
    }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today. Refuse a version you do not understand |
| `places_file` | string | The `rbxplace.toml` this was read from, as given or defaulted |
| `owner` | object | The top-level `[owner]`. **Absent** when the file sets none |
| `owner.type` / `.id` | string / integer | `user` or `group`, and the id |
| `envs` | array of objects | One per env, in name order. Narrowed to one entry by `--env <name>` |
| `envs[].name` | string | The section name: what `--env` takes |
| `envs[].universe_id` | integer | `universe_id` of the env |
| `envs[].env` | string | The `env` rename. **Absent** when unset, in which case `name` is the answer |
| `envs[].owner` | object | The per-env `[<env>.owner]` override. **Absent** when the env inherits the top-level one |
| `envs[].confirm` | boolean | Whether writes to this env prompt first |
| `envs[].codegen` | boolean | Whether `rbx env gen-module` emits this env |
| `envs[].places` | object | Place name to place id. Empty for envs used only at universe scope |

`envs[].owner` is the override as the file spells it, never the resolved value, so `.envs[].owner // .owner` reproduces the fallback and "inherited" stays distinguishable from "overridden". Optional fields are omitted rather than emitted as `null`, so `has("owner")` is a usable test.

A file with no envs is an empty `envs` array and exit 0, matching `--names` rather than the human listing, which errors. Both are read by scripts; the error is for the person who asked to see the file.

```sh
rbx env list --json | jq -r '.envs[] | select(.confirm) | .name'   # envs that prompt
rbx env list --json | jq -r '.envs[].places | keys[]' | sort -u    # every place name
```

### The two name listings

Two listings meant to be read by something other than a person. Both write one bare value per line to stdout, nothing else, no colors, and both exit 0 on a file that simply has nothing to list:

```sh
rbx env list --names          # dev
                              # prod
rbx env list --place-names    # lobby
                              # main
```

These are a supported surface, not debug output. The shell completions generated by `rbx completions` call exactly these two commands, and so can your scripts: the format is one value per line and will not grow columns, headers or color.

`--place-names` answers across every env by default, deduplicated: a place name is a role, and `main` defined in three envs is one candidate, not three. Narrow it with the global `--env`:

```sh
rbx env list --place-names --env prod
```

The union is the default because the question is usually asked before an env has been chosen: a completion for `--place` cannot wait for `--env` to be typed. The cost is that in a file where envs hold genuinely different places, the unnarrowed list offers names that the env you eventually pick does not have; the command you run then says so.

Both listings fail with an empty stdout when `rbxplace.toml` is missing or does not parse. The diagnostic goes to stderr, so `2>/dev/null` is all a caller needs to get silence instead of an error.

</details>

<details markdown="1">
<summary><code>rbx env get</code></summary>

Print a single value. The value goes to stdout bare (no label, no color, no trailing decoration) so it can be captured directly.

```sh
UNIVERSE=$(rbx env get universe-id --env prod)
PLACE=$(rbx env get place-id --env prod --place lobby)

rbx env get owner-id                       # top-level [owner], no --env needed
rbx env get universe-id --env all          # one "<env><TAB><value>" line per env
```

| Field | Value | Needs `--env` |
| --- | --- | --- |
| `universe-id` | `universe_id` of the env | yes |
| `place-id` | Place id from `[<env>.places]`, honoring `--place` | yes |
| `owner-id` | Owner id: `[<env>.owner]` if set, else top-level `[owner]` | no |
| `owner-type` | `user` or `group`, resolved the same way | no |

The file's own snake_case spellings are accepted as aliases (`universe_id`, `place_id`, …), so you can type what you see in the TOML.

Without `--place`, `place-id` follows the same defaulting rule as every other subcommand: `main` if it exists, otherwise the only entry, otherwise an error listing the available names.

With `--env all`, output is tab-separated so it pipes cleanly. A group behaves the same way, over its members in declared order rather than every env in name order:

```sh
rbx env get universe-id --env all | cut -f2
```

Missing envs, missing places, and a missing `[owner]` are errors (exit code 1) with the available options listed on stderr: never a silent empty value, so a failed lookup can't quietly become an empty shell variable.

### `--json`

Same answer, wrapped so a script can tell which field it asked for and which env replied. One JSON document on stdout, nothing else.

```sh
rbx env get universe-id --env prod --json
```

```json
{
  "schema_version": 1,
  "field": "universe-id",
  "value": "9876543211",
  "results": [{ "env": "prod", "value": "9876543211" }]
}
```

```sh
rbx env get universe-id --env all --json
```

```json
{
  "schema_version": 1,
  "field": "universe-id",
  "results": [
    { "env": "dev", "value": "9876543210" },
    { "env": "prod", "value": "9876543211" }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today |
| `field` | string | The field asked for, in its canonical spelling: `universe-id`, `place-id`, `owner-id`, `owner-type`: the alias you typed is normalized |
| `value` | string | The answer, when one env was targeted. **Absent** under `--env all` |
| `results` | array of objects | One entry per env answered, in the same order the human form prints. Always present |
| `results[].env` | string | The env. **Absent** when the lookup needed none: `owner-id` and `owner-type` answer from the top-level `[owner]` without `--env` |
| `results[].value` | string | The value |

Values are **always strings**, including the ids: it is the same text the bare form prints, one filter reads `owner-type` and `universe-id` alike, and a 64-bit place id is never handed to a consumer that would round it.

Which of `value` and `results` you get is decided by the invocation, never by the data. `--env all` omits `value` even against a file with exactly one env, so a filter cannot start working by accident and break when a second env lands.

```sh
UNIVERSE=$(rbx env get universe-id --env prod --json | jq -r .value)
rbx env get place-id --env all --json | jq -r '.results[] | "\(.env)=\(.value)"'
```

</details>

<details markdown="1">
<summary><code>rbx env gen-module</code></summary>

Export the env map as a module your game code can import, so runtime code branches on the env it's running in instead of hardcoding ids.

```sh
rbx env gen-module --out src/types/EnvironmentInfo.luau
rbx env gen-module --out src/environments.lua
rbx env gen-module --out config/environments.json
rbx env gen-module --out src/types/EnvironmentInfo.ts
```

| Flag | Description |
| --- | --- |
| `--out` | Output file path. Format inferred from the extension: `.lua`, `.luau`, `.json`, or `.ts`. Optional when `[codegen].output` is set |
| `--check` | Compare the existing file against what would be generated instead of writing it. Exits `2` on a difference |

### Declaring the path once

Rather than repeating `--out` in your shell, your hook and your CI job, put it in `rbxplace.toml`:

```toml
[codegen]
output = "src/shared/Envs.luau"   # relative to rbxplace.toml

[dev]
universe_id = 9876543210
```

```sh
rbx env gen-module           # writes src/shared/Envs.luau
rbx env gen-module --check   # verifies the same file
```

This is the form to prefer wherever the check runs. A `--check` spelled with a different path than the generator passes green while verifying a file nobody consumes: the one failure mode a drift guard cannot afford. An explicit `--out` still wins when passed.

`[codegen]` is a reserved section, like `[owner]`: it is not read as an env, and every tool that rewrites `rbxplace.toml` preserves it.

### Checking the committed copy

`--check` re-renders in memory and asserts the file on disk still matches `rbxplace.toml`, so a module that was edited by hand (or left stale after an env changed) fails instead of shipping. It stays offline, so it runs in a pre-commit hook and in CD. See [Guarding generated files](shop.md#guarding-generated-files) for the hook and CI snippets, and for the one thing that breaks the comparison.

The output is an array of environment objects, each with `env`, `universeId`, and `placeIds` (an array of `{ name, id }`). Luau and TypeScript additionally get a union type of every env name:

```luau
export type EnvironmentType = "dev" | "prod"

export type EnvironmentInfo = {
  env: EnvironmentType,
  universeId: number,
  placeIds: { { name: string, id: number } },
}

local envs: { EnvironmentInfo } = { ... }
return envs
```

The optional `env` key in `rbxplace.toml` overrides the name game code matches on (it defaults to the section name):

```toml
[dev]
universe_id = 1234567890
env = "Dev"        # what EnvironmentType will contain
[dev.places]
main = 111111111
```

It renames an env; it does not alias two envs onto one. Two sections resolving to the same name (whether through two `env` fields or an `env` that collides with another section's name) is rejected when the file loads, with both sections named. Env names are what game code matches on, so a duplicate would make every lookup by name ambiguous.

Envs and places are emitted in name order, so regenerating from an unchanged `rbxplace.toml` produces a byte-identical file: safe to commit and to run in a pre-commit hook.

</details>


<details markdown="1">
<summary><code>rbx env set</code></summary>

Write one local setting into `rbxplace.toml`, preserving comments and key order.

```sh
rbx env set owner --type group --id 1234567890
rbx env set codegen-output generated/shared/Environments.luau
rbx env set group shops dev,staging,prod
rbx env set codegen false --env ci
rbx env set confirm true  --env prod
```

| Setting | Writes | Needs `--env` |
| --- | --- | --- |
| `owner --type <group\|user> --id <id>` | `[owner]` | No |
| `codegen-output <path>` | `[codegen] output` | No |
| `group <name> <envs>` | one `[groups]` entry | No |
| `codegen <true\|false>` | the env's `codegen` | **Yes** |
| `confirm <true\|false>` | the env's `confirm` | **Yes** |

**Only these five, and the rule is not arbitrary.** Every other file in this suite can be rebuilt from Roblox (`shop init --from-remote`, `meta init --from-remote`, `config pull`). These five have no counterpart on Roblox, so no import can ever bring them back: which module to generate, which envs a name fans out over, which env is dangerous enough to ask about first, and who owns the universes when the group was made on the website rather than by [`rbx init create-group --record`](./init.md).

Ids are deliberately **not** settable. `rbx init` writes `universe_id` and `places.*` when it creates the thing they name, and a flag that lets you type one by hand is a flag that lets you point prod at a test universe.

**Setting a value the file already has writes nothing** and says so, so a bootstrap script can be re-run after a failure halfway through without producing a diff. Changing a value that differs asks first; `--yes` answers. Adding one that was absent never asks, including `confirm true` on an env that never mentioned it: nothing is being overwritten there, only the default it was inheriting.

**A change that would make the file unloadable is refused before it is written.** The new document is parsed by the same loader every other command uses, so `rbx env set group shops dev,qa` in a file with no `[qa]` fails with that loader's own message, listing the envs that do exist, and leaves the file alone.

`owner` and `codegen-output` create `rbxplace.toml` when it does not exist yet, which is what lets a fresh repository be pointed at an existing group. The other three name an env, so they need a file that has some.

</details>

<details markdown="1">
<summary><code>rbx env rm</code></summary>

Remove an env from `rbxplace.toml` and from every file keyed by it.

```sh
rbx env rm staging --dry-run   # list what would go
rbx env rm staging             # asks before writing
rbx env rm staging --yes       # for scripts
```

| Flag | Description |
| --- | --- |
| `--dry-run` | List what would be removed without writing anything |
| `-y`, `--yes` | Skip the confirmation prompt |
| `--places <path>` | Path to `rbxplace.toml` (global flag, default `rbxplace.toml`) |

The env is named as a positional argument, not read from the global `--env`. This is the one command where naming the wrong env deletes something, and `--env` is a flag people leave set in a shell for a whole session.

**What it touches**, when the file exists:

| File | What goes |
| --- | --- |
| `rbxplace.toml` | The `[<env>]` block |
| `rbxmeta.toml` | The `[envs.<env>]` overlay |
| `rbxmeta.lock.toml` | The `[envs.<env>]` section |
| `rbxshop.toml` | The `[envs.<env>]` overlay |
| `rbxshop.lock.toml` | The `[envs.<env>]` section |
| `rbxconfig.lock.toml` | The `[envs.<env>]` section |
| `rbxapikey.lock.toml` | The `[envs.<env>]` section |
| `rbxplace.toml` | The env's block, and its name out of every `[groups]` array. A group left empty is deleted with it |
| `rbxapikey.toml` | The env's name, out of every list that holds it |
| `<codegen.output>/<env>.luau` | The per-env module `rbx shop codegen` wrote |

`rbxapikey.toml` is the odd one. Every other file gives an env a table of its own, removed whole; this one names envs *inside arrays*: `[settings] default_envs`, and each key's `envs`, itself either one list or one list per named group. All of them are walked. Leaving a name behind would not be untidiness: an env that `rbxplace.toml` no longer defines is an error to the api key commands, not something they skip, so the next `rbx apikey` run would fail on a file you never edited.

Emptying one of those lists is reported rather than done quietly, because it changes what a key targets. A key whose own `envs` is empty falls back to `[settings] default_envs`, so a key that named only the removed env may now reach envs it never named: a removal *widening* something. An emptied group is the same problem from the other side: group names are key identity, so what is left is a key declaration targeting nothing. The command prints each list it emptied and leaves the decision to you.

Everything is planned before anything is written, so a file that fails to parse stops the run rather than leaving the project half-edited. Comments and key order survive: the files are edited as documents, not reserialised through the config model.

The aggregate generated files (`init.luau`, the type module, whatever `rbx env gen-module` writes) are *regenerated*, not deleted, so the command names them at the end instead of touching them.

**Nothing is deleted on Roblox, and nothing could be.** A game pass or a developer product cannot be deleted there at all, only taken off sale; a badge can only be disabled; a universe can be deactivated and is still there. A command called `destroy` would be describing something it does not do, on resources people paid money for. This removes the env, which is the part that really can be removed.

An env that is not in `rbxplace.toml` is refused, and the error names the ones that are: a typo must not report success having done nothing. `[owner]`, `[codegen]` and `[groups]` are top-level tables and not envs, so they are refused too.

</details>


## Every field, and where it goes

`rbxplace.toml` has exactly three reserved top-level tables. **Every other top-level table is an env**, named by its key.

```toml
# ── reserved ────────────────────────────────────────────────
[owner]                              # who owns this project
type = "group"                       # "user" or "group"
id = 1234567

[codegen]                            # where `rbx env gen-module` writes
output = "src/shared/Envs.luau"      # relative to this file

[groups]                             # named subsets of the envs below
nonprod = ["dev", "staging"]

# ── everything below is an env ──────────────────────────────
[prod]
universe_id = 9876543211
confirm = true                       # prompt before writes to this env
env = "Production"                   # what game code matches on
owner = { type = "user", id = 42 }   # overrides the top-level [owner]
[prod.places]
main = 234567890123456
lobby = 234567890999999

[ci]
universe_id = 555
codegen = false                      # tooling env: keep it out of the module
```

### Reserved tables

| Table | Field | Type | Default | Meaning |
| --- | --- | --- | --- | --- |
| `[owner]` | `type` | `"user"` \| `"group"` | - | Who owns the project. Tools without their own owner field fall back to this |
| `[owner]` | `id` | integer | - | The user or group id |
| `[codegen]` | `output` | path | - | Where `rbx env gen-module` writes, relative to this file. Omit and `--out` becomes required |
| `[groups]` | *(any name)* | array of env names | - | A named subset of the envs, usable anywhere `--env` is. See [Groups](#groups) |

### Env fields

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `universe_id` | integer | **required** | The universe this env targets |
| `places` | table | `{}` | Place name → place id. `main` is the default when `--place` is omitted |
| `confirm` | bool | `false` | Prompt before write operations on this env (`upload`, `sync`, `rollback`, `promote`) |
| `env` | string | the section name | What game code matches on. A **rename**, not an alias: two envs resolving to the same name is an error |
| `owner` | table | the top-level `[owner]` | Per-env owner override, for the rare env living under a different account |
| `codegen` | bool | `true` | `false` keeps the env out of the generated modules: see below |

Where a field carries a **(X.Y.Z+)** tag, it needs at least that release. This page describes `main`, which is where a feature lands before it ships (and `/blob/main/docs/env.md` is the URL links and search results hand you) so a tagged field is newer than whatever `rokit.toml` pins until you check `rbx --version`. Nothing in the table above is tagged today: every field here is in the latest release.

### `codegen = false`

For an env that exists for tooling and never ships: a universe you upload to from CI, for instance. It stays a normal env everywhere else: `--env` resolves it, `--env all` includes it, `list` and `get` report it. It is only kept out of the generated modules, so adding one does not widen `EnvironmentType` and force game code to acknowledge an env it never runs in.

The trade is real and worth stating: nothing then maps that universe back to an env at runtime. If the game boots there and your code resolves its env from `game.GameId`, it will not find one, and `rbx shop`'s dispatcher errors outright rather than guessing. Correct for a universe that only ever receives uploads; wrong for one that runs gameplay.

Marking every env `codegen = false` is refused rather than emitting a module whose type union has nothing in it.

### Groups

**(0.5.0+)** A group is a **name for a set of envs, and nothing more**.

```toml
[groups]
nonprod = ["dev", "staging", "qa"]
live    = ["prod"]

[dev]
universe_id = 109876543210987
[staging]
universe_id = 109876543210988
[qa]
universe_id = 109876543210989
[prod]
universe_id = 209876543210987
```

`--env nonprod` then runs what `--env all` would run, over those three envs instead of every env:

```sh
rbx shop sync --env nonprod          # three envs, not four
rbx check --env nonprod
rbx meta sync --env nonprod
rbx place upload --env nonprod --file place.rbxl
```

That is the whole feature. `--env all` becomes the group nobody has to declare rather than the only grouping that exists.

**A group is expanded before anything else happens.** No lockfile, no overlay and no generated module ever sees the name `nonprod`. That is not a convention, it is what keeps the feature to one idea: every lockfile in the suite keys on `(env name → universe_id)`, and a group has no universe of its own to record. `[envs.nonprod]` in `rbxmeta.toml` or `rbxshop.toml` is **not** a thing, and will not become one: an overlay shared by several envs is what the base table is for, with the exception written as the one env that diverges.

A group name is refused wherever `--env all` is refused, and with the same reasoning. `rbx ban`, `rbx restart`, `rbx data` and the rest act on one universe, so they name what you gave them and what to do instead:

```text
$ rbx restart launch --env nonprod
Error: `--env nonprod` is a group of 3 envs (dev, staging, qa); this command acts on one universe. Name one of them.
```

#### What is refused at load

All four fail the file for every command, not just the one you ran, so a bad group cannot be discovered halfway through a deploy:

| Written | Refused because |
| --- | --- |
| `nonprod = ["dev", "qa"]` with no `[qa]` | A member must be a declared env, or the group silently targets fewer envs than it reads as. The error lists the envs that do exist |
| `outer = ["inner"]` where `inner` is a group | Groups are flat. Nesting invites cycles and buys nothing a second line does not |
| `staging = ["dev"]` beside `[staging]` | `--env staging` cannot mean both, and picking either silently is worse than refusing |
| `all = [...]`, `owner`, `codegen`, `groups` | Reserved names. A group called `all` would shadow the selector every command already has |
| `nonprod = []` | A declaration that targets nothing is never what was meant |

#### Groups and `codegen`

A group is not an env, so it never reaches `EnvironmentType` in the generated module. Game code branches on the env it is running in, and no universe maps to `nonprod`; putting it in the union would hand the game a runtime env that resolves to nothing.

`rbx env list` prints the `[groups]` table with the file, under the same heading it has in the TOML, and `rbx env list --names` prints envs only. A group is not a value `--env` resolves to at runtime, it is a way of naming several.

### Unrecognised keys

A key no table in the list above claims is **ignored, and named on stderr**:

```
warning: rbxplace.toml: 1 unrecognised key, ignored by rbx 0.2.0:

  [assets] codgen
    known keys: universe_id, env, places, owner, confirm, codegen

An ignored key changes nothing. Either it is misspelled, or it comes from a
release newer than the one you are running: check the changelog for the
version that introduces it before assuming it took effect.
```

It stays a warning rather than an error on purpose. Every tool in the suite reads this one file into its own narrower struct, and a key must survive an rbx older than the release that introduced it: otherwise adopting a new field would mean upgrading every machine in the same instant. What it must not do is pass for *applied*: from the outside, an ignored key and an honoured one produced the same silent exit 0.

`gen-module --check` carries the same fact into its failure. Its normal advice (regenerate and commit) assumes the committed module is the stale side. When a key was ignored, the check itself is reading the inputs wrong, the committed file may be the correct one, and regenerating would bake the misreading in. So the check names that possibility instead of stating the fix unconditionally:

```
1 generated file no longer matches rbxplace.toml. Run `rbx env gen-module`
and commit the result, unless one of the following applies.

1 key in rbxplace.toml was ignored (listed above). If one of them was meant
to change what is generated, this check is reading the wrong inputs and the
committed file may be the correct one: regenerating would bake the
misreading in. Upgrade rbx, or fix the spelling, before running the fix.
```

Place names under `[<env>.places]` are data, not keys, and are never reported.

## Shell completions for `--env` and `--place`

`rbx completions <shell>` writes a script that completes both with the names in the `rbxplace.toml` of the directory you are standing in. It calls [`rbx env list --names`](#the-two-name-listings) and `--place-names` at TAB time rather than baking the values in, which is why those two listings are a supported surface rather than debug output.

The install paths, the four shells, and what happens outside a project are on [its own page](completions.md).

## Where the file comes from

Four commands write it, and each writes only its own part:

- `rbx init create-group --record`: the `[owner]` block, for a group it just created
- `rbx init create-universe` / `create-place`: the `[<env>]` section and `places.*` for what they created
- `rbx env set`: the local settings nothing on Roblox knows about, listed above
- `rbx place fetch --env <name> --write`: refresh one env's places from the live universe

Otherwise it is yours to write. A minimal file is one `[<env>]` section with a `universe_id`; `rbx init list-universes` prints the ids to put in it.

> **Nothing generates this file wholesale, and that is deliberate.** Every writer here only inserts lines. Reserializing the document through serde would drop comments, reorder keys, and silently delete any field it does not model, `env` overrides included.

## See also

- [`rbx init`](./init.md): create the universes and places this file points at
- [`rbx place`](./place.md): upload, download, and promote place files across these envs
- [`rbx open`](./open.md): launch Studio at one of these places
