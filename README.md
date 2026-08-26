# rbx-cli

Unified Roblox Open Cloud CLI. One binary, one install, every tool as a subcommand.

Add it to your project's `rokit.toml`, then run `rokit install`:

```toml
[tools]
rbx = "rbx-forge/rbx-cli@0.2.0"
```

The command is `rbx`: short, because you type it all day. The repository
carries the `-cli` suffix to distinguish it from the `rbx_*` libraries
(`rbx_dom`, `rbx_binary`, …), the same way `aws-cli` installs as `aws`.
Prefer `rokit add`? Pass the alias explicitly, or you get an `rbx-cli`
command instead: `rokit add rbx-forge/rbx-cli --alias rbx`.

Not using Rokit? Every release publishes one archive per platform on the
[releases page](https://github.com/rbx-forge/rbx-cli/releases), with a
`SHA256SUMS` beside them. There are two Linux builds and the difference
matters: `x86_64-unknown-linux-gnu` is compiled against the build runner's
glibc and will not start on an older distribution, while
`x86_64-unknown-linux-musl` is statically linked and does not care. Reach for
musl when the gnu binary reports a missing `GLIBC_...` version.

**Documentation site: <https://rbx-forge.github.io/rbx-cli/>**: the `docs/`
pages linked below, rendered with search and cross-links. Built from the same
files, so it is never a second copy to keep in sync.

## Two tools, one backbone

The command surface looks broad because it is two products that happen to share
a spine. The spine is the environment model: one `rbxplace.toml` maps env names
to universes and places, and every command below resolves `--env` through it.

**Declarative: Terraform for Roblox.** `init`, `env`, `apikey`, `place`,
`meta`, `config`, `rtbf`, `shop`. You write the desired state into a TOML file
you commit, and the tool reconciles Roblox to match it. Diffable, reviewable,
safe to run on every push, idempotent by construction.

**Operational: kubectl for Roblox.** `servers`, `analytics`, `ban`, `restart`,
`data`, `memorystore`, `message`. These act on state that only exists while the
game is running, and no TOML file can describe it. Banning a player is a
consequence of what happened in your game last night, not a checked-in
intention.

One command sits between the two. `secret` writes the credentials the game
reads at runtime, and they are the one part of a universe's configuration that
a repository must never contain, so there is no file to reconcile from, and
the value is sealed against the universe's public key before it leaves your
machine.

Comparable tools have the first pillar. Mantle never had the second, and
nothing else does either: you are otherwise clicking through the Creator Hub
or writing your own Open Cloud scripts. The second pillar is the difference
between deploying a game and *running* one, and it is why the surface is wide
on purpose rather than by accretion.

`rbx --help` already tells this story in its ordering and its `Live:` prefixes.

## Subcommands

Ordered top-to-bottom by typical user journey (bootstrap → auth → routine ops → local utilities), which is also the order `rbx --help` prints them in:

| Subcommand | What it does | Detailed docs |
| --- | --- | --- |
| `rbx init` | Bootstrap Roblox resources from scratch: groups, universes, places. | [docs/init.md](./docs/init.md) |
| `rbx import` | Adopt an existing universe: write every config and lockfile from it, in one command. | [docs/import.md](./docs/import.md) |
| `rbx env` | Read `rbxplace.toml`: list envs, print a single id, generate a module for game code (and verify it), remove an env from every file that mentions it. | [docs/env.md](./docs/env.md) |
| `rbx apikey` | Declaratively manage Open Cloud API keys from `rbxapikey.toml`. | [docs/apikey.md](./docs/apikey.md) |
| `rbx doctor` | Diagnose credentials, key validity and scope coverage, with one real read to prove it end to end. | [docs/doctor.md](./docs/doctor.md) |
| `rbx check` | Run every configured tool's check in one pass, with one aggregated exit code. The CI contract. | [docs/check.md](./docs/check.md) |
| `rbx status` | The same engine, grouped by environment and always exit 0: where the project stands, for a human. | [docs/check.md](./docs/check.md#rbx-status) |
| `rbx place` | Place file upload, download, promote between envs, rollback to past versions. | [docs/place.md](./docs/place.md) |
| `rbx meta` | Universe and place metadata (name, description, devices, social links, server fill, avatar rules, third-party permissions, paid access, ...). | [docs/meta.md](./docs/meta.md) |
| `rbx config` | In-experience live configs via the Open Cloud Configs API. | [docs/config.md](./docs/config.md) |
| `rbx secret` | Credentials the game reads through `HttpService:GetSecret`, sealed before they leave your machine. | [docs/secret.md](./docs/secret.md) |
| `rbx rtbf` | Which data store keys hold a user's data, so Roblox can delete them on a right-to-be-forgotten request, checked against the stores you actually have. | [docs/rtbf.md](./docs/rtbf.md) |
| `rbx shop` | Game passes, badges, developer products. Typed Luau codegen with runtime env dispatch, regenerable offline. | [docs/shop.md](./docs/shop.md) |
| `rbx open` | Launch Roblox Studio at a specific place by env name. | [docs/open.md](./docs/open.md) |
| `rbx download` | Download assets by id (public endpoint or Open Cloud). | [docs/download.md](./docs/download.md) |

### Live operations

The commands above reconcile state you declared in your repo, and are safe to run in CI on every push. These act on things that only exist while the game is running: the servers currently up, a player's data, who is allowed in. They are listed last, and each says `Live:` in `rbx --help`.

| Subcommand | What it does | Detailed docs |
| --- | --- | --- |
| `rbx servers` | Live and terminated servers, and the logs of one that crashed. Roblox keeps 30 days. | [docs/ops/servers.md](./docs/ops/servers.md) |
| `rbx analytics` | Query your own metrics: players, retention, ARPPU. CSV for charting elsewhere. | [docs/ops/analytics.md](./docs/ops/analytics.md) |
| `rbx ban` | Inspect and change player restrictions. Resolves usernames, dry-run and prompt before writing. | [docs/ops/ban.md](./docs/ops/ban.md) |
| `rbx restart` | Roll servers onto a published version, with Roblox's own impact forecast as the dry run. | [docs/ops/restart.md](./docs/ops/restart.md) |
| `rbx data` | Read, overwrite, copy and recover a data store entry; and `data ordered` for leaderboards. | [docs/ops/data.md](./docs/ops/data.md) |
| `rbx memorystore` | Write cache values from outside Roblox that servers read through `MemoryStoreService`, with a TTL. | [docs/ops/memorystore.md](./docs/ops/memorystore.md) |
| `rbx message` | Push a MessagingService message to every running server: the nudge that makes them re-read a memory store value. | [docs/ops/message.md](./docs/ops/message.md) |
| `rbx ads` | Launch and steer ad campaigns. Spends real money and reads no results back. | [docs/ops/ads.md](./docs/ops/ads.md) |
| `rbx probe` | Raw authenticated request to any Open Cloud path. Hidden from `--help`; for beta endpoints with no published schema. | [docs/ops/probe.md](./docs/ops/probe.md) |

Their writes are dry-run by default and need `--apply`. `--env all` is refused: each env is a different experience, and no command touching live players should fan out because it matched a glob.

**A second binary would make the boundary visible in the command name, and it is not on offer.** Rokit resolves one artifact per repository, so of two published binaries only one could ever be installed through it. It would not be the boundary that matters in any case: Roblox binds an API key to its scopes when you create it, so a deploy key cannot ban anybody whichever binary calls it.

**Keep read and write in separate keys**: that is the boundary. The key that ends up in a shell history during a debugging session should be the one that cannot ban anybody.

Start with [docs/ops.md](./docs/ops.md) for API keys and the safety model.

Built-in help is available at every level:

```sh
rbx -h                # compact summary (all subcommands + global flags)
rbx --help            # verbose: full flag descriptions
rbx shop --help       # per-subcommand help
rbx shop sync --help  # per-verb help
```

## Configuration files

Each subcommand owns its TOML config; the cross-tool file is `rbxplace.toml`. No giant monolithic config; each concern is separable.

| File | Owned by | Purpose |
| --- | --- | --- |
| `rbxplace.toml` | shared | Maps env names to universe + place IDs. Read by every subcommand that takes `--env`; inspect it with `rbx env`. |
| `rbxshop.toml` | `rbx shop` | Game passes, badges, developer products. Multi-env overlays. |
| `rbxmeta.toml` | `rbx meta` | Universe and place metadata. Multi-env overlays. |
| `rbxconfig.toml` | `rbx config` | In-experience configs. Multi-env overlays. |
| `rbxrtbf.toml` | `rbx rtbf` | Right-to-be-forgotten deletion templates. Declared once, published to any env. |
| `rbxapikey.toml` | `rbx apikey` | API key declarations (scopes, IP allowlist, expiry). |

Most of these have a lockfile beside them (`<config>.lock.toml`) tracking
remote IDs and content hashes for diff/sync. `rbxrtbf.toml` does not, for the
reason `rbxconfig.toml` needs no entry snapshot to compare against: the
published templates are readable in full, so the remote state is a fetch rather
than something that has to be remembered.

Those lockfiles are committed (all but `rbxapikey.lock.toml`, which holds secrets and **you** must gitignore: `rbx apikey create` refuses to create a key whose secret would land in a file git is not ignoring), so more than one person syncing means git will eventually hand you a conflict in one. [docs/teams.md](./docs/teams.md) is the procedure: which side to keep per file, why a dropped `rbxshop.lock.toml` entry becomes a duplicate paid resource on the next sync, and what concurrent syncs do per tool.

### Editor support

`schemas/` holds a JSON Schema per config file, derived from the same serde
models the CLI parses with, so an editor can validate as you type, complete key
names, and show the documentation on hover.

One of them is not derived from a parsing model. `rbxavatar.schema.json`
describes the avatar document `rbx meta` sends through verbatim, and there is no
model to derive it from precisely because nothing parses it. It is guidance
rather than a gate: `additionalProperties` stays open, so a key Roblox adds
tomorrow is one your editor stays quiet about and the tool sends anyway.

Every push to `main` publishes the schemas beside the documentation site, so
they have a URL and you do not need this repository checked out to use one:

```
https://rbx-forge.github.io/rbx-cli/schemas/rbxplace.schema.json
```

They are not wired up automatically yet: that needs the schemas published to
[SchemaStore](https://www.schemastore.org/), which is a pull request to their
catalog rather than something this repository can do on its own. Until then,
point your editor at them directly.

**taplo** (`.taplo.toml`, or `taplo.toml` at the repo root):

```toml
[[rule]]
include = ["**/rbxplace.toml", "**/rbxplace.example.toml"]
schema.path = "schemas/rbxplace.schema.json"

[[rule]]
include = ["**/rbxapikey.toml", "**/rbxapikey.example.toml"]
schema.path = "schemas/rbxapikey.schema.json"
```

**VS Code** with Even Better TOML, in `.vscode/settings.json`:

```json
{
  "evenBetterToml.schema.associations": {
    "rbxplace(\\.example)?\\.toml$": "./schemas/rbxplace.schema.json",
    "rbxmeta(\\.example)?\\.toml$": "./schemas/rbxmeta.schema.json",
    "rbxconfig(\\.example)?\\.toml$": "./schemas/rbxconfig.schema.json",
    "rbxapikey(\\.example)?\\.toml$": "./schemas/rbxapikey.schema.json",
    "rbxshop(\\.example)?\\.toml$": "./schemas/rbxshop.schema.json",
    "rbxrtbf(\\.example)?\\.toml$": "./schemas/rbxrtbf.schema.json",
    "rbxavatar\\.toml$": "./schemas/rbxavatar.schema.json"
  }
}
```

Even Better TOML takes a URL in the same place, so swap `./schemas/…` for
`https://rbx-forge.github.io/rbx-cli/schemas/…` in a project that does not
contain this repository.

`rbxavatar.toml` is the one whose name is a convention rather than a rule:
`game.engine_avatar_settings` in `rbxmeta.toml` names the file, so it can be
called anything. Calling it `rbxavatar.toml` is what makes the association
above match without editing it.

The patterns cover the `.example` templates too, so the file a newcomer copies
gets the same validation as the one they end up editing.

The schemas are generated, and CI fails if they drift from the models:

```sh
cargo run -p rbx-schema            # rewrite schemas/
cargo run -p rbx-schema -- --check # exit 2 if stale
```

**They are deliberately no stricter than the tools.** An unrecognised key is
warned about and ignored rather than rejected (see [docs/env.md](./docs/env.md)),
so no schema closes `additionalProperties`. An editor that painted a loadable
file red would teach you to stop reading the squiggles, and then the real
errors go unread too.

## Multi-environment workflow

Most subcommands route through a single shared `rbxplace.toml` that maps env names to universe/place ids:

```toml
[dev]
universe_id = 9876543210
[dev.places]
main = 123456789012345

[prod]
universe_id = 9876543211
[prod.places]
main = 234567890123456
```

With that in place, every subcommand accepts `--env <name>`:

```sh
rbx shop sync --env prod
rbx place upload --env prod --file game.rbxl
rbx meta sync --env prod
```

Some subcommands also support `--env all` to act on every env defined in `rbxplace.toml` in one shot:

```sh
rbx shop sync --env all          # sync passes/badges/products to every env
rbx meta check --env all         # show diff against every env
```

To read the file back rather than edit it, use `rbx env`:

```sh
rbx env list                                # every env, its universe id and places
UNIVERSE=$(rbx env get universe-id -e prod) # one bare value, for scripts and CI
```

## Generated files stay honest

Both generators write modules your game code imports, derived from files you
commit. They can also verify that what is committed still matches, instead of
writing:

```sh
rbx shop codegen --check     # against rbxshop.toml + rbxshop.lock.toml
rbx env gen-module --check   # against rbxplace.toml
```

No API key, no network: the inputs are local, so this runs in a pre-commit
hook and in CD. Exit code `2` means drift, kept separate from `1` so a pipeline
can tell "regenerate and commit" from "the command failed". Details and the
hook snippets: [Guarding generated files](./docs/shop.md#guarding-generated-files).

## Global flags

These are accepted by every subcommand and work both before and after the subcommand name:

| Flag | Env var | Description |
| --- | --- | --- |
| `--api-key <key>` | `RBX_API_KEY` | Open Cloud API key |
| `--cookie <value>` | `RBX_COOKIE` | `.ROBLOSECURITY` cookie for legacy endpoints |
| `--no-auto-cookie` | | Disable Studio cookie auto-detection |
| `--env <name>`, `-e` | | Target env. `all` expands to every env in `rbxplace.toml` |
| `--place <name>` | | Place within the env (for subcommands that operate at place scope) |
| `--places <path>` | | Path to `rbxplace.toml` (default `rbxplace.toml`) |
| `--universe-id <id>` | | Name a universe directly, skipping `rbxplace.toml`. Wins over `--env`. Accepted as `--universe` too |

**About that cookie.** For the handful of endpoints Open Cloud does not publish (`init`, some `meta` fields, `apikey`), `rbx` falls back to the `.ROBLOSECURITY` cookie of a local Roblox Studio install when you set none, and prints `using the Roblox Studio cookie (pass --no-auto-cookie or set RBX_COOKIE= to disable)` on stderr the first time it does. A session cookie is a full-account credential, strictly more powerful than any scoped key, so it gets its own page: [docs/cookie.md](./docs/cookie.md) covers which commands use it, which never will, how it is resolved, and why it is never written to disk.

### Without a config file

`--universe-id` exists so a command works in a directory that has no
`rbxplace.toml` at all: a one-off against a universe you never configured,
somebody else's game you are helping with, or a script on a server that has no
checkout:

```sh
export RBX_API_KEY="..."
rbx memorystore --map Cache set rotation --value '{"map":"desert"}' --ttl 300s --apply --universe-id 66778899001
```

Every live-operations command takes it except `servers`, which needs a place id as
well and has no flag to give one directly: that one still requires an env.
Checked command by command rather than assumed.

The `rbx` half is different by design: it reconciles state you committed, so
`shop`, `meta` and `config` read their own TOML for *what* to apply even when
`--universe-id` says *where*.

## Shell completions

```sh
rbx completions powershell -o $PROFILE       # PowerShell (single-file install)
rbx completions bash       -o ~/.local/share/bash-completion/completions/rbx
rbx completions zsh        -o "${fpath[1]}/_rbx"
rbx completions fish       -o ~/.config/fish/completions/rbx.fish
```

Without `-o`, the completion script is printed to stdout (pipe into a file or your profile yourself).

One binary, so one completion file. The live-operations commands are in it like any other.

`--env` and `--place` complete with the names in the `rbxplace.toml` of the directory you are in: the script calls `rbx env list --names` and `rbx env list --place-names` when you press TAB, so adding an env needs no regeneration. Outside a project, or with a file that does not parse, both complete to nothing and print nothing. `--no-dynamic` leaves that hook out. See [docs/env.md](docs/env.md#shell-completions-for---env-and---place) for the per-shell install steps.

## What this tool does not do

Deliberate omissions, with the reasoning, so "why not X?" is a link rather than
a debate.

**Building the `.rbxl` from source.** That is [rojo](https://rojo.space)'s job,
and it does it well. Composition through Rokit is the design: rojo builds the
place file, `rbx place upload` ships it. A tool that did both would be worse at
each, and would have to keep pace with rojo's project format forever.

**An asset upload pipeline.** That is
[asphalt](https://github.com/jackTabsCode/asphalt)'s job: hashing, uploading
and generating references for images, sounds and models. `rbx download` fetches
an asset by id, which is the opposite direction and a much smaller problem.

**Switching signed-in Studio accounts.** That is
[rbx-switch](https://github.com/rbx-dev-tools/rbx-switch), a separate tool by the
same author. It touches locally signed-in Studio accounts in the Windows
registry and Credential Manager, which is a desktop utility rather than a step
in reconciling a repository with Roblox. Nothing here depends on it.

**`destroy`.** The platform cannot really delete a universe, a badge or a game
pass; it can archive some of them and refuses on others. A `destroy` that
silently means "archive" is a command whose name lies at the worst possible
moment, and the risk of getting that wrong is not worth what it buys today. Ask
again if Roblox ships real deletion.

**Running Luau in the cloud.** Plausible future (post-deploy smoke tests
against a live server are the obvious use) but out of scope now. It needs an
execution model and a security story that nothing here currently has.

## Stability and support

### Stability policy

At 0.x, this is what will and will not move under you. The paragraph anyone
putting `rbx` in CI is betting on:

- **Config file formats are stable.** `rbxplace.toml`, `rbxshop.toml`,
  `rbxmeta.toml`, `rbxapikey.toml` and friends keep working. Keys get added;
  existing keys do not change meaning or disappear before 1.0. An unrecognised
  key is a warning on stderr, never a failure, so a file written for a newer
  release still loads on an older binary.
- **Exit codes are stable.** 0 is success, non-zero is failure, and a command
  changes which is which only in a release whose changelog says so. Scripts
  branching on `$?` keep working. The one exception so far: the `check`
  commands, which reported drift on the screen and exited 0, now exit `2`: a
  pipeline that passed on a drifting repository starts failing.
- **Lockfile formats may migrate.** `rbxshop.lock.toml`, `rbxmeta.lock.toml`
  and `rbxconfig.lock.toml` are tool-owned state, not user-authored input, and
  are allowed to change shape between releases. Delete one and the next `sync`
  rebuilds it; nothing in a lockfile is information you typed.
- **Command flags may change, with a changelog entry.** Renamed or removed
  flags are noted in [CHANGELOG.md](./CHANGELOG.md) for the release that does
  it, and get an alias where an alias is possible (as `--universe` did for
  `--universe-id`).

**MSRV: Rust 1.88.** Declared in the workspace manifest, so `cargo` refuses
with a clear message rather than failing three dependencies deep. It is
dependency-driven (our own sources need 1.82) and moves when a dependency
moves it. Most users never build from source; the precompiled binaries in each
[release](https://github.com/rbx-forge/rbx-cli/releases) have no toolchain
requirement at all.

### Support tiers

Both tiers ship in the same binary. This is a statement about what blocks a
release, not about where code lives: the two-binary split was tried and
reversed (see the live-operations section above).

**Core.** A bug here blocks a release: `place`, `shop`, `meta`, `config`,
`apikey`, `env`, `init`, and every live-ops command (`data`, `ban`, `restart`,
`servers`, `analytics`, `memorystore`, `message`). These are the commands
people put in CI and point at production.

**Tier 2.** A bug here never blocks a release: `open`, `download`, `ads`. Local
conveniences and one-off utilities. It gets fixed, just not on the critical
path, and this is also what makes a platform gap acceptable rather than
embarrassing: `open` dispatches a `roblox-studio:` URI and so is only as
portable as Studio itself, and that is a known state rather than an open wound.

### Maintenance expectations

One maintainer, maintained on my schedule. **Issues are triaged, not
promised**: a bug in a Core command gets attention first, everything else gets
looked at when it gets looked at. Reproducible bug reports and PRs with tests
are the fastest path to a fix. Nothing here is a support contract, and it is
better to say so before anyone finds out the hard way.

Security reports are the exception and go through
[SECURITY.md](./SECURITY.md), not the tracker.

Written with AI assistance. Every line is reviewed, tested and shipped by the
maintainer, who is responsible for it; the tests and the CI gates in this
repository are the actual answer to "was this checked", and they are the same
answer either way.

## Repository layout

```
crates/
├─ rbx/              # the binary: clap parsing and dispatch only
├─ rbx-core/         # shared: rbxplace.toml loader, GlobalFlags, HTTP client + retry, asset download
│
├─ rbx-init/         # group/universe/place bootstrap
├─ rbx-import/       # adopt an existing universe: writes every config and lockfile from it
├─ rbx-env/          # read rbxplace.toml (list, get one id, gen module)
├─ rbx-apikey/       # API key management
├─ rbx-doctor/       # diagnose credentials, key validity, scope coverage
├─ rbx-check/        # one pass over every configured tool's check: `rbx check` and `rbx status`
├─ rbx-place/        # place file upload/download/promote
├─ rbx-meta/         # universe and place metadata
├─ rbx-config/       # game configuration flags
├─ rbx-secret/       # universe secrets store, sealed client-side
├─ rbx-shop/         # game passes, badges, developer products
├─ rbx-open/         # Studio launcher
├─ rbx-download/     # asset downloader
│
├─ rbx-servers/      # live: server history
├─ rbx-analytics/    # live: analytics queries
├─ rbx-ban/          # live: player restrictions
├─ rbx-restart/      # live: rolling server restarts
├─ rbx-data/         # live: data store entries
├─ rbx-memorystore/  # live: memory store sorted maps
├─ rbx-message/      # live: MessagingService
├─ rbx-ads/          # live: ad campaigns
├─ rbx-probe/        # live: raw Open Cloud request
│
├─ rbx-spec-drift/   # test-only: asserts our endpoints still exist in spec/openapi.json
└─ rbx-schema/       # dev-only: writes schemas/*.json from the serde config models
docs/                # per-subcommand detailed docs (one file per tool); also the mdBook
                     # source: SUMMARY.md is the reading order, index.md the landing page
schemas/             # generated JSON Schemas for the config files (see "Editor support")
spec/                # vendored Roblox OpenAPI document + provenance (see "API drift check")
```

`rbx-core` is the shared layer every domain crate depends on: it parses `rbxplace.toml`, owns the cross-subcommand flag set (`GlobalFlags`), builds the Open Cloud HTTP client with retry on 429/5xx and transient network errors, and exposes the asset-download helper. Every domain crate exposes a `<Tool>Cli` clap `Args` group plus an `async fn run(cli, &GlobalFlags) -> Result<()>` that the top-level binary dispatches into.

## Contributing

[CONTRIBUTING.md](./CONTRIBUTING.md) is the entry point: setup, the crate
layout convention, the DCO sign-off every commit needs, and what makes a PR
mergeable. [ARCHITECTURE.md](./ARCHITECTURE.md) is the map of the workspace.
Vulnerabilities go through [SECURITY.md](./SECURITY.md), never the public
tracker. What follows here is the automation around all of that.

```sh
# One-time: install git hooks (runs cargo fmt + clippy on commit)
lefthook install
```

Commit messages ideally follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `chore:`, `feat!:` for breaking) (it keeps the CHANGELOG readable) but this isn't enforced.

### What runs automatically

Seven things, and nothing else. There is no release-plz and no auto-merge:
every version bump is a deliberate act.

| What | Trigger | Does |
| --- | --- | --- |
| `lefthook` (local) | `git commit` | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo doc` with warnings denied |
| `.github/workflows/ci.yml` | push / PR on `main` | eight jobs: Test, Rustfmt, Clippy, Doc, Schemas up to date, Shell completions, plus a Windows leg and a cross-target job (macOS check + musl build) |
| `.github/workflows/supply-chain.yml` | PR touching the dependency graph, **and weekly** | `cargo deny check` for advisories, licenses, bans and sources |
| `.github/workflows/release.yml` | pushing a `v*` tag | builds four targets (Linux x86-64 gnu and musl, Windows x86-64, macOS aarch64), zips them, generates `SHA256SUMS`, creates the GitHub Release |
| `.github/workflows/update-openapi.yml` | daily, 06:17 UTC | refreshes the vendored Roblox OpenAPI document and opens a PR if it changed |
| `.github/workflows/docs.yml` | push on `main`, PRs touching `docs/` | builds the mdBook site from `docs/`; deploys to GitHub Pages on `main` only |
| Dependabot | monthly, 06:00 Paris | one grouped PR bumping GitHub Actions versions |

The heavy jobs run on Linux, where runners bill at 1x; macOS is a
`cargo-zigbuild check` of the target the release ships rather than a rented
mac, and Windows runs only the crates whose code is platform-dependent.
Everything else is checked once. Full cross-platform *compilation* is proven at
tag time by `release.yml`, which builds all four targets before publishing
anything.

musl is the one cross target that is *built* rather than checked, and the
difference is the point: what breaks a static artifact is linking, which
`cargo check` never reaches. The job also asserts the result is actually static,
so a dependency quietly reintroducing a shared libc fails rather than producing
a musl binary with the same portability problem as the gnu one.

The weekly `supply-chain.yml` run is not redundant with its PR trigger. An
advisory is published against a version already in `Cargo.lock`: nothing in this
repository moves, and a check that only ran on push would stay green until
somebody happened to touch a manifest.

Dependabot handles **GitHub Actions only**. Cargo version updates are off
deliberately. Dependabot Security Alerts still open a PR when an advisory hits
`Cargo.lock`; `cargo-deny` is the second half of that story, because it also
answers the three questions an advisory feed does not, whether a license
reached a statically linked artifact it should not have, whether a crate came
from an unexpected registry, and how much duplication the graph has grown.

**That is not the same as nothing being wrong.** Alerts cover what GitHub's
database knows and rates as worth interrupting for; `cargo audit` reads the
RustSec advisory database and reports more, including unsound-but-unrated
crates. Nothing in CI runs it today, which is why an external audit found
advisories the Security tab was quiet about. Removing default features from
`image` and `rbx_cookie` took the dependency count from 424 to 354 and removed
two of those advisories by removing the crates that carried them.

### Open Cloud API drift check

When Roblox renames a field or moves an endpoint, we used to find out from a
user hitting `Failed to parse response` at runtime. CI now tells us first.

`spec/openapi.json` is a byte-for-byte snapshot of the OpenAPI document Roblox
publishes in [`Roblox/creator-docs`][creator-docs] (`content/en-us/reference/cloud/openapi.json`).
`spec/source.json` records the exact upstream commit it came from, and
`spec/NOTICE.md` carries the attribution: the document is CC BY 4.0, unlike
the rest of this repository.

The test in `crates/rbx-spec-drift/tests/openapi_drift.rs` reads every Roblox
URL the workspace builds and asserts each path still exists in that snapshot.
Matching is structural and host-aware: literal segments must match exactly,
`{...}` segments are wildcards (our Rust variable names never have to agree
with Roblox's parameter names), and a path only counts as present if the
document describes it on the same host we call it on. It runs as part of
`cargo test --workspace`, or on its own:

```sh
cargo test -p rbx-spec-drift --test openapi_drift
```

When it fails it names the missing path, every `file:line` that calls it, and
the closest paths Roblox still documents, which usually identifies the rename
outright.

**Its limits are documented in that file's module comment, and worth reading
before trusting it.** In short: it checks that endpoint *paths* exist. It
cannot see a renamed response field, so it narrows the blast radius rather than
eliminating it. Endpoints Roblox has never documented (the `experience-releases`
v1beta1 API, legacy `universes/v1` creation calls, API-key introspection) are
listed in `KNOWN_UNDOCUMENTED` with a reason each: that list is where we admit
we have no early warning for something.

**Refreshing the snapshot** is the `update-openapi` workflow's job. It runs
daily, and can be triggered by hand from the Actions tab. It re-fetches the
document, rewrites `spec/source.json`, runs the drift check, and opens a PR
with the result quoted in the body. It never pushes to `main`: whether a Roblox
change breaks us is a judgement call, so a human always reviews it. A refresh
that changes nothing opens no PR.

Do not hand-edit `spec/openapi.json` or reformat it: diffs between refreshes
have to show real Roblox changes and nothing else.

[creator-docs]: https://github.com/Roblox/creator-docs

### Cutting a release

1. Bump `[workspace.package].version` in `Cargo.toml`, then `cargo check` to refresh `Cargo.lock`
2. Move `## [Unreleased]` content to `## [X.Y.Z] - YYYY-MM-DD` in `CHANGELOG.md` (no `v` in the heading)

   Then check that every `docs/` page describing something in that section tags
   it `**(X.Y.Z+)**` with the version you just cut. `docs/` describes `main`,
   and `/blob/main/docs/<page>.md` is the URL links and search results hand
   people, so an untagged feature reads as available to everyone landing there
   from outside, while `CHANGELOG.md` says `[Unreleased]`. The two sources then
   contradict each other depending on which one is read first. That cost a
   consuming repo a wrong diagnosis and a wrong design in 2026-08.
3. Commit, then tag and push: `git tag -a vX.Y.Z -m "vX.Y.Z" && git push origin main --follow-tags`

   The `-a` is not optional. `--follow-tags` pushes **annotated** tags only, so a
   plain `git tag vX.Y.Z` is created locally, silently skipped by the push, and
   `release.yml` never fires: the commit lands on `main` and nothing else
   happens. Every release from v0.2.0 on is annotated.
4. `release.yml` picks up the tag and publishes the binaries

If you delete a release, delete its tag too (`git push origin :vX.Y.Z`).
Re-pushing an existing tag re-triggers `release.yml` against stale code: that
is how the cancelled v0.5.3 run of 2026-06-15 happened.

## Prior art and thanks

- **[Asphalt](https://github.com/jackTabsCode/asphalt)** (MIT): the
  lockfile-and-codegen pattern `rbx shop` uses for assets, and the alpha-bleed
  implementation, which is **adapted code**, not just inspiration. Notice in
  [THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md).
- **[Tarmac](https://github.com/Roblox/tarmac)** (MIT), where that alpha bleed
  originates; Asphalt adapted it first. Same notice file.
- **[ROpen](https://github.com/Barocena/ROpen)** by Barocena (MPL-2.0): the
  Luau Studio launcher `rbx open` was written against, and where the
  `roblox-studio:` URI dispatch was learned from, having contributed to it. The
  Rust command here is a reimplementation, not a port. Its 1.3.2 (August 2026)
  is also what pointed out that `xdg-open` cannot reach a Studio living on the
  Windows side of WSL: a gap this command had too.
- **[edit-roblox-place](https://github.com/rojo-rbx/edit-roblox-place)** (MIT):
  the same idea, six and a half years earlier: a Rust CLI sending
  `roblox-studio:1+task:EditPlace+placeId:<id>` to the desktop's URI handler,
  from rojo-rbx in August 2019. Found long after `rbx open` was written, and
  named here because a prior-art section listing only what its author happened
  to meet first is not one. `rbx open` sends the same URI; what it adds is that
  the place is named rather than numbered, resolved through `rbxplace.toml`.

  No source code from either is reused.
- **[Mantle](https://github.com/blake-mealey/mantle)**: the declarative-IaC
  precedent for Roblox, and the origin of the `rbx_cookie` crate this tool
  depends on.
- **[rbxcloud](https://github.com/Sleitnick/rbxcloud)**: the "a CLI for Open
  Cloud" precedent.

## License

[MPL-2.0](./LICENSE). Source files modified from this project must be kept under MPL-2.0; new files added by downstream users may be licensed independently.

Some files are adapted from MIT-licensed projects and carry their own notices:
see [THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md).

`rbx-cli` is a community tool. It is not affiliated with, endorsed by, or
sponsored by Roblox Corporation. "Roblox" is a trademark of Roblox Corporation,
used here only to say what this tool talks to.
