# rbx message

Send one MessagingService message to every running server.

**Not `rbx publish`.** "Publish" already means a deploy everywhere else here — `place upload --published` publishes a place, `config sync` publishes a config document, `place rollback` republishes live. A command called `publish` that sends an IPC message is the one somebody finds when they search for how to publish their place, and `rbx publish --topic cache --message reload` is plausible enough that they might not notice.

The push half of the pair [`memorystore`](./memorystore.md) is the pull half of. A memory store item is read when a server's own code decides to look, so a value written from outside appears on whatever polling interval the experience already has. This is what tells the servers to look now.

See [ops.md](../ops.md) for install, keys and the safety model.

This page describes `main`. Check `rbx --version` against what your `rokit.toml` pins.


Needs `universe-messaging-service:publish`, which is universe-targeted and not on most existing keys — a 403 here usually means the scope, not the key.

## Why an ops CLI sends these at all

Publishing is normally server-to-server IPC and belongs in game code, which is why this was left out at first. That reasoning assumes the publisher is a game server. It is not, in the case this exists for: the publisher is a VPS, a cron job or a deploy step that has just changed something and wants the running servers to notice. There is no in-experience way to originate that.

## Sending

```sh
# described, not sent
rbx message --topic cache --message reload --env prod

# for real
rbx message --topic cache --message reload --apply --env prod
```

`--topic` is the name the game passes to `MessagingService:SubscribeAsync`.

## The message is a string, not JSON

Roblox types `message` as a string. A structured payload therefore travels as text and is decoded in-experience:

```sh
rbx message --topic cache --payload '{"key":"rotation"}' --apply --env prod
```

`--payload` parses the value here and sends its serialisation, so a malformed payload fails before the publish rather than inside `HttpService:JSONDecode` on a live server, where it is a runtime error in game code with no obvious origin. `--message` sends whatever you give it, untouched.

`--json` is not this flag. On this command as on every other, `--json` writes the result as a document, and a payload flag that also took a value would have kept that name occupied — leaving `publish` the one command that cannot report what it did.

## The size limit is 1114 bytes, not 1 KB

Measured against the live API rather than read off the documentation, which says 1 KB. 1114 is accepted, 1115 answers:

```
400 The length of published message must be between 1 and 1114.
```

Both bounds are checked here, before the request, so the failure names the size instead of arriving as a 400 from inside a deploy. The floor matters too: an **empty** message is refused, so publishing `""` as a bare "something changed" signal fails — send a single character instead.

**Publish a reference, not a payload.** The pattern this is built for is: write the value into a memory store sorted map, then publish the key. The message stays small, the value can be any size, and a server that missed the message still finds the value on its next read.

```sh
rbx memorystore --map Cache set rotation --file rotation.json --ttl 1h --apply --env prod
rbx message --topic cache --payload '{"key":"rotation"}' --apply --env prod
```

## `--json`

The receipt. A publish cannot be recalled and Roblox reports nothing about who received it, so this is the only record that the call went out.

```sh
rbx message --topic cache --message reload --apply --json --env prod
```

```json
{
  "schema_version": 1,
  "topic": "cache",
  "universe_id": "5544332211",
  "bytes": 6,
  "applied": true
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Bumped when a documented field changes meaning or disappears. |
| `topic` | string | The topic, as the experience passes it to `SubscribeAsync`. |
| `universe_id` | string | A string, not a number: universe ids exceed 2^53. |
| `bytes` | integer | Encoded length of the message, which is what the limit applies to. |
| `applied` | boolean | `true` once it was sent. `false` on a dry run, which is the default. |
| `message` | string | The body that **would** be sent. Present on a dry run, absent once it has been. |

`--json` is a format, not an `--apply`: without `--apply` it still sends nothing and answers `applied: false`.

`message` follows the invocation rather than the data. A dry run prints the body it would send, so the document carries it; `--apply` prints only that it went, so the document does not. Echoing a published payload back to stdout would put it in whatever captured the log, for a line the command itself decided not to print.

A run that fails writes **nothing** to stdout: a malformed payload, an oversized message, an env that resolves to no universe, or a publish Roblox refuses. An empty stdout next to a non-zero exit says the publish did not happen without a consumer having to read a field to find out.

## What it cannot tell you

Whether anybody heard.

The call answers `200` once Roblox has accepted the message for delivery. There is no count of servers reached and no delivery receipt, and an experience with no running servers accepts a publish exactly like a busy one. The command says so on every success rather than letting a green tick imply more than it means.

Anything needing confirmation needs the servers to write back somewhere — which is what a memory store map is for.
