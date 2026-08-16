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
| `--apply` | Actually launch it. |
| `--yes` | Skip the prompt. |

**Bleed-off is the kind part.** During that window players stop being matchmade to the servers due for restart, so most of them leave on their own and are never kicked. Longer is gentler. The forecast number is what happens with no bleed-off at all, so the real impact is lower.

Nothing to restart short-circuits: if every server is already on the newest version, the command says so and does not prompt.

## Watching it

```sh
rbx restart status --env prod
```

States are `DELAYING` (the bleed-off, nothing closed yet), `RESTARTING` (servers closing), `SUCCEEDED`.

## Not verified end to end

`forecast` and `status` have been exercised against a live experience. **`launch --apply` never has been**, and cannot be without restarting production for real. It is covered by mocks, including the test that matters most: a launch without `--apply` fetches the forecast and never POSTs.

Treat the first real launch as the test it is: run `forecast` first, pick a quiet hour, and use a long bleed-off.
