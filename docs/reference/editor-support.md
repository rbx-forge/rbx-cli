# Editor support

`schemas/` holds a JSON Schema per config file, derived from the same serde models the CLI parses with, so an editor can validate as you type, complete key names, and show the documentation on hover.

`rbxavatar.schema.json` is the exception: it describes the avatar document `rbx meta` sends as is, without parsing it, so it catches misspelled keys and never blocks one Roblox adds.

## Where the schemas are

Every push to `main` publishes them beside this site, so you do not need the repository checked out to use one:

```
https://rbx-forge.github.io/rbx-deploy/schemas/rbxplace.schema.json
```

They are not wired up automatically yet: that needs the schemas published to [SchemaStore](https://www.schemastore.org/), which is a pull request to their catalog rather than something this repository can do on its own. Until then, point your editor at them.

## taplo

In `.taplo.toml`, or `taplo.toml` at the repository root:

```toml
[[rule]]
include = ["**/rbxplace.toml", "**/rbxplace.example.toml"]
schema.url = "https://rbx-forge.github.io/rbx-deploy/schemas/rbxplace.schema.json"

[[rule]]
include = ["**/rbxshop.toml", "**/rbxshop.example.toml"]
schema.url = "https://rbx-forge.github.io/rbx-deploy/schemas/rbxshop.schema.json"

[[rule]]
include = ["**/rbxmeta.toml", "**/rbxmeta.example.toml"]
schema.url = "https://rbx-forge.github.io/rbx-deploy/schemas/rbxmeta.schema.json"

[[rule]]
include = ["**/rbxconfig.toml", "**/rbxconfig.example.toml"]
schema.url = "https://rbx-forge.github.io/rbx-deploy/schemas/rbxconfig.schema.json"

[[rule]]
include = ["**/rbxrtbf.toml", "**/rbxrtbf.example.toml"]
schema.url = "https://rbx-forge.github.io/rbx-deploy/schemas/rbxrtbf.schema.json"

[[rule]]
include = ["**/rbxavatar.toml"]
schema.url = "https://rbx-forge.github.io/rbx-deploy/schemas/rbxavatar.schema.json"
```

## VS Code

With Even Better TOML, in `.vscode/settings.json`:

```json
{
  "evenBetterToml.schema.associations": {
    "rbxplace(\\.example)?\\.toml$": "https://rbx-forge.github.io/rbx-deploy/schemas/rbxplace.schema.json",
    "rbxmeta(\\.example)?\\.toml$": "https://rbx-forge.github.io/rbx-deploy/schemas/rbxmeta.schema.json",
    "rbxconfig(\\.example)?\\.toml$": "https://rbx-forge.github.io/rbx-deploy/schemas/rbxconfig.schema.json",
    "rbxshop(\\.example)?\\.toml$": "https://rbx-forge.github.io/rbx-deploy/schemas/rbxshop.schema.json",
    "rbxrtbf(\\.example)?\\.toml$": "https://rbx-forge.github.io/rbx-deploy/schemas/rbxrtbf.schema.json",
    "rbxavatar\\.toml$": "https://rbx-forge.github.io/rbx-deploy/schemas/rbxavatar.schema.json"
  }
}
```

Inside a checkout of this repository, `./schemas/<name>.schema.json` works in place of the URL.

`rbxavatar.toml` is a convention: `game.engine_avatar_settings` in `rbxmeta.toml` names the file, so it can be called anything, and the associations above match it only under that name. The patterns cover the `.example` templates too.

## No stricter than the tools

An unrecognised key is warned about and ignored rather than rejected (see [`rbx env`](../env.md)), so no schema closes `additionalProperties`.

## Regenerating

The schemas are generated, and CI fails if they drift from the models:

```sh
cargo run -p rbx-schema            # rewrite schemas/
cargo run -p rbx-schema -- --check # exit 2 if stale
```
