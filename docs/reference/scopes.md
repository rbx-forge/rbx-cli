# Scopes by command

What the Open Cloud API key needs for each command, gathered from the command pages, which stay the authority and carry the details. Create the key in the [Creator Hub](https://create.roblox.com/dashboard/credentials), bound to the experiences it acts on.

**Keep a read key and a write key.** Roblox fixes a key's scopes when you create it, so a read-only key stays read-only whatever calls it. The read key is the one that ends up pasted into a terminal while debugging; it should be the one that cannot publish.

| Command | Read | Write |
| --- | --- | --- |
| [`init`](../init.md) | no key: cookie only, Open Cloud has no creation endpoint | |
| [`import`](../import.md) | the read scopes of `shop`, `meta` and `config` below | |
| [`env`](../env.md) | none: local file only | |
| [`check`, `status`](../check.md) | the read scopes of each tool it finds configured; `--offline` needs none | |
| [`place`](../place.md) | `asset:read` (versions), `legacy-asset:manage` (download, promote source), `universe:read` (places) | `universe-places:write` (upload, promote), `asset:write` (rollback) |
| [`meta`](../meta.md) | `universe:read`, `universe.place:read` | `universe:write`, `universe.place:write`; a cookie for the fields Open Cloud does not expose |
| [`config`](../config.md) | `universe:read` | `universe:write` |
| [`rtbf`](../rtbf.md) | `universe:read` (check, pull), `universe-datastores.control:list` (verify) | `universe:write` (sync) |
| [`shop`](../shop.md) | `game-pass:read`, `developer-product:read`, `legacy-badge:manage`; `legacy-asset:manage` for icons as stored | `game-pass:write`, `developer-product:write`, `legacy-universe.badge:write`, `legacy-universe.badge:manage-and-spend-robux` |
| [`probe`](../probe.md) | whatever the path you call needs | |
| [`completions`](../completions.md) | none | |

Some of these come with a caveat on the command page: the meta thumbnail personalization scopes, the badge payment source, the icon upload host. Read the page before narrowing a key to this table.

## When a call is refused

A `403` that names a missing scope is reported with the scope it wanted, and the hint after it asks you to check which key is loaded: the commonest cause is a key from another shell or another project, not a key that lacks the scope.
