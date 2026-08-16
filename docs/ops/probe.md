# rbx probe

A raw authenticated request to any Open Cloud path, printing the response.

See [ops.md](../ops.md) for install, keys and the safety model.

**Hidden from `rbx --help` on purpose.** This is the tool for working out what an undocumented endpoint returns while writing a typed client for it, and for looking at raw bytes when one starts behaving oddly. It is not part of daily work, and listing it beside `servers` and `ban` would suggest otherwise. It is fully supported: `rbx probe --help` works, and so does everything below.

This exists because several endpoints worth using are in beta and absent from the Open Cloud reference. Writing a typed client against a schema guessed from a forum post is how you ship a parser that silently drops a field. `probe` fetches the truth first.

```sh
rbx probe "cloud/v2/universes/{universe}" --env prod
```

`{universe}` is replaced with the universe id of the resolved env, so you rarely need to paste ids.

`--universe-id <id>` works instead of `--env`, which is usually what you want here: the endpoints `probe` exists to explore are the ones no config knows about yet, and often belong to a universe that has no env at all.

```sh
rbx probe "cloud/v2/universes/{universe}" --universe-id 66778899001
```

```sh
# A write is described, not performed
rbx probe "cloud/v2/universes/{universe}/user-restrictions/123" \
  -X PATCH -d '{"gameJoinRestriction":{"active":true}}' --env test

# Actually send it
rbx probe ... --apply --env test
```

| Flag | Meaning |
| --- | --- |
| `-X, --method` | HTTP method. Anything but GET also needs `--apply`. |
| `-d, --data` | JSON body. Parsed before sending, so a typo fails here rather than as a confusing 400. |
| `--apply` | Actually send a non-GET request. |

## Gotcha on Git Bash for Windows

`probe /cloud/v2/...` arrives as `C:/Program Files/Git/cloud/v2/...`. That is MSYS rewriting anything that starts with `/` into a Windows path, not a bug in the tool.

**Drop the leading slash**: `probe cloud/v2/...` works everywhere. `MSYS_NO_PATHCONV=1` also works. PowerShell and real POSIX shells are unaffected.

---
