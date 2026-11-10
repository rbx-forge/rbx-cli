# TODO: deferred work

Items captured from design discussions but deferred to avoid scope creep
on the current release. Each item lists the target release.

---

## Done 2026-08-13: a duplicate name is a question, not a silent drop

**Was.** `init --from-remote` and `pull` key a newly discovered resource by its
display name. Roblox does not make those unique, so two passes named "VIP"
collapsed to one key and the second was dropped. It warned, but the warning
named the id being *skipped* rather than the one keeping the key, went to
stdout, and left the exit status at zero, so a real resource stopped being
managed and the message did not carry enough to act on.

**Fix.** `collision::resolve_duplicate` asks for a key where there is somebody
to ask, using the same terminal test as every other prompt. Off a terminal nothing prompts (CI must not stop on a question nobody
will answer) and the warning now names both ids and prints the TOML that binds
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
keyed by id, so a collision can only involve newly discovered resources:
nothing under management was ever displaced.

---

## Considered and declined: Open Cloud surface we are not going to cover

**Audited 2026-08-04.** Diffed the whole Open Cloud `openapi.json` (691 paths,
813 operations) against every URL the workspace calls. The tool's own embedded
scope catalog already names several capabilities it does not implement. Listed
here so the audit is not repeated, and so the answer stays "no, on purpose"
rather than "nobody looked".

**Re-audited 2026-10-09**, by the document's own categories rather than by path.
The coverage test cannot do this job: it only reports uncalled endpoints in a
family the workspace already calls, so an area nothing calls yet, as groups
were, never shows up in it.

Declined, because a useful tool is not a catch-all. Three rows have since left
this table, and all three left for the same kind of reason.

`memory-store/*` rested on a scope classification that turned out to be a
default rather than a finding. `universes/{id}:publishMessage` was declined as
"server-to-server IPC belonging to game code": true when the publisher is a
game server, and the case that reopened it has the publisher outside Roblox
entirely, where there is no in-experience way to originate the message.

A reason to decline is only as good as the fact under it, and both of these
were about the shape of the API rather than about who would be calling it.

| Surface | Why not |
| --- | --- |
| `users/{id}/notifications` | A whole domain, and nothing has anything to send yet. Players of the game using this tool can now opt in, so the reason is no longer the missing audience: it is the missing message. Roblox only sends these through Open Cloud, from outside the experience, so the day there is something to send, the question is a command or a scheduled service. The `rbx-ops` blurb used to promise it; the blurb was the bug, and it has been fixed. |
| `luau-execution-session-tasks` | Powerful (run Luau against a place or a version, with logs), but adequate tools already exist and duplicating one is not a reason to ship. The tool in use here is [jest-roblox-cli](https://github.com/christopher-buss/jest-roblox-cli), named so the claim is checkable rather than asserted. Reconfirmed 2026-08-15, when an MCP server covering the same endpoint prompted the question again. |
| `inventory-items`, subscriptions, `creator-store-products`, place `instances`, `:generateThumbnail`, `:translateText` | No demand, and each is a new domain rather than the completion of an existing command. |
| `groups/{id}/join-requests` | Only meaningful for a group that approves its members. The group this tool manages admits anyone, so there is never a request to act on. Revisit if that changes. |
| `place-version-history-api` (history filters, contributors, version notes) | `rbx place versions`, `upload` and `rollback` already list, publish and restore. Notes would only tie a published version to a commit, which git history and a deploy tag already do. |
| Experiments, analytics alerts, matchmaking | All EXPERIMENTAL, and no game using this tool calls them. |
| `virtual-events` (an experience's Events) | Not now rather than no. The Creator Hub is fine for an event now and then; a command earns its place only once events are created on a schedule, a weekly update say. EXPERIMENTAL. |
| Localization: translated names, descriptions and icons for the experience, passes, products and badges; supported languages; localization tables | Not now rather than no. No experience using this tool is translated. The day one is, this is the largest and most natural extension of `meta` and `shop`. EXPERIMENTAL. |

Taken instead: `data snapshot`, because it completes a command that already
exists and makes a claim in its own documentation true.

**Group memberships and roles have left this table.** They were declined as
"no demand, and a new domain"; the demand turned up, from a game that ranks
members by hand and generates a module from its roles. They shipped as
`rbx group` (`roles`, `member`, `members`, `rank`, `unrank`), with `rbx user`
alongside for the username lookup every one of them needs.

**`ordered-data-stores` has left this table.** It was declined as "only earns
its place in a tool whose users keep leaderboards", with "revisit on demand"
attached, and the demand turned up. It shipped as `rbx data ordered`, a
sibling mode under `data` exactly as the row predicted, minus the four verbs
Roblox does not offer on that resource (`snapshot`, `revisions`, `restore`,
`diff`).

---

## Backlog: publish the config schemas to SchemaStore

**Added 2026-08-14** with the schemas themselves. `schemas/*.json` exists and
is kept fresh by CI, but nothing picks it up automatically: a user has to
write a `taplo.toml` rule or a VS Code association by hand, which is exactly
the friction the schemas were meant to remove. docs/reference/editor-support.md documents both.

**Fix.** A pull request to
[SchemaStore/schemastore](https://github.com/SchemaStore/schemastore) adding
one catalog entry per file, matching on the file names:

| Schema | `fileMatch` |
| --- | --- |
| `rbxplace.schema.json` | `rbxplace.toml`, `rbxplace.example.toml` |
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

## Done 2026-08-13: every `api/` layer is now reachable by a mock server

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
| `rbx-init` | The CSRF retry, which every group, universe and place creation depends on and which had never run. Plus `groupId` travelling as a query parameter: dropped, it creates the universe under the signed-in user instead of the group. |
| `rbx-core` | The asset download's two-step: the bytes come from the `location` the delivery API names, and the api key must not follow them to the CDN. |
| `rbx-download` | Both sources, which differ on purpose: the cloud one puts the id in the path and a version in a further path segment, the public one asks by query parameter. |
| `rbx-shop` | The page-token loop, and the create call that puts a developer product up for sale. |

**A convention the whole thing turns on.** `rbx-spec-drift` works out which
host a `.join(...)` reaches by taking the receiver's name off the same line and
looking up `const <RECEIVER>_HOST`. So: name the const after the field, name a
bound base after the field too, and keep the receiver on the `.join(` line: a
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

## Done 2026-08-15: two binaries were not the boundary they looked like

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

## Done 2026-11-08: the split again, one repository per tool

**Was.** One binary carrying the reconciling commands, the live operations
behind their `Live:` prefix, API key management, the Studio launcher and the
asset downloader.

**Why the merge above did not settle it.** It fixed the install, not the
boundary. A tool meant to run on every push was still the one that could ban a
player, and the surface kept growing in directions that had nothing to
reconcile: no desired state for a ban, a launch or a download.

**What changed.** The measurement above was about two binaries from *one*
repository. One binary per repository has no such problem, so each of those
families left for a repository of its own, installable through Rokit like this
one. `rbx` keeps the commands that reconcile a committed file (`init`,
`import`, `env`, `check`, `status`, `place`, `meta`, `config`, `rtbf`, `shop`)
and `probe` beside them.

**What stayed shared.** `rbx-core` is still a library: the extracted binaries
depend on it by git, pinned through their own lockfiles, so nothing moved out
of it. Removing the cookie chain or the asset helper because `rbx` no longer
calls them would break those consumers on their next update and simplify
nothing here.
