# Editor support

`schemas/` holds a JSON Schema per config file, derived from the same serde models the CLI parses with, so an editor can validate as you type, complete key names, and show the documentation on hover.

One of them is not derived from a parsing model. `rbxavatar.schema.json` describes the avatar document `rbx meta` sends through verbatim, and there is no model to derive it from precisely because nothing parses it. It is guidance rather than a gate: `additionalProperties` stays open, so a key Roblox adds tomorrow is one your editor stays quiet about and the tool sends anyway.

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

`rbxavatar.toml` is the one whose name is a convention rather than a rule: `game.engine_avatar_settings` in `rbxmeta.toml` names the file, so it can be called anything. Calling it `rbxavatar.toml` is what makes the association above match without editing it.

The patterns cover the `.example` templates too, so the file a newcomer copies gets the same validation as the one they end up editing.

## No stricter than the tools

An unrecognised key is warned about and ignored rather than rejected (see [`rbx env`](../env.md)), so no schema closes `additionalProperties`. An editor that painted a loadable file red would teach you to stop reading the squiggles, and then the real errors go unread too.

## Regenerating

The schemas are generated, and CI fails if they drift from the models:

```sh
cargo run -p rbx-schema            # rewrite schemas/
cargo run -p rbx-schema -- --check # exit 2 if stale
```
