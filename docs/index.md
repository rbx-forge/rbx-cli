# rbx-deploy

Declarative deployment for Roblox. Describe an experience in TOML files you commit, and `rbx` makes Roblox match them, env by env.

```sh
rbx place upload --env staging --file build.rbxl   # ship a build
rbx shop sync --env all                            # passes, badges, products everywhere
rbx check --env all                                # exit 2 on drift, for CI
```

[Quick start](quickstart.md){ .md-button .md-button--primary } [Commands](#commands){ .md-button }

**One file per concern.** Places, metadata, live config, passes and right-to-be-forgotten templates each have their own TOML file and lockfile, all keyed by the envs in one `rbxplace.toml`.

**Safe on every push.** Plans before it writes, asks before touching an env marked `confirm`, and `rbx check` gives CI one exit code.

**Typed ids for game code.** Generated Luau modules hand the game the right ids for the universe it runs in.

## Install

=== "mise"

    ```sh
    mise use --pin github:rbx-forge/rbx-deploy
    ```

=== "Rokit"

    ```sh
    rokit add rbx-forge/rbx-deploy --alias rbx
    ```

    Without `--alias`, Rokit names the command `rbx-deploy`.

Either one pins the latest release in your project. Archives for every platform are on the [releases page](https://github.com/rbx-forge/rbx-deploy/releases).

## Commands

| Command | What it does |
| --- | --- |
| [`init`](init.md) | Create the group, universe and places |
| [`import`](import.md) | Adopt a universe that already exists |
| [`env`](env.md) | Read and edit `rbxplace.toml`, generate an env module |
| [`check`](check.md) | Every tool's check, one exit code; `status` for a person |
| [`place`](place.md) | Upload, download, promote, roll back place files |
| [`meta`](meta.md) | Name, icon, thumbnails, devices, visibility, avatar rules |
| [`config`](config.md) | The live in-experience config |
| [`rtbf`](rtbf.md) | Right-to-be-forgotten deletion templates |
| [`shop`](shop.md) | Passes, badges, developer products, with Luau codegen |
| [`probe`](probe.md) | A raw request to any Open Cloud path |
| [`completions`](completions.md) | Shell completions for `--env` and `--place` |

Not in scope: anything without a file to reconcile from, like acting on a running game or managing API keys. See [What rbx does not do](reference/non-goals.md).

---

A community tool under [MPL-2.0](https://github.com/rbx-forge/rbx-deploy/blob/main/LICENSE), not affiliated with Roblox Corporation. Source, changelog and contributing guide on [GitHub](https://github.com/rbx-forge/rbx-deploy).
