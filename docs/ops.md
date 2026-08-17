# Live operations

Acting on a running Roblox experience: what its servers are doing, how the ones that stopped ended, what its analytics say, and who is allowed in.

These are subcommands of `rbx`, listed last in `rbx --help` and prefixed `Live:` in their descriptions. They share the `rbxplace.toml`, the `--env` model and the HTTP layer with everything else.

| Subcommand | What it does | Docs |
| --- | --- | --- |
| `servers` | Live and terminated servers, and the logs of one that crashed. | [ops/servers.md](./ops/servers.md) |
| `analytics` | Your own metrics: players, retention, ARPPU. CSV for charting elsewhere. | [ops/analytics.md](./ops/analytics.md) |
| `ban` | Inspect and change player restrictions. | [ops/ban.md](./ops/ban.md) |
| `restart` | Forecast and launch a rolling server restart. | [ops/restart.md](./ops/restart.md) |
| `data` | Read, overwrite, copy and recover a data store entry; `data ordered` for leaderboards. | [ops/data.md](./ops/data.md) |
| `memorystore` | Write cache values servers read through `MemoryStoreService`. | [ops/memorystore.md](./ops/memorystore.md) |
| `publish` | Push a MessagingService message to every running server. | [ops/message.md](./ops/message.md) |
| `ads` | Launch and steer ad campaigns. Spends money, reads no results. | [ops/ads.md](./ops/ads.md) |
| `probe` | Raw authenticated request to any Open Cloud path. Hidden from `--help`. | [ops/probe.md](./ops/probe.md) |

## Why they are marked out

They do a different kind of work from the rest of `rbx`, and mixing them up is how accidents happen.

| | the rest of `rbx` | live operations |
| --- | --- | --- |
| Acts on | state declared in your repo | state that only exists at runtime |
| Source of truth | a TOML file you commit | the live game |
| Runs in CI | yes, on every push | no |
| A mistake costs | a failed deploy, retried in a minute | player data, irreversibly |

Banning a player has no desired state in a TOML file: it is a consequence of what happened in your game last night. So it is not something a tool reconciling your repo against Roblox should be doing on a push.

**A second binary would make the boundary visible in the command name, and it is not on offer.** Rokit resolves one artifact per repository, so of two published binaries only one could ever be installed through it. Dispatching on the name the binary was invoked by does not rescue it either: Rokit's shim passes the stored binary's own path as `argv[0]`, identically for every alias. Measured 2026-08-13 by replacing the stored binary with a probe.

It would not be the boundary that *holds* in any case. Roblox binds an API key to its scopes and universes when you create it, so a deploy key cannot ban anybody whichever binary calls it.

**Keep read and write in separate keys.** That is the boundary that holds, and it is the one worth arranging your `rbxapikey.toml` around. The same argument splits a key per environment, and `envs` written as a table of named groups does that from one declaration rather than three copied blocks — see [One key per environment](./apikey.md#one-key-per-environment).

## Install

Nothing separate to install. `rbx` carries these commands:

```toml
# rokit.toml
[tools]
rbx = "rbx-forge/rbx-cli@0.1.0"
```

```sh
rokit install
rbx servers list --env prod
```

One binary, one archive per platform, which is what makes it installable at all — see [why they are marked out](#why-they-are-marked-out) for what the old second binary cost.

## Getting a key

These commands authenticate with an Open Cloud API key, exactly like the rest of `rbx`. Declare it with `rbx apikey` rather than clicking through the Creator Hub, so the scopes are written down and reviewable.

```toml
# rbxapikey.toml
[settings]
default_envs = ["prod"]
default_secret_file = ".secrets/{name}.env"
name_prefix = "myproject_"

[keys.viewer]
description = "Read-only key for server history and analytics."
scopes = [
    "universe:read",             # servers, server logs, restart status
    "universe.analytics:read",   # analytics queries
]
```

```sh
rbx apikey create viewer
export RBX_API_KEY="$(rbx apikey resolve viewer)"
```

Scopes by subcommand:

| Subcommand | Scope | Read or write |
| --- | --- | --- |
| `servers` | `universe:read` | read |
| `analytics` | `universe.analytics:read` | read |
| `ban status` / `list` / `logs` | `universe.user-restriction:read` | read |
| `ban add` / `remove` | `universe.user-restriction:write` | **write** |
| `restart forecast` / `status` | `universe:read` | read |
| `restart launch` | `universe:write` | **write** |
| `data` reads | `universe-datastores.objects:read,list` | read |
| `data` writes | `universe-datastores.objects:create,update` + `universe-datastores.control:create` | **write** |
| `data revisions` / `restore` | `universe-datastores.versions:list,read` | read |
| `data ordered` reads | `universe.ordered-data-store.scope.entry:read` | read |
| `data ordered` writes | `universe.ordered-data-store.scope.entry:write` | **write** |
| `memorystore get` / `list` | `memory-store.sorted-map:read` | read |
| `memorystore set` / `delete` | `memory-store.sorted-map:write` | **write** |
| `publish` | `universe-messaging-service:publish` | **write** |
| `probe` | whatever the path you probe needs | depends |

**Keep read and write in separate keys.** The read key is the one that ends up in a shell history during a debugging session, and it should be the one that cannot ban anybody.

## Safety model

Three rules, all structural rather than conventions to remember.

**Writes are dry-run by default.** Any operation that changes something describes what it would do and stops. `--apply` performs it, and prompts. The safe outcome is what happens when you forget a flag.

**`--env all` is refused.** For `rbx shop sync` it makes sense to fan out over every environment. For anything touching live players it does not: each env is a different experience, and a command that quietly acted on production because it matched a glob is a command nobody can trust.

**Scopes are the real boundary.** Roblox binds a key to its scopes and universes at creation. A read-only key is read-only no matter what calls it.

## The Studio cookie

The `.ROBLOSECURITY` cookie is the one credential in this tool that the section above does not cover, and **no live-ops command accepts it**. `servers`, `analytics`, `ban`, `restart`, `data`, `memorystore`, `publish`, `ads` and `probe` take an API key and nothing else, with no cookie path to fall back to. That is on purpose: these are the operations that act on players, so they stay behind scoped keys where the scope list is the audit trail.

A session cookie is a complete account identity. It is not scoped to a universe, not scoped to an operation, and not revocable per tool, so it is strictly more powerful than any key `rbx` will ever ask you for. A handful of commands outside this page do need one, because Open Cloud publishes no equivalent endpoint.

**The trust model lives in one place: [docs/cookie.md](./cookie.md).** What the cookie is used for and never used for, the resolution order behind `--cookie`, `RBX_COOKIE`, `RBXAPIKEY_COOKIE` and `--no-auto-cookie`, the stderr notice when it is auto-detected, and why it is never written to disk.

## The development configs, and why they are not in git

Two directories in this repository hold configs for real Roblox universes:
`testenv/` for the throwaway experience the ops subcommands are developed
against, and `prodread/` for read-only access to the live one.

**Their `rbxapikey.toml` and `rbxplace.toml` are gitignored.** What is committed
is a `.example` next to each:

```
testenv/rbxapikey.example.toml    prodread/rbxapikey.example.toml
testenv/rbxplace.example.toml     prodread/rbxplace.example.toml
```

To work against real Open Cloud, copy each one and fill it in:

```sh
cd testenv
cp rbxapikey.example.toml rbxapikey.toml
cp rbxplace.example.toml rbxplace.toml
```

The values to fill in are in `.local/real-ids.toml`, which is gitignored too:
the public IP for the key allowlists, and the live universe and place ids.

**Why not just commit them with placeholders.** They were, and it worked, but
only by discipline. These files are useless without real values, so the first
thing anyone does is paste their own IP into `default_allowed_cidrs` to get a
call to stop returning 401 — and now a tracked file holds personal data, one
`git commit -a` away from being published. A public IP in an allowlist is
personal data *and* an operational disclosure: it announces which address is
authorised on the Open Cloud keys. Untracked paths remove the accident instead
of asking people not to have it.

Edit the `.example` only for changes worth sharing: a new key, a scope
decision, a comment. Those comments are the valuable part — they record why
each scope was chosen, and the rule that **nothing in `prodread/` may ever hold
a write scope**.

A local overlay file the tools read directly (`rbxapikey.local.toml` layered
over the committed one) would remove the copy step rather than only making it
safe. That is a change in how the tools load config, so it is not done here.

## Testing and fixtures

Every client here is tested against **recorded production responses**, in `crates/rbx-*/tests/fixtures/`. Everything is byte for byte what Roblox sent, except four things replaced with synthetic values: `playerIds`, `jobId`, pagination tokens and analytics operation paths.

The last two are not obvious and were missed on the first pass. A pagination token is an opaque base64 blob, but decoding one shows Roblox packs real data inside it, including a `LastGameId` that is a live server's job id. Replacing only the `jobId` field left a copy of it in the cursor. The tests only ever ask whether a token exists, never what is in it, so an opaque placeholder costs nothing.

**Place versions are kept as recorded, and that is a decision rather than an oversight.** A version number is a publish counter, so on its own it says how often some experience shipped — but the universe and place ids in these files are placeholders, so it is attached to nothing. Replacing them would cost the property the recordings exist for, since the ordering and the numeric-versus-string sorting of real version numbers is exactly the kind of detail a hand-written fixture gets wrong. Documentation is the opposite case: every figure on these pages is invented, because a page is read far more often than a fixture and nothing there needs to be real to make its point.

Recordings rather than hand-written JSON, because the specification is wrong or silent about several fields, and a hand-written fixture would encode the specification and agree with the bug. Caught this way:

- `nextPageToken` is `""` on `cloud/v2` and `null` on `server-management`. Reading `""` as a token requests the same page forever.
- `uptime` is a .NET `TimeSpan`, `[d.]hh:mm:ss[.fffffff]`, not an ISO 8601 duration. All three forms occur.
- `frameRate` is `null` on a new server and `0` on a stopped one.
- `dataPoints` needs camelCase renaming; without it every analytics query returns an empty series and reports "no data".

Re-record when Roblox changes something and a test starts failing:

```sh
python scripts/capture_fixtures.py
```

It only ever issues GET requests, and needs the read-only keys described above.
