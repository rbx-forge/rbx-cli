# rbx group

A group's roles, and who holds them. Roles are read here; members are moved between them.

See [ops.md](../ops.md) for install, keys and the safety model.

```bash
rbx group roles                          # the group in rbxplace.toml's [owner]
rbx group roles --group 1234567 --json   # any group, no file needed
rbx group rank builderman Moderator      # give a member a role
rbx group unrank name:12345 2788109      # take one away, by role id
```

## Which group

`--group <id>` names it, and works with no file on disk at all.

Without the flag, the group comes from `rbxplace.toml`: its `[owner]`, or an env's own `[envs.<name>.owner]` when `--env` names one. That is the block `rbx init create-group --record` writes. An owner that is a **user** is refused rather than read as a group id, since the two are different numbers that happen to look alike.

## What Open Cloud allows

**Roles are read-only.** The document describes two operations on `/roles` and both are `GET`: there is no endpoint to create, rename, re-rank or delete a role. Roles are made in the Creator Hub, and this command reads them and moves people between them.

Every group endpoint is marked **BETA** by Roblox.

| Subcommand | Endpoint | Scope |
| --- | --- | --- |
| `roles` | `GET /cloud/v2/groups/{id}/roles` | `group:read` |
| `rank` | `POST .../memberships/{id}:assignRole` | `group:write` |
| `unrank` | `POST .../memberships/{id}:unassignRole` | `group:write` |

`rank` and `unrank` also list the roles (to turn a name into an id) and the memberships (to find the member), so a key for them needs `group:read` as well.

## rbx group roles

Every role, lowest rank first. The listing follows Roblox's page token to the end: a page defaults to ten roles, so a group with eleven would otherwise look complete with one missing.

```text
 RANK  NAME                            MEMBERS  ID
    0  Guest                                 -  2788100
    1  Member                              412  2788101
   50  Moderator                             6  2788109
  255  Owner                                 1  2788115
```

`MEMBERS` is a dash for the guest role, because Roblox does not return a count for it. A dash means "not reported", not "nobody".

`--json` prints each role as Roblox sent it, `permissions` included, under a `group_id`:

```json
{
  "group_id": "1234567",
  "roles": [
    {
      "path": "groups/1234567/roles/2788109",
      "id": "2788109",
      "displayName": "Moderator",
      "rank": 50,
      "memberCount": 6,
      "permissions": { "changeRank": true, "banMembers": true, "...": "..." }
    }
  ]
}
```

`permissions` is passed through unread rather than modelled field by field. Roblox describes thirty of them today, and a typed copy would turn one it renames into a silent `false`, which for a permission reads as "cannot". `createTime` and `updateTime` appear only for the group's owner.

This is the output to feed a generator. Turning roles into a module for game code belongs to the game, which knows what it wants the module to look like.

## rbx group rank and unrank

```text
rbx group rank <USER> <ROLE> [--yes]
rbx group unrank <USER> <ROLE> [--yes]
```

`<USER>` takes everything [`rbx ban`](ban.md#naming-a-player) takes: an id, a username, `name:<name>`, `@<name>`, or a pasted profile link. On PowerShell, use `name:` rather than `@`.

`<ROLE>` is a role id or a role name, matched without regard to case. `name:<name>` forces the name reading for a role whose name is all digits. **A name that matches several roles is refused**, with the ids of each:

```text
Error: 2 roles of this group are called "owner", so the name does not say which
one you mean. Pass one of these ids instead:
  rank 254  id 2788114
  rank 255  id 2788115
```

Two roles can share a name, and picking whichever came back first is a mistake nobody notices until it matters.

`rank` **adds** a role; it does not replace the member's others. That is what makes a multi-role group possible, and when the member already held other roles the output says so. Roblox does nothing when the member already has the role.

Both ask before writing. `--yes` skips the prompt.

### Finding the member

Roblox addresses a role change to a *membership*, not to a user, so both subcommands look the member up first. They ask Roblox to filter by user (`user == 'users/<id>'`, the syntax the sibling join-requests endpoint documents), and if Roblox refuses that filter they walk the member list instead, a hundred at a time, up to fifty pages. Somebody who is not in the group is an error: a role can only be given to somebody who has already joined.
