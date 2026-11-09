# The Studio cookie

`.ROBLOSECURITY` is the only credential `rbx` uses besides the API key. It is a full account session: not scoped, not limited to a universe, and revocable only by signing out everywhere. So `rbx` uses it only where Open Cloud has no endpoint, never as a fallback for a rejected key, and never writes it anywhere.

## Which commands use it

| Command | What for | Cookie |
| --- | --- | --- |
| `rbx init create-group`, `create-universe`, `create-place`, `rename-place`, `rename-universe` | Open Cloud cannot create or rename these | **required** |
| `rbx init list-groups` | "The groups you are in" needs a session | **required** |
| `rbx init list-universes`, `list-places` | These listings are public | not needed |
| `rbx meta init`, `rbx meta pull` | Reading `server_fill`, `allow_copying`, `studio_access_to_apis_allowed`, `beta_mode`. Without a cookie they are skipped and reported | optional |
| `rbx meta sync` | Reading those fields back before it plans, when the config declares one: without a cookie they are compared with the lockfile. Writing them, plus `visibility`, `audience`, and the experience's own icon and thumbnails | optional to read, **required to write those fields** |
| `rbx import` | Its place listing is public; the `meta` step it runs resolves the cookie as usual | not needed for the listing |

Everything else (`place`, `config`, `shop`, `rtbf`, `check`, `env`, `probe`) is key-only or offline. A `403` from Open Cloud is never retried with the cookie.

The [cookie-only fields](meta-config.md#cookie-only-fields) of `rbx meta` are listed with their Roblox feature requests: each one stops needing the cookie when Roblox ships a key route for it.

## How it is resolved

1. `--cookie <value>` or `RBX_COOKIE`. An empty `RBX_COOKIE=` counts: it means "no cookie" and stops here.
2. `--no-auto-cookie` stops here with no cookie.
3. A signed-in local Roblox Studio (Windows registry, macOS plist), **only if you say yes**.

### Auto-detection is opt-in

| The run | Studio session |
| --- | --- |
| `--auto-cookie` | Sent |
| A terminal on stdin, stdout and stderr | Asked once per process: `[y/N]`, on stderr |
| Anything else: CI, a pipe, cron, `--json` into a file | **Not sent**, with one line on stderr saying how to supply one |

When it is used, one line on stderr says so:

```
using the Roblox Studio cookie (--auto-cookie to stop asking; --no-auto-cookie or RBX_COOKIE= to refuse)
```

`--auto-cookie` and `--no-auto-cookie` cannot be passed together. In CI, set `RBX_COOKIE` from a secret store for the few commands that need it, or better, keep those commands out of CI.

## Before a write

The commands that write with the cookie first check the session with one call to `users.roblox.com`, so an expired session is refused before anything changes:

| Command | What the check prevents |
| --- | --- |
| `rbx meta sync` with a cookie-only field in the plan | The Open Cloud half landing and the cookie half not |
| `rbx init create-group` | Paying 100 Robux for a call about to be refused |
| `rbx init create-universe`, `create-place` | A creation followed by a failed rename |
| `rbx init rename-place`, `rename-universe` | A public read mistaken for permission |

The check runs once per process. A network failure or a 5xx is a warning, not "session expired". The confirmation then names the account:

```
⚠ As builderman (156): create universe 'My Game' under group 1234567 and record it as [test]? [y/N]
```

!!! warning "Studio decides which account"
    Auto-detection follows whichever account Studio is signed into. Switching accounts there changes the account every later command acts as, and nothing checks that it matches the owner of your API key: a mismatch shows up as a permission error.

## What it does not hide

The existence, id and name of a universe or place are public, whatever its visibility: `develop.roblox.com/v1/universes/{id}/places` and `games.roblox.com/v2/groups/{id}/gamesV2` answer anonymously, private universes included. Withholding a cookie does not conceal them. If a name must not be seen, change the name.

## Never stored

The cookie lives in memory for the run and goes only into the `Cookie` header of the Roblox call that needs it. No file `rbx` writes can hold it, nothing logs it, and `--help` never prints the value of `RBX_COOKIE` or `RBX_API_KEY`.

## Turning it off

```sh
rbx meta pull --env prod --no-auto-cookie   # one run
export RBX_COOKIE=                          # the whole shell
```

Commands that require a cookie then fail and say how to supply one. Optional ones carry on and report what they could not read.
