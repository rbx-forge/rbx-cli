# rbx open

Open Roblox places in Studio directly from the command line, configured via a shared TOML file.

## Features

- **Simple CLI** - Open places with `rbx open <env> <place>`
- **Studio or the client** - `--play` joins the place in the Roblox client instead of opening it for editing
- **Interactive picker** - Select environment and place with a menu if needed
- **Shared config** - Uses the same `rbxplace.toml` as other `rbx` subcommands
- **Cross-platform** - Works on Windows, macOS, and Linux
- **Zero dependencies** - No API key needed, just launches Studio

## Quick start

1. **Create or reuse `rbxplace.toml`**

```toml
[prod]
universe_id = 9876543210
places.main = 123456789012345
places.lobby = 987654321

[dev]
universe_id = 9876543212
places.main = 345678901234567
```

2. **Open a place**

```bash
# Interactive picker
rbx open

# Open a specific place
rbx open prod main

# Open the only place in an environment
rbx open staging
```

## Usage

```
rbx open [ENV] [PLACE] [--play]
```

### Arguments

- `ENV` - Environment name (e.g., `prod`, `staging`). Falls back to the global `--env` flag, then to an interactive picker.
- `PLACE` - Place name within the environment (e.g., `main`, `lobby`). Falls back to the global `--place` flag, then to an interactive picker (auto-picks when the env has exactly one place).

### Flags

- `--play` - Join the place in the Roblox client instead of opening it in Studio. See [Playing instead of editing](#playing-instead-of-editing-090)

### Playing instead of editing **(0.9.0+)**

```sh
rbx open prod main --play      # join the place in the Roblox client
rbx open --place-id 123 --play # without a project
```

`--play` changes the app and nothing else: the same `<env> <place>`, `--place-id` and `--universe-id` name the place, and there is still no network call. It is refused with `--new` and with a `.rbxl` on disk, which name a place that is not published and so has nothing to join.

The link sent is `roblox://experiences/start?placeId=<id>`, the deep link the Roblox client handles itself, rather than the `roblox-player:1+launchmode:play+...` the website sends. The website's form carries `gameinfo:<ticket>`, a single-use authentication ticket issued for one account; the same URI without it (`roblox-player:1+launchmode:play+placeId:<id>`) was measured on Windows starting the client and stopping at its home screen, with nothing to join.

**It does not choose an account.** Neither form of link carries an identity the client will act on, so the place is joined as whoever is signed into the Roblox app. Signing in as somebody else means doing it in the app. [`rbx-switch`](https://github.com/rbx-dev-tools/rbx-switch) does not help here: it switches the accounts signed into **Studio**.

### Without a project

```sh
rbx open --place-id 123456789
```

`--place-id` skips `rbxplace.toml` entirely, so this works in any directory: a place you have not configured, or somebody else's game you are helping with. It is the same global flag `--universe-id` is, and it wins over `ENV` / `PLACE` when both are given.

Worth having here more than anywhere: this command builds a `roblox-studio:` URI out of one number and makes no network call at all, so reading a config file to find that number was the only thing tying it to a project.

### A file on disk **(0.3.0+)**

```sh
rbx open game.rbxl
rbx open ./builds/staging.rbxlx
rbx open --file weird-name        # for a path without the extension
```

Recognised by extension, not by looking on disk: `rbx open prod` has to stay an
environment even in a folder that happens to contain a file called `prod`.

The path is handed to the desktop's opener rather than wrapped in a
`roblox-studio:` URI, because that URI is parsed by splitting on `+` and `:`,
which both a Windows path and any filename containing a `+` would break. Studio
ends up in the same place either way; its log says
`createAndShowIDEDoc with task EditFile`.

### A new place, with no project and no id **(0.3.0+)**

```sh
rbx open --new              # pick a template, then open it
rbx open --new --baseplate  # the stock one, no picker and no network
rbx open --new --template 6560363541
```

This is Studio's "New Experience" button, and it is that button rather than an
imitation of it. Clicking it logs:

```
[FLog::PlaceManager] PlaceManager::createAndShowIDEDoc with task EditPlace
[FLog::StudioKeyEvents] open place (identifier = 95206881)
```:

 an ordinary `roblox-studio:` open of place `95206881`, Roblox's stock
baseplate, which is exactly what `--new --baseplate` sends.

**Nothing is created on Roblox.** Studio binds the session to the template long
enough to fetch its content and then sets the place id back to `0`:

```
[FLog::CloseDataModel] Setting place ID 95206881
[FLog::CloseDataModel] Setting place ID 0
```

The content arrives, the identity does not. That unbound state is what makes it
a *new* place rather than someone else's: there is nothing for a save to
overwrite, so the first save to Roblox has to create the experience. Either
"Save to Roblox As" or "Publish to Roblox As" will do it; the difference is
whether players get the new version, not whether the experience exists. It is
also why DataStores do not work until then: there is no universe to address.

To create the experience outright instead, `rbx init create-universe` asks
Roblox to clone the same template server-side and hands back real ids.

#### The template list

`--new` on its own lists Roblox's templates and asks. There is no template API:
the list is the public games of account `998796`, which is where Studio gets it
too, and it needs no credential. The baseplate is lifted to the top because
Roblox returns them newest-first, which otherwise buries it.

`--baseplate` and `--template` both skip the picker, so they work without a
terminal and without the network call. `--new` cannot be combined with
`--place-id`, `--universe-id`, or an env/place argument: each of those names a
place, which is the thing `--new` says there is not one of yet.

## Configuration

`rbx open` reads from `rbxplace.toml` (override with the global `--places <path>`). Each top-level section is an environment:

```toml
[prod]
universe_id = 9876543210  # Required: Roblox universe ID
places.main = 123456789   # Optional: Map place names to place IDs
places.lobby = 987654321
confirm = true            # Optional: For compatibility with rbx place

[staging]
universe_id = 9876543211
places.main = 234567890
```

### Behavior

- **No arguments** - Interactive picker for environment, then place
- **Environment only** - If environment has one place, opens directly; otherwise shows picker
- **Environment + place** - Opens immediately
- **Unknown environment** - Shows error with available options
- **`--env all`** - Rejected; `rbx open` operates on one place at a time

## How it works

`rbx open` constructs a `roblox-studio:` URI handler request:

```
roblox-studio:1+task:EditPlace+placeId:123456789+universeId:0
```

This is handled by your system's Roblox Studio installation.

On Linux the URI goes to `xdg-open`, and if that is not there (or if this is
WSL, where Studio lives on the Windows side) it crosses to the Windows host
instead, via `powershell.exe Start-Process`, then `rundll32`, then `cmd.exe
start`, then `wslview`. Studio has no native Linux build, so a Linux `rbx` with
nowhere to hand the URI says so rather than failing silently.

## Prior art

[ROpen](https://github.com/Barocena/ROpen) (Luau, MPL-2.0) is where this command comes from: the launcher it was written against, and where the `roblox-studio:` URI dispatch was learned from, having contributed to it.

That dispatch is older than either of us. [rojo-rbx/edit-roblox-place](https://github.com/rojo-rbx/edit-roblox-place) (Rust, MIT) was doing the same thing in **August 2019**: one command, one place id, and the same `roblox-studio:1+task:EditPlace+placeId:<id>` this command sends. Nobody here knew of it until long after `rbx open` shipped; it is named because the honest version of "prior art" is not "whatever its author met first".

What `rbx open` adds to either is the part that belongs to this tool: the place is named rather than numbered. `rbx open prod main` resolves through `rbxplace.toml`, so the id nobody remembers stays in the file that already holds it.

No code from either project is reused, and none is owed: see [THIRD-PARTY-NOTICES.md](https://github.com/rbx-forge/rbx-cli/blob/main/THIRD-PARTY-NOTICES.md).
