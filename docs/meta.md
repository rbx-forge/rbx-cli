# rbx meta

Declaratively manage Roblox game/universe metadata from a single TOML config: name, description, icon, thumbnails, devices, social links, private servers, server fill mode, copying permission, visibility, Studio API access, and Beta mode. Multi-env aware via a shared `rbxplace.toml`.

`rbx meta` syncs your local metadata to Roblox, tracks remote state in a per-env lockfile, detects media changes with [BLAKE3](https://github.com/BLAKE3-team/BLAKE3) hashing, and uses the Open Cloud API by default with an optional `.ROBLOSECURITY` cookie fallback for fields Open Cloud doesn't expose.

## Features

- **Declarative config** - Define every metadata field in a single `rbxmeta.toml`
- **Multi-environment** - Manage dev / staging / prod from one toml, with `[envs.<name>]` overlays merged on top of base
- **Differential pull** - Writes only what diverges from your base config so the toml stays minimal
- **Two-way sync** - Push local config to Roblox or pull remote state back into your toml + lockfile (comments preserved via [`toml_edit`](https://docs.rs/toml_edit))
- **Open Cloud first** - Runs in CI with just an API key; cookie only needed for the handful of cookie-only fields
- **Cookie fallback** - Auto-detects `.ROBLOSECURITY` from a local Roblox Studio install for `server_fill`, `allow_copying`, `visibility`, `studio_access_to_apis_allowed`, and `beta_mode`
- **Smart visibility ordering** - When toggling private→public, `rbx meta` activates the experience *first* so dependent patches (e.g. paid private servers) don't 500
- **Preflight validations** - Refuses obviously-invalid combinations (e.g. `private_server.price` 1-9 Robux, or paid private servers on a private experience) before sending a request
- **Per-env media namespacing** - `pull --accept-remote --env dev` saves to `<media.dir>/dev/icon.png` so envs never overwrite each other on disk
- **Crash-safe lockfile** - Saved after every successful API call so a mid-sync crash never leaves remote and lockfile in disagreement
- **Smart media diff** - Icons and thumbnails are hashed with BLAKE3 so re-uploads only happen when bytes actually change
- **Thumbnail ordering** - Preserves the order from your TOML, auto-reorders on Roblox as needed
- **Alpha bleed** - Applies alpha bleeding to icons and thumbnails before uploading (enabled by default)
- **CSRF handled** - Cookie requests transparently retry on 403 with a fresh token

## Quick start

### Multi-env (recommended): point at a shared `rbxplace.toml`

If you already use `rbx place` / `rbx config`, you have a `rbxplace.toml` like:

```toml
[dev]
universe_id = 9876543210
[dev.places]
main = 123456789012345

[prod]
universe_id = 9876543211
[prod.places]
main = 234567890123456
```

Pull each env once to populate `rbxmeta.toml`:

```sh
rbx meta init                                          # commented template
rbx meta pull --env dev --accept-remote --api-key KEY  # fills [game], [media] + [envs.dev] deltas
rbx meta pull --env prod --accept-remote --api-key KEY # adds only diverging fields to [envs.prod]
rbx meta sync --env dev                                # apply changes back to dev
```

### Standalone (no rbxplace.toml)

```sh
rbx meta init --from-remote --universe-id 123456789 --place-id 987654321 --api-key KEY
rbx meta sync --api-key KEY
```

This embeds `[experience]` in `rbxmeta.toml` and uses it as the implicit "default" env (lockfile section: `[envs.default]`).

### Hybrid: --from-remote with --env

```sh
rbx meta init --from-remote --env dev --api-key KEY
```

Resolves `universe_id` / `place_id` from `rbxplace.toml` and writes the lockfile under `[envs.dev]`. Useful for starting fresh on an existing experience without copy-pasting IDs.

## Multi-environment

Three concepts:

1. **Base** (`[game]` + `[media]`): the values shared across every env.
2. **Overlay** (`[envs.<name>]`, `[envs.<name>.devices]`, `[envs.<name>.media]`, etc.): per-env diffs layered on top of base.
3. **Resolution**: `--env <name>` resolves `(universe_id, place_id)` from `rbxplace.toml` and merges base + overlay for that env.

`sync --env dev` applies `[game] + [envs.dev]` to dev. `sync --env prod` applies `[game] + [envs.prod]` to prod. Same `rbxmeta.toml`, different effective state.

### Pull behavior (differential)

For each field, when you `pull --env <name>`:

| State | Remote action |
| --- | --- |
| Base unset | Write remote to base, clear overlay |
| Remote == base | Clear overlay (no-op if absent) |
| Remote != base | Write remote as overlay |

Concrete example: starting from an empty config, `pull --env prod` (visibility=public) writes `[game] visibility = "public"`. Then `pull --env dev` (visibility=private) only writes `[envs.dev] visibility = "private"`. Subsequent pulls are idempotent.

Pull never auto-promotes overlays back into base. To DRY, edit the toml manually to move a shared value into `[game]`; the next pull will detect the match and remove the now-redundant overlay.

### Place selection

When the env in `rbxplace.toml` has multiple places (`[prod.places.lobby]`, `[prod.places.world]`), pass `--place <name>`. Defaults to `main` if present, otherwise the only entry.

**One place per env, and the tool enforces it.** `rbxmeta.lock.toml` keys its sections by env and holds a single `place_id` in each, while `name`, `description` and `server_size` are place-level fields written to that place. So syncing `--place lobby` and then `--place main` under one env would leave `[envs.prod]` recording one place's metadata under the other's id — and every later diff, which is what decides whether a field gets sent at all, would be computed against the wrong baseline.

`sync` and `pull` therefore refuse a place that disagrees with the one the section already tracks:

```
Lockfile env 'prod' tracks place_id 234567890123456 but the resolved target is 234567890999999.
This env has more than one place and the lockfile holds one, so writing the second would record
its metadata under the first one's id. Use one place per env, or delete the [envs.prod] section
if you meant to move it.
```

If you genuinely manage metadata on two places, give them an env each in `rbxplace.toml` pointing at the same `universe_id`. Universe-level fields (`voice_chat`, `devices`, `private_server`, `social_links`, visibility) apply to the whole experience either way, so declare those in `[game]` and keep the per-env overlays to the place-level ones.

## Commands

<details>
<summary><code>rbx meta init</code></summary>

Initialize a new config file. Without flags, writes a commented template.

| Flag | Description |
| --- | --- |
| `--from-remote` | Populate config from live universe/place state |
| `--universe-id` | Universe ID (standalone mode; requires `--place-id`) |
| `--place-id` | Root place ID (standalone mode; requires `--universe-id`) |

With `--from-remote` and `--env <name>` instead of `--universe-id`/`--place-id`, init resolves IDs from `rbxplace.toml` and writes the lockfile under `[envs.<name>]` (no `[experience]` block in the toml).

</details>

<details>
<summary><code>rbx meta sync</code></summary>

Apply the config (base + env overlay) to Roblox. Diffs against the lockfile's `[envs.<name>]` section to only send changed fields, then updates that section after each successful API call.

| Flag | Description |
| --- | --- |
| `--dry-run` | Show what would change without applying |
| `--yes` / `-y` | Skip the confirmation prompt. What CI passes; see below |

`sync` prompts before applying when the env has `confirm = true` in `rbxplace.toml`. `--yes` answers it in advance, which is how a pipeline gets through — and is the only thing standing between an unattended run and a write, so it belongs in the job that was reviewed rather than in a shell alias.

</details>

<details>
<summary><code>rbx meta check</code></summary>

Validate the config and print the diff against the lockfile for the targeted env. Read-only.

Exit codes: `0` nothing pending, `2` the config no longer matches the lockfile, `1` the check could not answer. Drift sits on its own code so a CI step can gate on the status alone.

</details>

<details>
<summary><code>rbx meta pull</code></summary>

Pull remote state for the targeted env into the config and lockfile. Differential: writes only what diverges (see [Multi-environment](#multi-environment) above for the algorithm). Comments are preserved via [`toml_edit`](https://docs.rs/toml_edit).

| Flag | Description |
| --- | --- |
| `--dry-run` | Show what would change without writing the lockfile or config |
| `--accept-remote` | Download the current icon and thumbnails into `media.dir` (or `media.dir/<env>` for named envs) and update the config paths |
| `--accept-local` | Clear media hashes (next `sync` re-uploads local icon and thumbnails) |
| `--yes` / `-y` | Skip the confirmation prompt |

Media downloads use the public `thumbnails.roblox.com` service (512x512 for icon, 768x432 for thumbnails). The downloaded image is what Roblox serves now, not necessarily the original upload. Without `--accept-remote` / `--accept-local`, pull leaves media hashes untouched and prints a hint.

**Per-env path namespacing**: when `--env <name>` is passed (anything other than the implicit standalone "default" env), downloaded files go to `<media.dir>/<env>/icon.png` and `<media.dir>/<env>/thumbnail_NN.png`, and the resulting path is written as an overlay under `[envs.<name>.media]`, never to the `[media]` base. This guarantees that pulling dev then prod doesn't overwrite a shared `assets/icon.png`. The `[media]` base is reserved for paths you manage manually.

**Skip if unchanged**: pull skips re-downloading any icon or thumbnail whose `image_id` matches the lockfile *and* whose local file still exists on disk. Delete the file (or clear the lockfile entry) to force a re-download.

</details>

### Per-tool flags

| Flag | Description |
| --- | --- |
| `--config <path>` | Path to `rbxmeta.toml` (default `rbxmeta.toml`) |

## Configuration

`rbx meta` requires a `rbxmeta.toml` file in the working directory (or specify with `--config`).

`[experience]` is **optional** and only used in standalone mode (no `--env`). When you pass `--env <name>`, `universe_id` / `place_id` come from `rbxplace.toml`. Per-env overrides go under `[envs.<name>]` and are layered on top of `[game]` / `[media]`.

```toml
[experience]                 # optional, omit if you always pass --env
universe_id = 123456789
place_id = 987654321         # root place, name/description/server_size go here

[game]
name = "My Awesome Game"
description = "A really fun multiplayer game."
server_size = 50             # max concurrent players per server
voice_chat = false

# The fields below have no Open Cloud endpoint, so a `sync` whose plan touches
# one needs a session cookie. Leave them out and the rest of this file syncs
# with an API key alone. See "Cookie-only fields" further down.
visibility = "public"                   # "public" | "private", write requires cookie
studio_access_to_apis_allowed = true    # cookie-only, Studio can call DataStore/Open Cloud
beta_mode = false                       # cookie-only, true = hides from Home Recommendations

[game.private_server]
price = 100                  # Robux. 0 = free, >= 10 = paid. Omit table to disable private servers.

[game.devices]
desktop = true
mobile = true
tablet = true
console = false
vr = false

[game.server_fill]
mode = "custom"              # cookie-only
reserved_slots = 5           # only with mode = "custom"

[game.social_links.discord]
title = "Join our Discord"
url = "https://discord.gg/example"

[media]
icon = "assets/icon.png"
thumbnails = ["assets/thumb1.png", "assets/thumb2.png"]
dir = "assets"               # destination for `pull --accept-remote` downloads
bleed = true
language_code = "en_us"

# Per-env overrides (optional). Layered on top of [game] / [media] when
# --env <name> is passed.
[envs.dev]
visibility = "private"
```

<details>
<summary><code>[experience]</code></summary>

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `universe_id` | `u64` | **Yes** | Your Roblox universe ID |
| `place_id` | `u64` | **Yes** | Root place ID - destination for `name`, `description`, `server_size` |

</details>

<details>
<summary><code>[game]</code></summary>

Scalar fields live directly under `[game]`. Grouped multi-field settings (devices, social links, etc.) have their own sub-tables below.

| Field | Type | API | Description |
| --- | --- | --- | --- |
| `name` | `string` | Open Cloud | Display name (written to the root place) |
| `description` | `string` | Open Cloud | Experience description (written to the root place) |
| `server_size` | `u32` | Open Cloud | Max concurrent players per server |
| `voice_chat` | `bool` | Open Cloud | Enable in-experience voice chat |
| `allow_copying` | `bool` | Cookie | Let anyone take a copy of this place from its Roblox page. Defaults to `false` and the only interesting value is `true`, for a place published deliberately as a template or as open source. It is **not** a protection: it governs a button on a page, not who can reach the file, so setting it to `false` hardens nothing that was not already the default |
| `visibility` | `string` | Open Cloud read / Cookie write | `"public"` or `"private"` |
| `studio_access_to_apis_allowed` | `bool` | Cookie | Allow Studio scripts to call Open Cloud / data store APIs |
| `beta_mode` | `bool` | Cookie | Enable Experience Beta mode (hides from Home Recommendations) |

</details>

<details>
<summary><code>[game.private_server]</code></summary>

Omit this table entirely to disable private servers.

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `price` | `u64` | **Yes** | Price in Robux. `0` = free private servers, `>= 10` = paid. Values `1-9` are rejected by Roblox (and by `rbx meta`'s preflight). Paid private servers (`> 0`) also require `visibility = "public"`. |

</details>

<details>
<summary><code>[game.devices]</code></summary>

Omit a field to leave that device unchanged on Roblox.

| Field | Type | Description |
| --- | --- | --- |
| `desktop` | `bool` | Allow desktop players |
| `mobile` | `bool` | Allow phone players |
| `tablet` | `bool` | Allow tablet players |
| `console` | `bool` | Allow console players |
| `vr` | `bool` | Allow VR players |

</details>

<details>
<summary><code>[game.server_fill]</code></summary>

Server fill mode. **Requires cookie**: not exposed by Open Cloud.

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `mode` | `string` | **Yes** | `"automatic"`, `"empty"`, or `"custom"` |
| `reserved_slots` | `u32` | Only with `mode = "custom"` | Number of slots reserved per server |

</details>

<details>
<summary><code>[game.social_links.&lt;platform&gt;]</code></summary>

Omit a section to remove that link from Roblox. Available platforms: `facebook`, `twitter`, `youtube`, `twitch`, `discord`, `roblox_group`, `guilded`.

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `title` | `string` | **Yes** | Display title |
| `url` | `string` | **Yes** | Link URL |

</details>

<details>
<summary><code>[media]</code></summary>

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `icon` | `string` | `(unset)` | Path to a PNG icon (relative to the config file) |
| `thumbnails` | `string[]` | `[]` | Up to 10 PNG thumbnail paths, displayed on Roblox in this order |
| `dir` | `string` | `(unset)` | Destination directory used by `pull --accept-remote` to save downloaded icon and thumbnails |
| `bleed` | `bool` | `true` | Apply alpha bleed to PNGs before upload |
| `language_code` | `string` | `"en_us"` | Locale used for icon and thumbnail upload |

</details>

### Field coverage

| Field | API | Notes |
| --- | --- | --- |
| `game.name`, `game.description` | Open Cloud | Written to the root place |
| `game.server_size` | Open Cloud | Max players per server |
| `game.voice_chat` | Open Cloud | |
| `game.private_server.price` | Open Cloud | Omit table to disable |
| `game.devices.*` | Open Cloud | desktop / mobile / tablet / console / vr |
| `game.social_links.*` | Open Cloud | 7 platforms |
| `media.icon` | Open Cloud | Localized via `legacy-game-internationalization` |
| `media.thumbnails[]` | Open Cloud | Up to 10, ordered |
| `game.server_fill` | **Cookie** | `socialSlotType` + `customSocialSlotsCount` |
| `game.allow_copying` | **Cookie** | `copyingAllowed` |
| `game.visibility` | Open Cloud read / **Cookie** write | Legacy `activate` / `deactivate` |
| `game.studio_access_to_apis_allowed` | **Cookie** | Legacy `/v2/universes/{id}/configuration` |
| `game.beta_mode` | **Cookie** | `apis.roblox.com/experience-releases/.../release_status` |

### Not supported

Open Cloud does not expose these fields and `rbx meta` does not (yet) handle them via cookie:

- Genre
- Friends-only visibility (only `public` / `private` supported)
- Paid access (`isForSale` + price)
- Avatar configuration (type, animation, collision, scales, asset overrides)
- Age rating (write)
- Third-party purchase / teleport permissions
- Badges, game passes, developer products - use [rbx shop](./shop.md) instead

### Cookie-only fields

The Open Cloud API doesn't expose every metadata field. These fields require a cookie:

- `game.server_fill`
- `game.allow_copying`
- `game.visibility` (write only; read is via Open Cloud)
- `game.studio_access_to_apis_allowed`
- `game.beta_mode`

The cookie is provided via the global `--cookie` flag, the `RBX_COOKIE` env var, or a local Roblox Studio install (Windows registry, macOS plist) — that last one opt-in, asked once or declined where there is nobody to ask. `--auto-cookie` is the standing yes and `--no-auto-cookie` the standing no. `pull` and `init` skip these fields and say so when no cookie is available; `sync` stops before applying anything if the plan touches one.

A cookie that exists is not a cookie that still works, so when the plan touches one of these fields `sync` also asks Roblox once, before the confirmation prompt and before the first write, whether the session is still valid. An expired one refuses the whole run rather than applying the Open Cloud half and failing on the legacy half. See [what is checked](./cookie.md#what-is-checked-and-when).

The credential itself is documented once, in [docs/cookie.md](./cookie.md): the full resolution order, what an auto-detected cookie prints on stderr, and why it is never written to disk.

**Tip**: pipe the cookie from [Lune](https://lune-org.github.io/) without touching Studio's local files yourself:

```sh
# helper script (Lune)
echo 'local roblox = require("@lune/roblox"); io.write(roblox.getAuthCookie(true) or "")' > get-cookie.luau

# bash
export RBX_COOKIE=$(lune run get-cookie)
# PowerShell
$env:RBX_COOKIE = (lune run get-cookie)
```

### Required API scopes

| Resource | Scopes | Documentation |
| --- | --- | --- |
| Universe | `universe:read`, `universe:write` | [Universe API](https://create.roblox.com/docs/cloud/reference/Universe) |
| Place | `universe.place:read`, `universe.place:write` | [Place API](https://create.roblox.com/docs/cloud/reference/Place) |
| Icons & thumbnails | `universe.image:read`, `universe.image:write` | Uses `legacy-game-internationalization` endpoints |

## Lockfile

`rbx meta` generates a `rbxmeta.lock.toml` next to the config that tracks the last-applied state **per env**:

```toml
version = 1

[envs.dev]
universe_id = ...
place_id = ...
[envs.dev.game]
# ... mirror of [game] + [envs.dev] resolved state
[envs.dev.media.icon]
hash = "..."
image_id = ...

[envs.prod]
# ...
```

Standalone mode (no `--env`) writes under `[envs.default]`. Commit the lockfile to version control.

`sync --env <name>` is idempotent: only fields differing between the resolved (base + overlay) config and `[envs.<name>]` in the lockfile are sent. Media re-uploads happen only when the local file hash differs from the lockfile hash.

## How it works

`sync --env <name>` resolves `(Game, MediaConfig)` for that env (base + overlay), builds a `SyncPlan` against `[envs.<name>]` in the lockfile, and applies it in this order:

0. **Check the session** (cookie), only when the plan contains a cookie-only field. One call, before anything is sent, so a dead session changes nothing at all.
1. **Activate** (legacy / cookie) if `visibility` is going from private to public. Must be first so dependent patches (like paid private servers) don't 500.
2. **PATCH universe** (Open Cloud): voice chat, private server price, devices, social links
3. **PATCH place** (Open Cloud): name, description, server size
4. **PATCH place legacy** (cookie): `server_fill`, `allow_copying`
5. **PATCH universe configuration legacy** (cookie): `studio_access_to_apis_allowed`
6. **POST experience-releases** (cookie): `beta_mode` toggle
7. **Deactivate** (legacy / cookie) if `visibility` is going from public to private. Last so the universe stays in its permissive state until everything else is patched.
8. **Upload icon** if its BLAKE3 hash differs from the lockfile
9. **Delete** thumbnails removed from config, **upload** new ones, **reorder** to match the toml order

The lockfile is saved after **every** successful API call (including each individual thumbnail delete and upload) so a crash mid-sync never leaves remote and lockfile in disagreement.

### Preflight validations

Before sending any request, `rbx meta` validates locally:

- `private_server.price` is `0` or `>= 10` (Roblox rejects 1-9 Robux)
- `visibility = "private"` with `private_server.price > 0` is invalid (Roblox requires public)
- Referenced `media.icon` and `media.thumbnails[]` paths exist on disk

When a Roblox call still fails for a known reason `rbx meta` couldn't detect locally (e.g. the 60-day cooldown on private server price changes), the error message includes a hint pointing to Creator Hub for the real diagnostic.

### Retries

429 and 5xx responses are retried with exponential backoff (max 3 attempts). 403 responses carrying `x-csrf-token` (cookie flow) are retried transparently after caching the new token.

## Attributions

The alpha bleeding implementation, used through `rbx shop`, is adapted from [Asphalt](https://github.com/jackTabsCode/asphalt) (MIT), which itself adapted it from [Tarmac](https://github.com/Roblox/tarmac) (MIT). Thank you to both. The license notices are in [THIRD-PARTY-NOTICES.md](https://github.com/rbx-forge/rbx-cli/blob/main/THIRD-PARTY-NOTICES.md).
