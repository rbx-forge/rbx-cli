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

**`permissions` is missing from some roles, and that is Roblox deciding what the key may see**, not a role without permissions. The rule, per role, follows the account that created the API key:

| Role | `permissions` returned |
| --- | --- |
| The guest role | always, no scope needed |
| The key creator's own role, when they are a member | with `group:read` |
| Every role | only when the key creator **owns** the group, with `group:read` |
| Anything else | the field is left out |

So a listing with permissions on the guest role and one other came from a member's key, and one with permissions everywhere came from the owner's. A generator reading this must treat a missing `permissions` as *unknown*, never as "no rights": the safe move is to refuse to generate, not to write roles that can do nothing.

This is the output to feed a generator. Turning roles into a module for game code belongs to the game, which knows what it wants the module to look like.

## rbx group member

```bash
rbx group member builderman
rbx group member name:12345 --json
```

Every role the member holds, highest rank first, not only the highest one. A role the key cannot see in the listing is still shown, by id, rather than dropped. The member is found the way [`rank`](#rbx-group-rank-and-unrank) finds one, so the same key scopes apply.

## rbx group members

```bash
rbx group members                       # everybody, a page at a time
rbx group members Moderator             # only those holding Moderator
rbx group members Moderator --limit 500 --json
```

**Paged, never exhaustive.** A group can have a million members, so one run reads at most twenty pages of a hundred and stops, and every run that did not reach the end prints the command that carries on:

```text
2 found, 2000 member(s) read this run.
More: rbx group members Moderator --cursor eyJvZmZzZXQiOjIwMDB9
```

- `--limit <n>` returns at most `n` members, 100 by default, exactly. A run may stop in the middle of a page.
- `--cursor <token>` resumes with the first member the previous run held back. It is Roblox's own page token, prefixed with `<n>:` when the previous run stopped inside a page, meaning "read that page again and skip the `n` already returned". It is valid for the same group and the same role, and nothing else. If the group changes between two runs, that skip can be off by as many members as joined or left on that page; a paged listing of a live group is never a snapshot.

Where two roles share a name, a member's roles print with their rank, `Owner (255), Owner (254)`, since the name alone would not say which.

### Two ways to list a role

With a role, there are two listings, and which one a run uses depends on whether it has a Roblox session:

| | With a session | Without one |
| --- | --- | --- |
| Listing | `groups.roblox.com/v1/groups/{id}/roles/{role}/users`, the Creator Hub's own Members tab | Open Cloud's `/memberships`, every member |
| Reads | the role's holders only | everybody, keeping the holders |
| A rare role in a million members | one call per hundred holders | up to 500 runs that mostly find nobody |
| Each member's other roles | not reported, so `ROLES` shows `-` and `--json` leaves `roles` out | all of them |

The session is the one `rbx` already knows how to use: `--cookie`, `RBX_COOKIE`, or a signed-in Studio, which it asks about before sending (`--auto-cookie` to stop asking, `--no-auto-cookie` to refuse). Both listings count a role held beside a higher one. The session route is not in Roblox's OpenAPI document, so the drift check cannot watch it; it is listed as known-undocumented, with what was probed.

Without a session, the role is matched on this side, against **every** role a member holds. Roblox documents no way to filter the Open Cloud listing by role, and the one field such a filter could plausibly read holds only a member's highest role, which would drop everybody holding the role beside a higher one. A rare role in a large group may then take several runs that each find nobody, which the summary line makes visible rather than hiding behind a long wait.

A cursor belongs to the listing it came from. A session cursor starts with `S` (`S0:<token>`), and replaying one on the other listing is refused: it would page through the wrong list.

Names come with the session listing, and from one batched call per page on the Open Cloud one. A member whose account Roblox no longer returns is listed by id.

`--json` prints `source` (`session` or `open-cloud`), `members` (each with `user_id`, `username`, `display_name`, and `roles` when the listing reports them), `scanned`, and `next_cursor`, which is `null` at the end.

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
