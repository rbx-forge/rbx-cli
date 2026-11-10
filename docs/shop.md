# rbx shop

Game passes, badges and developer products, declared in `rbxshop.toml` and synced to every env. A lockfile tracks the Roblox ids, and a generated Luau module gives game code the right ids at runtime.

```sh
rbx shop init --from-remote --env dev   # adopt what dev already has
rbx shop pull --env prod                # record where prod differs
rbx shop sync --env all --dry-run       # what would change, everywhere
rbx shop sync --env all
```

Without `rbxplace.toml`, `rbx shop init --from-remote --universe-id 123456789` writes the universe into `rbxshop.toml` as `[experience]`, and later commands use it when no `--env` is given.

Every table the file accepts is on [Configuration](shop-config.md); the generated modules are on [Code generation](shop-codegen.md).

## Envs

`[passes.X]`, `[badges.X]` and `[products.X]` are the base, shared by every env. `[envs.<name>.passes.X]` overrides fields for one env. `sync --env prod` applies base plus the `prod` overlay to prod's universe.

`pull --env <name>` writes only differences:

| Remote state | Written to |
| --- | --- |
| A resource not in the base, pulled without an env | the base |
| A resource not in the base, pulled for an env | that env's overlay |
| A field equal to the base | removed from the overlay |
| A field different from the base | the overlay |

Pull never moves a field from the overlays back into the base: do that by hand, and the next pull removes the redundant overlay entries. A resource that exists only in one env's overlay is stubbed to `0` in the other envs' modules, which `MarketplaceService` and `BadgeService` treat as a no-op.

`pull` and `rename` edit `rbxshop.toml` in place: comments, key order and keys `rbx` does not know survive. An unrecognised top-level key is kept and named on stderr.

## Gift products

`create_gift = true` on a pass or product makes `sync` maintain a developer product that players buy to gift the item:

```toml
[gifts]
label = "[GIFT] "   # prefixed to the display name

[passes.VIP]
name = "VIP Pass"
price = 499
icon = "icons/vip.png"
create_gift = true
```

This creates a product named `[GIFT] VIP Pass` with the same price, description and icon, under the key `GiftVIP` (in code: `GameIds.products.GiftVIP`). The twin is never written into `rbxshop.toml`: edit the source and the twin follows.

- The icon is uploaded again for the twin; Roblox takes image bytes, not an asset id.
- `rename` renames the twin's lockfile entry too, so no duplicate is created.
- Turning `create_gift` off leaves the product on Roblox. `check` reports it until `rbx shop forget products GiftVIP`.
- `create_gift` can be overridden per env.
- A derived key that collides with a declared product is an error.

### Adopting `create_gift` on an existing game

Gift products made by hand are imported as ordinary products. `--gift-label` recognises them at import:

```sh
rbx shop init --from-remote --env dev --gift-label "[GIFT] " --dry-run
rbx shop init --from-remote --env dev --gift-label "[GIFT] "
```

A product named `label` + a source's name **and with the same price** becomes that source's twin. A name match with a different price is reported and left alone. To link one by hand: `rbx shop rename products "[GIFT] VIP Pass" GiftVIP`, delete its `[products.GiftVIP]` block (keep the lockfile entry), and add `create_gift = true` to the source.

## Pricing

| In the file | What `sync` sends |
| --- | --- |
| nothing | nothing: Roblox's setting stays |
| `managed_pricing = true` / `false` | Roblox's managed pricing (regional pricing and price optimization) on or off |
| `regional_pricing = true` | the older regional-pricing switch, deprecated by Roblox |

Setting both is refused. `managed_pricing` has no default on purpose: Roblox enables it on passes by itself, and a `false` default would turn it off everywhere. To state it once:

```toml
[settings]
default_managed_pricing = true

[passes.Legacy]
price = 299
managed_pricing = false     # opts out
```

Precedence: `[settings]`, then the item, then its env overlay. Gift twins inherit like any resource.

Whether an item is under price optimization or keeps your price is set per item in the Creator Hub, not through the API. `rbx shop list passes --json` reports it: `UserFixedPrice` in `pricing_features` means your price was kept. A developer product also needs scripted prices and `GetUsersPriceLevelsAsync` in the game for managed pricing to apply.

## Duplicate names

Roblox allows two passes with the same name, and `init --from-remote` and `pull` key new resources by name. On a terminal, you are asked for a key for the second one (empty skips it). Without a terminal, it is skipped and the fix is printed:

```toml
[passes.<your_key>]
id = 222
```

A resource filed under a key that is not its name gets `name = "..."` written beside it, so the next sync does not rename it.

### Creating a duplicate

!!! warning "Commit `rbxshop.lock.toml`"
    `sync` creates every resource missing from the lockfile. Without the lockfile, that is all of them, and Roblox cannot delete a pass or a product: the best repair is `for_sale = false`, forever.

So `sync` lists what exists before it plans, a dry run included, and **stops the whole run** if a name it would create is taken (case-insensitive), suggesting `rbx shop pull` to adopt them. `--allow-duplicate-names` creates them anyway.

## Commands

### rbx shop init

Write a new `rbxshop.toml`.

| Flag | Description |
| --- | --- |
| `--from-remote` | Fill the config and lockfile from what the universe already has |
| `--gift-label <label>` | With `--from-remote`: recognise existing gift products. See [above](#adopting-create_gift-on-an-existing-game) |
| `--dry-run` | With `--from-remote`: show the import, write nothing |

With `--env`, the universe comes from `rbxplace.toml` and the lockfile section is that env's. With `--universe-id`, `[experience]` is written instead.

### rbx shop sync

List what Roblox has, then apply the config (base plus overlay) where it differs.

| Flag | Description |
| --- | --- |
| `--dry-run` | Show the plan, apply nothing. Lists Roblox and sends nothing, so a read key is enough |
| `--only` | `passes`, `badges`, `products`, comma-separated |
| `--badge-cost` | Robux you accept to pay per badge (default `0`) |
| `--yes` / `-y` | Skip the confirmation |
| `--allow-duplicate-names` | Create even when the name is taken. See [Duplicate names](#duplicate-names) |
| `--overwrite-newer` | Replace what was changed on Roblox since your last sync or pull |

Every env is planned before the first one is written, and one confirmation covers the run.

!!! warning "A change made on Roblox is not overwritten silently"
    If Roblox holds another value than the lockfile recorded (a price edited on the Creator Dashboard, a pass taken off sale, a sync from another branch or machine) and the config would replace it, `sync` shows the plan and refuses, a dry run included. `rbx shop pull` brings those values in; `--overwrite-newer` replaces them.

What the plan compares the config with:

- **Roblox**, for the name, description, price, `for_sale`, `enabled`, `store_page` and `managed_pricing` of every resource the lockfile tracks. The lockfile ties each key to a Roblox id, so a resource renamed on Roblox stays under its key.
- **The lockfile**, for `regional_pricing`, which Roblox reports as on whenever managed pricing is, for icons, by content hash, and for what Roblox does not report of an item off sale: its store page, and its price when the listing leaves it out.

A resource made by hand on Roblox, which no lockfile entry tracks, is never touched: `rbx shop pull` adopts it. A resource the lockfile tracks and Roblox does not list keeps its entry and is named, so it is never created a second time.

When the plan creates badges, `sync` first reads the universe's free badge quota. With `--badge-cost 0` and too few left, it stops before any write.

!!! warning "`--yes` turns a reviewed plan into an unattended one"
    `sync` creates things Roblox cannot delete, and badges can cost Robux. Run `--dry-run` in the pull request and `--yes` only in the approved job. See [Working in a team](teams.md#habits-that-keep-this-from-happening).

### rbx shop pull

Write remote state into the config and lockfile, as [above](#envs).

| Flag | Description |
| --- | --- |
| `--dry-run` | Show the changes, write nothing |
| `--accept-remote` | When icons differ, download Roblox's |
| `--accept-local` | When icons differ, keep yours and upload them on the next sync |
| `--yes` / `-y` | Write without asking |

When `pull` finds an icon on Roblox that differs from yours:

```
! [prod] pass 'VIP': icon differs from remote
  Local:  icons/vip.png (blake3: a1b2c3d4e5f6...)
  Remote: asset 987654321012345
```

`--accept-remote` downloads Roblox's; `--accept-local` keeps yours and uploads it on the next sync.

### rbx shop check

Compare the config with the lockfile, offline. Exits `0` in sync, `2` on drift, `1` if it could not answer. It does not contact Roblox: `rbx check` does, in its `shop/live` row. See [Comparing with Roblox](check.md#comparing-with-roblox).

### rbx shop codegen

Regenerate the modules from `rbxshop.toml` and the lockfile, offline. `sync` already does it after a successful run.

```sh
rbx shop codegen
rbx shop codegen --check   # exit 2 on a difference
```

| Flag | Description |
| --- | --- |
| `--check` | Compare instead of writing |

Modules the lockfile no longer produces are deleted, but only files with the `@generated` header. See [Code generation](shop-codegen.md).

### rbx shop rename

`rbx shop rename <resource> <old_key> <new_key>`

Rename a key in the base, every overlay and every lockfile section. The display name is kept.

```sh
rbx shop rename passes VIP vip_pass
```

### rbx shop forget

`rbx shop forget <resource> <key>`

Remove a resource the config no longer declares from the lockfile, and print its id. Nothing is sent to Roblox, so it stays on sale if it was. To retire an item: set `for_sale = false` and sync, remove it from the config, then forget it. Refused while the config still declares the key.

```sh
rbx shop forget passes OldPass --env prod
```

### rbx shop list

`rbx shop list <resource>`

What Roblox has, for one env.

#### `--json`

```json
{
  "schema_version": 1,
  "env": "prod",
  "universe_id": "9876543211",
  "resource": "passes",
  "resources": [
    { "id": "987654321", "name": "VIP Pass", "price": 199, "for_sale": true, "icon_asset_id": "123456789" }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `env` | string | Absent when the target came from `[experience]` |
| `universe_id` | string | |
| `resource` | string | `passes`, `badges` or `products` |
| `resources[].id` | string | |
| `resources[].name`, `.description` | string | |
| `resources[].price` | integer | Absent when Roblox reports none |
| `resources[].for_sale` | boolean | Passes and products |
| `resources[].enabled` | boolean | Badges |
| `resources[].store_page` | boolean | Products |
| `resources[].icon_asset_id` | string | |
| `resources[].managed_pricing`, `.pricing_features` | boolean, string[] | As Roblox reports them |

### rbx shop show

What `sync` would apply, from the local file with defaults filled in. Offline.

```sh
rbx shop show
rbx shop show --sort price     # name (default), price or key
rbx shop show --flat           # one list with a type column
```

#### `--json`

```json
{
  "schema_version": 1,
  "config_file": "rbxshop.toml",
  "env": "prod",
  "passes": {
    "VIP": { "name": "VIP Pass", "price": 299, "for_sale": true, "regional_pricing": false, "create_gift": true }
  },
  "badges": {},
  "products": {}
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `config_file` | string | The file read |
| `env` | string | The overlay applied. Absent without `--env` or with `--env all` |
| `experience` | object | `[experience]` as written. Absent if none |
| `passes` / `badges` / `products` | object | Keyed by TOML key. Gift twins included |
| `*.name` | string | The override. Absent when the key is the name |
| `*.price` | integer | Absent on a free pass |
| `*.for_sale`, `*.regional_pricing`, `*.create_gift`, `products.*.store_page`, `badges.*.enabled` | boolean | Always present |
| `*.managed_pricing` | boolean | Absent when the file says nothing, which is not `false` |
| `*.description`, `*.icon`, `*.path` | string | Absent if unset |

`--json` cannot be combined with `--sort` or `--flat`. Whether the declared and remote sides agree is [`rbx check`](check.md).

### Per-tool flags

| Flag | Description |
| --- | --- |
| `--config <path>` | Path to `rbxshop.toml` (default `rbxshop.toml`) |

## Lockfile

`rbxshop.lock.toml` records, per env, the universe and, for each resource, its Roblox id and what Roblox held at the last `sync` or `pull`. It ties each config key to an id, and tells a change made in the config from a change made on Roblox. Without `--env` the section is `[envs.default]`. Commit it: an entry missing from it is created again on the next sync (see [Creating a duplicate](#creating-a-duplicate) and [Working in a team](teams.md)).

## Required API scopes

`sync` lists the kinds it syncs before it writes: `--only` narrows the listing with the writes.

| Resource | Scopes |
| --- | --- |
| Passes | `game-pass:read`, `game-pass:write` |
| Products | `developer-product:read`, `developer-product:write` |
| Badges | `legacy-badge:manage` to read; `legacy-universe.badge:write` and `legacy-universe.badge:manage-and-spend-robux` to create |
| Icons | `legacy-asset:manage` to download them as stored; without it, a rescaled copy from the public thumbnail service |
| Badge payer | none: Roblox does not check `universe:read` on the universe read; a failed read falls back to `[owner]` |

## Attributions

Alpha bleeding is adapted from [Asphalt](https://github.com/jackTabsCode/asphalt) (MIT), which adapted it from [Tarmac](https://github.com/Roblox/tarmac) (MIT). The code lives in [rbx-core](https://github.com/rbx-forge/rbx-core), and its notices in that repository's [THIRD-PARTY-NOTICES.md](https://github.com/rbx-forge/rbx-core/blob/main/THIRD-PARTY-NOTICES.md).
