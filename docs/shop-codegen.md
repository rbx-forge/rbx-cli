# rbx shop: code generation

With `codegen.output` set, `rbx shop` writes a Luau module folder after every `sync`, and `rbx shop codegen` rewrites it offline. Game code requires one module and gets the ids of the universe it runs in.

## Code generation

```
src/shared/GameIds/
├─ init.luau          -- picks the env from game.GameId
├─ GameIdsType.luau   -- the GameIds type
├─ dev.luau
└─ prod.luau
```

Names follow the folder: `src/shared/Assets` gives `Assets` and `AssetsType.luau`.

```lua
local GameIds = require(ReplicatedStorage.shared.GameIds)
MarketplaceService:PromptGamePassPurchase(player, GameIds.passes.VIP)
```

Each env module goes through a typed identity function, so strict mode flags a missing or extra id right there. The dispatcher matches every env name exhaustively, so a new env without a branch fails type checking, and an unknown universe errors at runtime:

```lua
-- dev.luau
return Types.gameIds({
    passes = { VIP = 67890, BetaPass = 11111 },
    badges = { Welcome = 98765 },
    products = { Coins100 = 22222 },
})
```

A resource that exists in another env only is `0` here. Prompting a purchase of `0` silently does nothing.

A pass off sale or a badge disabled keeps its real id (players still own it) with a comment, taken from the lockfile:

```lua
LegacyFounder = 11111, -- not for sale
```

### Styles

| `style` | Output | Suits |
| --- | --- | --- |
| `flat` (default) | String keys: `GameIds["passes.VIP"]` | roblox-ts |
| `nested` | Nested tables: `GameIds.passes.VIP` | Luau |

`typescript = true` adds an `init.d.ts`.

## Guarding generated files

`--check` regenerates in memory and compares with the committed files. Offline, so it fits a git hook. Exits `0` clean, `2` different, `1` error.

```sh
rbx shop codegen --check   # against rbxshop.toml + rbxshop.lock.toml
rbx env codegen --check    # against rbxplace.toml
```

lefthook:

```yaml
pre-commit:
  commands:
    codegen:
      glob: "{rbxshop.toml,rbxshop.lock.toml,rbxplace.toml,src/shared/GameIds/*}"
      run: rbx shop codegen --check
      fail_text: "Generated ids are stale or hand-edited. Run `rbx shop codegen` and re-stage."
```

CI:

```yaml
- run: |
    rbx shop codegen --check
    rbx env codegen --check
```

It proves the files match the config and lockfile, not that the lockfile matches Roblox: that is `rbx shop pull --dry-run`. If the generated files are gitignored, run the check in CD instead of a hook.

!!! warning "Keep formatters off the generated folder"
    The comparison is byte for byte (line endings aside). A formatter rewriting those files makes every check fail. Exclude the folder, for example in `.styluaignore`, and from editors that trim whitespace on save.

To fold the folder in GitHub diffs, add to `.gitattributes`:

```
src/shared/GameIds/** linguist-generated=true
```
