# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0]

### Added

- **`rbx env rm <name>`** — take an env out of every file that mentions it:
  the block in `rbxplace.toml`, every overlay and lockfile section keyed by it,
  and the generated per-env module. Not called `destroy`, because Roblox does
  not let a tool keep that promise: a game pass cannot be deleted, only taken
  off sale.
- **`rbx data ordered`** — the leaderboard resource, with `list`, `get`, `set`,
  `increment` and `delete`. Ordering and `--min` / `--max` filtering happen on
  Roblox rather than after the fact.
- **Avatar rules, third-party permissions, paid access and genre** in
  `rbxmeta.toml`: `game.avatar` (type, animation, collision, joint
  positioning, min and max scales, asset overrides),
  `game.permissions`, `game.paid_access` and `game.genre`. Plus
  `game.engine_avatar_settings`, an opaque passthrough for the modern avatar
  document, and `schemas/rbxavatar.schema.json` to check that document in an
  editor.
- **`rbx shop` refuses to create a resource whose name Roblox already has.**
  The guard against a lockfile that was never committed: without it a second
  `sync` mints a duplicate game pass, which cannot then be deleted.
- **Disabled and not-for-sale resources are annotated** in the generated shop
  module rather than emitted as though they were live.
- **A static `x86_64-unknown-linux-musl` binary** in every release, for the
  distributions the glibc build refuses to start on. The release fails rather
  than publishes if that artifact comes out dynamically linked.
- **A supply-chain workflow** running `cargo-deny` (advisories, bans, licenses,
  sources) on pull requests and weekly, so an advisory published against a
  version already in `Cargo.lock` is not waited on until somebody edits a
  manifest.
- **An MSRV job pinned to 1.88**, which is what makes `rust-version` in the
  manifest a checked claim rather than a comment.

### Changed

- **TLS is now rustls.** `reqwest`'s default `native-tls` links `openssl-sys`
  dynamically on Linux, so the published binary needed the exact `libssl.so.3`
  the runner had, on top of its glibc. The system trust store is still read, so
  a corporate proxy's CA keeps working.
- **Every `cargo` invocation in CI and release passes `--locked`**, so the
  dependency graph the tests exercise and `cargo-deny` audits is the graph the
  released binary is built from.
- **`rbx open --universe-id` resolves the place** instead of being accepted and
  ignored.

### Fixed

- **`rbxmeta.toml` swallowed unknown keys in silence.** A config full of new
  keys got "everything is in sync". Unrecognised keys are now reported with
  their full path. Two internally-tagged tables remain a documented blind spot,
  pinned by a test.
- **The universe config read was broken by an asymmetry in Roblox's own API**:
  the v1 read answers with enum *names* (`"MorphToR15"`) while the v2 write
  takes integers. Both spellings are accepted now.
- **`meta pull` could overwrite the lockfile with confident nulls** for fields
  no read returns. What a read cannot confirm is now carried over from the
  previous lockfile by construction rather than by remembering to.
- **`servers list` and the memorystore commands changed page size mid-walk**,
  which Roblox's own page-token rule forbids, and ignored `--limit`:
  `--limit 150` returned 200 rows.
- **`meta sync` would write the avatar settings twice, in two shapes.**
  `engineAvatarSettings` restates the legacy avatar fields rather than
  extending them, and neither side can be read back, so the contradiction only
  surfaced when Studio next opened the place. Sending both is now refused.
- **`env rm` skipped three of the files it promised to clear**, including
  `rbxapikey.toml`, where a leftover env name makes the next `rbx apikey` run
  fail outright.
- **Windows builds overflowed the 1 MB main-thread stack**, so a debug binary
  could not print `--version`.
- **Scopes named in the documentation were invented.** `universe.image:read` is
  not a scope type and `legacy-universe.badge:read` has no `read` operation, so
  a key declared from the docs was refused. A test now checks every scope the
  prose names against the catalog.
- **A `?` in a resource name crashed `shop pull` on Windows.** The filename
  sanitiser moved to `rbx_core::fs_name` where every writer reaches it.
- **`apikey create` names the account it is about to mint a key on**, which is
  not always the one the reader pictured.

## [0.1.0]

First release.

`rbx` is one binary covering two kinds of work against Roblox Open Cloud, which
share an environment model and nothing else. One `rbxplace.toml` maps env names
to universes and places, and every command resolves `--env` through it.

### Declarative

State you write into a TOML file and commit, reconciled against Roblox.
Diffable, reviewable, safe on every push.

- **`init`** — create a group, a universe, places, and record their ids.
- **`import`** — adopt a universe that already exists: every config and
  lockfile written from what is live, in one pass, so that `check` is green
  immediately after with nothing in between.
- **`env`** — read `rbxplace.toml`, print one id for a script, generate a Luau,
  Lua, JSON or TypeScript module so game code branches on env instead of
  hardcoding ids.
- **`apikey`** — declare Open Cloud keys and scopes in `rbxapikey.toml`, create
  and rotate them, and see every key the account holds rather than only the
  ones this project made. `readonly = true` refuses a write scope at load.
- **`doctor`** — prove the loaded key works with one real read.
- **`check`** / **`status`** — every configured tool's check in one pass, one
  exit code for CI; the same engine rendered for a person.
- **`place`** — upload, download, promote between envs, roll back.
- **`meta`** — universe and place metadata, including the fields Open Cloud
  does not expose.
- **`config`** — the live in-experience config, with revisions and rollback.
- **`shop`** — game passes, badges and developer products, with typed Luau
  codegen and a `--check` that proves the committed module was not hand-edited.

### Operational

State that only exists while the game is running, which no TOML file can
describe. Dry run by default, `--apply` to write, `--env all` refused.

- **`servers`** — servers up now, how the stopped ones ended, and what a
  crashed one logged.
- **`analytics`** — players, retention, revenue per payer; CSV for charting
  elsewhere.
- **`ban`** — inspect and change player restrictions.
- **`restart`** — forecast how many players a rolling restart would disconnect,
  then launch it.
- **`data`** — read, overwrite, copy and recover one data store entry, with a
  local backup written before every write.
- **`memorystore`** — cache values servers read through `MemoryStoreService`.
- **`message`** — push a MessagingService message to every running server.
- **`ads`** — launch and steer ad campaigns.
- **`probe`** — a raw authenticated request to any Open Cloud path.

### Local

- **`open`** — launch Studio at a place, by name or by id.
- **`download`** — fetch an asset by id.
- **`completions`** — shell completions that read your `rbxplace.toml` at TAB
  time, so a new env completes without regenerating anything.

### Credentials

Open Cloud API keys everywhere Roblox offers the endpoint. The
`.ROBLOSECURITY` cookie only where it does not, never as a fallback for a
rejected key, and never on a command that acts on live players. Studio
auto-detection is opt-in, announces itself, and is refused outright where there
is nobody to ask. Cookie-authenticated writes name the account they will act
as. The cookie is never written to disk. See `docs/cookie.md`.

### Machine-readable output

`--json` on the reads writes one document to stdout and nothing else, with
documented field names and a `schema_version`. Ids are strings, prices are
numbers, and an optional field is absent rather than null.

[Unreleased]: https://github.com/rbx-forge/rbx-cli/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/rbx-forge/rbx-cli/releases/tag/v0.2.0
[0.1.0]: https://github.com/rbx-forge/rbx-cli/releases/tag/v0.1.0
