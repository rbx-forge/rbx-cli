# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/rbx-forge/rbx-cli/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/rbx-forge/rbx-cli/releases/tag/v0.1.0
