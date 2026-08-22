# rbx secret

Store the credentials an experience needs and the repository must never hold.

A secret is a value the running game uses (a Discord webhook, a payment provider's key, the token for whatever service a `DataStore` reconciles against) that has no business being in a `.lua` file, a `.toml` file, or anybody's clipboard. Roblox keeps it per universe and hands it to the server as a `Secret` userdata that Luau can *send* but cannot read, print, or concatenate.

```lua
local HttpService = game:GetService("HttpService")

HttpService:RequestAsync({
    Url = "https://api.example.com/v1/orders",
    Method = "POST",
    Headers = { Authorization = HttpService:GetSecret("api_token") },
})
```

Before this command, the only way to put a value there was the Creator Dashboard, by hand, one universe at a time. That is exactly the shape of chore that goes wrong quietly: staging keeps last quarter's key because somebody rotated production and stopped there.

This page describes `main`. Check `rbx --version` against what your `rokit.toml` pins.

## The value is encrypted before it is sent

Roblox does not accept a secret in the clear, even over TLS. The content is sealed against the universe's own public key first, so what crosses the wire (and what lands in any corporate proxy's log along the way) is ciphertext that only this universe can open.

`rbx secret set` does that for you. It fetches the universe's public key, seals the value with a **LibSodium sealed box**, and sends the result base64-encoded. A sealed box generates a throwaway keypair for each call and discards the private half, which means the ciphertext cannot be opened by anything, *including the process that produced it*. That is what makes this safe to run in CI: a sealed value in a build log is not a leak.

Two consequences worth knowing:

- Every write is two requests, because the public key has to be fetched first.
- The write carries the `key_id` of the key it sealed against. If Roblox has rotated the key in between, the write fails cleanly rather than storing something that can never be decrypted.

## Writing a secret

```sh
# described, not sent
rbx secret set api_token --stdin --domain api.example.com --env prod

# for real
printenv API_TOKEN | rbx secret set api_token --stdin --domain api.example.com --apply --env prod
```

Three ways to give the value, in descending order of how much you should like them:

| Flag | Reads from | Notes |
| --- | --- | --- |
| `--stdin` | standard input | One trailing newline is stripped, because a pipe adds one |
| `--file <path>` | a file, byte for byte | Nothing is trimmed: a PEM key keeps its trailing newline |
| `--value <text>` | the command line | Readable in the process list, and recorded by command-line auditing |

`--value` prints a note on stderr saying so.

The exposure is the command line, not the shell history, and the difference matters because the history is what people check. `--value $(bw get password token)` stores *the substitution* in history rather than its result (that is true of PSReadLine, bash and zsh alike) so the history looks clean and the warning looks like a false alarm. Meanwhile the expanded value really did travel as an argument to the process: readable in the process list by anything running as you, and captured whole by command-line auditing (Windows event 4688, Sysmon, an EDR agent, `auditd`). Those logs persist, and on a managed machine they leave it.

`--value` is there for a scratch terminal; `--stdin` is there for everything else.

### A Windows trap worth knowing before you pipe

`--stdin` is the right answer, with one caveat on Windows. Windows PowerShell 5.1 routes a pipe between two native programs through a text conversion whose `$OutputEncoding` defaults to `us-ascii`, so every non-ASCII byte becomes `?`:

```
p + é + é   →   powershell 5.1   →   70 3f 3f 3f 3f 0d 0a     ("p????" + CRLF)
p + é + é   →   pwsh 7           →   70 c3 a9 c3 a9           (intact)
```

The trailing CRLF is stripped for you; the `?` substitution is not, and cannot be: it happened before the value reached this process. Since Roblox never hands the value back, a credential mangled this way stores cleanly and only surfaces in the game as the far end answering `401`.

Use `pwsh` 7, where the pipe is byte-exact, and check with `$PSVersionTable.PSVersion` if you are unsure. To see exactly what a pipe delivers before storing anything:

```powershell
bw get password <item> | python -c "import sys;d=sys.stdin.buffer.read();print(len(d), d.hex())"
```

A leading `efbbbf` is a BOM, a trailing `0d0a` is a newline, and any `3f` where you expected an accent is this bug. Compare the length against the `N bytes` that `set` reports: it is the only end-to-end check available, because nothing can read the value back afterwards.

Passing the value as an argument (`--value $(bw get password ...)`) sidesteps the encoding problem entirely, since arguments reach the process as UTF-16 and convert cleanly whatever the shell version. It buys that at the cost of the command-line exposure above. On `pwsh` 7 you need not choose: `--stdin` is exact *and* private.

**`set` is create-or-replace.** There is no separate create step and no error on a name that already exists. Underneath it is a `POST`, falling back to a `PATCH` when Roblox answers `409`, which means it needs no read scope and two pipelines racing on the same secret both land as writes rather than one of them failing.

The output says which it turned out to be:

```
✓ updated secret "api_token"
  38 bytes, domain api.example.com
  read it in Luau with HttpService:GetSecret("api_token")
```

### Names

ASCII letters, digits and underscores, 1 to 64 characters, not starting with a digit. This is Roblox's rule, and it is the string `HttpService:GetSecret("...")` takes.

A name that breaks it is refused here rather than sent, because the `400` that comes back does not say which of the four constraints was broken. The two that catch people: `api-key` and `api.key` both look legal and are not.

### The domain is not optional

Every `set` needs `--domain <pattern>` or `--no-domain`. There is no default, and that is deliberate.

A secret's domain decides which hosts `HttpService` will attach it to:

```sh
--domain api.example.com     # that host
--domain '*.example.com'     # any subdomain
--domain '*'                 # anywhere
--no-domain                  # nowhere: usable for signing inside the server, never sent
```

`--no-domain` is correct for a private key you sign with in-process. It is a trap for an API token: the credential simply never goes out with the request, and you find out in the game, at runtime, from a service answering `401`.

**A `set` replaces the whole secret, domain included.** So the domain has to be restated on every write, including a rotation that only means to change the value. The alternative (carrying the stored domain forward) would mean reading it first, which would make a read scope necessary on a command that a deployment key should be able to run with write access alone.

## Reading

You cannot. Not a missing subcommand and not a missing scope: **Roblox never sends stored content back to anybody.** A listing carries names, domains, key ids and timestamps, and that is all there is.

```sh
rbx secret list --env prod
```

```
api_token  api.example.com  updated 2026-08-02T10:00:00Z
signing    (no domain: server-side use only)  updated 2026-07-11T14:22:03Z
2 secret(s)
```

That is the design working rather than a gap to route around. A secrets store you can read from is a secrets store that one leaked API key drains.

The practical consequence: there is no way to check whether a stored value is the right one. To find out, replace it. It is also why `set` cannot report "no change" the way [`rbx config sync`](./config.md) does: it has nothing to compare against.

## Rotating

```sh
printenv API_TOKEN_NEW | rbx secret set api_token --stdin --domain api.example.com --apply --env prod
```

Same command as the first write. Servers pick the new value up on their next `GetSecret` call; nothing is pushed to them.

**The previous value is gone the moment this succeeds.** Not in a backup file, not in a version history, not behind a support ticket. If the old credential is not in your password manager, it is not anywhere, which is why writes need `--apply`.

Across every environment:

```sh
for env in dev staging prod; do
  printenv "API_TOKEN_${env^^}" \
    | rbx secret set api_token --stdin --domain api.example.com --apply --env "$env"
done
```

Deliberately not `--env all`: one token per environment is the point of having environments, and a loop that reads a different variable each time is harder to run by accident than a flag that fans one value out to production.

## Deleting

```sh
rbx secret delete api_token --apply --env prod
```

Irreversible, and the game stops being able to read it immediately: `GetSecret` on a name that is not there errors, so a live server calling it starts failing rather than degrading. Without `--apply` it says what it would delete and sends nothing.

## Sealing somewhere else

```sh
rbx secret public-key --env prod
```

Prints the base64 X25519 public key on stdout and its `key_id` on stderr. `set` fetches this itself; the subcommand is for the case where the encryption has to happen elsewhere: a deployment system that holds the plaintext and will not hand it to a CLI, or a language binding doing the sealed box itself.

The key is public. Publishing it is what it is for. What you cannot do is seal against a key and submit it with a different `key_id`: Roblox stores what you send and would never be able to decrypt it.

## Machine-readable output

`--json` writes one JSON document to stdout and nothing else. Counts and notes go to stderr, where they cannot corrupt it.

**No document this command emits has a field for secret content**, and that is a property of the types rather than a convention to be careful about: there is nowhere for one to go. `--json` output is the form most likely to be redirected into a file or pasted into an issue.

```sh
rbx secret list --env prod --json
```

```json
{
  "schema_version": 1,
  "universe_id": 7654321,
  "limit": 500,
  "limit_reached": false,
  "count": 2,
  "secrets": [
    {
      "id": "api_token",
      "domain": "api.example.com",
      "key_id": "key-2026-08",
      "create_time": "2026-07-01T09:00:00Z",
      "update_time": "2026-08-02T10:00:00Z"
    },
    { "id": "signing", "key_id": "key-2026-08", "create_time": "2026-07-11T14:22:03Z" }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today. Refuse a version you do not understand |
| `universe_id` | integer | The universe the listing is of |
| `limit` | integer | The `--limit` in force |
| `limit_reached` | boolean | The run stopped at `--limit`, not at the end. Raise it to see the rest |
| `count` | integer | Rows in `secrets` |
| `secrets[].id` | string | The name `GetSecret` takes. **Absent** when Roblox sent none |
| `secrets[].domain` | string | **Absent** for a secret with no domain, which cannot leave the server at all |
| `secrets[].key_id` | string | Which public key the stored value was sealed under |
| `secrets[].create_time` / `.update_time` | string | ISO 8601 |

`set --apply --json` and `delete --apply --json` emit a result document too. A dry run emits none: `--apply` is what makes it a result, and a consumer that got a document either way would have to read a field to find out whether anything happened.

```json
{
  "schema_version": 1,
  "universe_id": 7654321,
  "id": "api_token",
  "action": "updated",
  "bytes": 38,
  "domain": "api.example.com",
  "key_id": "key-2026-08"
}
```

`action` is `created` or `updated`. Worth branching on: `updated` means a previous value is gone. `bytes` is the plaintext length: the nearest thing to a checksum that is safe to print, and enough to catch the classic failure of piping an empty file or a shell variable that never expanded.

Secrets left behind by a key rotation:

```sh
current=$(rbx secret public-key --env prod --json | jq -r .key_id)
rbx secret list --env prod --json \
  | jq -r --arg k "$current" '.secrets[] | select(.key_id != $k) | .id'
```

### Required API scopes

| Operation | Scope |
| --- | --- |
| `list` | `universe.secret:read` |
| `public-key` | `universe.secret:read` |
| `set` | `universe.secret:read` **and** `universe.secret:write` |
| `delete` | `universe.secret:write` |

`set` needs the read scope despite only writing: fetching the public key is a read, and there is no way to seal a value without it.

The scope is **universe-targeted**, so a key restricted to one experience stays restricted to it. Declare it in `rbxapikey.toml` like any other:

```toml
[keys.deploy]
scopes = [
  { type = "universe.secret", operations = ["read", "write"] },
]
```

`universe.secret` is a BETA scope in Roblox's own document. It has been stable in practice, but treat a sudden `403` on a key that worked yesterday as a Roblox-side change rather than a bug in your config, and check [`rbx doctor`](./doctor.md).

## Limits

| | |
| --- | --- |
| Secrets per universe | 500 |
| Name | 1-64 characters, `[A-Za-z_][A-Za-z0-9_]*` |
| Rate | 120 requests per minute, per API key owner |

## Secrets do not reach running servers until they look

Nothing wakes a server when a secret changes. `GetSecret` is read when the experience's own code decides to read it, so a rotation lands on whatever cadence that already has: often "next request", sometimes "next server start" if the result was cached in a variable.

If a rotation has to take effect at a known moment, that is [`rbx message`](./ops/message.md)'s job, the same pairing as a memory store value:

```sh
printenv API_TOKEN_NEW | rbx secret set api_token --stdin --domain api.example.com --apply --env prod
rbx message --topic secrets --payload '{"rotated":"api_token"}' --apply --env prod
```

Never publish the value itself. The message is not encrypted, it is capped at 1114 bytes, and the whole point of the store is that the value travels one way only.

## Two API details worth knowing

Both are handled for you, and both are surprising enough to write down:

**The secrets resource is `snake_case`.** `key_id`, `create_time`, `update_time`, where every other `cloud/v2` surface sends `keyId` and `createTime`. The list envelope around it is not (`secrets` and `nextPageCursor`) and it pages with a `cursor` rather than the `pageToken`/`nextPageToken` pair used everywhere else in `cloud/v2`.

**A create is a `POST` to the collection and an update is a `PATCH` to the item**, and the id lives in the body of one and the path of the other. Sending `id` in a `PATCH` body does nothing: the specification is explicit that a secret's id cannot be changed.
