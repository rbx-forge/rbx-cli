# rbx user

A username to a user id, or an id to a username. No API key, no file: the lookup is public.

```bash
rbx user builderman
rbx user 156 builderman name:12345
rbx user builderman --id
rbx user 156 --name
rbx user builderman --json
```

## Naming a user

Every argument takes what [`rbx ban`](ops/ban.md#naming-a-player) takes, mixed freely:

```text
156                                        a user id
builderman                                 a username
name:12345                                 a username that looks like an id
@12345                                     the same, but see the PowerShell note
https://www.roblox.com/users/156/profile   a pasted profile link
```

**On PowerShell, use `name:` and not `@`.** `@` is the splatting operator there, so an unquoted `@builderman` vanishes before the program sees it.

A name or an id Roblox does not know is an **error** naming it, never a silently missing line. Usernames are not display names: the name people quote from a chat is often the display name, which is neither unique nor what this resolves.

## Output

Without a flag, one line per user, in the order given, with the display name when it differs:

```text
builderman (156)
somebody "Some Body" (1234567890)
```

`--id` and `--name` print the bare value, one per line, in the order given. They exist to compose with commands that take an id inside something else, which `rbx` cannot resolve for you because only the game knows the shape. A data store key is the usual case:

```powershell
rbx data get --datastore PlayerData "User_$(rbx user builderman --id)"
```

`--json` prints an array, each entry with `user_id` (a string, as everywhere in this tool), `username`, `display_name`, `has_verified_badge` and `profile_url`.
