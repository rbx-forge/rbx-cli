# rbx shop: configuration

Every table `rbxshop.toml` accepts. The commands are on [rbx shop](shop.md).

```toml
[experience]                   # only without --env
universe_id = 123456789

[codegen]
output = "src/shared/GameIds"  # a folder

[passes.VIP]
name = "VIP Pass"              # display name; the key is used if omitted
price = 499
icon = "icons/vip.png"
create_gift = true

[badges.Welcome]
description = "Welcome to the game!"
icon = "icons/welcome.png"

[products.Coins100]
price = 99
icon = "icons/coins.png"

[envs.prod.passes.VIP]
price = 999                    # prod only

[envs.dev.passes.BetaPass]     # exists in dev only
price = 0
```

## `[passes.<key>]`

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `name` | string | the key | Display name |
| `price` | integer | free | Robux |
| `description` | string | | |
| `icon` | path | | |
| `for_sale` | bool | `true` | |
| `managed_pricing` | bool | unset | Unset sends nothing. See [Pricing](shop.md#pricing) |
| `regional_pricing` | bool | `false` | Deprecated by Roblox |
| `create_gift` | bool | `false` | See [Gift products](shop.md#gift-products) |
| `path` | string | | Codegen path for this item |

## `[badges.<key>]`

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `name` | string | the key | Display name |
| `description` | string | | |
| `icon` | path | | |
| `enabled` | bool | `true` | |
| `path` | string | | Codegen path for this item |

## `[products.<key>]`

As passes, with `price` **required**, plus `store_page` (bool, default `false`): show it on the store page.

## `[envs.<name>.*]`

`[envs.dev.passes.VIP]` overrides fields of `[passes.VIP]` in `dev`. A resource that exists only in an overlay belongs to that env alone and is `0` in the other envs' modules.

## `[experience]`

`universe_id`, used only when no `--env` is given.

## `[owner]`

`type` (`"user"` or `"group"`) and `id`. Only read when creating a badge, to say who pays: Roblox charges the universe's owner. You rarely need it, since `sync` asks Roblox who owns the universe (with `universe:read`). It is the fallback when that fails, before `[<env>.owner]` and `[owner]` in `rbxplace.toml`.

## `[codegen]`

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `output` | path | | Folder for the generated modules. No codegen without it |
| `typescript` | bool | `false` | Also write `init.d.ts` |
| `style` | string | `"flat"` | `"flat"` or `"nested"`. See [Code generation](shop-codegen.md#code-generation) |

`[codegen.paths]` renames the section of each kind (`passes`, `badges`, `products`); dots make a prefix (flat) or nested tables (nested). `[codegen.extra]` adds fixed ids to every module:

```toml
[codegen.extra]
"passes.legacy_vip" = 1234567
```

## `[icons]`

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `bleed` | bool | `true` | Alpha bleed before upload. Changing it does not re-upload |
| `dir` | path | `"icons"` | Where `pull --accept-remote` saves icons |

## `[gifts]`

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `label` | string | `"[GIFT] "` | Prefixed to the gift product's display name |
| `key_prefix` | string | `"Gift"` | Prefixed to the key in the lockfile and module: `VIP` gives `GiftVIP` |
| `capitalize_key` | bool | `false` | Capitalise the key after the prefix: `gift` + `vipPass` gives `giftVipPass` |

## `[settings]`

| Field | Type | Description |
| --- | --- | --- |
| `default_managed_pricing` | bool | `managed_pricing` for items that set none. See [Pricing](shop.md#pricing) |

## `[include]`

```toml
[include]
files = ["rbxshop.badges.toml"]
```

Merge resources from other files, relative to this one. Included files may hold only `[passes.*]`, `[badges.*]`, `[products.*]` and their `[envs.*]` overlays, and a key may live in one file only. `pull` and `rename` update an entry in the file that holds it; new entries go to the main file.

## Lockfile

`rbxshop.lock.toml` records, per env, the universe and each resource's Roblox id, name and icon hash. Without `--env` the section is `[envs.default]`.

!!! warning "Commit it"
    An entry missing from the lockfile is created again on the next sync. See [Working in a team](teams.md).

## Icon conflicts

When `pull` finds an icon on Roblox that differs from yours:

```
! [prod] pass 'VIP': icon differs from remote
  Local:  icons/vip.png (blake3: a1b2c3d4e5f6...)
  Remote: asset 987654321012345
```

`--accept-remote` downloads Roblox's; `--accept-local` keeps yours and uploads it on the next sync.

## Required API scopes

| Resource | Scopes |
| --- | --- |
| Passes | `game-pass:read`, `game-pass:write` |
| Products | `developer-product:read`, `developer-product:write` |
| Badges | `legacy-badge:manage` to read; `legacy-universe.badge:write` and `legacy-universe.badge:manage-and-spend-robux` to create |
| Icons | `legacy-asset:manage` to download them as stored; without it, a rescaled copy from the public thumbnail service |
| Badge payer | `universe:read`, optional: falls back to `[owner]` |
