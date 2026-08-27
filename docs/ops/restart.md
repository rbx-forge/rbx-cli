# rbx restart

Push a published version out to servers still running the old one, without waiting for them to cycle.

Publishing does not restart anything. Servers already up keep running the code they started with until they empty out, which on a busy experience takes hours. Fine for a feature, not for a fix.

See [ops.md](../ops.md) for install, keys and the safety model. Reading needs `universe:read`, launching needs `universe:write`.

## The dry run is not a simulation

Roblox has a forecast endpoint that answers how many **real players would be kicked right now**. So `restart launch` without `--apply` shows that number and stops. You decide against a fact rather than against a guess.

```sh
rbx restart forecast --env prod
```

```text
PLACE                      PLAYERS HIT       INSTANCES HIT  NEWEST VERSION
55443322110099                   0/840                0/210  412

0 player(s) would be disconnected, 0 instance(s) closed.
Hit is not total: a server already on the newest version is left alone.
```

Read that carefully: 840 players are online, and **0** would be affected, because every server already runs 412. "Hit" and "total" are different numbers and only the first one costs you anything.

## Launching

```sh
# shows the forecast, sends nothing
rbx restart launch --env prod

# for real: prompts, then schedules
rbx restart launch --env prod --bleed-off 30 --apply
```

| Flag | Meaning |
| --- | --- |
| `--bleed-off <minutes>` | Delay before servers begin closing. Default 30, Roblox accepts 1 to 240. |
| `--attribute <k=v>` | One entry in the dictionary the servers receive. Repeatable. |
| `--payload <json>` | The whole dictionary as JSON, for values that are not strings. |
| `--apply` | Actually launch it. |
| `--yes` | Skip the prompt. |

**Bleed-off is the kind part.** During that window players stop being matchmade to the servers due for restart, so most of them leave on their own and are never kicked. Longer is gentler. The forecast number is what happens with no bleed-off at all, so the real impact is lower.

Nothing to restart short-circuits: if every server is already on the newest version, the command says so and does not prompt.

## Telling the game why **(0.5.0+)**

A server scheduled to close fires:

```lua
game.ServerRestartScheduled:Connect(function(restartTime, source, attributes)
    -- restartTime: when this server will be closed
    -- source: who asked
    -- attributes: whatever the launch sent, empty if it sent nothing
end)
```

That third argument is what turns a restart from "kick everyone politely" into "tell the game it is about to close and let it teleport people out". It pairs with the bleed-off rather than replacing it: the bleed-off buys the time, the attributes say what to do with it.

```sh
rbx restart launch --env prod --bleed-off 30 \
  --attribute reason=hotfix \
  --attribute message="Back in 5 minutes" --apply

rbx restart launch --env prod \
  --payload '{"reason":"hotfix","urgency":3,"silent":false}' --apply
```

`--attribute` sends **strings**, always. `--attribute urgency=3` arrives in Luau as `"3"`, not `3`, because guessing types would make the value a different Lua type than the text says. For a number, a boolean or nesting, use `--payload`. The two cannot be combined.

Both are validated before the first request goes out, which is the point of parsing locally at all: a typo in a deploy script costs nothing, and above all is not discovered after the confirmation prompt. Two rules, both Roblox's:

- it must be a JSON **object**, because the game indexes it by key. A bare string or array is refused by name.
- it must serialise to at most **500 bytes**. Over that, send a reference rather than the content: a version string, or a key the game can look up.

Notifications fire only for **delayed** restarts, which is the only kind `rbx restart launch` issues.

### Two endpoints, two limits

Worth knowing if you read Roblox's own documentation and find a contradiction. `rbx restart` uses `POST /server-management/v1/universes/{id}/restarts`, whose request accepts `attributes` and a bleed-off of **1 to 240** minutes. The Universes reference documents a different endpoint, `cloud/v2/universes/{id}:restartServers`, which takes no `attributes` and caps the bleed-off at **60**. Both are right about their own endpoint. The bound this tool checks is the one for the endpoint it calls, and both come from the vendored OpenAPI document in `spec/`.

## Watching it

```sh
rbx restart status --env prod
```

States are `DELAYING` (the bleed-off, nothing closed yet), `RESTARTING` (servers closing), `SUCCEEDED`.

## Not verified end to end

`forecast` and `status` have been exercised against a live experience. **`launch --apply` never has been**, and cannot be without restarting production for real. It is covered by mocks, including the test that matters most: a launch without `--apply` fetches the forecast and never POSTs.

`--attribute` and `--payload` inherit that: what is verified is that the object reaches the request body under the name Roblox's schema gives it, and that a bad one costs no request. Whether the dictionary arrives intact in `ServerRestartScheduled` is a question only a real restart answers. The bounds enforced locally are the vendored schema's, not measurements.

Treat the first real launch as the test it is: run `forecast` first, pick a quiet hour, and use a long bleed-off.
