# rbx memorystore

Read and write memory store sorted map items.

The one Open Cloud storage surface a game server can read without an HTTP call. Something outside Roblox (a VPS, a cron job, a dashboard) writes a value here, and every running server picks it up through `MemoryStoreService:GetSortedMap()` without paying a data store round trip for it.

See [ops.md](../ops.md) for install, keys and the safety model.

This page describes `main`. Check `rbx --version` against what your `rokit.toml` pins.


Needs `memory-store.sorted-map:read` to read and `:write` to write. Those scopes are **universe-targeted**, like the data store ones: a key restricted to one experience stays restricted to it.

## Sorted maps only

Queues are the other half of the memory store and a different shape of problem: ordering, claiming, discarding, visibility timeouts. Nothing has needed them yet, and inventing a queue CLI before there is a queue to drive is how you ship the wrong verbs. When one is needed it belongs here as a second mode.

`flush` (which empties an experience's entire memory store) is also absent. It is a single irreversible call affecting every map and queue at once, and it is not part of writing a cache value.

## Writing a value

```sh
# described, not sent
rbx memorystore --map Cache set rotation \
  --value '{"map":"desert","weight":3}' --ttl 300s --env prod

# for real
rbx memorystore --map Cache set rotation \
  --value '{"map":"desert","weight":3}' --ttl 300s --apply --env prod
```

`--file path.json` reads the value from a file instead, which is easier than quoting JSON in a shell.

**`set` is an upsert.** Creating and updating are the same request, so the first write to a new item is not a special case and two writers racing both land as writes. There is no separate "create map" step either: naming a map that has never existed is normal, and the map comes into being on its first write.

### TTL

```sh
--ttl 300s    --ttl 10m    --ttl 2h
```

Omit it and the item stays until something removes it. For a cache that is usually wrong: a TTL is what stops a stale value outliving whatever was producing it. The response reports the computed expiry:

```
✓ wrote "rotation"
  expires 2026-08-13T09:43:49Z
```

### Sort keys

`--sort-key <number>` or `--string-sort-key <text>` set the ordering used by `list`. They are part of the sorted-map model rather than an extra: a map with no sort key is still a sorted map, it just has nothing to sort by.

## Reading

```sh
rbx memorystore --map Cache get rotation --env prod
rbx memorystore --map Cache get rotation --out value.json --env prod
```

`get` prints the value as JSON on stdout and the expiry on stderr, so a pipe gets the value alone. A missing item is an error rather than `null`, so a script that reads a key which was supposed to be there stops instead of carrying on with nothing.

```sh
rbx memorystore --map Cache list --env prod
rbx memorystore --map Cache list --values --limit 500 --env prod
```

`list` prints ids with their sort keys and expiry; `--values` adds each value. It follows pages up to `--limit`.

**An empty listing is not proof the map is empty.** A map that has never been written to answers exactly the same way as one whose items have all expired. There is no way to tell them apart, and no reason to need to.

## Machine-readable output

`--json` on `get` and `list` writes one JSON document to stdout and nothing else. The expiry line, the item count and the "map is empty" line go to stderr, where they cannot corrupt it.

The writes do not take it. `set` and `delete` report what they did in prose, and nothing in this crate needs to script off that.

```sh
rbx memorystore --map Cache get rotation --env prod --json
```

```json
{
  "schema_version": 1,
  "map": "Cache",
  "item": "rotation",
  "expire_time": "2026-08-15T09:43:49Z",
  "value": { "map": "desert", "weight": 3 }
}
```

The cached value is nested as JSON under `value`, not escaped into a string: `jq .value.map` works and a stored number stays a number. `expire_time` is **absent** when the item has no TTL, which is a different fact from an expiry of null. A missing item is still an error rather than a document saying so, exactly as in the human form: a script reading a key that was supposed to be there should stop. `etag` and `path` are on every item Roblox returns and in neither document, because the human form has never printed them.

```sh
rbx memorystore --map Cache list --values --limit 500 --env prod --json
```

```json
{
  "schema_version": 1,
  "map": "Cache",
  "limit": 500,
  "limit_reached": false,
  "count": 2,
  "items": [
    {
      "id": "rotation",
      "numeric_sort_key": 3.5,
      "expire_time": "2026-08-15T09:43:49Z",
      "value": { "map": "desert" }
    },
    { "id": "banner", "string_sort_key": "zulu", "value": "hello" }
  ]
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Document format, shared with `rbx check --json`. `1` today. Refuse a version you do not understand |
| `map` | string | The sorted map, as `--map` named it |
| `limit` | integer | The `--limit` in force |
| `limit_reached` | boolean | The run stopped at `--limit`, not at the end of the map. Raise it to see the rest |
| `count` | integer | Rows in `items` |
| `items[].id` | string | The item id. **Absent** when Roblox sent none, which the human listing prints as `<no id>` |
| `items[].numeric_sort_key` / `.string_sort_key` | number / string | At most one of the two, since the flags that set them are exclusive |
| `items[].expire_time` | string | **Absent** when the item has no TTL |
| `items[].value` | any | **Absent** without `--values`. The same flag decides it in both formats, so nothing reads a value it did not ask for |

An empty map is an empty `items` array and exit 0, not silence, so `.count` reads the same whether the command found nothing or printed nothing. It still cannot tell a map that was never written to from one whose items have all expired, for the reason above: nothing can.

```sh
# the ids about to expire in the next five minutes
rbx memorystore --map Cache list --limit 500 --env prod --json \
  | jq -r --arg t "$(date -u -d '+5 min' +%Y-%m-%dT%H:%M:%SZ)" \
      '.items[] | select(.expire_time != null and .expire_time < $t) | .id'
```

## Deleting

```sh
rbx memorystore --map Cache delete rotation --apply --env prod
```

For removing a value before its TTL runs out. Without `--apply` it describes what it would delete.

## Why there is no confirmation prompt

`rbx data` prompts before overwriting and writes a backup file first, because a player profile is irreplaceable and an overwrite destroys the previous value. None of that holds here: these items are a cache, they carry a TTL, they are rebuilt from whatever produced them, and the thing writing them is a script on a schedule rather than a person at a terminal. A prompt would sit exactly where nobody can answer it.

Writes still need `--apply`, because that is the rule for every live-operations command and a rule with exceptions is not one you can rely on when you are tired.

## Servers do not learn about a write until they look

Nothing wakes a running server when a value changes. A server reads the map when its own code decides to, so a value written here appears on whatever polling interval the experience already has.

That is a fact about the servers, not about this command: `set` and `delete` above write through `memory-store.sorted-map:write`, and they write immediately.

Pushing the change to servers immediately is MessagingService's job, and [`rbx message`](./message.md) sends that message. The pairing is a memory store item for the value and a publish for the nudge:

```sh
rbx memorystore --map Cache set rotation --file rotation.json --ttl 1h --apply --env prod
rbx message --topic cache --payload '{"key":"rotation"}' --apply --env prod
```

Publish a reference rather than the value itself. The message is capped at 1114 bytes, the item is not, and a server that missed the message still finds the value on its next read.

## Two API details worth knowing

Both cost a request to discover, and both are handled for you:

**The item id is a query parameter.** `POST .../items` with `{"id": ...}` in the body answers `400 INVALID_ARGUMENT "The id field is required."`: an error naming the field you just sent. The id belongs in the URL.

**Roblox signals "no more pages" with an empty string here**, where the data store endpoints use `null`. Treating `""` as a real page token fetches the same page forever.
