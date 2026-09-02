# rbx init

Bootstrap Roblox resources from the command line: create groups, universes, and places, then list IDs to plug into the other `rbx` subcommands.

`rbx init` covers the missing first step in a Roblox project bootstrap. You start with nothing and you need a group, a universe, and a few places before `rbx place`, `rbx meta`, `rbx config`, or `rbx shop` have anything to point at. It hits Roblox's authenticated creation endpoints (cookie-based, since Open Cloud doesn't expose these yet) and the listing endpoints so you can pipe fresh IDs straight into your other configs.

## Features

- **Create a group**: `rbx init create-group --name ... --icon icon.png`, with `--record` to write it straight into `rbxplace.toml` as the `[owner]`
- **Create a universe**: `rbx init create-universe [--group <id>]`, returns the universe ID and the root place ID in one call
- **Create a place** inside an existing universe: `rbx init create-place --universe-id <id>`
- **Auto-record into `rbxplace.toml`**: every create appends what it made to the shared env map, prompting for the env/place name: comments and formatting in the file are preserved
- **Rename a place / universe** by id: `rbx init rename-place` / `rbx init rename-universe`
- **List your groups**: `rbx init list-groups` (cookie required)
- **List a group's universes**: `rbx init list-universes --group <id>` (no credential needed; see [what these listings expose](#the-listings-need-no-credential))
- **List a universe's places**: `rbx init list-places --universe-id <id>` (no credential needed)
- **Cookie auto-detect**: offers to use the `.ROBLOSECURITY` of a local Roblox Studio install when no cookie was supplied. Opt-in: an interactive run is asked once, a run with nowhere to ask declines and says so. `--auto-cookie` is the standing yes. See [docs/cookie.md](./cookie.md#auto-detection-is-opt-in)
- **Friendly errors**: common cases (name already taken, moderation rejection) are surfaced with a clear message instead of raw HTTP

## Quick start

Bootstrap a brand-new project with a group, a universe, and an extra lobby place:

```sh
# 1. Create the group, and record it as [owner] (creates rbxplace.toml)
rbx init create-group --name "My Studio" --icon assets/group-icon.png --public --record

# 2. Create a universe. The owner comes from [owner], so no id to pass
rbx init create-universe --name "[TEST] My Game" --env test

# 3. Add a second place inside the universe
rbx init create-place --universe-id 987654321 --name "Lobby" --place lobby
```

Every step records what it made in `rbxplace.toml` as it goes, so there is nothing to copy by hand and no id in transit between commands. Omit `--env`/`--place`/`--name` and you'll be asked for them instead.

`--record` is what makes step 2 able to omit `--group`: it writes the `[owner]` block that `create-universe` then resolves its owner from. It is opt-in, because it is the one create that may bring `rbxplace.toml` into existence rather than only extend it. It also refuses, *before* the 100 Robux are spent, when the file already declares an owner: a script that died at step 2 can be re-run without buying a second group.

Starting from an existing group with universes already created? `rbx init list-universes --group <id>` prints their ids; write the `[<env>]` sections yourself, then check the result with [`rbx env list`](./env.md). See [the field reference](./env.md#every-field-and-where-it-goes) for what goes where.

## Commands

<details markdown="1">
<summary><code>rbx init create-group</code></summary>

Create a new Roblox group. An icon is required by Roblox (PNG or JPEG).

| Flag | Required | Description |
| --- | --- | --- |
| `--name` | **Yes** | Group display name |
| `--icon` | **Yes** | Path to a PNG/JPEG icon file |
| `--description` | No | Group description (default: empty) |
| `--public` | No | Make the group publicly joinable (default: invite-only) |
| `--record` | No | Write the new group as `[owner]` in `rbxplace.toml`, creating the file if needed |
| `--yes` / `-y` | No | Skip the confirmation prompt |

> Roblox requires the authenticated user to be eligible to create groups (verified email + minimum account age). If the chosen name is already taken, `rbx init` prints a clear message instead of a raw HTTP error.

`--record` appends the block and never rewrites lines already on disk, so comments, key order and CRLF endings survive. It refuses when the file already declares a top-level `[owner]`, and it checks that before contacting Roblox: creating a group costs 100 Robux and cannot be undone, so the refusal has to come while nothing has been spent. A file that exists but does not parse stops the run for the same reason, rather than being treated as "no owner" and appended to.

Without `--record` the command only prints the id, which is the right default when you are creating a group that has nothing to do with the directory you happen to be standing in.

</details>

<details markdown="1">
<summary><code>rbx init create-universe</code></summary>

Create a new universe with a root place. The default template is Roblox's empty baseplate; override with `--template-place-id` to clone from a specific place you own.

The new universe is recorded in `rbxplace.toml` as a new `[<env>]` block, so you don't have to copy ids by hand afterwards.

| Flag | Required | Description |
| --- | --- | --- |
| `--group` | No | Group ID to create the universe under |
| `--user` | No | User ID to create the universe under. Mutually exclusive with `--group` |
| `--template-place-id` | No | Template place ID to clone from (defaults to Roblox's empty baseplate) |
| `--name` | No | Rename the universe's root place to this name after creation (Roblox displays the root place name as the universe name). Prompted for when omitted |
| `--env` | No | Env name to record the universe as. Prompted for when omitted. **Refused with `--no-record`** |
| `--place` | No | Place key for the root place (default `main`). **Refused with `--no-record`** |
| `--no-record` | No | Don't touch `rbxplace.toml`. Refused alongside `--env` or `--place`, which would be asking for the record and refusing it in one command |
| `--yes` / `-y` | No | Skip the confirmation prompt. Whether the universe is still recorded depends on `--env`: see below |

**Owning it without a group.** Omitting both `--group` and `--user` is not an error and does not require a group: the owner falls back to `[owner]` in `rbxplace.toml`, and with no `[owner]` either, to your own user account. So a personal universe is `rbx init create-universe` with neither flag.

Run it bare and it asks for what it needs, then confirms everything in one line:

```sh
$ rbx init create-universe
  Universe name (empty to keep the template's): My Test Game
  Env name in rbxplace.toml: (my_test_game) test
⚠ Create universe 'My Test Game' under group 1234567 and record it as [test]? [y/N] y
Creating universe under group 1234567 ...
Renaming root place 111222333 to My Test Game ...
Created universe (id 9876543299) with root place (id 111222333)
  name: My Test Game
Added [test] to rbxplace.toml
```

Or name everything up front for an unattended run:

```sh
rbx init create-universe --name "[TEST] My Game" --env test -y
```

Both prompts appear only when the corresponding flag is missing **and** stdin is a terminal.

Whether anything is recorded follows one rule, and `--env` is what decides it:

| The run | Recorded? |
| --- | --- |
| `--no-record` | Never, whatever else is passed |
| `--env <name>` given | **Yes**, including under `--yes` and off a terminal. A missing `rbxplace.toml` or an env that already exists is an error, not a silent skip |
| neither, on a terminal, with the file present | Yes, after asking for the env name |
| neither, under `--yes`, off a terminal, or with no `rbxplace.toml` | No, silently |

The middle row is the one worth knowing: `--env` is a request, so it is honoured rather than suppressed by `--yes`. That is what makes the unattended example above record. Without `--env`, `--yes` means "ask me nothing", and since recording is driven by the prompt it is skipped rather than guessed at.

This command extends an existing `rbxplace.toml`; it does not create one.

If `--name` is given, the env name is suggested from it: a `[TEST] ...` prefix becomes `test`, otherwise the name is slugified.

Every question is asked *before* the universe is created. Creating one is irreversible, so aborting at a prompt costs nothing more than a re-run.

</details>

<details markdown="1">
<summary><code>rbx init create-place</code></summary>

Add a new place inside an existing universe.

The new place is recorded under the env whose `universe_id` matches `--universe-id`, so there's no env to pick: only the key name is asked for.

| Flag | Required | Description |
| --- | --- | --- |
| `--universe-id` | **Yes** | Universe ID to create the place in |
| `--template-place-id` | No | Template place ID to clone from (defaults to Roblox's empty baseplate) |
| `--name` | No | Rename the new place to this name after creation. Prompted for when omitted |
| `--place` | No | Place key to record in `rbxplace.toml`. Prompted for when omitted (suggested from `--name`). **Refused with `--no-record`** |
| `--no-record` | No | Don't touch `rbxplace.toml`. Refused alongside `--env` or `--place` |
| `--yes` / `-y` | No | Skip the confirmation prompt. Same recording rule as `create-universe` |

```sh
$ rbx init create-place --universe-id 9876543299
  Place name (empty to keep the template's): Lobby
  Place name under [test.places]: (lobby) 
⚠ Create place 'Lobby' under universe 9876543299 and record it as [test].places.lobby? [y/N] y
Created place (id 444555666) in universe 9876543299
  name: Lobby
Added places.lobby to [test] in rbxplace.toml
```

Same skip rules as `create-universe`. If no env points at `--universe-id`, recording is skipped silently, unless you asked for it explicitly with `--env`/`--place`, in which case it's an error. If several envs point at the same universe, pass `--env` to disambiguate.

</details>

<details markdown="1">
<summary><code>rbx init rename-place</code></summary>

Rename a place by id.

| Flag | Required | Description |
| --- | --- | --- |
| `--place` | **Yes** | Place **ID** to rename, not a place name |
| `--name` | **Yes** | New display name |
| `--yes` / `-y` | No | Skip the confirmation prompt |

> `--place` means something different here than anywhere else in `rbx`. Everywhere else it is a key from `[<env>.places]`; on this one subcommand it shadows that with a raw place id, so `--place lobby` fails to parse rather than resolving. Pass the number.

</details>

<details markdown="1">
<summary><code>rbx init rename-universe</code></summary>

Rename a universe by id. Roblox stores the display name on the root place; this resolves the universe's root place and renames it.

| Flag | Required | Description |
| --- | --- | --- |
| `--universe-id` | **Yes** | Universe ID |
| `--name` | **Yes** | New display name |
| `--yes` / `-y` | No | Skip the confirmation prompt |

</details>

<details markdown="1">
<summary><code>rbx init list-groups</code></summary>

List every group the authenticated user belongs to, with role and rank. **Cookie required.**

</details>

<details markdown="1">
<summary><code>rbx init list-universes</code></summary>

List the universes owned by a group, published or not. No credential is required and a cookie adds nothing to the result: see [the listings need no credential](#the-listings-need-no-credential).

| Flag | Required | Description |
| --- | --- | --- |
| `--group` | **Yes** | Group ID |

</details>

<details markdown="1">
<summary><code>rbx init list-places</code></summary>

List every place inside a universe.

| Flag | Required | Description |
| --- | --- | --- |
| `--universe-id` | **Yes** | Universe ID |

</details>

## Authentication

`rbx init` only uses cookie auth. Roblox does **not** expose group, universe, or place creation through Open Cloud, so there's no API key option. The cookie is supplied via the global `--cookie` flag, the `RBX_COOKIE` env var, or a local Roblox Studio install.

That last one is **opt-in**: finding a signed-in Studio is not the same as being allowed to send its session. `--auto-cookie` is the standing yes, an interactive run is asked once, and a run with nowhere to ask (CI, a pipe, a cron job) declines and says so. `--no-auto-cookie` is the standing no.

This is the command with the least choice about it, so it is worth knowing what you are handing over: see [docs/cookie.md](./cookie.md) for the resolution order in full, the stderr notice on auto-detection, and why the cookie never reaches disk.

| Command | Cookie required? |
| --- | --- |
| `create-group` | **Yes** |
| `create-universe` | **Yes** |
| `create-place` | **Yes** |
| `rename-place` | **Yes** |
| `rename-universe` | **Yes** |
| `list-groups` | **Yes** |
| `list-universes --group <id>` | **No.** The listing answers in full without one |
| `list-places --universe-id <id>` | **No.** The listing answers in full without one |

### The listings need no credential

`list-universes` and `list-places` are the two read commands here, and neither
one is gated. Measured against a private universe that has never had a player,
with no cookie, no API key and no session:

```
GET develop.roblox.com/v1/universes/{id}/places  → 200, every place, with names
GET games.roblox.com/v2/groups/{id}/gamesV2      → 200, every game
```

The second one is worth being precise about, because the query parameter looks
like a permission and is not. Measured on one group, anonymously:

| Request | Games returned |
| --- | --- |
| `accessFilter=2` | 0 |
| `accessFilter=1` | 4 |
| no `accessFilter` | 4 |

`accessFilter=2` is the *public* filter. `1`, and omitting it, are the
unfiltered form, and unfiltered means unfiltered for anybody. `rbx init` sends
`1`, so it sees a group's staging copies and unreleased projects, and so does
anyone else who asks.

**Roblox treats the existence, id and name of a universe or place as public.**
What stays behind a session is the *content*: `develop.roblox.com/v1/places/{id}`
answers 404 anonymously, and whether a place is playable is not in these
listings at all.

Two practical consequences. Do not rely on a universe being unlisted to keep a
project quiet before announcing it. And rename test places before creating
them under a real account: Roblox's default place names embed the owner's
username, and those names come back to an anonymous caller.

## How it works

- Group creation hits `groups.roblox.com/v1/groups/create` (multipart upload with the icon).
- Universe creation hits `apis.roblox.com/universes/v1/universes/create` with a `templatePlaceId`.
- Place creation hits `apis.roblox.com/universes/v1/user/universes/{id}/places`.
- Listing a group's universes uses `games.roblox.com/v2/groups/{id}/gamesV2?accessFilter=1`. `accessFilter=2` is the public-only filter; `1` is unfiltered, for any caller (see [above](#the-listings-need-no-credential)).
- Listing a universe's places uses `develop.roblox.com/v1/universes/{id}/places`.

All write endpoints transparently handle CSRF: a 403 response with an `x-csrf-token` header caches the token and retries once. Listing endpoints retry on 429 / 5xx with exponential backoff (max 3 attempts).
