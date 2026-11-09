# rbx env

Read and edit `rbxplace.toml`, the file that maps env names to universe and place ids. Every other command resolves `--env` through it. `rbx env` is offline: no API key, no cookie.

```sh
rbx env list                            # every env, its universe and places
rbx env get universe-id --env prod      # 9876543211
```

## Commands

### rbx env list

Show the envs in `rbxplace.toml`, in the file's own shape. `--env <name>` shows one.

```sh
rbx env list
rbx env list --env prod
rbx env list --json
```

| Flag | Description |
| --- | --- |
| `--names` | Env names only, one per line |
| `--place-names` | Place names only, one per line, deduplicated across envs unless `--env` narrows it |
| `--json` | One JSON document on stdout. Not combinable with the two above |

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

`confirm` after a header means writes to that env prompt first. A per-env `owner` line appears only when it overrides the top-level `[owner]`.

#### The two name listings

`--names` and `--place-names` print one bare value per line and nothing else. They are a stable interface: the [shell completions](completions.md) call them, and so can your scripts. A missing or broken `rbxplace.toml` gives an empty stdout, with the error on stderr.

#### `--json`

```json
{
  "schema_version": 1,
  "places_file": "rbxplace.toml",
  "owner": { "type": "group", "id": 1234567 },
  "envs": [
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
| `schema_version` | integer | `1` |
| `places_file` | string | The file read |
| `owner` | object | Top-level `[owner]`: `type` (`user` or `group`) and `id`. Absent if unset |
| `envs[]` | array | One per env, in name order |
| `envs[].name` | string | What `--env` takes |
| `envs[].universe_id` | integer | |
| `envs[].env` | string | The `env` rename. Absent if unset |
| `envs[].owner` | object | Per-env override as written. Absent when inherited |
| `envs[].confirm` / `.codegen` | boolean | |
| `envs[].places` | object | Place name to id |
| `envs[].root` | string | The start place. Absent if unset |

A file with no envs gives an empty `envs` array and exit 0.

```sh
rbx env list --json | jq -r '.envs[] | select(.confirm) | .name'
```

### rbx env get

Print one value, bare, for `$(...)`.

```sh
UNIVERSE=$(rbx env get universe-id --env prod)
PLACE=$(rbx env get place-id --env prod --place lobby)
rbx env get owner-id
rbx env get universe-id --env all      # "<env><TAB><value>" per line
```

| Field | Value | Needs `--env` |
| --- | --- | --- |
| `universe-id` | the env's `universe_id` | yes |
| `place-id` | from `[<env>.places]`: `--place`, else the start place, else the only place | yes |
| `owner-id` | `[<env>.owner]` if set, else `[owner]` | no |
| `owner-type` | `user` or `group`, resolved the same way | no |

The TOML spellings (`universe_id`, `place_id`, ...) work too. A missing env, place or owner exits `1` and lists what exists, so a failed lookup never becomes an empty variable.

#### `--json`

```json
{
  "schema_version": 1,
  "field": "universe-id",
  "value": "9876543211",
  "results": [{ "env": "prod", "value": "9876543211" }]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `field` | string | The field, in its canonical spelling |
| `value` | string | The answer when one env was targeted. Absent with `--env all` or a group |
| `results[]` | array | One `{ env, value }` per env. `env` is absent for `owner-*` read from the top-level `[owner]` |

Values are always strings, ids included.

### rbx env codegen

Write the env map as a module for game code.

```sh
rbx env codegen --out src/shared/Envs.luau
rbx env codegen --check
```

| Flag | Description |
| --- | --- |
| `--out` | Output path. `.luau`, `.lua`, `.json` or `.ts`, by extension. Optional when `[codegen].output` is set |
| `--check` | Compare the file on disk instead of writing it. Exits `2` on a difference |

Set the path once so the generator and the check cannot disagree:

```toml
[codegen]
output = "src/shared/Envs.luau"   # relative to rbxplace.toml
```

The module is an array of `{ env, universeId, rootPlaceId, placeIds }`. Luau and TypeScript also get a union of the env names:

```luau
export type EnvironmentType = "dev" | "prod"

export type EnvironmentInfo = {
  env: EnvironmentType,
  universeId: number,
  rootPlaceId: number?,
  placeIds: { { name: string, id: number } },
}
```

Output is sorted, so an unchanged `rbxplace.toml` regenerates a byte-identical file. Hook and CI snippets: [Guarding generated files](shop-codegen.md#guarding-generated-files).

### rbx env set

Write one setting that Roblox cannot give back, preserving comments and key order.

```sh
rbx env set owner --type group --id 1234567890
rbx env set codegen-output src/shared/Envs.luau
rbx env set group nonprod dev,staging
rbx env set codegen false --env ci
rbx env set confirm true --env prod
```

| Setting | Writes | Needs `--env` |
| --- | --- | --- |
| `owner --type <group\|user> --id <id>` | `[owner]` | no |
| `codegen-output <path>` | `[codegen] output` | no |
| `group <name> <envs>` | one `[groups]` entry | no |
| `codegen <true\|false>` | the env's `codegen` | yes |
| `confirm <true\|false>` | the env's `confirm` | yes |

Ids are not settable: `rbx init` writes them when it creates what they name. Setting a value already there writes nothing; changing one asks first (`--yes` answers). A change that would make the file unloadable is refused before writing. `owner` and `codegen-output` create the file if it does not exist.

### rbx env rm

Remove an env from `rbxplace.toml` and from every file keyed by it.

```sh
rbx env rm staging --dry-run
rbx env rm staging
```

| Flag | Description |
| --- | --- |
| `--dry-run` | List what would be removed |
| `-y`, `--yes` | Skip the confirmation |

The env is a positional argument, never read from `--env`, which people leave set in a shell.

| File | What goes |
| --- | --- |
| `rbxplace.toml` | The `[<env>]` block, and the env from every `[groups]` list (an emptied group goes too) |
| `rbxmeta.toml`, `rbxshop.toml` | The `[envs.<env>]` overlay |
| `rbxmeta.lock.toml`, `rbxshop.lock.toml`, `rbxconfig.lock.toml`, `rbxapikey.lock.toml` | The `[envs.<env>]` section |
| `rbxapikey.toml` | The env's name, from every list that holds it |
| `<codegen.output>/<env>.luau` | The per-env shop module |

Everything is planned before anything is written. A list in `rbxapikey.toml` that ends up empty is reported, since a key with no `envs` falls back to `default_envs`. Aggregate generated files are named for you to regenerate, not deleted.

!!! warning "Nothing is deleted on Roblox"
    Passes and products cannot be deleted there, badges can only be disabled, universes only deactivated. `rm` removes the env from your files; what it pointed at stays live.

## The file

Three top-level tables are reserved. Every other top-level table is an env.

```toml
[owner]                              # who owns the project
type = "group"                       # "user" or "group"
id = 1234567

[codegen]
output = "src/shared/Envs.luau"

[groups]
nonprod = ["dev", "staging"]

[prod]
universe_id = 9876543211
confirm = true                       # prompt before writes
env = "Production"                   # the name game code matches on
owner = { type = "user", id = 42 }   # overrides [owner]
root = "lobby"                       # start place, when it is not `main`
[prod.places]
main = 234567890123456
lobby = 234567890999999

[ci]
universe_id = 555
codegen = false                      # keep out of generated modules
```

### Env fields

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `universe_id` | integer | required | The universe |
| `places` | table | `{}` | Place name to id |
| `root` | string | `main` | Which place is the start place. See [`root`](#root) |
| `confirm` | bool | `false` | Prompt before `upload`, `sync`, `rollback`, `promote` |
| `env` | string | section name | What game code matches on. Two envs resolving to one name is an error |
| `owner` | table | `[owner]` | Per-env owner |
| `codegen` | bool | `true` | `false` keeps the env out of generated modules |

### `root`

The universe's start place, and the default target when `--place` is omitted. Unset means `main`. A name missing from `places` is refused at load. An env with neither `root` nor `main` has no start place, so codegen leaves `rootPlaceId` out. `rbx check` compares it with Roblox in its [`env/root`](check.md) row.

### `codegen = false`

For an env that never runs gameplay, such as a universe CI uploads to. It stays a normal env everywhere except the generated modules. Game code booting there will not find its env, and `rbx shop`'s dispatcher errors rather than guessing.

### Groups

A group names a set of envs. `--env nonprod` then does what `--env all` does, over those envs only:

```sh
rbx shop sync --env nonprod
rbx check --env nonprod
```

A group is expanded before anything else, so no lockfile, overlay or module ever sees its name. It is refused wherever `--env all` is.

| Refused at load | Why |
| --- | --- |
| A member that is not a declared env | The group would silently target fewer envs |
| A group inside a group | Groups are flat |
| A group named like an env | `--env` could not mean both |
| `all`, `owner`, `codegen`, `groups` | Reserved |
| An empty group | Targets nothing |

### Unrecognised keys

A key nothing claims is ignored and named on stderr, so a file written for a newer release still loads. If `codegen --check` fails while a key was ignored, it says so: the committed file may be the right one, and the fix is to upgrade or correct the spelling, not to regenerate.

## Who writes the file

- `rbx init create-group --record`: `[owner]`
- `rbx init create-universe` / `create-place`: the env and its places
- `rbx env set`: the settings above
- `rbx place fetch --env <name> --write`: one env's places, from the live universe

Each inserts lines and never rewrites the file, so comments and key order survive. Otherwise it is yours to write: one `[<env>]` with a `universe_id` is enough, and `rbx init list-universes` prints the ids.
