# The Studio cookie

`.ROBLOSECURITY` is the one credential `rbx` uses that is not an Open Cloud API key. This page is the single description of what the tool does with it: which commands send it, which commands will never send it, how one gets resolved, and why it is never written anywhere. Every other page that mentions the cookie links here instead of restating it.

## Why it gets its own page

Roblox binds an API key to its scopes and its universes at creation, so the worst a leaked key can do is exactly what its scope list says, and you can delete that one key without touching anything else. A `.ROBLOSECURITY` cookie is a full account session: unscoped, good for everything the account can do including the things `rbx` has no commands for, and revocable only by signing out everywhere, which kills all your other sessions at the same time.

That asymmetry is the reason the rest of the toolkit is arranged the way it is. Keys are preferred wherever Roblox offers the endpoint, cookie auth is refused outright for anything that touches live players, and an auto-detected cookie announces itself instead of being discovered later.

## What it is used for

Only the endpoints Open Cloud does not cover. The table is every place in the codebase that attaches a `Cookie: .ROBLOSECURITY=...` header.

| Command | Why the cookie | Required? |
| --- | --- | --- |
| `rbx init create-group`, `create-universe`, `create-place`, `rename-place`, `rename-universe` | Open Cloud can read groups, universes and places but cannot create or rename them. These go to `groups.roblox.com` and `develop.roblox.com`, which authenticate a person. | **required** |
| `rbx init list-groups` | The listing is "the groups *you* are in", which is a question only a session can answer. It resolves the signed-in account through `users.roblox.com` first. | **required** |
| `rbx init list-universes`, `list-places` | Nothing. Both listings answer in full without a credential, private universes included, so the cookie is accepted and changes no result. See [what these listings are not](#what-the-cookie-does-not-protect). | **never needed** |
| `rbx meta init`, `rbx meta pull` | Reads the fields Open Cloud does not expose (`server_fill`, `allow_copying`, `studio_access_to_apis_allowed`, `beta_mode`) from `develop.roblox.com` and the experience-releases endpoint. Without a cookie those fields are skipped and reported as skipped, never guessed. | optional |
| `rbx meta sync` | Writes those same fields, plus visibility flips (activate and deactivate). Everything else `meta` writes goes through Open Cloud with the API key. | **required for those fields** |
| `rbx meta` icon and thumbnail reads | The public thumbnails service answers either way; the cookie is attached when one is available. | optional |
| `rbx apikey create`, `update`, `regenerate`, `delete`, `prune`, `list --remote`, `status --remote`, `can-manage` | Roblox's key administration endpoints authenticate a person, not a key. Asking a key to describe or create keys is circular: a key only ever covers the universes it was bound to. | **required** |
| `rbx download` on the public backend | The legacy `assetdelivery` and `economy` endpoints answer for public assets without auth, and reach what your account can see with the cookie attached. The Open Cloud backend (any run with an API key, and every `--version` run) sends the key and no cookie. | optional |
| `rbx import` | Nothing, for its own place listing: it goes to `develop.roblox.com`, which answers without a credential. The `meta` step it runs afterwards is where a cookie matters. | **never needed for the listing** |
| `rbx doctor` | Reports which source the cookie came from and whether Roblox still accepts it, then reuses the `rbx apikey` calls above to identify the loaded key and read its scopes. Without a cookie those checks are marked skipped with the reason, not passed. | optional |

Two details the table would otherwise flatten:

- **`rbx import` does not auto-detect for its own call.** It passes `--cookie` / `RBX_COOKIE` straight through to the place listing. That used to be described here as a limitation; it is not one, because the listing answers without a credential either way. The `meta` step that `import` runs afterwards is an ordinary `rbx meta` invocation and does resolve the cookie normally, auto-detection included.
- **`rbx apikey introspect` resolves a cookie it does not send.** It shares the client constructor with the rest of `rbx apikey`, but the introspect endpoint authenticates with the key secret itself. So the command can print the auto-detection notice without a cookie ever leaving the process.

## What the cookie does not protect

This page is about a credential, so it is worth being exact about one thing it
is **not** the gate for. Two of Roblox's listings are open, and this page used
to say otherwise.

Measured against a private universe that has never had a player, with no
cookie, no API key and no session at all:

```
GET develop.roblox.com/v1/universes/{id}/places  → 200, every place, with names
GET games.roblox.com/v2/groups/{id}/gamesV2      → 200, every game
```

On that second endpoint `accessFilter=2` is the *public* filter and returned
zero for the group measured; `accessFilter=1`, and omitting the parameter,
returned four. Unfiltered is unfiltered for everybody.

So **the existence, id and name of a universe or place are public information**,
whatever the experience's visibility says. A cookie does not reveal them and
withholding one does not conceal them. What stays behind a session is the
*content*: `develop.roblox.com/v1/places/{id}` answers 404 anonymously, and
nothing in these listings says whether a place is playable.

The reason this belongs on the trust-model page rather than only on
[docs/init.md](./init.md#the-listings-need-no-credential): the table above once
described the cookie as what "reveals the private ones your account can see",
and a reader could reasonably have concluded that not passing one kept an
unreleased project quiet. It never did. If a name would be a problem to
publish, the answer is to change the name, not to withhold a credential.

## What it is never used for

**No live operation.** `servers`, `analytics`, `ban`, `restart`, `data`, `memorystore`, `publish`, `ads` and `probe` take an API key and nothing else. There is no cookie path in any of them to fall back to. Those are the commands that act on players and player data, and keeping them key-only is what makes the scope list on the key the audit trail: a read key cannot ban anybody, whatever calls it. See [the safety model](./ops.md#safety-model).

**Never as a fallback for a missing key.** `place`, `config`, `shop`, `check`, `env` and `open` are key-only or fully offline. No Open Cloud call in the toolkit is retried with the cookie when the key is rejected: a `403` from Open Cloud means the key is wrong, and answering it by escalating to a full account session would defeat the point of having scoped the key at all.

**Never for anything Open Cloud covers.** Every cookie call above exists because Roblox publishes no Open Cloud equivalent. When one appears, the cookie call is the thing to delete.

## How a cookie is resolved

`GlobalFlags::resolve_cookie` in `rbx-core` is the only resolver, and it is consulted in this order:

1. **`--cookie <value>` or `RBX_COOKIE`.** Explicit, and highest priority.
2. **`RBXAPIKEY_COOKIE`.** The one per-tool variable that survived `rbx apikey` becoming a subcommand. Explicit too, so it beats auto-detection.
3. **`--no-auto-cookie`.** If set, resolution stops here with no cookie.
4. **Auto-detection from a local Roblox Studio install** (Windows registry, macOS plist), via the [`rbx_cookie`](https://github.com/blake-mealey/mantle/tree/main/rbx_cookie) crate. What it finds is a **candidate**, not yet a credential: see [auto-detection is opt-in](#auto-detection-is-opt-in).

Four points where the order is load-bearing rather than arbitrary:

- **Steps 1 and 2 come before the `--no-auto-cookie` check, deliberately.** The flag governs auto-detection, which is the only step you did not ask for. It was never meant to suppress a variable you set on purpose.
- **`RBXAPIKEY_COOKIE` beats auto-detection.** It used to lose, because `rbx apikey` only reached it after the shared resolver returned nothing, which on any machine with Studio installed never happened. A variable set on purpose was silently overridden by a cookie nobody asked for.
- **An empty `RBX_COOKIE=` is an answer, not an absence.** It counts as explicit and stops the Studio lookup. A command that genuinely needs a cookie will then send the empty one and be refused by Roblox, rather than stopping locally with the "no cookie" message. An empty `RBXAPIKEY_COOKIE` is treated as unset instead: it has no flag to spell "no cookie" with, so an empty one is more likely a leftover in a shell profile than an instruction.
- **The Studio lookup happens in exactly one place.** It used to happen in two, and the second site did not know about `--no-auto-cookie`, so every `rbx apikey` subcommand read the session cookie with no working way to refuse. An escape hatch that silently does nothing is worse than no escape hatch, because you believe you opted out. Two lookup sites are what made that divergence possible; one cannot diverge from itself.

### Auto-detection is opt-in

Finding a signed-in Studio on the machine is not the same as being allowed to send its session. So step 4 produces a candidate, and something has to say yes before it is sent:

| The run | What happens |
| --- | --- |
| `--auto-cookie` passed | Sent. The standing yes, for a person who has decided once. |
| A terminal on all three streams | Asked, once, and the answer is remembered for the process. |
| Anything else: CI, a pipe, a cron job, `--json` into a file | **Not sent**, with one line on stderr naming the two ways forward. |

The question is drawn on stderr, never stdout, so it cannot land inside a document a command is emitting:

```
Roblox Studio is signed in on this machine. Send that session cookie? It is a full-account credential, more powerful than any API key. [y/N]
```

Once per process rather than once per call, because a command that builds three clients would otherwise ask three times for one decision. Anything other than `y` or `yes` is a no, including an answer that could not be read at all: an unanswerable question is not consent.

A run with nowhere to ask says so rather than failing silently:

```
a Roblox Studio session is signed in on this machine and was not used: sending it needs --auto-cookie, or set RBX_COOKIE to a value you chose. Nothing was sent.
```

**This is the property worth having in CI.** A runner that happens to have a developer's Studio profile on it cannot reach into that session by accident, whatever the job does. `--no-auto-cookie` remains the standing no, and clap refuses the two flags together, so a stray `--auto-cookie` in one invocation cannot quietly undo a `--no-auto-cookie` set in a profile.

### Auto-detection announces itself

Once the answer is yes, the first cookie produced by step 4 puts one line on stderr:

```
using the Roblox Studio cookie (--auto-cookie to stop asking; --no-auto-cookie or RBX_COOKIE= to refuse)
```

It names the yes as well as the two noes, and that is not symmetry for its own sake. Detection is opt-in, so the person reading this line has just been asked and answered; they are exactly who `--auto-cookie` exists for, and this sentence is the only place they would learn it exists. A notice that lists only the ways to refuse teaches half the control.

Once per process, not once per call, because a single command can build two or three clients and three identical notices read like three separate reads of the credential. Commands that only used the API key print nothing, so the line appearing at all is the signal.

The username is deliberately not in the sentence. Naming it would mean a network round trip to `users.roblox.com` on every command that touches a cookie, which is a real cost for a nicer sentence, and the notice is printed at resolution, before anything has decided whether this run will talk to Roblox at all. The commands that are about to write with the cookie do make that call, one line later (see [what is checked](#what-is-checked-and-when)); `rbx doctor` and `rbx apikey list --remote` will name the account when you want to know.

Both controls in that sentence work, and both are tested: `--no-auto-cookie` skips the lookup, and `RBX_COOKIE=` counts as explicit. In CI the line never appears at all, because a run with nowhere to ask does not reach it. Set `RBX_COOKIE` there only for the few commands that need it, from a secret store, and prefer arranging the pipeline so that none of them do.

`rbx doctor` reports which of these you are in: explicit, auto-detected (a `!`, not a `✓`), or absent.

### What is checked, and when

Resolution itself still answers only "is there a cookie". The check is a separate step, and it happens where an unusable cookie would otherwise do damage: **one `users.roblox.com/v1/users/authenticated` call before the first write that needs the cookie**. An expired session becomes a refusal that changes nothing, instead of a failure partway through (#63).

These commands check, because these are the ones that write with it:

| Command | What the check prevents |
| --- | --- |
| `rbx meta sync`, when the plan contains at least one cookie-only field | The Open Cloud half landing and the legacy half not. A plan with nothing cookie-only skips the check: it writes only through Open Cloud. |
| `rbx init create-group` | Paying 100 Robux for a call that is about to be refused. |
| `rbx init create-universe`, `create-place` | An irreversible creation followed by a rename that fails, leaving a resource named after nothing. |
| `rbx init rename-place`, `rename-universe` | A read that answers for a public resource being mistaken for proof the rename will be accepted. |
| `rbx apikey create`, `prune` | Nothing extra: both already ask who the session belongs to for their own reasons, and that is now the same cached call. |
| `rbx apikey update`, `regenerate`, `delete` | `--all` walking the list and stopping halfway, with some keys rotated or deleted and the rest not. |

These do not check, and pay no round trip for it: `rbx meta init`, `meta pull`, `meta check`, every `rbx init list-*`, `rbx apikey list`, `status`, `can-manage`, `introspect`, `resolve`, `rbx download`, `rbx import`. They read, or they attach the cookie only to reveal more of a read, so a refusal costs an output that did not print and leaves nothing behind. `rbx doctor` does check, and reports it as a line of its own, because "is my cookie still good" is the question it exists to answer.

Four rules the check follows:

- **Once per run.** The verdict is cached for the life of the process, keyed on the cookie. A `meta sync` that checks before the prompt and again before the first write asks Roblox once, and `apikey create` gets its creator id out of the same answer.
- **No answer is not a refusal.** Being offline, a 5xx, a rate limit or anything else that is not a flat rejection leaves the check unanswered: the run prints one warning line and carries on, and the calls themselves report the network for what it is. Turning an unreachable host into "your session expired, sign in again" sends you to re-authenticate a session that was fine.
- **The message names the state and the way out.** A refusal says the session expired and how to renew it, in both directions (sign in to Studio again, or supply a fresh `--cookie` / `RBX_COOKIE`), rather than quoting a status code you cannot act on.
- **An empty cookie is answered locally.** `RBX_COOKIE=` is a deliberate "no cookie", so a command that needs one says exactly that instead of sending an empty value and reporting the refusal as an expired session. No request is made.

### The confirmation names the account

Every cookie-authenticated write asks its question *as* somebody:

```
⚠ As builderman (156): create universe 'My Game' under group 1234567 and record it as [test]? [y/N]
```

It costs nothing. The check above has just identified the account, the verdict is cached for the process, and the prompt reads it back rather than asking again. A run with no cookie, or one whose check could not answer, prompts exactly as it did before: the tool never claims an identity nothing established.

This is the cheap half of the identity problem below. Auto-detection follows whichever account Studio is signed into, silently, and the same key names recur across accounts, so "wrong account" is the realistic mistake here, and a question about a group id or a key name cannot catch it. Naming the account turns it into something a person can answer.

It also moved the session check ahead of the prompt everywhere, which is worth having on its own: being asked to approve an irreversible creation and only then learning the session was dead wastes the decision.

Two things are still not checked:

- **Shape.** Beyond empty, the value is taken as given; the only normalisation is prefixing `.ROBLOSECURITY=` when the raw value does not already carry it. The check sends the same header the later calls will send, so a value Roblox reads as nonsense comes back as a refusal rather than as a local guess about its format.
- **Identity.** Signing into a different account in Studio changes which account auto-detection finds, silently and for every later command. The check names the account in `rbx doctor`, but nothing cross-checks that it is the account that owns the key in `RBX_API_KEY`, so that mismatch still reads as a permission error on the resource rather than as "wrong account".

## It is never written to disk

`rbx` reads the cookie from one of the four sources above, holds it in memory for the life of the process, and sends it as a `Cookie` header to the Roblox host of the call that needs it. Nothing else happens to it. Specifically:

- **No file the tool writes ever receives it.** What `rbx` writes is `rbxplace.toml`, the per-tool config and lockfiles, generated Luau modules, downloaded assets, shell completions, and (for `rbx apikey`) key secrets into the secret backend you configured. None of those code paths has the cookie in scope, and none of the config formats has a field it could land in. The config files are meant to be committed, which is the reason to keep it that way.
- **Nothing logs it.** The workspace has no logging framework, no debug dump of request headers, and the only cookie-related line printed anywhere is the fixed notice above, which contains no value.
- **It stays out of `--help`.** `--cookie` and `--api-key` are declared so that clap prints the flag without the current value of the environment variable behind it. Without that, every pasted help output, CI log and screenshot of a help page would carry the credential.
- **No cache, no keychain write, no session file.** The tool has no cookie store of its own to populate, and nothing sends the cookie to any host other than the Roblox endpoint of the call being made.

The deliberate contrast is API key secrets, which *are* written to disk, because that is the point of them: to the secret backend `rbxapikey.toml` names, with `rbxapikey.lock.toml` to be gitignored for exactly that reason, which `rbx apikey create` refuses to proceed without. A key you can write down and rotate per project is the credential this toolkit wants you to be using. See [docs/apikey.md](./apikey.md).

## Turning it off

```sh
rbx meta pull --env prod --no-auto-cookie   # skip the Studio lookup for one run
export RBX_COOKIE=                          # skip it for the whole shell
```

Either way, the commands in the table above that are marked required will fail, and the error names the three ways to supply one rather than leaving you with whatever Roblox said about an unauthenticated request. The ones marked optional carry on and report what they could not read, which is the outcome to prefer: an import or a pull that says "these fields need a cookie" is recoverable, one that silently records defaults is not.

## Related

- [docs/ops.md](./ops.md) - the API key posture the cookie is the exception to, and the scope table per subcommand
- [docs/apikey.md](./apikey.md) - the key workflow, and why key administration is cookie-authenticated
- [docs/init.md](./init.md) - the creation commands, per-command cookie requirements
- [docs/meta.md](./meta.md#cookie-only-fields) - the exact list of cookie-only metadata fields
- [docs/import.md](./import.md) - what an import without a cookie leaves unset
- [docs/download.md](./download.md) - backend selection, and which one sends what
- [docs/doctor.md](./doctor.md) - the credential report, cookie source included
