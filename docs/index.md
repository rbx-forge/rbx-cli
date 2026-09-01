# rbx-cli

Unified Roblox Open Cloud CLI. One binary, one install, every tool as a
subcommand. The command is `rbx`, short because you type it all day.

This site is a rendering of the `docs/` directory of the
[rbx-forge/rbx-cli](https://github.com/rbx-forge/rbx-cli) repository. Every
page is the file the repository already serves, built by CI on every push to
`main`, so the site and the repository cannot disagree. The "Suggest an edit"
link at the top right of each page opens the exact file it came from.

## Two tools, one backbone

The command surface looks broad because it is two products that happen to share
a spine. The spine is the environment model: one `rbxplace.toml` maps env names
to universes and places, and every command resolves `--env` through it.

**Declarative, Terraform for Roblox.** `init`, `env`, `apikey`, `place`,
`meta`, `config`, `rtbf`, `shop`. You write the desired state into a TOML file you
commit, and the tool reconciles Roblox to match it. Diffable, reviewable, safe
to run on every push, idempotent by construction.

**Operational, kubectl for Roblox.** `servers`, `analytics`, `ban`, `restart`,
`data`, `memorystore`, `message`. These act on state that only exists while the
game is running, and no TOML file can describe it. Banning a player is a
consequence of what happened in your game last night, not a checked-in
intention.

One command sits between the two on purpose. `secret` writes the credentials
the game reads at runtime, and they are the one part of a universe's
configuration that a repository must never contain, so there is no file to
reconcile from, and the value is sealed before it leaves your machine.

Comparable tools have the first pillar. Mantle never had the second, and
nothing else does either: you are otherwise clicking through the Creator Hub or
writing your own Open Cloud scripts. The second pillar is the difference
between deploying a game and *running* one, and it is why the surface is wide
on purpose rather than by accretion.

## Every command

| Command | What it does |
| --- | --- |
| [`init`](init.md) | Create the group, universe and places, and write them into `rbxplace.toml` |
| [`import`](import.md) | Adopt a universe that already exists: every config and lockfile, from what is live |
| [`env`](env.md) | Read `rbxplace.toml`: list envs, print one id, generate a module for game code |
| [`apikey`](apikey.md) | Declare Open Cloud keys and their scopes, create and rotate them |
| [`doctor`](doctor.md) | Prove the loaded key works, with one real read rather than a syntax check |
| [`check`](check.md) | Every configured tool's check in one pass, one exit code. `status` is the same engine for a human |
| [`place`](place.md) | Place files: upload, download, promote between envs, roll back |
| [`meta`](meta.md) | Universe and place metadata: name, icon, thumbnails, devices, visibility |
| [`config`](config.md) | The live in-experience config, with revisions and rollback |
| [`secret`](secret.md) | Credentials the game reads through `HttpService:GetSecret`, written encrypted |
| [`rtbf`](rtbf.md) | Which data store keys hold a user's data, so a right-to-be-forgotten request can delete them |
| [`shop`](shop.md) | Game passes, badges and developer products, with typed Luau codegen |
| [`servers`](ops/servers.md) | **Live.** Servers up now, how the stopped ones ended, and what a crashed one logged |
| [`analytics`](ops/analytics.md) | **Live.** Players, retention, revenue per payer. CSV for charting elsewhere |
| [`ban`](ops/ban.md) | **Live.** Inspect and change player restrictions |
| [`restart`](ops/restart.md) | **Live.** Forecast and launch a rolling server restart |
| [`data`](ops/data.md) | **Live.** Read, overwrite, copy and recover one data store entry |
| [`memorystore`](ops/memorystore.md) | **Live.** Write cache values servers read through `MemoryStoreService` |
| [`message`](ops/message.md) | **Live.** Push a MessagingService message to every running server |
| [`ads`](ops/ads.md) | **Live.** Launch and steer ad campaigns. Spends money, reads no results |
| [`probe`](ops/probe.md) | **Live.** A raw authenticated request to any Open Cloud path |
| [`open`](open.md) | Launch Studio at a place, by name or by id |
| [`download`](download.md) | Fetch a Roblox asset by id |
| [`completions`](completions.md) | Shell completions that read your `rbxplace.toml` at TAB time |

Everything marked **Live** acts on a running game and shares one safety model:
dry run by default, `--apply` to write, `--env all` refused. That model, and the
keys it wants, are on [Live operations](ops.md).

## Install

Add it to your project's `rokit.toml`, then run `rokit install`. Pin the
version you want from the
[releases page](https://github.com/rbx-forge/rbx-cli/releases):

```toml
[tools]
rbx = "rbx-forge/rbx-cli"
```

Or let Rokit write that entry for you. Pass the alias explicitly, or you get an
`rbx-cli` command instead of `rbx`:

```sh
rokit add rbx-forge/rbx-cli --alias rbx
```

The precompiled binaries attached to every release need no Rust toolchain.
Building from source does, at the MSRV declared in the workspace manifest.

## Where to start

In a hurry, or starting from nothing? [Quick start](quickstart.md) walks the
whole path on one page, from pinning the tools to a staging deploy running in
CI, and links here for the depth rather than before it.

The longer route, and the two ways in depending on whether the experience
exists yet:

- **Nothing on Roblox yet.** [`rbx init`](init.md) creates the group, universe
  and places, and writes the `rbxplace.toml` that everything else reads.
- **A universe that already exists.** [`rbx import`](import.md) adopts it:
  every config and lockfile written from what is live, in one command.

Both paths converge on the same three pages. [`rbx env`](env.md) explains the
`rbxplace.toml` they produce and how `--env` resolves through it,
[`rbx apikey`](apikey.md) manages the Open Cloud keys declaratively, and
[`rbx doctor`](doctor.md) proves those keys work with one real read rather than
a syntax check.

From there:

- **Putting it in CI.** [`rbx check`](check.md) runs every configured tool's
  check in one pass with one aggregated exit code. That page is also where
  `rbx status`, the same engine for a human rather than a pipeline, is
  documented.
- **Shipping a build.** [`rbx place`](place.md) uploads, downloads, promotes
  between envs and rolls back to a past version.
- **Everything that is not the place file.** [`rbx meta`](meta.md) for universe
  and place metadata, [`rbx config`](config.md) for in-experience live configs,
  [`rbx shop`](shop.md) for passes, badges and developer products with typed
  Luau codegen.
- **Answering a deletion request.** [`rbx rtbf`](rtbf.md) declares which data
  store keys hold a user's data, and checks the declaration against the stores
  you actually have. A template that matches nothing is accepted by Roblox and
  deletes nothing, which is the failure worth catching before somebody asks.
- **Acting on a running game.** [Live operations](ops.md) is the entry point
  and the safety model: dry run by default, `--apply` to write, `--env all`
  refused.
- **More than one person on the repository.** [Working in a team](teams.md) is
  the lockfile conflict procedure. Worth reading before you need it, because a
  badly resolved `rbxshop.lock.toml` creates a second paid game pass that this
  tool cannot delete.

## Not on this site

Documentation about the repository rather than the tool stays in the
repository:
[README](https://github.com/rbx-forge/rbx-cli/blob/main/README.md) for the full
command table and the stability policy,
[ARCHITECTURE.md](https://github.com/rbx-forge/rbx-cli/blob/main/ARCHITECTURE.md)
for the crate layout,
[CONTRIBUTING.md](https://github.com/rbx-forge/rbx-cli/blob/main/CONTRIBUTING.md)
for setup and what makes a PR mergeable,
[CHANGELOG.md](https://github.com/rbx-forge/rbx-cli/blob/main/CHANGELOG.md) for
what changed in a release, and
[SECURITY.md](https://github.com/rbx-forge/rbx-cli/blob/main/SECURITY.md) for
where vulnerability reports go, which is never the public tracker.

`rbx-cli` is a community tool under
[MPL-2.0](https://github.com/rbx-forge/rbx-cli/blob/main/LICENSE). It is not
affiliated with, endorsed by, or sponsored by Roblox Corporation.
