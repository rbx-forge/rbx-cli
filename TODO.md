# TODO — deferred work

Items captured from design discussions but deferred to avoid scope creep
on the current release. Each item lists the target release.

---

## Done 2026-08-13 — a placeholder owner is no longer reused as a real one

**Was.** `sync_envs` writes `(User, 0)` into `[envs.<name>]` when no scope on
the key being built needs creator-targeting, because the schema has nowhere to
say "not asked". The cache test was `universe_id` alone, so a later key that
*did* need the owner found a matching entry and reused the placeholder —
building a creator-targeted scope over `U0`, which is nobody.

**Fix.** `reusable_cache_entry` now treats the placeholder as a miss when the
caller asked for owners, and is a pure function so all four combinations are
tested without a Roblox call.

Chosen over the two options this item used to list. Rejecting `owner_id = 0` at
load would break every lockfile already carrying one — including `testenv`'s.
Always fetching costs a request per env per create even when nothing needs it.
Treating it as a miss costs a request only when one is actually needed, needs no
schema change, and repairs existing lockfiles on the first command that wants a
real owner.

`0` is a safe sentinel because Roblox issues no user or group with that id.
`LockEnv::owner_is_placeholder` exists so that stays a stated fact with one
reader rather than a `== 0` scattered around.

**Exposure was smaller than it looks, and shrank further today.** The scope
target fix moved `memory-store*`, `developer-product` and `game-pass` off
`creator`, so fewer keys trigger owner resolution at all. What still does:
`asset`, `user.*`, `ad.*`, `group`, and the rest of the creator-target family.

---

## Done 2026-08-13 — a duplicate name is a question, not a silent drop

**Was.** `init --from-remote` and `pull` key a newly discovered resource by its
display name. Roblox does not make those unique, so two passes named "VIP"
collapsed to one key and the second was dropped. It warned, but the warning
named the id being *skipped* rather than the one keeping the key, went to
stdout, and left the exit status at zero — so a real resource stopped being
managed and the message did not carry enough to act on.

**Fix.** `collision::resolve_duplicate` asks for a key where there is somebody
to ask, using the same terminal test `rbx-ops ads` uses before offering a
picker. Off a terminal nothing prompts — CI must not stop on a question nobody
will answer — and the warning now names both ids and prints the TOML that binds
the resource permanently.

Auto-suffixing, which this item used to propose, was rejected: `VIP_2` is an
identifier the developer then lives with for as long as the resource exists,
chosen by a tool that has no idea which "VIP" is the premium one. It survives
only as the prompt's *default*, a suggestion rather than a decision.

**The part that would have been a live bug.** In `init` the display name was
also the config key, and `PassConfig.name = None` means "the key is the name".
Filing a second "VIP" under `VIP_2` without recording its real name would have
made the next `sync` rename the live pass to `VIP_2`. `display_name_override`
writes the name whenever key and name diverge.

**Scope was smaller than it looked.** In `pull`, anything already tracked is
keyed by id, so a collision can only involve newly discovered resources —
nothing under management was ever displaced.

---

## Done 2026-08-13 — `rbx-ops publish`

MessagingService, shipped as `rbx-ops publish --topic <t> --message|--json`,
behind `--apply`. Documented in [docs/ops/message.md](docs/ops/message.md).

Un-declined for the reason recorded above: the original objection assumed the
publisher is a game server. Paired with `memorystore`, which is pull-only —
the map holds the value, the message says to go and read it.

**Worth knowing.** The message is a string, not JSON, so a payload is text the
game decodes itself; `--json` does that serialisation so malformed input fails
here rather than in `JSONDecode` on a live server. The cap is 1 KB, which makes
publishing a *reference* — the memory store key — the shape that scales.

**What it cannot report.** Whether anybody heard. Roblox answers 200 once it
has accepted the message, with no count of servers reached, and an experience
with none running answers identically. The command prints that on every
success rather than letting a green tick imply delivery.

**Verified against the live API, 2026-08-13.** A temporary `msgpublish` key was
created in `testenv`, used, and deleted. Dry-run, `--message` and `--json` all
answered 200. Reception is still not observable — the test universe has no
servers running, and Roblox reports no delivery either way — but acceptance was
the half that was untested.

**It found the size limit is wrong.** The crate enforced 1024 bytes, from the
documented "1 KB". Probing the endpoint directly: 1025 is accepted, 2000 is
refused with `The length of published message must be between 1 and 1114.`,
1114 answers 200 and 1115 does not. So the guard was refusing messages Roblox
takes. Corrected to 1114, and the floor added — an empty message is a 400,
which a caller publishing `""` as a bare signal would have discovered in
production.

---

## Done 2026-08-13 — `rbx-ops memorystore`

Shipped as `memorystore sorted-map`-shaped subcommands: `get`, `set`, `delete`,
`list`, writes behind `--apply`. Documented in
[docs/ops/memorystore.md](docs/ops/memorystore.md).

Kept from the backlog entry, because they are the reasons rather than the
result:

**Why it had been declined, and why that was wrong.** The stated ground was
"its scopes are creator-target, so such a key spans every universe you own".
That came from the catalog, where all three `memory-store*` scopes said
`creator` — the fall-through default of `infer_target_type`, not a fact about
the API. Fixed and confirmed by readback: Roblox stores these keys with a
`universeIds` list, so they are universe-target exactly like the datastore
scopes. The second ground stands and was never an objection: these are
ephemeral, which places the command in `rbx-ops` beside `data` and `ban` rather
than in `rbx`.

**Deliberately not built.** Queues, the memory store's other half, because
nothing needs them and inventing verbs for a queue nobody is driving is how the
wrong ones ship. And `flush`, a single irreversible call that empties every map
and queue in an experience at once, which is not part of writing a cache value.

**Two API details found by probing, now handled in the client and recorded in
the docs.** The item id is a query parameter — `POST .../items` with `{"id":
...}` in the body answers `400 INVALID_ARGUMENT "The id field is required."`,
naming the field you just sent. And the write is `PATCH ?allowMissing=true`
rather than `POST`, so create and update are one request and two writers racing
both land.

**Still true of MemoryStore itself.** It is pull-only: nothing wakes a running
server when a value changes, so a write appears on whatever polling interval
the experience already has. Push would be MessagingService
(`universes/{id}:publishMessage`), still declined below as game-code IPC.

**Verified against the live universe**, not only against wiremock: `list` on a
map that never existed, `set` dry-run, `set --apply`, `get`, an upsert over the
existing item, `list --values`, `get` on a missing id, `delete` dry-run,
`delete --apply`, `list` after it, and the missing-`--map` error. The
`memstore` key was recreated from its `testenv` block for the run and deleted
after.

---

## Done 2026-08-13 — `infer_target_type` now defers to the spec

Was: the spec publishes `targetResourceSpecifier` next to each scope name, and
it contradicted the heuristic for `developer-product` and `game-pass` (spec
`universes`, heuristic `none`). Both are used by `rbx shop`, which worked, so
this was latent rather than broken.

`resolve_target_type` now prefers the spec where it speaks and falls back to
the name heuristic where it does not. `""` counts as silence — Roblox writes it
for several scopes and it names no resource — and an unrecognised value falls
back rather than being written through, since a target type `scope_builder`
does not know builds a malformed key request that fails at key creation.

Five catalog entries changed: `developer-product` and `game-pass` `none` →
`universe` (from the spec), and the three `memory-store*` scopes `creator` →
`universe` (see below). The first two narrow a grant that was being built over
`*`.

**Still open, smaller than it was.** The spec states a specifier for four scope
families out of forty-odd, so most of the catalog still rests on the name
heuristic, including all of `user.*` (`creator`) and the datastore scopes
(`universe-datastore`). Nothing contradicts them today. Revisit if Roblox fills
the field in more widely.

**Verified against the live API for memory-store, 2026-08-13.** `testenv`'s
`memstore` key was created with `memory-store.sorted-map:read,write` and read
back with `apikey introspect`. Roblox stored:

```json
"scopes": [
  {
    "name": "memory-store.sorted-map",
    "operations": ["read", "write"],
    "universeIds": ["66778899001"]
  }
]
```

`universeIds`, holding the one universe the key config names. Universe-target,
settled — not by the spec, which is silent, but by what Roblox stored.

And the key works, which is a separate claim from being stored correctly: a
read and a write against the test universe's memory store both answered `200`
(transcript in the memorystore backlog item above).

**`developer-product` and `game-pass` verified the same way, 2026-08-13.** They
were the riskier half of the change: unlike the memory-store scopes they sit on
a path that already works, since `rbx shop` creates keys with them. A temporary
`shopscopes` key was created in `testenv` with `developer-product:read` and
`game-pass:read` — read operations only, because the target is a property of
the scope rather than of its operations, so `:read` answers the question
without being able to change anything. Roblox stored both as:

```json
{ "name": "developer-product", "operations": ["read"],
  "universeIds": ["66778899001"] }
{ "name": "game-pass",         "operations": ["read"],
  "universeIds": ["66778899001"] }
```

and both read endpoints answered 200 with the key:

- `GET /developer-products/v2/universes/{id}/developer-products/creator`
- `GET /game-passes/v1/universes/{id}/game-passes/creator`

The key was deleted in the same session, secret file included, and its absence
confirmed against `apikey list --remote`. All five changed entries are now
confirmed by readback rather than by inference.

**Use a binary built from this tree when testing a catalog change.** The
catalog is embedded with `include_str!`, so an installed `rbx` carries whatever
catalog it was built with — the one on PATH here was 0.4.0 against a 0.8.0
workspace, and would have introspected an old classification back as if it were
a result.

---

## Considered and declined — Open Cloud surface we are not going to cover

**Audited 2026-08-04.** Diffed the whole Open Cloud `openapi.json` (691 paths,
813 operations) against every URL the workspace calls. The tool's own embedded
scope catalog already names several capabilities it does not implement. Listed
here so the audit is not repeated, and so the answer stays "no, on purpose"
rather than "nobody looked".

Declined, because a useful tool is not a catch-all. Two rows have since left
this table, and both left for the same kind of reason.

`memory-store/*` rested on a scope classification that turned out to be a
default rather than a finding. `universes/{id}:publishMessage` was declined as
"server-to-server IPC belonging to game code" — true when the publisher is a
game server, and the case that reopened it has the publisher outside Roblox
entirely, where there is no in-experience way to originate the message.

A reason to decline is only as good as the fact under it, and both of these
were about the shape of the API rather than about who would be calling it.

| Surface | Why not |
| --- | --- |
| `users/{id}/notifications` | A whole domain: needs notification templates configured in the experience and opted-in players. The `rbx-ops` blurb used to promise it; the blurb was the bug, and it has been fixed. |
| `ordered-data-stores` | Leaderboards. Would fit under `data` as a sibling mode (integer values, ordered listing), but only earns its place in a tool whose users keep leaderboards. Revisit on demand. |
| `luau-execution-session-tasks` | Powerful (run Luau against a place or a version, with logs), but adequate tools already exist and duplicating one is not a reason to ship. The tool in use here is [jest-roblox-cli](https://github.com/christopher-buss/jest-roblox-cli), named so the claim is checkable rather than asserted. Reconfirmed 2026-08-15, when an MCP server covering the same endpoint prompted the question again. |
| groups memberships/roles, `inventory-items`, subscriptions, `creator-store-products`, place `instances`, `:generateThumbnail`, `:translateText` | No demand, and each is a new domain rather than the completion of an existing command. |

Taken instead: `data snapshot`, because it completes a command that already
exists and makes a claim in its own documentation true.

### Undecided — universe secrets, and the reason it is not a wrapper

`/cloud/v2/universes/{id}/secrets` (GET, POST, PATCH, DELETE) plus
`/secrets/public-key`, scope `universe.secret:read,write`. The one item from
that audit that is a genuine fit rather than scope creep: `rbx apikey` already
manages secrets, and these are the secrets an *experience* reads at runtime.

**What makes it not a one-afternoon job**, per the spec:

- `GET /secrets/public-key` returns a universe-specific public key, derived
  from a master key. The `id` field is static (`"public-key"`); the key itself
  is in `secret`, and the response carries a `key_id`.
- Creating or updating a secret means encrypting the content client-side with
  **LibSodium sealed box**, base64-encoding it, and sending the `key_id` that
  was used alongside it. There is no plaintext path.
- `GET` never returns content, only metadata. So the tool can never show a
  secret back, which rules out the `rbx apikey resolve` shape entirely and
  means the lockfile can hold no copy worth having.
- Deletion is irreversible, 500 secrets max per universe.

So it needs a crypto dependency, and the tool's usual "write it down so you can
read it back" model does not apply. A pure-Rust sealed box exists (`dryoc`, or
the narrower `sodiumbox`), so this need not pull in C libsodium — worth
confirming against the encryption Roblox actually accepts before committing.

Not declined, not scheduled. Decide it deliberately when there is a reason to,
rather than letting it fall through the gap between the two lists again.

---

## Backlog — publish the config schemas to SchemaStore

**Added 2026-08-14** with the schemas themselves. `schemas/*.json` exists and
is kept fresh by CI, but nothing picks it up automatically: a user has to
write a `taplo.toml` rule or a VS Code association by hand, which is exactly
the friction the schemas were meant to remove. README documents both.

**Fix.** A pull request to
[SchemaStore/schemastore](https://github.com/SchemaStore/schemastore) adding
one catalog entry per file, matching on the file names:

| Schema | `fileMatch` |
| --- | --- |
| `rbxplace.schema.json` | `rbxplace.toml`, `rbxplace.example.toml` |
| `rbxapikey.schema.json` | `rbxapikey.toml`, `rbxapikey.example.toml` |
| `rbxmeta.schema.json` | `rbxmeta.toml`, `rbxmeta.example.toml` |
| `rbxconfig.schema.json` | `rbxconfig.toml`, `rbxconfig.example.toml` |
| `rbxshop.schema.json` | `rbxshop.toml`, `rbxshop.example.toml` |

Two things to settle first, both because a catalog entry is a public promise
about a URL:

- **Serve them from a stable URL.** SchemaStore entries point at a raw URL, so
  `main` would hand editors an unreleased schema. A tag-pinned raw URL, or
  copying the schemas into the SchemaStore repo, both work; the first keeps one
  source of truth and needs the release process to remember to bump.
- **Wait for `rbxshop.toml`.** Submitting four of five names is a worse first
  impression than submitting five, and the missing one is the largest config
  in the suite. Its model was owned by another change when the rest landed.

The `.example` patterns matter as much as the real ones: the template is the
file a newcomer opens first, and it is the copy they learn the format from.

---

## Backlog — the scope catalog comes from the docs, not from the service

**Found 2026-08-04** while measuring the API-key list endpoint. Loading the
Creator Hub's credentials page shows it fetching
`GET https://apis.roblox.com/cloud-authentication/v1/scopes` — the scope
catalog, served by *the service that validates the scopes*.

`commands/catalog.rs` builds our catalog from
`creator-docs/…/reference/cloud/openapi.json` on GitHub instead: the published
documentation, which can lag or disagree with what the service enforces.

**Why it matters.** Less than it did. This was written as "very likely the
answer" to the `infer_target_type` item, but that one was settled on
2026-08-13 by readback: a key was created for each contested scope and Roblox
echoed the target it stored. So this no longer unblocks anything.

What it still buys is the difference between a catalog built from
documentation and one built from the service that enforces it. `create` builds
its requests from this catalog, and a wrong target fails at key creation rather
than at compile time — so the gap is worth closing before it is discovered by a
failed create rather than by a diff.

**Fix.** Measure `/v1/scopes` (cookie auth, GET, no parameters observed), diff
its scope list against the embedded catalog and against
`infer_target_type`, then decide whether it replaces the openapi.json source or
becomes a second opinion `catalog regenerate` reconciles against. Do not switch
the source before the diff: the catalog is what `create` builds its requests
from, and a wrong target fails at key creation, not at compile time.

---

## Backlog — `canUseApiKeys` may answer what `can-manage` infers

**Found 2026-08-04**, same page load:
`POST https://apis.roblox.com/cloud-authentication/v1/canUseApiKeys`, called
before the credentials page renders. Request and response shapes were not
captured.

`can-manage` currently answers with `develop.roblox.com`'s `canManage`, and
`docs/apikey.md` is explicit that "can administer this experience" ⇒ "can use
API keys here" is an *inference from three observations*, on a legacy endpoint
with no Open Cloud equivalent.

An endpoint named `canUseApiKeys`, on the cloud-authentication service itself,
is a candidate for the question we are actually asking.

**Fix.** Capture its request/response, check whether it takes a universe or is
account-wide (the name suggests account-wide, in which case it answers a
*different*, still useful question and does not replace `can-manage`). Only
then decide. Keep the measured caveats in the docs either way.

---

## Backlog — group-owned keys are not listed

`list --remote` and `prune` list the authenticated user's own keys. The listing
route takes an optional `groupId`, exposed as `--group-id`, but the user must
supply the id.

The Creator Hub enumerates them with
`GET https://apis.roblox.com/creator-home-api/v1/groups?surface=CreatorHub`
(observed on the credentials page; response shape not captured).

**Fix.** Measure that response, then let `list --remote` sweep every group the
account belongs to — one listing call per group — so "every key on the account"
is true without arguments. Until then the commands say so explicitly rather
than implying completeness.

---

## Done 2026-08-13 — every `api/` layer is now reachable by a mock server

**Was.** `wiremock` was a dev-dependency and `rbx-core` used it for the retry
tests, but the domain crates built their URLs inline, so none of them could be
pointed at a mock server. Pagination, CSRF retry and error mapping had never
run anywhere.

Closed one crate per PR, as this item asked. `rbx-probe` and `rbx-apikey` were
done earlier; `rbx-config` and `rbx-place` turned out to have been done without
being struck off, which is why the list overstated the work by two crates when
it was next read.

| Crate | What became testable |
| --- | --- |
| `rbx-meta` | The two PATCHes that write to a live universe and a live place. Roblox ignores any body field the `updateMask` does not name, so a wrong mask is a write that answers 200 and changes nothing. |
| `rbx-init` | The CSRF retry, which every group, universe and place creation depends on and which had never run. Plus `groupId` travelling as a query parameter — dropped, it creates the universe under the signed-in user instead of the group. |
| `rbx-core` | The asset download's two-step: the bytes come from the `location` the delivery API names, and the api key must not follow them to the CDN. |
| `rbx-download` | Both sources, which differ on purpose — the cloud one puts the id in the path and a version in a further path segment, the public one asks by query parameter. |
| `rbx-shop` | The page-token loop, and the create call that puts a developer product up for sale. |

**A convention the whole thing turns on.** `rbx-spec-drift` works out which
host a `.join(...)` reaches by taking the receiver's name off the same line and
looking up `const <RECEIVER>_HOST`. So: name the const after the field, name a
bound base after the field too, and keep the receiver on the `.join(` line — a
chain rustfmt splits resolves to `apis.roblox.com` and reports drift that is
not there. The drift test caught exactly that twice during this work, which is
what it is for. Const resolution is now per crate rather than per file, so the
const may live in `api/mod.rs` while the call lives in `api/groups.rs`.

**What the tests found that review had not.** A fixture is only worth what its
field names are: `DeveloperProduct` renames `id` to `productId`, and a fixture
using `id` passed every length assertion while every product came back with no
id at all. Assert on the field you renamed.

**Still literals, deliberately.** `develop.roblox.com` and
`thumbnails.roblox.com` in `rbx-meta`, and `INTROSPECT` in `rbx-apikey`. They
are separate services and nothing tests them yet; converting a host no test
needs is churn.

---

## Done 2026-08-15 — two binaries were not the boundary they looked like

The post-mortem of `rbx-ops`, moved here when the shim was deleted in 0.12.0,
because it is the kind of design a future maintainer re-proposes if nobody
wrote down that it was tried and measured.

**Was.** Live operations shipped as a second binary, so that a tool CI installs
could not ban a player: `rbx` reconciled repo-declared state, `rbx-ops` acted on
a live experience and its players.

**Why it did not hold.** The boundary it drew was never the one that mattered.
Roblox binds an API key to its scopes and universes at creation, so a deploy key
cannot ban anybody whichever binary calls it. The split bought a *signal*, not a
capability: `rbx-ops ban` announces itself in a way `rbx ban` does not.

**What it cost.** Rokit resolves one artifact per repository, so of two binaries
published from one repo only one could ever be installed through it. Measured
against v0.5.0 and again against v0.9.0: `rbx-ops` had to be built from source
both times, which is a real barrier for the operators it was aimed at.

**What replaced it.** The signal lives in the `Live:` prefix on each command's
description and in their position at the end of `rbx --help`. The commands moved
to `rbx <name>` in 0.10.0, the shim forwarded with a deprecation notice for two
minor releases, and 0.12.0 deletes it. Two minor releases of notice was chosen
so that anyone pinning a version in `rokit.toml` met the warning during a normal
upgrade, without the shim becoming load-bearing.
