# rbx init

Create a group, a universe and its places from the command line, and record them in `rbxplace.toml` as you go. Open Cloud cannot create these, so `rbx init` uses the [session cookie](cookie.md).

```sh
rbx init create-group --name "My Studio" --icon assets/group-icon.png --public --record
rbx init create-universe --name "[TEST] My Game" --env test
rbx init create-place --universe-id 987654321 --name "Lobby" --place lobby
```

Each step writes what it created into `rbxplace.toml`, so no id is copied by hand. Missing names are asked for on a terminal. Every question comes before the creation, which cannot be undone.

Already have a group and universes? `rbx init list-universes --group <id>` prints the ids; write the envs yourself ([the file](env.md#the-file)) and check with `rbx env list`.

## Commands

### rbx init create-group

Create a group. Costs 100 Robux.

| Flag | Required | Description |
| --- | --- | --- |
| `--name` | yes | Display name |
| `--icon` | yes | PNG or JPEG |
| `--description` | no | Description |
| `--public` | no | Anyone can join (default: invite only) |
| `--record` | no | Write the group as `[owner]` in `rbxplace.toml`, creating the file if needed |
| `--yes` / `-y` | no | Skip the confirmation |

`--record` refuses before spending anything if the file already has an `[owner]` or does not parse, so a failed script can be re-run without buying a second group. Roblox only lets eligible accounts create groups (verified email, minimum account age).

### rbx init create-universe

Create a universe with a root place, from Roblox's empty baseplate or a template.

| Flag | Description |
| --- | --- |
| `--group` / `--user` | Owner. Without either: `[owner]` in `rbxplace.toml`, else your account |
| `--template-place-id` | Place to clone (default: empty baseplate) |
| `--name` | Name, which Roblox shows from the root place |
| `--env` | Env to record it as. Asked for on a terminal |
| `--place` | Key for the root place (default `main`; another key also writes `root`) |
| `--no-record` | Leave `rbxplace.toml` alone. Not combinable with `--env` or `--place` |
| `--yes` / `-y` | Skip the confirmation |

```
$ rbx init create-universe
  Universe name (empty to keep the template's): My Test Game
  Env name in rbxplace.toml: (my_test_game) test
⚠ As builderman (156): create universe 'My Test Game' under group 1234567 and record it as [test]? [y/N] y
Created universe (id 9876543299) with root place (id 111222333)
Added [test] to rbxplace.toml
```

| The run | Recorded in `rbxplace.toml` |
| --- | --- |
| `--no-record` | no |
| `--env <name>` | yes, even with `--yes` or without a terminal. A missing file or an existing env is an error |
| no `--env`, on a terminal, file present | yes, after asking |
| no `--env`, with `--yes`, without a terminal, or no file | no |

It extends `rbxplace.toml` but never creates it.

### rbx init create-place

Add a place to an existing universe. It is recorded under the env whose `universe_id` matches.

| Flag | Description |
| --- | --- |
| `--universe-id` | The universe (required) |
| `--template-place-id` | Place to clone |
| `--name` | Name. Asked for on a terminal |
| `--place` | Key under `[<env>.places]`. Asked for on a terminal |
| `--no-record` | Leave `rbxplace.toml` alone |
| `--yes` / `-y` | Skip the confirmation |

If several envs use the universe, pass `--env`.

### rbx init rename-place

| Flag | Description |
| --- | --- |
| `--place` | The place **id** (required) |
| `--name` | New name (required) |
| `--yes` / `-y` | Skip the confirmation |

!!! warning "`--place` is an id here"
    Everywhere else `--place` is a key from `rbxplace.toml`. Here it is a number; `--place lobby` does not parse.

### rbx init rename-universe

Rename a universe, through its root place.

| Flag | Description |
| --- | --- |
| `--universe-id` | The universe (required) |
| `--name` | New name (required) |
| `--yes` / `-y` | Skip the confirmation |

### rbx init list-groups

The groups your account is in, with role and rank. Needs the cookie.

### rbx init list-universes

A group's universes, published or not. Needs no credential.

| Flag | Description |
| --- | --- |
| `--group` | The group (required) |

### rbx init list-places

A universe's places. Needs no credential. Takes `--universe-id`.

## The listings are public

`list-universes` and `list-places` answer anonymously, for private universes too: the existence, id and name of a universe or place are public on Roblox. Do not count on an unlisted universe to keep a project quiet, and rename test places before creating them under a real account, since Roblox's default names include the owner's username.
