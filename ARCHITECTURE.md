# Architecture

This document explains how the `rbx` workspace is organized so a new
contributor can find their way around quickly. For *what* each command does,
see the root `README.md` and each crate's `README.md`.

## Workspace layout

The repo is a Cargo workspace: **one binary crate + one crate per domain +
one shared core**.

```
crates/
  rbx          # the binary: clap parsing and dispatch only (src/main.rs)
  rbx-core     # shared building blocks (no domain logic)

  # Reconciling what a repo declares
  rbx-init       # bootstrap groups / universes / places   -> rbx init
  rbx-import     # adopt an existing universe wholesale    -> rbx import
  rbx-env        # read rbxplace.toml (list/get/gen-module)-> rbx env
  rbx-apikey     # Open Cloud API key lifecycle            -> rbx apikey
  rbx-place      # place files: upload/download/promote    -> rbx place
  rbx-meta       # universe & place metadata               -> rbx meta
  rbx-config     # in-experience live config               -> rbx config
  rbx-shop       # passes / badges / developer products    -> rbx shop
  rbx-check      # every configured tool's check, one pass -> rbx check, rbx status
  rbx-doctor     # credentials, key validity, scope cover  -> rbx doctor
  rbx-open       # launch Studio at a place                -> rbx open
  rbx-download   # download assets by id                   -> rbx download

  # Live operations: acting on what only exists while the game runs
  rbx-servers     # live and terminated servers, logs      -> rbx servers
  rbx-analytics   # the experience's own metrics           -> rbx analytics
  rbx-ban         # player restrictions                    -> rbx ban
  rbx-restart     # rolling server restarts                -> rbx restart
  rbx-data        # data store entries                     -> rbx data
  rbx-memorystore # memory store sorted map items          -> rbx memorystore
  rbx-message     # MessagingService messages              -> rbx message
  rbx-ads         # ad campaigns                           -> rbx ads
  rbx-probe       # raw authenticated request (hidden)     -> rbx probe

  rbx-spec-drift  # test-only: alarms when Roblox's OpenAPI moves
```

`crates/rbx/src/main.rs` is deliberately thin: it defines the top-level `Tool`
enum, flattens `rbx_core::GlobalFlags`, and forwards each subcommand to its
crate's `run(cli, &global)`. No domain logic lives in the binary. Clap renders
subcommands in declaration order, so that enum *is* the `--help` output, which
is why it reads as a user journey and ends with the live-operations commands.

**There used to be two binaries.** `rbx-ops` carried the live-operations
commands so that the tool CI installs could not run them. That drew the boundary
in the command name, but not where it holds: Roblox binds an API key to its
scopes at creation, so a deploy key cannot ban anybody whichever binary calls
it. What it cost was installability — Rokit resolves one artifact per
repository, so only one of two published binaries could ever be installed
through it. The commands merged into `rbx` in 0.10.0, a forwarding shim carried
the old name for two minor releases, and 0.12.0 deleted it. The signal now lives
in the `Live:` prefix and in the ordering; the post-mortem of the split is in
[TODO.md](./TODO.md). See [docs/ops.md](./docs/ops.md).

## rbx-core — the shared layer

Everything reused across crates lives here, so domain crates never duplicate
infrastructure:

- **`GlobalFlags`** (`env.rs`): the global CLI flags (`--api-key`, `--cookie`,
  `--env`, `--place`, `--places`) plus `resolve_cookie()` (explicit flag →
  `RBX_COOKIE` → Studio auto-detection).
- **`api`**: the shared HTTP layer.
  - `build_client()` / `build_client_with_user_agent()` — the one place a
    `reqwest::Client` is constructed (gzip + request timeout).
  - `execute_with_retry` / `execute_json` / `RetryPolicy` — retry on 429/5xx
    and transient network errors, with `retry-after` support, then JSON-parse
    with the raw body kept in the error message.
  - `download_asset` — Open Cloud asset download.
- **`places`** / **`owner`** / **`env`**: `rbxplace.toml` parsing, the shared
  `[owner]` block, and env-target resolution.

## Concurrency: sequential on purpose

The workspace is async and has **zero** `tokio::spawn`. That is the design, not
an omission. `--env all`, multi-place uploads and multi-asset downloads all run
one after another.

Two reasons, and they are worth more than the seconds parallelism would save:

- **Deterministic output.** A CLI's progress lines are its UI. Interleaved
  output from concurrent uploads is unreadable, and worse, a failure part-way
  through a fan-out leaves a set of places in a state the log no longer
  explains. Sequential means the last line printed is the last thing that
  happened.
- **Deterministic write order.** Reconciliation writes to Roblox. Ordering them
  makes a failed run resumable by reading the output, and makes two runs of the
  same config produce the same sequence of API calls.

`tokio` is therefore declared with the features that are actually used —
`rt-multi-thread`, `macros`, `time`, `sync` — rather than `full`. It is here as
`reqwest`'s runtime first and the CLI's own runtime second.

Bounded concurrency for the *read-heavy* fan-outs (icon and media downloads)
was considered and declined for now: it would buy a few seconds on the paths
where waiting is cheapest, at the cost of the two properties above on the paths
where they matter most. If it ever lands, it belongs on reads only, and writes
stay ordered.

The corollary: blocking calls (`std::fs`, `dialoguer` prompts) inside `async fn`
are harmless here because nothing else is scheduled to starve. That stops being
true the day a `spawn` appears, so a change that introduces concurrency owns
moving those too.

## Conventions a domain crate follows

Each domain crate is shaped the same way:

- `lib.rs` — the clap `Args`/`Subcommand` for that tool + `run(...)` dispatch.
- `api/` — a thin `RbxClient` newtype wrapping `reqwest`, with one method per
  endpoint. The client is built via `rbx_core::api::build_client*` and most
  calls go through `rbx_core::api::execute_json`.
- `commands/` — one module per subcommand; this is where the logic lives.
- `config.rs` / `lockfile.rs` — the declarative `*.toml` model (serde) and, for
  stateful tools, the lockfile that records what was last synced to Roblox.

### One retry loop, per-crate terminal statuses

**Every crate delegates retries to `rbx_core::api`.** There is one loop, in
`execute_with_retry_policy`, and it decides one thing: whether a failure is
worth another attempt (429, 5xx, and network timeouts/connect errors are;
everything else is not).

What is *not* shared is the meaning of a terminal status. Two crates overload
one to mean something specific, and both map it after the shared helper has
given up rather than during the retrying:

- **`rbx-place`** — `409 Conflict` on the version endpoints means one thing:
  somebody has the place open in Team Create. Left as a bare status it reads
  like a merge conflict and sends people to look at their file instead of at
  Studio. `place_write_error` turns it into that sentence
  (`crates/rbx-place/src/api/mod.rs`).
- **`rbx-config`** — `404` on the config repository means the universe has
  never published a config. That is the starting state, not a failure, so it
  becomes an empty snapshot (`crates/rbx-config/src/api/mod.rs`).

Both used to hold a private retry loop so the raw response could be inspected
before the helper turned it into an error. Once the status became recoverable
*from* the error (`ApiError`, `is_api_status`), the loops were deletable and
the distinctions survived intact.

The rule that falls out: a crate that needs to say something specific about a
status maps the error, it does not fork the retry policy. Retrying is
mechanical and belongs in one place; what a 409 *means* is domain knowledge and
belongs in the domain crate.

## Auth models

- **Open Cloud** (`apis.roblox.com`, `create.roblox.com`): `x-api-key` header,
  from `--api-key` / `RBX_API_KEY`. The sanctioned, stable path.
- **Legacy / web** (`economy`, `develop`, `badges`, `games`, `users`, …):
  `.ROBLOSECURITY` cookie, often with a CSRF-token retry dance. Used where
  Open Cloud has no equivalent. These endpoints are undocumented and can change
  without notice.

The default `rbx download` path uses public, unauthenticated endpoints for
read-only inspection of third-party games.

## Config vs lockfile

Stateful tools (shop, meta, apikey, config) follow a declarative model:

- The user edits a `*.toml` describing the desired state.
- A lockfile records the last-synced remote state (ids, hashes).
- `check`/`diff` compares config ↔ lockfile; `sync` applies config → Roblox and
  updates the lockfile; `pull` brings remote state back into config + lockfile.

### Dispatching over resource kinds

`rbx-shop` manages three kinds of resource (pass, badge, developer product) and
handles them through **one** enum, `config::ResourceKind` — never by matching on
`"pass"` / `"badge"` / `"product"`. A stringly-typed match needs a `_` arm, and
a `_` arm is where a typo goes to be swallowed; every match on `ResourceKind` is
exhaustive, so a fourth kind would be a compile error rather than a silent
no-op.

Each of the three flows that has to behave the same for all kinds is written
once, generic over a small trait that supplies only what genuinely differs:

| Flow | Trait | What the kind supplies |
| --- | --- | --- |
| `diff.rs` | `Diffable` | which maps to read, and which fields count as a change |
| `commands/sync.rs` | `Appliable` | its two API calls, and how the response lands in the lockfile |
| `commands/pull.rs` | `Pullable` | its field lists for base entries and env overlays |

Shared per-kind accessors (`kind.base_icon_mut(...)`, `kind.rename_base(...)`,
`kind.overlay_envs(...)`) live on `ResourceKind` itself in `config.rs`.

## CI & quality gates

`.github/workflows/ci.yml` enforces, on every push: `cargo fmt --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test
--workspace`, `cargo build`, and `cargo doc --document-private-items`. A
`lefthook` pre-commit hook runs fmt + clippy locally. Dead code is **not**
allowed crate-wide; intentionally-unused items carry a narrow, documented
`#[allow(dead_code)]`.

### Snapshot tests

The two crates that emit generated modules for user game code — `rbx-shop`
(`codegen.rs`) and `rbx-env` (`gen-module`) — assert their output with `insta`
snapshots of the **whole** file, not `contains()` on fragments: the emitted
Luau and TypeScript is a contract with code that runs in a live experience, and
a fragment assertion passes while everything around it degrades.

When output changes on purpose, `cargo insta review` accepts the new file:

```sh
cargo install cargo-insta   # once
cargo insta test --review   # or: cargo test, then cargo insta review
```

The `.snap` files are committed and reviewed like any other expected output.
Behavioural tests live next to them and stay: env dispatch, stub semantics and
the `--check` exit codes say *why* the output looks the way it does, which a
snapshot never states.
