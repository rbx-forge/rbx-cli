# rbx shop

Declaratively manage Roblox game passes, badges, and developer products from a single TOML config file - with first-class **multi-environment** support (dev/staging/prod universes from one source).

`rbx shop` syncs your local configuration to Roblox, tracks remote state in a per-env lockfile, detects icon changes with [BLAKE3](https://github.com/BLAKE3-team/BLAKE3) hashing, and generates a typed Luau module folder that resolves the right asset IDs at runtime via `game.GameId`.

## Features

- **Declarative config**: define all your passes, badges, and products in a single `rbxshop.toml`
- **Multi-env overlays**: `[envs.<name>]` overlays layered on top of base; one config drives every universe
- **Two-way sync**: push local changes to Roblox or pull remote state into config + lockfile
- **`--env all`**: operate on every env defined in `rbxplace.toml` in one command
- **Auto overlay writes**: pull writes diverging fields to `[envs.<name>]`, clears them when remote matches base
- **Typed codegen**: generates a folder with an `init.luau` dispatcher (exported `GameIds` type) and one module per env, dispatching on `game.GameId` at runtime
- **Offline regeneration + drift guard**: `rbx shop codegen` rebuilds that folder without credentials, and `--check` proves the committed copy still matches its inputs (see [Guarding generated files](#guarding-generated-files))
- **Icon management**: upload icons, detect changes via BLAKE3 hashing, download remote icons
- **Conflict detection**: detects when remote icons differ from local and asks which to keep
- **TypeScript output**: optional `init.d.ts` for roblox-ts consumers
- **Alpha bleed**: applied to icons before upload (enabled by default)
- **Duplicate detection**: when two remote resources share a name, asks which key the second should take (see [Duplicate names](#duplicate-names))
- **Gift products**: `create_gift = true` on a pass or product derives a matching "GiftX" developer product automatically (see [Gift products](#gift-products))
- **JSON**: `--json` on `show` and `list` writes one document to stdout and nothing else, with documented field names, for `jq` and CI. `show` is the declared side, `list` the remote one, and neither claims they agree — that is `rbx check --json`

## Quick start

### Multi-env (recommended): point at a shared `rbxplace.toml`

If you already use `rbx place` / `rbx config`, you have a `rbxplace.toml` like:

```toml
[dev]
universe_id = 9876543210

[prod]
universe_id = 9876543211
```

Initialize from one of the envs, then layer the other:

```sh
rbx shop init --from-remote --env dev --api-key KEY      # populates base from dev
rbx shop pull --env prod --api-key KEY                    # writes [envs.prod] overlay for diverging fields
rbx shop sync --env dev --api-key KEY                     # apply config back to dev
rbx shop sync --env all --api-key KEY                     # apply to every env in one shot
```

### Standalone (no `rbxplace.toml`)

```sh
rbx shop init --from-remote --universe-id 123456789 --api-key KEY
rbx shop sync --api-key KEY
```

This embeds `[experience].universe_id` in `rbxshop.toml` and treats it as the implicit `default` env.

## Multi-environment

Three concepts:

1. **Base** (`[passes.X]`, `[badges.X]`, `[products.X]`): the logical schema shared across every env.
2. **Overlay** (`[envs.<name>.passes.X]`, etc.): per-env diffs layered on top of base.
3. **Resolution**: `--env <name>` resolves `universe_id` from `rbxplace.toml`, merges base + overlay, and uses the result as the effective config.

`sync --env dev` applies `(base + envs.dev)` to dev. `sync --env prod` applies `(base + envs.prod)` to prod. Same `rbxshop.toml`, different effective state per env.

### Pull behavior (differential)

For each field on each resource, when you `pull --env <name>`:

| State | Resulting action |
| --- | --- |
| Resource missing in base, env is `default` | Add to base |
| Resource missing in base, env is named | Add to `[envs.<name>]` overlay |
| Remote field == base field | Clear that field from overlay (no-op if absent) |
| Remote field != base field | Write the diverging field to the overlay |

Pull never auto-promotes overlays back into base. If you notice a field is the same across every env, edit the toml manually to lift it into base - the next pull will detect the match and remove the now-redundant overlay entries.

### What a write-back preserves

`pull` and `rename` edit `rbxshop.toml` in place rather than regenerating it, so a write-back keeps:

- **your comments**, including the ones attached to a resource that was edited, and to one that was renamed;
- **your key order**, so the diff of a pull is the fields that actually changed;
- **keys rbx does not model**, whether a whole top-level table or a stray field inside a `[passes.*]` entry.

Only the fields rbx owns are touched. A default value is written out only where you already wrote it, or where the value diverges from the default - a pull does not sprinkle `for_sale = true` through a file that never mentioned it.

> Both commands only insert and update lines, never reserialize the document. Round-tripping it through serde would drop comments, reorder keys, and silently delete any field it does not model — the same rule `rbxplace.toml` follows, see [`rbx env`](./env.md).

### Unrecognised keys

A top-level key rbx does not read is kept, not deleted - but it is named on stderr at load, because from the outside an ignored key looks exactly like an honoured one:

```
warning: rbxshop.toml: 1 unrecognised top-level key, ignored by rbx 0.1.0:

  pases
    known keys: experience, owner, codegen, icons, gifts, include, passes, badges, products, envs
```

An ignored key changes nothing. Either it is misspelled, or it comes from a release newer than the one you are running.

### Env-exclusive resources

A resource defined only in `[envs.<name>.passes.X]` is treated as exclusive to that env. In the codegen output, missing entries are **stubbed to 0** in every other env's module (a 0 asset ID is silently a no-op when passed to `MarketplaceService` / `BadgeService`, so the same code can run safely across envs).

## Gift products

Roblox has no built-in way to gift a game pass or developer product to another player. The common workaround is a second developer product whose purchase, once handled server-side, grants the original item to a friend instead of the buyer. Setting `create_gift = true` on a `[passes.X]` or `[products.X]` entry provisions that twin product for you:

```toml
[gifts]
label = "[GIFT] "   # prefixed to the source's display name

[passes.VIP]
name = "VIP Pass"
price = 499
description = "VIP access to exclusive areas"
icon = "icons/vip.png"
create_gift = true
```

`sync` will create (and keep in sync) a **developer product** named `"[GIFT] VIP Pass"` with the same price, description, and icon, resolved under the key `GiftVIP` (source key prefixed with `Gift` - a prefix, not a suffix, so every gift twin autocompletes together when a Luau dev types `Gift...`). You'll find it in the generated module at `GameIds.products.GiftVIP` alongside your other products.

The gift twin is **entirely derived** - it is never written into `rbxshop.toml` as its own `[products.GiftVIP]` entry. This is the key property: you only ever edit the source (`[passes.VIP]`), and the twin's price/description/icon/name follow automatically on the next `sync`. There is no second entry to remember to keep in sync.

A few consequences worth knowing:

- **Icons are re-uploaded, not shared.** Roblox's product/pass APIs only accept raw image bytes on create/update, not a reference to an existing asset ID, so the gift's icon costs a separate upload each time the source icon changes.
- **Renaming the source renames the twin.** `rbx shop rename passes VIP vip_pass` also renames the twin's lockfile entry (`GiftVIP` -> `Giftvip_pass`) so the next sync updates the existing remote product instead of creating a duplicate.
- **Turning `create_gift` off doesn't delete anything remotely** (Roblox products can't be deleted via the API). The twin just stops appearing in the resolved config; `check`/`sync` will warn that it "exists in lockfile but not in resolved config" - the same warning any other removed resource gets.
- **`create_gift` can be overlaid per env** just like any other field (`[envs.dev.passes.VIP] create_gift = false`), including on env-exclusive resources.
- **Collisions are rejected.** If the derived key (`Gift<key>`) collides with a real `[products.*]` entry, or two gift-enabled sources derive the same key, `sync`/`check` fail with a clear error rather than silently merging them.
- **`pull` won't re-import the twin.** It recognizes the remote gift product by its derived key and skips writing it back into `rbxshop.toml`.

### Adopting `create_gift` on an existing game

Roblox has no relationship between a pass/product and its gift twin - that link only exists in `rbxshop.toml`, once you declare it. So if a game already has manually-created gift products (built before ever using `rbx shop`, or by a different tool), plain `init --from-remote` or `pull` cannot know that "GIFT - VIP Pass" is meant to be the twin of "VIP Pass" - they import it as its own independent, literal entry.

`rbx shop init --from-remote --gift-label "<label>"` closes that gap by scanning the freshly-imported resources for the pattern once, at import time:

```sh
rbx shop init --from-remote --env dev --gift-label "[GIFT] " --dry-run   # preview first
rbx shop init --from-remote --env dev --gift-label "[GIFT] "             # then for real
```

For every pass/product, it looks for a developer product named exactly `label + <source's name>`. A match is only folded into the `create_gift` convention automatically when **the price also matches** - a name that happens to fit the pattern with a different price is left untouched and reported instead of merged, since that's a much weaker signal of an actual twin:

```
✓ Detected gift twin: pass 'VIP' <- product '[GIFT] VIP Pass' (now `create_gift = true`, tracked as 'GiftVIP')
! Product '[GIFT] Coins100' looks like a gift twin of 'Coins100' but its price differs (149 vs 99) — left as a separate entry, review manually.
```

On a merge, `create_gift = true` is set on the source, the twin's literal config entry is dropped, and its lockfile entry is rekeyed to the derived `Gift<key>` - so the very first `sync` afterward recognizes the existing remote product and updates it instead of creating a duplicate. `--gift-label` requires `--from-remote`; without it, nothing about existing gift products is touched.

`--dry-run` (also `--from-remote`-only) previews the whole import - counts, and every detected merge/mismatch - without downloading icons or writing `rbxshop.toml`/the lockfile, so you can review the plan before committing to it.

If you'd rather do it by hand for a single item instead of scanning everything, or `--gift-label` didn't pick something up (e.g. the price genuinely diverges and you want to force it anyway), the manual recipe is:

1. `rbx shop rename products "GIFT - VIP Pass" GiftVIP` - aligns the existing product's key with what `create_gift` would derive, keeping its remote ID in the lockfile.
2. Delete the now-redundant `[products.GiftVIP]` block from `rbxshop.toml` by hand (keep the lockfile entry).
3. Add `create_gift = true` to `[passes.VIP]`.

The next `sync` will then update the existing remote product rather than create a new one.

## Duplicate names

Roblox does not require game pass, badge or developer product names to be unique. `init --from-remote` and `pull` key a newly discovered resource by its display name, because that is the only human-meaningful handle the API offers — so two passes both called "VIP" want the same key.

**On a terminal, you are asked which key the second should take:**

```
! Two passes are named 'VIP': id 111 already has the key, and id 222 does not
  Give this one its own key, or leave it empty to skip it.
  Key for pass 222: VIP_2
```

The default is a suggestion, not a decision — type `vip_premium` if that is what it is. Leaving it empty skips the resource, which is the old behaviour kept as a deliberate choice rather than a default.

**Off a terminal — CI, a pipe, a cron job — nothing prompts.** A command that stops on a question nobody will answer is worse than one that skips loudly. Instead you get the ids and the entry that fixes it permanently:

```
! Duplicate pass name 'VIP' — skipping id 222 (id 111 keeps the key).
  To manage it, add an entry naming its id, then re-run:

      [passes.<your_key>]
      id = 222
```

**A resource filed under a key that is not its name keeps its real name in the config.** `name` is normally omitted, meaning "the key is the display name". When you file id 222 under `vip_premium`, `name = "VIP"` is written alongside it — without that, the next `sync` would read the key as the name you wanted and rename the live pass.

The tool never invents the key itself. A generated `VIP_2` is an identifier you would live with for as long as the resource exists, chosen by something that has no idea which "VIP" is the premium one.

Only newly discovered resources can collide. Anything already tracked is keyed by its id, so `pull` never displaces a resource the config is already managing.

### The other direction: a lockfile that went missing

Everything above is about *reading* duplicates. The costlier mistake is creating one, and it has a single ordinary cause: `rbxshop.lock.toml` was never committed.

`sync` decides to create a resource from one fact — the key is absent from the lockfile. On a clean checkout with no lockfile, that is every resource in the config, and they all already exist. The duplicates that run would mint cannot be undone: Roblox has no delete for a game pass or a developer product, and the best available repair is setting the accidental twin to `for_sale = false`, which leaves it in the experience forever, visible to everyone who already owns it.

So before creating anything, `sync` lists the experience's existing passes, badges and products and stops if a name it is about to create is already taken:

```
Error: 2 resources would be created under a name that already exists on Roblox:

  pass 'VIP' (key 'VIP') — already id 111
  product '100 Coins' (key 'Coins') — already id 333

The usual cause is a rbxshop.lock.toml that was never committed, which makes every
resource look new.

Adopt what already exists, then sync:
    rbx shop pull --env prod

If these really are meant to be new resources with the same names, re-run with
--allow-duplicate-names. Passes and products cannot be deleted once created.
```

Details worth knowing:

- **It only runs when the plan contains a create.** A sync that only updates resources asks Roblox nothing, so a write-only API key that worked yesterday still works.
- **It stops the whole run**, not just the colliding resource. A partial sync would leave the lockfile describing an env that was half applied.
- **Matching is case-insensitive**, on the resolved display name rather than the config key. A name differing only in case is far more likely to be the resource the lockfile lost track of than a deliberate second one, and the two mistakes do not cost the same: a false stop is a flag away, a false create is permanent.
- **`--allow-duplicate-names`** is the escape hatch for a duplicate you mean. It does not skip the listing, so the run still prints what it matched.

## Commands

<details>
<summary><code>rbx shop init</code></summary>

Initialize a new config file.

| Flag | Description |
| --- | --- |
| `--from-remote` | Populate config and lockfile from existing remote resources |
| `--universe-id` | Universe ID (standalone mode; cannot be combined with `--env`) |
| `--gift-label <label>` | Requires `--from-remote`. Detect pre-existing gift-twin products (name == `label` + source name, same price) and mark `create_gift = true` automatically — see [Adopting `create_gift` on an existing game](#adopting-create_gift-on-an-existing-game) |
| `--dry-run` | Requires `--from-remote`. Preview passes/badges/products (and gift merges) without downloading icons or writing any files |

With `--from-remote --env <name>`, init resolves universe_id from `rbxplace.toml`, fetches remote, writes the lockfile under `[envs.<name>]`, and skips the `[experience]` section in the config. With `--from-remote --universe-id <id>`, the standalone path is taken and `[experience]` is written.

</details>

<details>
<summary><code>rbx shop sync</code></summary>

Apply the resolved (base + overlay) config to Roblox.

| Flag | Description |
| --- | --- |
| `--dry-run` | Show what would change without applying |
| `--only` | Only sync specific types: `passes`, `badges`, `products` (comma-separated) |
| `--badge-cost` | Expected cost in Robux when creating a badge (default: `0`) |
| `--yes` / `-y` | Skip the confirmation prompt. What CI passes |
| `--allow-duplicate-names` | Create resources even when Roblox already has one by that name. See [Duplicate names](#duplicate-names) |

`--yes` is worth thinking about once rather than reaching for. `sync` is the command that issues `Create`, Roblox has no delete verb for passes, badges or products, and badge creation spends Robux — so this flag is what turns a reviewed plan into an unattended one. Put `--dry-run` in the pull request and `--yes` in the job that was approved, not the other way round. See [Working in a team](teams.md#habits-that-keep-this-from-happening).

Use `--env <name>` for a specific env, `--env all` for every env in `rbxplace.toml`, or no flag to fall back to `[experience]`.

</details>

<details>
<summary><code>rbx shop pull</code></summary>

Pull remote state into the config and lockfile. Differential overlay writes (see [Multi-environment](#multi-environment)).

| Flag | Description |
| --- | --- |
| `--dry-run` | Show what would change without writing anything |
| `--accept-remote` | Download remote icons and update local files |
| `--accept-local` | Keep local icons and re-upload on next sync |

</details>

<details>
<summary><code>rbx shop check</code></summary>

Validate the config and report sync state for the targeted env(s). Read-only.

Exit codes: `0` every env is in sync, `2` at least one env has resources to create or update, `1` the check could not answer. Drift sits on its own code so a CI step can gate on the status alone.

</details>

<details>
<summary><code>rbx shop codegen</code></summary>

Regenerate the codegen folder from `rbxshop.toml` + `rbxshop.lock.toml`. Offline — no API key, no network.

`sync` already does this at the end of a successful run. This command exists so that regenerating does not require credentials: rebuild after a `git pull`, or after changing `style` / `paths` / `extra`, without touching Roblox.

```sh
rbx shop codegen           # write the folder
rbx shop codegen --check   # compare instead; exits 2 on a difference
```

| Flag | Description |
| --- | --- |
| `--check` | Compare the folder against what would be generated instead of writing it. Exits `2` on a difference |

Writing also **prunes** modules the current lockfile no longer produces — delete an env and its `<env>.luau` goes with it. Only files carrying the `@generated` header are ever removed, so anything you wrote yourself in that folder is left alone. `--check` reports those leftovers as drift rather than ignoring them: a dead module still looks generated and can still be `require`d.

See [Guarding generated files](#guarding-generated-files).

</details>

<details>
<summary><code>rbx shop rename &lt;resource&gt; &lt;old_key&gt; &lt;new_key&gt;</code></summary>

Rename a resource key across the base, every env overlay, and every env lockfile section. The display name is preserved automatically.

```sh
rbx shop rename passes VIP vip_pass
```

</details>

<details>
<summary><code>rbx shop list &lt;resource&gt;</code></summary>

List remote resources for a single env (does not support `--env all`).

### `--json`

One JSON document on stdout, nothing else. Diagnostics — the unrecognised-key warning in particular — stay on stderr, so the document parses even when `rbxshop.toml` has something wrong with it.

This is the **remote** side: what Roblox has right now. `rbx shop show --json` is the **declared** side. Neither says whether the two agree; that is `rbx check --json`, which reports this domain as `shop/lockfile` and `shop/codegen` and is the only one of the three that carries an `outcome`.

```sh
rbx shop list passes --env prod --json
```

```json
{
  "schema_version": 1,
  "env": "prod",
  "universe_id": "9876543211",
  "resource": "passes",
  "resources": [
    {
      "id": "987654321",
      "name": "VIP Pass",
      "description": "the good one",
      "price": 199,
      "for_sale": true,
      "icon_asset_id": 123456789
    },
    { "id": "987654322", "name": "Starter Pack", "for_sale": false }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today. Refuse a version you do not understand |
| `env` | string | The env named on the command line. **Absent** when there was none and the target came from `[experience]` instead |
| `universe_id` | string | The universe that was queried |
| `resource` | string | The kind asked for, in its CLI spelling: `passes`, `badges`, `products`. On the envelope, not repeated on every row: one invocation lists one kind |
| `resources` | array of objects | One per remote resource, in the order Roblox returned them |
| `resources[].id` | string | The Roblox id. The handle for a remote resource, the way the TOML key is the handle for a declared one |
| `resources[].name` / `.description` | string | As Roblox holds them |
| `resources[].price` | integer | Robux. **Absent** when Roblox reported no price — `Free` for a pass, `-` for a product in the table |
| `resources[].for_sale` | boolean | Passes and products |
| `resources[].enabled` | boolean | Badges only |
| `resources[].store_page` | boolean | Developer products only |
| `resources[].icon_asset_id` | string | The icon asset, which the table has no column for |

Fields that do not apply to a kind are simply absent, as is anything Roblox did not send, so `has("price")` is a usable test and nothing is ever `null`.

```sh
rbx shop list passes --env prod --json | jq -r '.resources[] | "\(.id)\t\(.name)"'
rbx shop list badges --env prod --json | jq '[.resources[] | select(.enabled | not)] | length'
```

</details>

<details>
<summary><code>rbx shop show</code></summary>

Pretty-print the local `rbxshop.toml` with defaults filled in, so you see what `sync` would actually resolve rather than what you typed. Read-only, touches nothing remote.

```sh
rbx shop show
rbx shop show --sort price     # name (default), price, or key
rbx shop show --flat           # one global list with a type column
```

`--sort price` puts entries without a price last. `--flat` merges passes, badges and products into a single sorted list instead of grouping by section — the view for "what is the cheapest thing in this game", which the grouped one cannot answer at a glance.

### `--json`

The same resolved state as one JSON document on stdout, nothing else. Warnings and the per-env overlay hint move to stderr, so the document parses even when there is something to say about the file.

This is the **declared** side: `rbxshop.toml` with defaults filled in and the `--env` overlay applied, which is what `sync` would resolve. `rbx shop list --json` is the **remote** side. Whether the two agree is a third question, and `rbx check --json` is the command that answers it — its rows for this domain are `shop/lockfile` and `shop/codegen`, and they carry `outcome`, `summary` and `details`. None of those three words appears here, so a filter written for one document cannot half-read the other.

```sh
rbx shop show --json
rbx shop show --env prod --json
```

```json
{
  "schema_version": 1,
  "config_file": "rbxshop.toml",
  "env": "prod",
  "experience": { "universe_id": "9876543210" },
  "passes": {
    "VIP": {
      "name": "VIP Pass",
      "price": 299,
      "for_sale": true,
      "regional_pricing": false,
      "create_gift": true,
      "description": "the good one",
      "icon": "icons/vip.png"
    },
    "starter": { "for_sale": false, "regional_pricing": false, "create_gift": false }
  },
  "badges": {
    "first_win": { "name": "First Win", "enabled": true }
  },
  "products": {
    "coins_100": {
      "price": 50,
      "for_sale": true,
      "regional_pricing": false,
      "store_page": true,
      "create_gift": false
    }
  }
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today |

**Every id here is a string, and every price is a number.** Ids identify rather than count, a place id already exceeds 2^53, and a consumer parsing JSON with doubles would round one. Robux is a quantity, so it stays a number and arithmetic on it keeps working. Same rule in every document this tool writes.
| `config_file` | string | The `rbxshop.toml` this was read from, as given or defaulted. `rbx shop list --json` has no such field, because it reads no local file |
| `env` | string | The env whose overlay was applied. **Absent** for the base view — no `--env`, or `--env all`, which has no single overlay to resolve. Same omission rule `rbx check --json` uses for its own `env` |
| `experience` | object | The `[experience]` section as the file spells it. **Absent** when there is none. Nested rather than a bare `universe_id`, because it is a declared fallback target and not necessarily the universe `--env` resolves to |
| `passes` / `badges` / `products` | object | Keyed by TOML key: the handle `--env` overlays, `rename` moves and codegen emits. Empty objects when nothing is declared |
| `*.name` | string | The `name` override. **Absent** when unset, in which case the key is the display name |
| `passes.*.price` | integer | Robux. **Absent** when the file sets none, which for a pass means free |
| `products.*.price` | integer | Robux. Always present: the field is required |
| `*.for_sale`, `*.regional_pricing`, `*.create_gift`, `products.*.store_page`, `badges.*.enabled` | boolean | Always present, with the serde default filled in |
| `*.description` / `*.icon` / `*.path` | string | As the file spells them. **Absent** when unset |

There is no `totals` object anywhere in these two documents. `rbx check --json` has one and it counts outcomes; one here would count rows under the same name. `.passes | length` is the count, and it cannot be misread.

`name` is the override and never the resolved fallback, so `.passes | to_entries[] | (.value.name // .key)` reproduces what the table prints while "renamed" stays distinguishable from "named by its key". Products derived by `create_gift` appear in `products` exactly as the human view shows them, since both read the same resolved state.

`--json` is rejected together with `--sort` and `--flat`: both are layouts over a listing, and the document is an object keyed by TOML key, which has neither an order to pick nor a flat variant to ask for.

```sh
rbx shop show --json | jq -r '.passes | to_entries[] | select(.value.for_sale) | .key'
rbx shop show --env prod --json | jq '[.products[].price] | add'   # full-price basket
```

</details>

### Per-tool flags

| Flag | Description |
| --- | --- |
| `--config <path>` | Path to `rbxshop.toml` (default `rbxshop.toml`) |

## Configuration

```toml
# Standalone fallback. Optional - omit if you always use --env.
[experience]
universe_id = 123456789

# Who owns this project. Global to the config (same for every env), and only
# consulted when a badge is created: ownership is what decides which balance
# Roblox charges. Omit it and [owner] in rbxplace.toml answers instead.
[owner]
type = "group"         # "user" or "group"
id = 123456

[codegen]
output = "src/shared/GameIds"   # FOLDER path - will contain init.luau + per-env modules
# typescript = false           # Also generate init.d.ts inside the folder
# style = "flat"               # "flat" (default) or "nested"

[icons]
bleed = true           # Apply alpha bleed before uploading (default: true)
dir = "icons"          # Directory for downloaded icons (default: "icons")

[gifts]
label = "[GIFT] "      # prefixed to the source's display name for derived gift products
key_prefix = "Gift"    # prefixed to the source's TOML key for the codegen/lockfile key (default)
# capitalize_key = false  # true: "gift" + "vipPass" -> "giftVipPass" instead of "giftvipPass"

# Split passes/badges/products across extra files if this one gets unwieldy (optional)
# [include]
# files = ["rbxshop.badges.toml"]

[passes.VIP]
name = "VIP Pass"      # explicit display name on Roblox (defaults to key)
price = 499
description = "VIP access to exclusive areas"
icon = "icons/vip.png"
create_gift = true     # also provisions a "[GIFT] VIP Pass" developer product - see below

[badges.Welcome]
description = "Welcome to the game!"
icon = "icons/welcome.png"
enabled = true

[products.Coins100]
price = 99
description = "100 coins"
icon = "icons/coins.png"

# Per-env overrides. Layered on top of base when --env <name> is passed.
[envs.prod.passes.VIP]
price = 999                    # VIP costs more in prod

[envs.dev.passes.BetaPass]     # pass exclusive to dev (0-stubbed in prod's module)
price = 0
description = "Beta tester pass"
icon = "icons/beta.png"
```

<details>
<summary><code>[experience]</code></summary>

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `universe_id` | `u64` | **Yes** (if section present) | Your Roblox universe ID |

The whole section is optional in multi-env mode: with `--env <name>`, `universe_id` is resolved from `rbxplace.toml` instead, and `[experience]` is never consulted. It only matters for standalone mode (no `--env`).

</details>

<details>
<summary><code>[owner]</code></summary>

**The one thing it decides is the payment source for badge creation.** It is sent as `paymentSourceType` on the badge-create call, beside `expectedCost`; the scope Roblox wants there is named `legacy-universe.badge:manage-and-spend-robux`. Nothing else in `rbx shop` reads it.

**It is called `owner` because the payer is the owner, necessarily.** Roblox charges a group-owned game's badge to group funds and a user-owned game's to the user's, with no way to cross them — paying for a group game's badge with personal Robux has been an open feature request [since 2018](https://devforum.roblox.com/t/we-should-be-able-to-purchase-a-badge-for-a-group-game-with-personal-funds/131369) and is still not possible. So there is no second party to name, and a field called `payer`, or the `creator` this used to be, would both imply a choice that does not exist.

**You probably do not need to write it at all.** Roblox already knows who owns the universe, and `sync` asks — one `GET /cloud/v2/universes/{id}` before creating a badge, which `universe:read` covers. Ownership is not something a config should have to restate, and a config that restates it can be wrong: `type = "user"` on a group-owned game is a create Roblox refuses, and nothing local would have caught it.

The declaration is the fallback, not the source. When the call cannot answer — a key without `universe:read`, or Roblox reporting neither field — `sync` falls back to `[owner]` here, then to `rbxplace.toml`: the env's own `[<env>.owner]` first, then the top-level one. That is what keeps `universe:read` off the required list for every key that syncs a shop.

Same shape as `[owner]` in `rbxplace.toml`, because it is the same fact.

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `type` | `string` | **Yes** (if section present) | `"user"` or `"group"` |
| `id` | `u64` | **Yes** (if section present) | Roblox user or group ID |

</details>

<details>
<summary><code>[codegen]</code></summary>

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `output` | `string` | -- | **Folder path** for the generated module (omit to disable codegen). Must not end in `.lua`/`.luau` - it's a folder, not a script; `sync` rejects it with a suggested fix if it does |
| `typescript` | `bool` | `false` | Also generate `init.d.ts` |
| `style` | `string` | `"flat"` | `"flat"` or `"nested"` (see [Code generation](#code-generation)) |

</details>

<details>
<summary><code>[codegen.paths]</code></summary>

Override the default section name for each resource type. Dot-separated segments become either a prefix (flat) or nested tables (nested).

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `passes` | `string` | `"passes"` | Path for game passes |
| `badges` | `string` | `"badges"` | Path for badges |
| `products` | `string` | `"products"` | Path for developer products |

</details>

<details>
<summary><code>[codegen.extra]</code></summary>

Inject asset IDs into every env's generated module. Useful for manually managed assets or assets from other universes.

```toml
[codegen.extra]
"passes.legacy_vip" = 1234567
"products.starter_pack" = 9876543
```

</details>

<details>
<summary><code>[icons]</code></summary>

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `bleed` | `bool` | `true` | Apply alpha bleed before uploading. Changing this won't reupload existing images |
| `dir` | `string` | `"icons"` | Directory for icons downloaded by `pull --accept-remote` |

</details>

<details>
<summary><code>[passes.&lt;name&gt;]</code></summary>

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `name` | `string` | No | Display name (defaults to the TOML key) |
| `price` | `u64` | No | Price in Robux (omit for free) |
| `description` | `string` | No | Pass description |
| `icon` | `string` | No | Path to icon file |
| `for_sale` | `bool` | No | Whether the pass is for sale (default: `true`) |
| `regional_pricing` | `bool` | No | Enable regional pricing (default: `false`) |
| `create_gift` | `bool` | No | Derive a "Gift\<name\>" developer product twin (default: `false`) - see [Gift products](#gift-products) |
| `path` | `string` | No | Override the codegen path for this item |

</details>

<details>
<summary><code>[badges.&lt;name&gt;]</code></summary>

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `name` | `string` | No | Display name (defaults to the TOML key) |
| `description` | `string` | No | Badge description |
| `icon` | `string` | No | Path to icon file |
| `enabled` | `bool` | No | Whether the badge is active (default: `true`) |
| `path` | `string` | No | Override the codegen path for this item |

</details>

<details>
<summary><code>[products.&lt;name&gt;]</code></summary>

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `name` | `string` | No | Display name (defaults to the TOML key) |
| `price` | `u64` | **Yes** | Price in Robux |
| `description` | `string` | No | Product description |
| `icon` | `string` | No | Path to icon file |
| `for_sale` | `bool` | No | Whether the product is for sale (default: `true`) |
| `regional_pricing` | `bool` | No | Enable regional pricing (default: `false`) |
| `store_page` | `bool` | No | Show on the store page (default: `false`) |
| `create_gift` | `bool` | No | Derive a "Gift\<name\>" developer product twin (default: `false`) - see [Gift products](#gift-products) |
| `path` | `string` | No | Override the codegen path for this item |

</details>

<details>
<summary><code>[gifts]</code></summary>

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `label` | `string` | `"[GIFT] "` | Prefixed to the source's resolved display name for every derived gift product |
| `key_prefix` | `string` | `"Gift"` | Prefixed to the source's **TOML key** to build the resolved/codegen key. Must be non-empty. E.g. with `key_prefix = "Gift"`: `VIP` -> `GiftVIP`, `vip_pass` -> `Giftvip_pass` |
| `capitalize_key` | `bool` | `false` | Uppercase the first letter of the source key *in the derived key only* (never in the source's own TOML key). With `key_prefix = "gift"`: `vipPass` -> `giftVipPass` instead of the default `giftvipPass` |

Note the three are independent: `label` controls the name shown on Roblox, `key_prefix`/`capitalize_key` control the identifier in the generated Luau/TS module. None of them transforms the source's own key or name in `rbxshop.toml` - only the derived copies. `capitalize_key` exists because a lowercase `key_prefix` run directly into a lowercase-starting key (`giftvipPass`) reads as broken rather than as a compound identifier; capitalizing just that derived copy fixes it without touching how you write your own keys.

</details>

<details>
<summary><code>[include]</code></summary>

Split `passes`/`badges`/`products` across extra files, merged in at load time - only useful if a single `rbxshop.toml` becomes unwieldy. Optional; a single file is the default and requires nothing here.

```toml
# rbxshop.toml
[include]
files = ["rbxshop.badges.toml", "rbxshop.subscriptions.toml"]
```

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `files` | `string[]` | `[]` | Paths (relative to this file) to merge in |

Rules:

- Only meaningful in the main file (the one passed via `--config`); an included file's own `[include]` is rejected.
- Included files may only contain `[passes.*]`, `[badges.*]`, `[products.*]`, and their `[envs.<name>.*]` overlays — nothing else. Any `experience`/`owner`/`codegen`/`icons`/`gifts`/`include` section in an included file is rejected with a clear error. So `[envs.prod.passes.VIP]` can live in `rbxshop.passes.toml` right alongside `[passes.VIP]` — but a `(env, key)` pair still can't be declared in more than one file.
- The same resource key can't appear in more than one file (main or included) - `sync`/`check`/`show` fail with a clear error naming the file and the key.
- **`pull` and `rename` write back to whichever file currently owns the entry.** Updating an existing pass/badge/product (or its `[envs.<name>.*]` overlay) rewrites it in place, wherever it happens to live - never duplicated into the main file. A brand-new entry `pull` discovers, or a new overlay for a resource that doesn't have one yet, is written to the main file by default (a new overlay is instead co-located next to its base resource's file, if that base lives in an included file). If you want something to live in a particular file going forward, move it there yourself - `pull`/`rename` never relocate an existing entry across files, only update it in place.

</details>

<details>
<summary><code>[envs.&lt;name&gt;.*]</code></summary>

Per-env overlays. Each section mirrors a base resource section (`[envs.dev.passes.VIP]` overrides fields of `[passes.VIP]` for env `dev`). All fields are optional - unset fields inherit from the base.

A resource defined only in an overlay (not in base) is treated as env-exclusive: it appears in that env's generated module and is 0-stubbed elsewhere.

</details>

### Required API scopes

| Resource | Scopes |
| --- | --- |
| Game Passes | `game-pass:read`, `game-pass:write` |
| Developer Products | `developer-product:read`, `developer-product:write` |
| Badges | `legacy-badge:manage` to list and read, `legacy-universe.badge:write` and `legacy-universe.badge:manage-and-spend-robux` to create |
| Assets (icons) | `legacy-asset:manage` to read the asset as stored. Without it the public thumbnail service answers instead, at a rescaled size. Uploading a badge icon goes to `legacy-publish`, covered by the badge scopes above |
| Badge payment source | `universe:read` — optional. `sync` uses it to read who owns the universe before creating a badge, and falls back to `[owner]` in the config when the key lacks it |

## Code generation

When `codegen.output` is set, `rbx shop` writes a **folder** at that path after every `sync`. The folder follows Rojo's `init.luau` convention and uses two patterns for compile-time safety: an **identity wrapper function** (validates each env's shape at definition site) and an **exhaustive match** (fails to compile if a new env is added without an accompanying branch in the dispatcher).

```
src/shared/GameIds/
├─ init.luau          -- dispatcher with if/elseif + exhaustiveMatch
├─ GameIdsType.luau   -- type GameIds + gameIds(x) identity wrapper
├─ dev.luau           -- Types.gameIds({...})
└─ prod.luau          -- Types.gameIds({...})
```

The variable name (`GameIds`), the wrapper function (`gameIds`), and the type module file (`GameIdsType.luau`) are derived from the folder name in `codegen.output`. Point it at `src/shared/Assets` and you get `Assets` / `assets(x)` / `AssetsType.luau`.

### Example output (nested style)

**`GameIdsType.luau`** holds the shape contract and the wrapper :

```lua
-- This file is automatically @generated by rbx shop.
-- It is not intended for manual editing.

export type GameIds = {
    passes: {
        VIP: number,
        BetaPass: number,
    },
    badges: {
        Welcome: number,
    },
    products: {
        Coins100: number,
    },
}

local function gameIds(x: GameIds): GameIds
    return x
end

return {
    gameIds = gameIds,
}
```

**`dev.luau`** wraps its data through the identity function so Luau strict mode validates the literal at this exact spot. A missing badge, an extra key, a wrong type, anything off: error here, not somewhere downstream.

```lua
local Types = require(script.Parent.GameIdsType)

return Types.gameIds({
    passes = {
        VIP = 67890,
        BetaPass = 11111,
    },
    badges = {
        Welcome = 98765,
    },
    products = {
        Coins100 = 22222,
    },
})
```

**`prod.luau`** (BetaPass stubbed to `0` since it only exists in dev) :

```lua
local Types = require(script.Parent.GameIdsType)

return Types.gameIds({
    passes = {
        VIP = 99999,
        BetaPass = 0,
    },
    badges = {
        Welcome = 88888,
    },
    products = {
        Coins100 = 77777,
    },
})
```

**`init.luau`** is the dispatcher. It uses a string-literal union for env names and an `exhaustiveMatch(value: never): never` helper so adding a new env to the union without a matching `elseif` branch fails at compile time, not at runtime.

```lua
-- This file is automatically @generated by rbx shop.
-- It is not intended for manual editing.

local Types = require(script.GameIdsType)
export type GameIds = Types.GameIds
export type EnvName = "dev" | "prod"

local UNIVERSE_TO_ENV: { [number]: EnvName } = {
    [9876543210] = "dev",
    [9876543211] = "prod",
}

local function exhaustiveMatch(value: never): never
    error(`rbx shop: unhandled env in dispatcher: {value :: any}`)
end

local env = UNIVERSE_TO_ENV[game.GameId]
if not env then
    error(`rbx shop: unknown universe {game.GameId}`)
end

if env == "dev" then
    return require(script.dev)
elseif env == "prod" then
    return require(script.prod)
else
    exhaustiveMatch(env)
    error("luau")
end
```

Consumer:

```lua
local GameIds = require(ReplicatedStorage.shared.GameIds)
MarketplaceService:PromptGamePassPurchase(player, GameIds.passes.VIP)
```

Autocomplete and strict typing work because the dispatcher returns `GameIds` for every branch (inferred from the wrapper, no `::` cast needed).

### Styles

| Style | Output | When to pick |
| --- | --- | --- |
| `nested` | Nested tables, full per-field types | Best for Luau (direct access, full autocomplete) |
| `flat` | Dot-separated string keys (`GameIds["passes.VIP"]`) | Good for roblox-ts (string-literal keys play nice with TS) |

### Switched-off resources

A pass taken off sale, or a badge that was disabled, still appears in the generated module with its real id — and carries a comment saying so:

```lua
return Types.gameIds({
    passes = {
        VIP = 67890,
        LegacyFounder = 11111, -- not for sale
    },
    badges = {
        Welcome = 98765, -- disabled
    },
})
```

**Keeping the id is deliberate.** A pass that is off sale still has owners, so game code needs the id to answer "does this player own it". Filtering those out would break ownership checks on exactly the passes somebody retired.

What was missing was any way to tell. `VIP = 67890` reads identically whether the pass is on sale or was retired six months ago, so a prompt that silently never opens looks like a bug in the prompt. The annotation carries the answer as far as the module.

The state comes from the lockfile — `for_sale` for passes and products, `enabled` for badges — so it describes what Roblox has as of the last sync, not what the config would like. Ids under `[codegen.extra]` are never annotated: they belong to resources this tool does not manage, so there is nothing to know about them.

### Stubbing semantics

When a resource exists in one env but not another, the missing env's module gets a `0` stub. `MarketplaceService:PromptGamePassPurchase(player, 0)` is silently a no-op, so the same code runs safely across envs - but **prompting purchase of a 0-stubbed ID will silently do nothing**. Generated files include a comment header to flag this.

### TypeScript

With `typescript = true`, an `init.d.ts` file lives alongside `init.luau`:

```typescript
// This file is automatically @generated by rbx shop.
// It is not intended for manual editing.

declare const GameIds: {
    passes: { VIP: number; BetaPass: number }
    badges: { Welcome: number }
    products: { Coins100: number }
}

export = GameIds
```

## Guarding generated files

The generated modules carry an `It is not intended for manual editing` header, but a header is a request, not a guarantee. `--check` is the enforcement: it re-renders the files in memory and asserts the committed copies still match their inputs.

```sh
rbx shop codegen --check                        # rbxshop.toml + rbxshop.lock.toml
rbx env gen-module --out src/Envs.luau --check  # rbxplace.toml
```

Both are **offline** — no API key, no network — which is what makes them usable from a git hook and from CD.

Exit codes: `0` clean, `2` drift, `1` the command itself failed. Drift sits on its own code so a pipeline can tell "regenerate and commit" from "something broke".

### In lefthook

```yaml
pre-commit:
  parallel: true
  commands:
    codegen:
      glob: "{rbxshop.toml,rbxshop.lock.toml,rbxplace.toml,src/shared/GameIds/*}"
      run: rbx shop codegen --check
      fail_text: "Generated ids are stale or hand-edited. Run `rbx shop codegen` and re-stage."
```

### In CI

```yaml
- name: Generated files match their inputs
  run: |
    rbx shop codegen --check
    rbx env gen-module --out src/shared/Envs.luau --check
```

### What it does and does not prove

It proves the committed files equal `f(config, lockfile)`. It does **not** prove the lockfile matches Roblox. Neither does `rbx shop check`, which compares the config against the lockfile and is offline too. The command that asks Roblox is `rbx shop pull --dry-run`, which needs credentials and a network. The three answer different questions and are worth running in different places; see [docs/teams.md](teams.md).

It also only helps if the generated files are committed. If yours are gitignored, this belongs in CD before the build, not in a pre-commit hook.

### Keep formatters off the generated path

The comparison is byte for byte (modulo CRLF, which is normalized — a Windows checkout and a Linux CI runner agree). That only holds while the generator is the **single producer** of those files. Point stylua or prettier at them and every check fails forever, because two tools are writing the same bytes differently and regenerating cannot settle it.

So exclude the generated folder from your formatters:

```
# .styluaignore
src/shared/GameIds/
```

The same goes for an editor set to trim trailing whitespace on save. When every difference turns out to be whitespace, the check says so explicitly instead of printing a diff that regenerating would not fix.

### Collapsing the diffs on GitHub

The generated files carry an `@generated` header, which several review tools recognize. GitHub's own mechanism is `.gitattributes`:

```
src/shared/GameIds/** linguist-generated=true
```

That folds the folder in pull request diffs and excludes it from language stats. `rbx shop` does not write this for you — how your repo is configured is your call.

## Lockfile

`rbx shop` generates a `rbxshop.lock.toml` (sectioned by env) tracking remote state - asset IDs, icon hashes, and metadata per env:

```toml
version = 2

[envs.dev]
universe_id = 9876543210
[envs.dev.passes.VIP]
id = 67890
name = "VIP"
icon_asset_id = ...
icon_hash = "..."

[envs.prod]
universe_id = 9876543211
[envs.prod.passes.VIP]
id = 22222
# ...
```

Standalone mode (no `--env`) writes under `[envs.default]`. Commit the lockfile to version control.

## Icon conflict resolution

When you run `pull` and a remote icon differs from what's in the lockfile:

```
! [prod] pass 'VIP': icon differs from remote
  Local:  icons/vip.png (blake3: a1b2c3d4e5f6...)
  Remote: asset 987654321012345
```

Resolve with:
- `--accept-remote`: downloads the remote icon to your local path
- `--accept-local`: keeps your local icon and re-uploads it on next `sync`

## Attributions

The alpha bleeding implementation is adapted from [Asphalt](https://github.com/jackTabsCode/asphalt) (MIT), which itself adapted it from [Tarmac](https://github.com/Roblox/tarmac) (MIT). Thank you to both. The license notices are in [THIRD-PARTY-NOTICES.md](https://github.com/rbx-forge/rbx-cli/blob/main/THIRD-PARTY-NOTICES.md).
