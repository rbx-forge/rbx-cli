# rbx-deploy

Declarative Roblox Open Cloud CLI. You write the desired state of an experience into TOML files you commit, and `rbx` reconciles Roblox to match them: diffable, reviewable, safe to run on every push.

**Documentation: <https://rbx-forge.github.io/rbx-deploy/>**

## Install

With [mise](https://mise.jdx.dev):

```sh
mise use --pin github:rbx-forge/rbx-deploy
```

With [Rokit](https://github.com/rojo-rbx/rokit):

```sh
rokit add rbx-forge/rbx-deploy --alias rbx
```

Either one writes the latest release into your project's `mise.toml` or `rokit.toml`, so every machine and CI runner gets the same version. Without `--alias`, Rokit names the command `rbx-deploy`.

Neither? Every release publishes one archive per platform on the [releases page](https://github.com/rbx-forge/rbx-deploy/releases). `gh attestation verify <archive> --repo rbx-forge/rbx-deploy` proves an archive was built by this repository's release workflow; the `SHA256SUMS` beside them only catch a corrupt download. On Linux, take `x86_64-unknown-linux-musl` if the gnu build reports a missing `GLIBC_...` version.

## Usage

One `rbxplace.toml` maps env names to universes and places, and every command resolves `--env` through it:

```toml
[staging]
universe_id = 9876543210
places.main = 123456789012345

[prod]
universe_id = 9876543211
places.main = 234567890123456
```

```sh
export RBX_API_KEY="..."
rbx place upload --env staging --file build.rbxl
rbx shop sync --env all
rbx check --env all        # exit 2 on drift, for CI
```

The [quick start](https://rbx-forge.github.io/rbx-deploy/quickstart/) walks the whole path, from pinning the tool to a staging deploy running in CI.

## Commands

| Command | What it does |
| --- | --- |
| [`rbx init`](./docs/init.md) | Create the group, universe and places, and write them into `rbxplace.toml` |
| [`rbx import`](./docs/import.md) | Adopt a universe that already exists: every config and lockfile, from what is live |
| [`rbx env`](./docs/env.md) | Read `rbxplace.toml`: list envs, print one id, generate a module for game code |
| [`rbx check`](./docs/check.md) | Every configured tool's check in one pass, one exit code. `rbx status` is the same engine for a human |
| [`rbx place`](./docs/place.md) | Place files: upload, download, promote between envs, roll back |
| [`rbx meta`](./docs/meta.md) | Universe and place metadata: name, icon, thumbnails, devices, visibility, avatar rules, paid access |
| [`rbx config`](./docs/config.md) | The live in-experience config, with revisions and rollback |
| [`rbx rtbf`](./docs/rtbf.md) | Which data store keys hold a user's data, so a right-to-be-forgotten request can delete them |
| [`rbx shop`](./docs/shop.md) | Game passes, badges and developer products, with typed Luau codegen |
| [`rbx probe`](./docs/probe.md) | A raw authenticated request to any Open Cloud path. Hidden from `--help` |
| [`rbx completions`](./docs/completions.md) | Shell completions that read your `rbxplace.toml` at TAB time |

## Reference

- [Global flags](./docs/reference/global-flags.md): `--env`, `--api-key`, the cookie, working without a config file
- [Config files](./docs/reference/config-files.md) and their lockfiles, and [editor support](./docs/reference/editor-support.md) through JSON Schemas
- [Exit codes and `--json`](./docs/reference/exit-codes-and-json.md): what a script can rely on
- [Scopes by command](./docs/reference/scopes.md): what the API key needs
- [Stability and support](./docs/reference/stability.md): what will not change under you at 0.x
- [What rbx does not do](./docs/reference/non-goals.md), and why

## Contributing

[CONTRIBUTING.md](./CONTRIBUTING.md) covers setup, what makes a PR mergeable, the DCO sign-off, the automation and the release procedure. [ARCHITECTURE.md](./ARCHITECTURE.md) is the map of the workspace. Vulnerabilities go through [SECURITY.md](./SECURITY.md), never the public tracker.

## Prior art and thanks

- **[Asphalt](https://github.com/jackTabsCode/asphalt)** (MIT): the lockfile-and-codegen pattern `rbx shop` uses for assets, and the alpha-bleed implementation, which is **adapted code**, not just inspiration. Notice in [THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md).
- **[Tarmac](https://github.com/Roblox/tarmac)** (MIT), where that alpha bleed originates; Asphalt adapted it first. Same notice file.
- **[Mantle](https://github.com/blake-mealey/mantle)**: the declarative-IaC precedent for Roblox, and the origin of the `rbx_cookie` crate this tool depends on.
- **[rbxcloud](https://github.com/Sleitnick/rbxcloud)**: the "a CLI for Open Cloud" precedent.

## License

[MPL-2.0](./LICENSE). Source files modified from this project must be kept under MPL-2.0; new files added by downstream users may be licensed independently. Some files are adapted from MIT-licensed projects and carry their own notices: see [THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md).

`rbx-deploy` is a community tool. It is not affiliated with, endorsed by, or sponsored by Roblox Corporation. "Roblox" is a trademark of Roblox Corporation, used here only to say what this tool talks to.
