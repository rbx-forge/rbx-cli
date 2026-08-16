# rbx ban

Who is allowed into the experience. Reading is free; writing is deliberately awkward.

See [ops.md](../ops.md) for install, keys and the safety model.

## Naming a player

Every subcommand here accepts any of these, mixed freely:

```text
156                                        a user id
builderman                                 a username
name:12345                                 a username that looks like an id
@12345                                     the same, but see the PowerShell note
https://www.roblox.com/users/156/profile   a link pasted from a report
```

**On PowerShell, use `name:` and not `@`.** `@` is the splatting operator there, so a bare `@builderman` expands to a variable that does not exist and the argument vanishes before the program is reached, which shows up as `the following required arguments were not provided`. `"@builderman"` quoted works too. `name:` needs no quoting in any shell.

Bare digits mean an **id**, because that is what they almost always are. Roblox does allow all-digit usernames, so `@` forces the name reading.

A username that does not exist is an **error**, never a silent skip:

```text
Error: no Roblox user is named: ThisNameDoesNotExist99999x. Usernames are not
display names; pass a user id if you are unsure.
```

That matters more than it looks. Roblox's lookup endpoint simply omits names it cannot find, so asking about three players and hearing about two is the only signal that one was wrong. And the name people paste from a Discord report is usually a **display name**, which is not unique and is not what this resolves.

## Checking someone

```sh
rbx ban status builderman --env prod
```

```text
builderman (156)
  https://www.roblox.com/users/156/profile
  RESTRICTED  for 7d
  since        2026-08-01T10:12:03Z
  your note    exploit: fly hack, clip of 3 Aug
  player sees  Banned 7 days for cheating
```

Two reasons are stored: `your note` is private and for your records, `player sees` is shown to them on their next join attempt.

## Listing and auditing

```sh
rbx ban list --env prod              # everyone currently restricted
rbx ban list --env prod --include-inactive
rbx ban logs --env prod              # the audit trail
```

`list` returns ids, not names: the endpoint does not send names, and resolving every row would be one extra request per player. Run `ban status <id>` on a row you care about.

## Restricting

```sh
rbx ban add builderman \
  --reason "exploit: fly hack, clip of 3 Aug" \
  --display-reason "Banned 7 days for cheating" \
  --duration 7d \
  --env prod
```

Nothing is sent. You get the resolved account, links to check it, and the exact payload:

```text
restrict 1 player(s):
  builderman (156)
    https://www.roblox.com/users/156/profile

{
  "gameJoinRestriction": {
    "active": true,
    "duration": "604800s",
    "privateReason": "exploit: fly hack, clip of 3 Aug",
    "displayReason": "Banned 7 days for cheating"
  }
}

Nothing sent. Re-run with --apply to perform it.
```

Add `--apply` and you also get a y/N prompt. `--yes` skips the prompt, for scripts.

Two gates rather than one, because the input to this command is a name typed by a person under pressure and the output is a real player locked out. The links are the point: open the profile, confirm it is the right account, then apply. There is more than one `Builderman`.

| Flag | Meaning |
| --- | --- |
| `--reason` | Required. Private, 1000 characters. A ban you cannot explain in six months is a ban you cannot defend. |
| `--display-reason` | Shown to the player, 400 characters. |
| `--duration` | `30m`, `12h`, `7d`, `2w`. **Omit for permanent.** |
| `--allow-alts` | Let alt accounts through. Roblox propagates a restriction to linked alts **by default**, which is what you want for an exploiter; this turns that off. Named for what it does, not after Roblox's `excludeAltAccounts` field, where `true` means "do not propagate". |
| `--apply` | Actually send it. |
| `--yes` | Skip the prompt. |

Permanent is expressed by leaving `--duration` out, and `--duration permanent` is rejected on purpose: the harshest outcome should be reachable by deliberately omitting something, not by a word you could mistype.

## Lifting a restriction

```sh
rbx ban remove builderman --env prod --apply
```

## Machine-readable output

`--json` on the two reads — `status` and `list` — writes one JSON document to stdout and nothing else. Everything that is not the result (the count, the "names are not returned" note, the unknown-key warning from `rbxplace.toml`) goes to stderr, so `jq` reads the pipe and a human still reads the terminal.

`add` and `remove` do not take it. Both stop and ask before they act, and a format that owns stdout cannot stop and ask: the prompt would land in the document, or in a pipeline where nobody can answer it. So the flag is not there to be refused at runtime, it does not exist on those subcommands at all. `logs` has no document yet either; the audit trail is worth one and nobody has asked.

### Permanent is stated, never implied

Roblox expresses a permanent restriction by sending no duration at all. That is the one place where a missing field means the *worst* outcome rather than "nothing to report", and a consumer reading `.duration // "none"` would report a permanent ban as no ban. So every restriction carries `permanent`, and `duration` — Roblox's own `604800s`, not the `7d` the table renders — is absent exactly when `permanent` is true.

### `rbx ban list --json`

```json
{
  "schema_version": 1,
  "env": "prod",
  "universe_id": "5544332211",
  "include_inactive": false,
  "limit": 100,
  "limit_reached": false,
  "count": 2,
  "restrictions": [
    {
      "user_id": "156",
      "active": true,
      "permanent": false,
      "duration": "604800s",
      "private_reason": "exploit: fly hack, clip of 3 Aug"
    },
    { "user_id": "881", "active": true, "permanent": true }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today. Refuse a version you do not understand |
| `env` | string | The env that named the universe. **Absent** under a bare `--universe-id` |
| `universe_id` | string | The experience this is a listing of |
| `include_inactive` | boolean | Whether entries that exist without locking anybody out were included |
| `limit` | integer | The `--limit` in force, a maximum and not a promise |
| `limit_reached` | boolean | The walk stopped at `--limit` rather than at the end of the listing. Raise it to see the rest |
| `count` | integer | Rows in `restrictions` |
| `restrictions[].user_id` | string | **Absent** when the id could not be read out of the resource path, which the table prints as `?` |
| `restrictions[].active` | boolean | False only under `--include-inactive`; without this field the two kinds of row are indistinguishable |
| `restrictions[].permanent` | boolean | Nothing will lift this restriction |
| `restrictions[].duration` | string | As Roblox sent it. **Absent** when `permanent` |
| `restrictions[].private_reason` | string | Your note, the `REASON` column. **Absent** when there is none |

No names: the endpoint does not send them, which is the same reason the table prints ids. Nobody restricted is `"count": 0`, an empty array and exit 0, never silence.

### `rbx ban status --json`

One document for every player asked about, in the order they were given.

```json
{
  "schema_version": 1,
  "env": "prod",
  "universe_id": "5544332211",
  "count": 1,
  "players": [
    {
      "user_id": "156",
      "username": "builderman",
      "display_name": "builderman",
      "profile_url": "https://www.roblox.com/users/156/profile",
      "restricted": true,
      "permanent": false,
      "duration": "604800s",
      "start_time": "2026-08-01T10:12:03Z",
      "private_reason": "exploit: fly hack, clip of 3 Aug",
      "display_reason": "Banned 7 days for cheating"
    }
  ]
}
```

`permanent`, `duration`, `start_time` and both reasons are **absent** for a player who is not restricted. `permanent` in particular is absent rather than false there: false would read as "restricted, but not for ever". A lifted restriction leaves its record behind on Roblox, reasons included, and reports only that it is lifted, which is what the human form prints too.

### What these documents do not say

They are about real players locked out of a real game, so they say no more than the human form already says out loud.

**`list` carries no `display_reason` and no `start_time`.** The listing prints neither, and the text a banned player is shown is not a field a monitoring job asked for. `status` prints both, under `player sees` and `since`, and carries both.

**`inherited` and `exclude_alt_accounts` are in neither.** They are on every restriction Roblox returns and nothing here has ever printed them, so nothing promises them. `path` and `update_time` are absent for the duller version of the same reason: unprinted today, so unpromised today.

## Scopes

| Subcommand | Scope |
| --- | --- |
| `status`, `list`, `logs` | `universe.user-restriction:read` |
| `add`, `remove` | `universe.user-restriction:write` |

Put the write scope on a **separate key** from everything else. The read key is the one that ends up in your shell history.

Retries are safe: each write carries an idempotency key, so a request that times out and is retried is recognised by Roblox as the same operation rather than applied twice.

---
