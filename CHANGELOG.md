# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **A test that asks which documented endpoints we never call.** The spec drift
  check answers whether everything this workspace calls still exists. It cannot
  answer the reverse, because an endpoint nobody calls contributes nothing to a
  scan of call sites, so the vendored spec can document a capability for months
  and nothing says so. `Cloud_ListDataStores` sat there unimplemented until
  somebody asked in conversation; it shipped as `rbx data stores` in 0.6.0 and
  nothing in CI had ever mentioned it.

  Scope is derived rather than curated: an endpoint is reported only when the
  workspace already calls something on the same resource, so the several
  hundred documented endpoints in areas this tool has no business in stay
  silent without needing a line each. The first run over 697 documented paths
  reported eleven.

  Four of those turned out to be **called already**, through URLs built by
  appending to a helper, which the extractor cannot resolve. They are listed
  with their call sites, and the list is a record of calls the sibling drift
  check does not protect either: if Roblox renames one of those paths, nothing
  notices. Two were a real gap, filed as #57. The rest are declined with
  reasons.

- **A test that catches a flag documented nowhere.** The dual of the docs drift
  check: that one hands every `rbx ...` line in the pages to clap and catches
  prose describing a CLI that no longer exists, and it is blind to the reverse
  by construction, because it only reads what the pages already say.

  The reverse is the one that rots quietly. A stale page is caught the first
  time somebody follows it; an undocumented flag is never caught, because
  nobody looks for a thing they were not told about.

  It walks the clap tree and asks that each long flag appear literally in the
  page `mkdocs.yml` sends a reader to, with an allowlist carrying a reason per
  entry. First run found two: `rbx rtbf --config`, now documented, and
  `rbx ban --users-url`, a hidden test seam listed with the reason it stays
  unwritten.

- **`rbx download` names a carousel preview video `.m3u8`.** `GamePreviewVideo`
  (AssetTypeId 86) used to land as `.bin`. It is not a video file: the asset is
  an HLS master playlist, `#EXTM3U` followed by five vp9/opus renditions from
  1280x720 at 60fps down to 160x96 at 5fps, and it labels itself with
  `RBX-VIDEOTYPE="GamePreviewVideo"`. Measured on a live public asset rather
  than inferred, which matters here because the obvious guess by analogy with
  `Video = 62` is `mp4`, and that would have named a 1.1 KB text manifest after
  the thing it points at.

  `AdsVideo` (81) and `StorePreviewVideo` (85) are almost certainly the same
  shape and are deliberately left as `.bin`. Nobody has downloaded one.

- **`created` on the `upload` and `promote` write documents**, and an
  `(unchanged)` marker on the human form. Boolean, absent when the run could
  not answer. See below for what it is answering.

### Fixed

- **An upload that changed nothing reported a version as if it had written
  one.** Roblox creates no version for a file a place already holds: it answers
  with the number the place is already at. The command printed that number and
  `Upload complete.`, in output identical to a real write, so a `--file`
  pointing at a stale artifact or a build step that failed quietly looked
  exactly like a successful deploy.

  Measured rather than assumed, against a test place: the same file uploaded
  twice answered the same number both times and left the version list untouched;
  one byte changed, in the reserved bytes of the binary header that the format
  ignores, produced a new version two minutes later. So the deduplication is on
  the bytes sent and no cooldown is involved.

  Each upload now reads the place's current version first and compares. That
  read is best effort: a key that may upload and may not list versions leaves
  the question unanswered, `created` absent and the output as it was, because a
  diagnostic must never be the reason a write fails. `promote` sends its bytes
  through the same endpoint and gets the same treatment; `rollback` does not,
  its endpoint has not been measured.

- **`rbx apikey regenerate` announced `Old secrets invalid as of <time>` after a
  run that rotated nothing.** The line printed unconditionally after the loop
  while each key's failure was swallowed, so a run refused for a missing cookie
  still said the old secrets were gone. That is the line a reader acts on,
  because it is the one saying the value in their CI is dead.

  It now prints only when something was actually rotated, and names the count
  when only some of the keys were. Relatedly, the command exited zero after
  failing to rotate anything; any key that did not come through now makes it
  exit non-zero.

  The two questions are tracked separately, because a rotation Roblox performed
  and this tool then failed to store answers yes to "is the old secret dead" and
  no to "did this work".

### Changed

- **Scope entries naming the same type and target are merged before being
  sent.** `universe:read` and `universe:write` on two lines of `rbxapikey.toml`
  went out as two entries; Roblox stores them as one holding both operations,
  confirmed by reading a key back through `apikey introspect` (eight entries
  sent, seven stored). The payload now has the shape the API answers in, so the
  key created is comparable to the config that asked for it. No key gains or
  loses a permission from this.

## [0.6.0]

### Added

- **`--json` on `set`, `reset`, `restore` and `delete`.** These four said what
  they did in prose on stdout, so anything driving them had two ways to know
  and both were poor: parse that sentence, or read the exit code and learn
  only that it worked. The document carries the action, whether it was applied
  or was a dry run, whether the entry existed, where the backup went, and the
  revision the entry is at now, which is the one fact a caller wants afterwards
  because it is what `data revisions --revision` takes.

  This overturns a decision the crate had pinned with a test: `--json` was
  confined to the subcommands that never prompt, because `OutputFormat::Json`
  refuses to prompt and a write asks through `confirm_always`. That reasoning
  was right, so the flag now **requires `--yes`**, and clap refuses the pair
  without it. The guarantee stays where it was, at parse time, rather than
  moving into a check at run time that somebody has to remember.

  `copy`, `increment` and `snapshot` still carry no document.

- **`rbx data delete`**: removes an entry the way `RemoveAsync` does, which
  until now had no equivalent here. `set` and `reset` were the only ways to
  change an entry from outside the game, and neither leaves the experience in
  the state a removed key does.

  It is the gentler of the two despite the name. A read then answers nothing,
  so a game that builds a fresh profile when it finds none builds one from its
  own template instead of from a JSON copy that has to be kept in step. And
  the value survives: soft-deleted, listed under `--show-deleted`, readable
  through `data revisions` for thirty days, where an overwrite destroys it on
  landing.

  The local copy is written first regardless, since thirty days is a deadline.
  A key that is not there is reported rather than treated as a failure, and
  nothing is sent.

  A live session holding the profile writes it back when it ends, so this is
  for a key nobody is currently playing. The `--help` and the docs both say
  so, because the failure it produces looks like the command doing nothing.

- **`rbx data stores`**: lists the data stores in an experience. Every other
  `data` subcommand takes `--datastore <name>`, and nothing told you what the
  names were, so the one question you have before all the others was the one
  question the tool could not answer. `Cloud_ListDataStores` was already in the
  bundled spec and already covered by the `universe-datastores.control:list`
  scope the `data` key carries; only the wiring was missing.

  Experience-wide, so it takes neither `--datastore` nor `--scope`, and it
  paginates on `pageToken` up to `--limit`. `--show-deleted` includes stores
  soft-deleted and not yet purged, and marks them in both output formats.

  Expect names nobody chose. A store exists from its first write rather than
  from the first `GetDataStore`, and a game running in Studio writes wherever
  its own wrapper points, so a `-studio` twin of the live store and a wrapper
  library's bookkeeping store both show up next to the data you meant to find.

### Fixed

- **`rbx place versions` and `rbx place download` refused `--place-id` without
  `--env`.** Both commands declared `--env` as required, so clap turned the
  invocation away before anything looked at the id. `--place-id` is documented
  as skipping `rbxplace.toml` and reaching the reads, its own help says it wins
  over `--env` when both are given, and the resolver behind it already answered
  from the id alone: only the parse rule disagreed.

  `--env` is now required *unless* `--place-id` is present, on those two reads
  only. The writes still refuse an id on its own, which is deliberate and
  unchanged: `confirm = true` is declared on an env, and a write with no env
  would walk past a guard somebody set.

  One consequence for consumers: `place versions --json` **omits** `env` under
  a bare `--place-id`, rather than inventing one. That is the rule `place
  places --json` already follows under a bare `--universe-id`, and no document
  that carried the field before loses it, since the combination that omits it
  could not be parsed until now.

  Found by a new test that hands every `rbx …` line in `docs/` to clap without
  running it. The pages described the behaviour that was designed and the CLI
  had the other one.

- **`rbx apikey create` reported false scope drift on any target Roblox does
  not return under `universeIds`** (#37). The post-create check announced
  that a scope had not been stored on its universe *and* that the same scope
  had appeared unasked on `*`, the two warnings mirroring each other, for a
  key Roblox had stored exactly as requested.

  The target is not always in one field. `universe-datastores.objects` and
  `.versions` come back under `universeDatastores: [{universeId}]`, and a
  creator-targeted scope such as `asset` under `groupIds`, while
  `universe-datastores.control` uses `universeIds` like everything else. That
  is why one key could contain both the bug and its counter-example. The
  reader took only `universeIds`, and a scope whose target it did not find
  fell through to the `*` that means "no target at all", which is what
  produced the second warning.

  Cosmetic in effect and not in consequence: the check exists to catch a key
  that cannot do its job, and a warning that cries wolf on a correct key is
  how a real one gets ignored.

  A target under a field this build does not know is now **reported** rather
  than read as a wildcard, so a fourth shape shows up as "this build cannot
  read the answer" instead of as confident, wrong drift. That includes a data
  store named inside the target, which a key scoped to one store is sent
  with: no capture shows how it comes back, and quietly dropping it would
  verify a narrow key against a wide request and call it a match. Ids are
  read whether they arrive as strings or numbers, since that is the same
  target written differently rather than a shape nobody understands.

## [0.5.0]

### Added

- **`rbx config --repository <name>`, and a `repository` field in
  `rbxconfig.toml`.** The Configs API takes the repository as a path parameter
  and its `Repository` enum holds eight values; this tool had it as
  `const REPOSITORY: &str = "InExperienceConfig"`. So the whole
  draft/publish/revisions/restore lifecycle was already repository-agnostic
  underneath a parameter modelled as a constant.

  `InExperienceConfig` and `DataStoresConfig` are the only two with a
  documented entry schema, and the other six are forward declarations of
  products that are not out yet. That is why there is no bespoke command per
  repository: a generic flag works the day Roblox documents
  `LeaderboardsConfig`, with no release of this tool. `rbx config` carries the
  transport and takes no view on what any repository's entries mean.

  A flag contradicting the file is **refused**, naming both and the file that
  holds one of them, because a `sync` into a repository the file does not
  describe overwrites a live config wholesale and this command cannot undo it.
  Every existing invocation is unchanged: no flag and no field is still
  `InExperienceConfig`.

- **`rbx rtbf`**: the failure this prevents is a deletion template that
  matches nothing. Roblox accepts one, stores it, reports it as configured,
  and deletes nothing, so a right-to-be-forgotten request goes unfulfilled and
  nothing says so until somebody asks why. Roblox's own advice is to compare
  the patterns against your live Luau by hand in the Creator Hub and then to
  confirm within 30 days that the data went, which is an admission that
  nothing verifies it for you.

  `rbxrtbf.toml` declares which data store keys and stores hold a user's data,
  reconciled against the `DataStoresConfig` repository of the Configs API:
  `init`, `show`, `check`, `sync`, `pull` and `verify`. No lockfile, for the
  reason `rbx config` has none: the published set is readable in full, so the
  remote state is a fetch.

  `{UserId}` is case-sensitive, and `{userId}` is the mistake Roblox's own
  best-practices list puts first: it is stored happily and matches nothing.
  That, a pattern carrying no token at all, and a near-miss token in a `scope`
  are all refused locally, before a publish makes them authoritative. So is a
  file that declares nothing while naming a root table this release does not
  read: `[[keys]]` for `[[key]]` parses to an empty declaration, and `sync`
  publishing that would replace every template the universe had.
  `rbx rtbf verify` then goes further and lists the stores the universe really
  has, so a template naming one you renamed last year is caught before a legal
  request depends on it.

  `rbx config --repository DataStoresConfig` reaches the same place and will
  keep working. What it cannot do is check any of the above, because its entry
  model holds an opaque value, and this command exists for exactly those
  checks. `rbx check` picks `rbxrtbf.toml` up as two rows, the local one
  running under `--offline`.

  `show` and `verify` carry `--json`, keeping the contract the rest of the
  suite does: one document, a `schema_version`, ids as strings, an optional
  field absent rather than null. `verify`'s is the one nothing else produces,
  so a CI step branches on `.ok` rather than grepping a listing for a red
  cross, and its `verdict` has three values rather than being a boolean:
  `unverifiable` is a limit of Open Cloud, not a broken template, and folding
  it into a failure would break a build over an ordered store nothing can
  list. `check` has none, deliberately: no per-tool check in this suite does,
  because `rbx check --json` is the machine-readable drift document and two
  shapes for one question is how a consumer reads the wrong one.

- **`rbx restart launch --attribute k=v` and `--payload <json>`**: the free
  dictionary servers scheduled to close receive as the third argument of
  `game.ServerRestartScheduled(restartTime, source, attributes)`. Until now that
  table was always empty for anything this tool restarted, so a game could be
  told it was about to be restarted and nothing about why. `--attribute` is
  repeatable and sends strings; `--payload` takes the whole object as JSON, for
  a number, a boolean or nesting. The two are mutually exclusive.

  Both are parsed and bounds-checked before the first request, so a malformed
  body fails locally rather than as a 400 from inside a deploy, and never after
  the confirmation prompt. Roblox's limits are enforced here: it must be a JSON
  object, and at most 500 bytes serialised.

  Settling this needed no live probe, only the vendored spec: the devforum
  announcement and the Universes reference describe **two different endpoints**.
  `rbx restart launch` calls `Restarts_LaunchRestart`, which accepts
  `attributes` and a bleed-off of 1 to 240 minutes;
  `Cloud_RestartUniverseServers` accepts no `attributes` and caps the bleed-off
  at 60. Both are right about their own endpoint, and the 240 this tool checks
  was already correct.

- **`[groups]` in `rbxplace.toml`**: named subsets of your envs, usable anywhere
  `--env` is.

  ```toml
  [groups]
  nonprod = ["dev", "staging", "qa"]
  ```

  `rbx shop sync --env nonprod` runs what `--env all` would run, over those
  three envs instead of every env. Until now the only grouping was `all`, which
  reaches production, so "every env except the live one" meant three invocations
  or nothing.

  **A group is an alias and nothing more.** It is expanded to its members before
  anything else happens, so no lockfile, no overlay and no generated module ever
  sees the name: every lockfile in the suite keys on `(env name, universe_id)`,
  and a group has no universe of its own to record. `[envs.nonprod]` in
  `rbxmeta.toml` is deliberately not a thing, and will not become one: an
  overlay shared by several envs is what the base table is for, with the
  exception written as the one env that diverges.

  A group is refused wherever `--env all` is, and now says so usefully:
  `` `--env nonprod` is a group of 3 envs (dev, staging, qa); this command acts
  on one universe. Name one of them.`` Four sites used to spell that refusal out
  separately, two of them differing only in punctuation, and `all` was compared
  as a bare string in nine places across six crates. It is one type now,
  `EnvSelector`, which is what let a group be refused everywhere without a tenth
  site being forgotten.

  Refused at load, so a bad group fails the file for every command rather than
  halfway through a deploy: a member that is not a declared env, a group naming
  another group (they are flat), a group sharing a name with an env, a reserved
  name, an empty group, and one env named twice.

  `[groups]` had to be claimed in **three** parsers, not one: `rbx place` and
  `rbx config` each keep a narrower parse of `rbxplace.toml` with a flattened
  catch-all, so a top-level table neither claims is read as an env and fails the
  whole file on a missing `universe_id`. That is why `[owner]` and `[codegen]`
  are claimed there too. `rbx place` also keeps the table across
  `place fetch --write` rather than deleting somebody's groups on the way
  through.

- **`rbx place upload --env all`, and `--env <group>` with it.** `place` was one
  of the last two commands with no fan-out, so shipping one build to three envs
  meant three invocations and three chances to forget one. `all` walks the
  file's envs alphabetically, a group walks its members in declared order, and
  `--place` / `--all-places` are resolved inside each env.

  The four decisions are the ones `rbx shop sync` already made, because a
  fan-out that answers them differently is a second model to learn. Everything
  is resolved before the first byte goes out, so a `--place` name one env does
  not declare fails the run rather than failing it after two envs are written.
  The `.rbxl` is read once and the buffer shared, so `--env all` costs one read
  of a multi-megabyte file. The confirmation is asked once for the whole run,
  gated on whether *any* target env sets `confirm = true`, and it names every
  env: per env it would be reached mid-walk, after writes had landed elsewhere,
  which is too late for a "no" to mean anything. The `env: <name>` header
  appears only when there is more than one target, so a single-env run's output
  is byte for byte what it was.

  Under `--json`, a plural `--env` emits a new envelope holding one receipt per
  env; a single env emits exactly the `WriteDocument` it always has.
  `WriteDocument` is untouched, deliberately: `promote` and `rollback` share it
  and every consumer reads its `env` and `universe_id` as scalars. Fields are in
  `docs/place.md`.

  `download` and `promote` refuse a plural selector rather than ignoring it: N
  downloads would land on the one path `--out` names, and promote names its two
  envs itself with `--from` and `--to`. `rollback`, `versions`, `places` and
  `fetch` refuse one too, through a single guard rather than five, because they
  resolve their env through a config model that knows nothing about groups and
  would have answered `Environment 'nonprod' not found`: the one thing that is
  not wrong.

- **`rbx meta check`, `sync` and `pull` take `--env all`, and `--env <group>`
  with it.** `meta` was the other command with no fan-out, so holding three
  envs' metadata in step meant nine invocations and a mental note about which
  one was skipped.

  `check` fails on drift in *any* env rather than in the last one it looked at,
  which is the question a CI step is asking. `sync` plans every env before it
  sends anything: planning is offline, so the whole run's pending changes are on
  screen before the one prompt, and a config error in the last env stops the run
  before the first one is touched. `pull` reads every env first and writes
  `rbxmeta.toml` once at the end.

  Each env's lockfile entry is derived from the snapshot **that env's own read**
  produced, before any later env had a turn. That is the only correct instant,
  and it is equivalent to a sequential `pull --env dev` then
  `pull --env prod`. Reading the config back after the whole loop would hand one
  env a value another env's remote supplied: the differential promotes a field
  to the base `[game]` the first time an env has one, so a post-loop read
  records `prod`'s private-server price as `dev`'s *confirmed remote state*,
  `check --env dev` then reports agreement, and `sync --env dev` sends nothing
  forever over a universe that has no private servers.

### Fixed

- **`rbx apikey create` failed for any key declared without a `description`.**
  Roblox refuses a key whose name or description carries a brand
  (`Response.InvalidNameOrDescription`), and the auto-generated description read
  `Managed by rbxapikey (...)`, which contains `rbx`. So the failure sat on the
  path the documentation recommends: the field is optional. The fallback no
  longer names the tool, and a test keeps the brand out, because the failure is
  invisible in review and only appears against the live API.

  The refusal itself names neither of the two fields it covers, which is what
  made this cost a session to find. It is now answered with both values that
  were sent, so which one carries a brand is visible at a glance. Nothing is
  refused locally, and the rule is why: `testenv/rbxapikey.example.toml` already
  recorded it as `rbx` or `roblox` glued to an API or commerce term, which is a
  judgement rather than a substring. Every approximation of it rejects text
  Roblox accepts, and nobody debugs a check that fires wrongly.

- **`rbx check --env <typo>` reported drift on an env that does not exist.**
  A named env was taken at its word without ever reading `rbxplace.toml`, so
  `--env prd` answered `! shop/lockfile [prd] 1 to create, 0 to update: run
  \`rbx shop sync\``: a confident, actionable-looking row about nothing, naming
  a `sync` that could not work. In CI that is indistinguishable from real drift.
  The name is now checked against the file and refused with the available envs
  listed, the same wording every other command uses.

  Only when the file loads. A project with no `rbxplace.toml` is one `check`
  still has useful things to say about, and a file that does not parse is one
  failing row from `tools::env` beside every other tool's findings rather than
  an abort that collapses the whole run to a raw TOML message.

- **`rbx config sync` no longer discards a draft somebody staged in the
  Creator Hub.** Roblox documents `previousDraftHash` on `draft:overwrite` as
  optimistic concurrency: the request fails when the hash does not match the
  server's current draft. `overwrite_draft` took the parameter and
  `sync_and_publish` passed `None`, so the check was threaded through the code
  and hardcoded off. A staged draft was overwritten in silence, and it was gone
  before its author could learn it had existed.

  The draft is now read first, its hash travels back with the write, and what
  was replaced is named on the way past. Not a refusal: pipelines legitimately
  overwrite, and a hard stop would break them. The extra request is the point
  of the change rather than a cost of it.

- **The documented Configs limits are checked before the publish, not by
  Roblox.** 100 entries per repository and 256 characters per key. The 101st
  key failed as a 400 from inside a deploy, naming neither the key nor the
  limit. `sync --dry-run` enforces them too, because a dry run reporting a
  clean plan for a publish that cannot happen is the wrong answer. Key length
  counts characters rather than bytes, since the guide says "256 characters"
  and a byte count would refuse 200 accented characters Roblox accepts.

- **A config publish no longer clears the repository's conditional rules.**
  The Configs API's `draft:overwrite` treats an omitted `conditionalRules` as
  an instruction: "when omitted on overwrite, all published conditional rules
  are cleared". This tool never sent the property, so the first `rbx config
  sync` against a repository carrying conditional rules deleted every one of
  them, silently, with nothing about it in the replaced-draft report the
  command prints. An entry still referencing a deleted conditional then made
  the request fail with an opaque 4xx, which reads as a bad payload rather
  than as a rule that is gone.

  The rules are now restated from the draft when it stages any, and from the
  published configuration when it does not. That layering is the API's own,
  stated on the `PATCH` side of the same field, and the second half is what
  matters in practice: a first sync usually meets a repository with published
  rules and no draft at all, so echoing the draft alone would have left the
  loss exactly where it was. Rules travel as opaque JSON, because on overwrite
  a property this tool cannot name is a rule it would delete.

- **`rbx config rollback` addresses the documented path.** It built
  `.../revisions/{id}:restore`, a custom-method form that appears nowhere
  under `creator-configs` in Roblox's OpenAPI document, which describes
  `POST .../revisions/{revisionId}/restore`. The only `:restore` in the whole
  document belongs to the Assets API. The test asserted the tool's own
  spelling straight back at it, which is why a wrong URL stayed green.

## [0.4.0]

### Added

- **`rbx secret`**: the universe secrets store `HttpService:GetSecret` reads
  from: `list`, `set`, `delete`, and `public-key`. Until now the only way to
  put a credential in front of a running experience was the Creator Dashboard,
  by hand, one universe at a time, which is how staging ends up still holding
  last quarter's key.

  Writes are encrypted **before they are sent**, because Roblox does not accept
  a secret in the clear even over TLS: the value is sealed with a LibSodium
  sealed box against the universe's own public key, so the ciphertext cannot be
  opened by anything, including the process that produced it. That is what
  makes `printenv API_TOKEN | rbx secret set api_token --stdin --domain
  api.example.com --apply` safe to run from CI.

  There is deliberately no `rbx secret get`. Roblox never sends stored content
  back, a listing carries metadata only, and no document this command emits has
  a field a secret value could go in.

  `set` requires `--domain <pattern>` or `--no-domain` on every write, with no
  default for either. A secret with no domain cannot be attached to an outgoing
  request at all (right for a signing key, silent breakage for an API token)
  and since a `set` replaces the whole secret, an unstated domain would be a
  cleared one.

## [0.3.0]

### Added

- **`rbx open <file.rbxl>`**: open a place file from disk, by extension
  (`.rbxl` / `.rbxlx`) or explicitly with `--file`. An env is still an env: a
  folder holding a file named after one does not change what the name means.
- **`rbx open` works under WSL.** It used to call `xdg-open`, which cannot
  reach a Studio that lives on the Windows side, so the command did nothing at
  all there. WSL is now detected and the target crosses to the Windows host.
  The gap was pointed out by ROpen 1.3.2, which fixed the same one in Luau; the
  implementation here is independent.
- **`rbx open --new`**: open a new, empty place, the way Studio's own "New
  Experience" button does. `--new` lists Roblox's templates and asks; `--baseplate`
  takes the stock one without a picker or a network call; `--template <place-id>`
  names one outright. Nothing is created on Roblox: Studio fetches the template's
  content and then unbinds the session from it, so the first save to Roblox is
  what creates the experience. `rbx init create-universe` remains the way to create one
  outright.

### Fixed

- **`rbx ban list` honours `--limit`.** It stopped fetching once it held
  `--limit` rows but never trimmed to them, so with the page size nailed to
  100, `--limit 5` answered with up to 100 rows under a JSON document claiming
  `limit: 5, count: 100, limit_reached: true`: three fields contradicting each
  other, and the ones a script reads. The walk also refused to end on an empty
  page that still carried a token, which could spin.
- **`h2` 0.4.16**, for RUSTSEC-2026-0258: unbounded queuing of empty DATA
  frames. `rbx` is a client, so reaching it takes a hostile host, but a
  permanently red advisory check is a check nobody reads.

### Changed

- **The vendored Roblox OpenAPI document and scope catalog** are refreshed to
  the 2026-08-20 upstream commit.
- **Every CI job declares `timeout-minutes`.** Without it GitHub allows 360
  minutes, so one hung job cost six hours of runner time.

## [0.2.0]

### Added

- **`rbx env rm <name>`**: take an env out of every file that mentions it:
  the block in `rbxplace.toml`, every overlay and lockfile section keyed by it,
  and the generated per-env module. Not called `destroy`, because Roblox does
  not let a tool keep that promise: a game pass cannot be deleted, only taken
  off sale.
- **`rbx data ordered`**: the leaderboard resource, with `list`, `get`, `set`,
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

- **`init`**: create a group, a universe, places, and record their ids.
- **`import`**: adopt a universe that already exists: every config and
  lockfile written from what is live, in one pass, so that `check` is green
  immediately after with nothing in between.
- **`env`**: read `rbxplace.toml`, print one id for a script, generate a Luau,
  Lua, JSON or TypeScript module so game code branches on env instead of
  hardcoding ids.
- **`apikey`**: declare Open Cloud keys and scopes in `rbxapikey.toml`, create
  and rotate them, and see every key the account holds rather than only the
  ones this project made. `readonly = true` refuses a write scope at load.
- **`doctor`**: prove the loaded key works with one real read.
- **`check`** / **`status`**: every configured tool's check in one pass, one
  exit code for CI; the same engine rendered for a person.
- **`place`**: upload, download, promote between envs, roll back.
- **`meta`**: universe and place metadata, including the fields Open Cloud
  does not expose.
- **`config`**: the live in-experience config, with revisions and rollback.
- **`shop`**: game passes, badges and developer products, with typed Luau
  codegen and a `--check` that proves the committed module was not hand-edited.

### Operational

State that only exists while the game is running, which no TOML file can
describe. Dry run by default, `--apply` to write, `--env all` refused.

- **`servers`**: servers up now, how the stopped ones ended, and what a
  crashed one logged.
- **`analytics`**: players, retention, revenue per payer; CSV for charting
  elsewhere.
- **`ban`**: inspect and change player restrictions.
- **`restart`**: forecast how many players a rolling restart would disconnect,
  then launch it.
- **`data`**: read, overwrite, copy and recover one data store entry, with a
  local backup written before every write.
- **`memorystore`**: cache values servers read through `MemoryStoreService`.
- **`message`**: push a MessagingService message to every running server.
- **`ads`**: launch and steer ad campaigns.
- **`probe`**: a raw authenticated request to any Open Cloud path.

### Local

- **`open`**: launch Studio at a place, by name or by id.
- **`download`**: fetch an asset by id.
- **`completions`**: shell completions that read your `rbxplace.toml` at TAB
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

[Unreleased]: https://github.com/rbx-forge/rbx-cli/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/rbx-forge/rbx-cli/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/rbx-forge/rbx-cli/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/rbx-forge/rbx-cli/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/rbx-forge/rbx-cli/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/rbx-forge/rbx-cli/releases/tag/v0.2.0
[0.1.0]: https://github.com/rbx-forge/rbx-cli/releases/tag/v0.1.0
