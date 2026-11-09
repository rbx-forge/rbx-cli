# rbx probe

Send an authenticated request to any Open Cloud path and print the response: for endpoints in beta, absent from the reference, before writing a client for them. Hidden from `rbx --help`, fully supported.

```sh
rbx probe "cloud/v2/universes/{universe}" --env prod
rbx probe "cloud/v2/universes/{universe}" --universe-id 66778899001
```

`{universe}` is replaced with the target universe's id. The key's scopes are the limit: use a read-only key unless you are exploring a write.

```sh
# described, not sent
rbx probe "cloud/v2/universes/{universe}:publishMessage" \
  -X POST -d '{"topic":"announce","message":"hello"}' --env test

# sent
rbx probe ... --apply --env test
```

| Flag | Meaning |
| --- | --- |
| `-X, --method` | HTTP method. Anything but GET also needs `--apply` |
| `-d, --data` | JSON body, parsed before sending |
| `--apply` | Send a non-GET request |

On Git Bash for Windows, drop the leading slash (`cloud/v2/...`, not `/cloud/v2/...`): MSYS rewrites `/...` into a Windows path. `MSYS_NO_PATHCONV=1` also works.
