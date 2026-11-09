# rbx meta: configuration

Every table `rbxmeta.toml` accepts. The commands are on [rbx meta](meta.md).

`[experience]` is only read without `--env`; with it, ids come from `rbxplace.toml`. `[envs.<name>]` overrides `[game]` and `[media]` for one env. Fields marked **cookie** need a [session cookie](cookie.md) to write; the rest go through Open Cloud with the API key.

```toml
[experience]                 # only without --env
universe_id = 123456789
place_id = 987654321         # the root place

[game]
name = "My Awesome Game"
description = "A really fun multiplayer game."
server_size = 50
voice_chat = false
visibility = "public"        # cookie to write
studio_access_to_apis_allowed = true   # cookie
beta_mode = false                      # cookie

[game.private_server]
price = 100                  # omit the table to disable private servers

[game.devices]
desktop = true
mobile = true
console = false

[game.social_links.discord]
title = "Join our Discord"
url = "https://discord.gg/example"

[media]
icon = "assets/icon.png"
thumbnails = ["assets/thumb1.png", "assets/thumb2.png"]

[envs.dev]
visibility = "private"
```

## `[experience]`

| Field | Type | Description |
| --- | --- | --- |
| `universe_id` | integer | The universe |
| `place_id` | integer | The root place, where `name`, `description` and `server_size` are written |

## `[game]`

| Field | Type | Write | Description |
| --- | --- | --- | --- |
| `name` | string | key | Display name |
| `description` | string | key | Description |
| `server_size` | integer | key | Max players per server |
| `voice_chat` | bool | key | In-experience voice chat |
| `visibility` | string | cookie | `"public"`, `"limited"` or `"private"`. See [Visibility and audience](#visibility-and-audience) |
| `audience` | string[] | cookie | With `limited` only: `"playtesters"`, `"friends"` (Community Members on a group experience), or both |
| `allow_copying` | bool | cookie | The "copy this place" button. Not a protection |
| `studio_access_to_apis_allowed` | bool | cookie | Studio scripts may call data stores and Open Cloud |
| `beta_mode` | bool | cookie | Experience Beta: hidden from Home recommendations |
| `genre` | string | cookie | Legacy genre: `all`, `tutorial`, `scary`, `town_and_city`, `war`, `funny`, `fantasy`, `adventure`, `sci_fi`, `pirate`, `fps`, `rpg`, `sports`, `ninja`, `wild_west` |
| `engine_avatar_settings` | path | cookie | A `.toml` or `.json` avatar document. See [below](#gameengine_avatar_settings) |

## `[game.private_server]`

| Field | Type | Description |
| --- | --- | --- |
| `price` | integer | Robux. `0` is free, paid starts at `10`; `1` to `9` is refused. Paid needs `visibility = "public"` |

Omit the table to disable private servers.

## `[game.devices]`

`desktop`, `mobile`, `tablet`, `console`, `vr`: booleans. An omitted device is left as it is.

## `[game.social_links.<platform>]`

`title` and `url`, both required. Platforms: `facebook`, `twitter`, `youtube`, `twitch`, `discord`, `roblox_group`, `guilded`. Omitting a platform removes its link.

## `[game.server_fill]`

Cookie.

| Field | Type | Description |
| --- | --- | --- |
| `mode` | string | `"automatic"`, `"empty"` or `"custom"` |
| `reserved_slots` | integer | Slots kept free per server, with `custom` only |

## `[game.paid_access]`

Cookie. Omit the table to leave paid access alone; `mode = "free"` turns it off.

| Field | Type | Description |
| --- | --- | --- |
| `mode` | string | `"free"` or `"paid"` |
| `price` | integer | Robux, with `paid` only |

## `[game.permissions]`

Cookie, [write-only](#write-only-fields). **All four fields are required**: Roblox takes the object whole.

| Field | Description |
| --- | --- |
| `third_party_teleport` | Other experiences may teleport players in |
| `third_party_asset` | This experience may load assets it does not own |
| `third_party_purchase` | This experience may prompt other creators' products |
| `client_teleport` | Client-initiated teleports are allowed |

## `[game.avatar]`

Cookie.

| Field | Values |
| --- | --- |
| `type` | `r6`, `r15`, `player_choice` |
| `animation` | `standard`, `player_choice` |
| `collision` | `inner_box`, `outer_box` |
| `joint_positioning` | `standard`, `artist_intent` |

## `[game.avatar.min_scale]` / `[game.avatar.max_scale]`

Cookie, write-only. **All five fields are required in each**: `height`, `width`, `head`, `body_type` (0 to 1), `proportion` (0 to 1).

!!! warning "Not yet sent to Roblox"
    Roblox's model has a sixth field, `depth`, which this does not send, following Mantle's long use of the same five. Nothing here has sent this object to a live experience yet. If avatars come back squashed after a sync, look here first.

## `[game.avatar.asset_overrides]`

Cookie, write-only. **All ten slots are required**, since Roblox replaces the whole list: `face`, `head`, `torso`, `left_arm`, `right_arm`, `left_leg`, `right_leg`, `t_shirt`, `shirt`, `pants`. Each is an asset id, which forces that asset, or `"player_choice"`.

```toml
[game.avatar.asset_overrides]
face = "player_choice"
head = "player_choice"
torso = "player_choice"
left_arm = "player_choice"
right_arm = "player_choice"
left_leg = "player_choice"
right_leg = "player_choice"
t_shirt = "player_choice"
shirt = "player_choice"
pants = 12345678
```

## `game.engine_avatar_settings`

Cookie, write-only. A path, relative to `rbxmeta.toml`, to a TOML or JSON file holding Roblox's modern avatar rules. The file is parsed, hashed and sent as is: `rbx` does not model its content, because Roblox marks the field experimental and publishes no schema for it.

```toml
# avatar-settings.toml
version = 1

[AvatarRules]
AvatarType = 1

[AvatarCollisionRules]
CollisionMode = 1
SingleColliderSize = [2, 3, 1]
```

!!! warning "Nothing confirms it took effect"
    Roblox accepts any content and returns nothing to compare against, and the result does not show in the Creator Hub. Keep the document small and check in game. `schemas/rbxavatar.schema.json` catches misspelled keys in your editor (see [Editor support](reference/editor-support.md)), and is the only check in the loop.

- **It overlaps the avatar tables.** `AvatarBodyRules` covers the scales and the `Custom*Id` keys cover the asset overrides. A sync that would send both for the same setting is refused, since a contradiction later breaks Studio's avatar settings.
- **`AvatarType` uses different integers** from `[game.avatar] type`: here `0` R6, `1` R15, `2` both.
- An absent key leaves the settings alone; a file containing `{}` clears them.
- Reformatting the file is not a change: the lockfile records a hash of the canonical form.

The most complete public example is [Phoenix-CLI's `ConfigToFile.luau`](https://github.com/PhoenixEntertainment/Phoenix-CLI/blob/main/Test/ConfigToFile.luau).

## `[media]`

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `icon` | path | | PNG icon |
| `thumbnails` | path[] | `[]` | Up to 10 PNGs, in display order |
| `home_thumbnails` | path[] | `[]` | Up to 5 PNGs for the Home Page. See [below](#home-page-thumbnails) |
| `dir` | path | | Where `pull --accept-remote` saves downloaded media |
| `bleed` | bool | `true` | Alpha bleed before upload |
| `language_code` | string | | Write that language's translation instead of the experience's own media. See [below](#own-media-or-a-translation) |

Images are matched by content: renaming or reordering files uploads nothing.

### Own media or a translation

| | `language_code` unset | `language_code = "fr_fr"` |
| --- | --- | --- |
| Writes | the experience's own icon and thumbnails | that language's translation, shown to its players instead |
| Credential | cookie | API key |

The source language cannot be a `language_code`; `sync` refuses it. A translation for a locale the Creator Hub does not list (`en_us`, typically) cannot be removed there, yet is still served in place of the own thumbnails. Delete it with `rbx probe` and a key holding `legacy-universe:manage`:

```text
GET    https://gameinternationalization.roblox.com/v1/game-thumbnails/games/<universe>/images
DELETE /legacy-game-internationalization/v1/game-thumbnails/games/<universe>/language-codes/en_us/images/<id>
```

`sync` reports thumbnails on Roblox that the lockfile does not know (left by an interrupted sync) and never deletes them.

### Home Page thumbnails

A separate set, with no language. Roblox serves one image per player through a personalization configuration and keeps statistics per image. A `sync` uploads the new images, waits for moderation, updates the active configuration (which keeps its statistics), then deletes the images it no longer lists. Uses the API key with `universe.thumbnail:read` and `universe.thumbnail:write`; Roblox marks these endpoints experimental. Images set by hand in the Creator Hub are left alone.

## Visibility and audience

| Creator Hub | `rbxmeta.toml` |
| --- | --- |
| Private | `visibility = "private"` |
| Limited, Playtesters | `visibility = "limited"`, `audience = ["playtesters"]` |
| Limited, Friends or Community Members | `visibility = "limited"`, `audience = ["friends"]` |
| Limited, both | `visibility = "limited"`, `audience = ["playtesters", "friends"]` |
| Public | `visibility = "public"` |

`audience` is required with `limited` and refused otherwise. Reading needs no credential, so `pull` gets it without a cookie; writing needs the cookie.

## Write-only fields

Roblox returns these from no endpoint:

- `game.permissions`
- `game.avatar.min_scale`, `game.avatar.max_scale`
- `game.avatar.asset_overrides`
- `game.engine_avatar_settings`

So `pull` never adopts them, and `check` compares them with the lockfile, not with Roblox. A change made in the Creator Hub is not reported as drift; the next `sync` that touches the field overwrites it.

## Cookie-only fields

The API key cannot write these, so a `sync` whose plan touches one needs a cookie, and checks the session before writing anything ([details](cookie.md#before-a-write)). Without a cookie, `pull` and `init` skip them and say so.

| Field | Roblox feature request for a key route |
| --- | --- |
| `game.visibility`, `game.audience` | [Friends-only access](https://devforum.roblox.com/t/friends-only-access-cant-be-managed-in-automated-deployments/4892105) |
| `game.server_fill` | [Server fill](https://devforum.roblox.com/t/a-places-server-fill-settings-cant-be-kept-consistent-across-environments-or-managed-in-automated-deployments/4892162) |
| `game.allow_copying` | [Allow Copying](https://devforum.roblox.com/t/a-places-allow-copying-setting-cant-be-read-or-enforced-in-automated-deployments/4892136) |
| `game.studio_access_to_apis_allowed` | [Studio access to API services](https://devforum.roblox.com/t/enable-studio-access-to-api-services-cant-be-managed-per-environment-in-automated-deployments/4892091) |
| `game.beta_mode` | |
| `game.genre` | [Experience genre](https://devforum.roblox.com/t/experience-genre-cant-be-kept-consistent-across-environments-or-managed-in-automated-deployments/4892081) |
| `game.avatar.*`, `game.engine_avatar_settings` | [Avatar settings](https://devforum.roblox.com/t/experience-avatar-settings-cant-be-kept-consistent-across-environments-or-managed-in-automated-deployments/4892110) |
| `game.paid_access` | [Paid access](https://devforum.roblox.com/t/paid-access-settings-cant-be-kept-consistent-across-environments-or-managed-in-automated-deployments/4892088) |
| `game.permissions` | [Third-party sales and teleports](https://devforum.roblox.com/t/third-party-sales-and-teleport-permissions-cant-be-managed-in-automated-deployments/4892096) |
| `media.icon`, `media.thumbnails` (own media) | [Experience icons](https://devforum.roblox.com/t/add-open-cloud-support-for-updating-experience-icons/4529017) |

To feed the cookie from [Lune](https://lune-org.github.io/) instead of letting `rbx` read Studio:

```sh
echo 'local roblox = require("@lune/roblox"); io.write(roblox.getAuthCookie(true) or "")' > get-cookie.luau
export RBX_COOKIE=$(lune run get-cookie)
```

## Not supported

- Age rating
- `optInRegions` / `optOutRegions`: the only real value is China, and availability there is decided by Roblox moderation, not by a setting
- Passes, badges, products: [rbx shop](shop.md)
