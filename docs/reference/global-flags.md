# Global flags

Every subcommand accepts these, before or after its name: `rbx --env prod shop sync` and `rbx shop sync --env prod` are the same command.

| Flag | Env var | Description |
| --- | --- | --- |
| `--api-key <key>`, `-a` | `RBX_API_KEY` | Open Cloud API key |
| `--cookie <value>` | `RBX_COOKIE` | `.ROBLOSECURITY` cookie, for the endpoints Open Cloud does not expose |
| `--auto-cookie` | | Use a signed-in local Studio session without asking first |
| `--no-auto-cookie` | | Never look for a local Studio session |
| `--env <name>`, `-e` | | Target env from `rbxplace.toml`. Commands that can act on several envs also take `all` or a group name |
| `--place <name>` | | Place within the env, for subcommands that operate at place scope |
| `--places <path>` | | Path to `rbxplace.toml` (default `rbxplace.toml`) |
| `--universe-id <id>` | | Name a universe directly, skipping `rbxplace.toml`. Wins over `--env`. Accepted as `--universe` too |
| `--place-id <id>` | | Name a place directly, skipping `rbxplace.toml`. Wins over `--env` and `--place` |

**Prefer `RBX_API_KEY` to `--api-key`.** A flag lands in your shell history and in any process listing; the variable does not. [Scopes by command](scopes.md) lists what the key needs.

## The cookie

For the handful of endpoints Open Cloud does not publish (`init`, some `meta` fields), `rbx` can use the `.ROBLOSECURITY` cookie of a signed-in local Roblox Studio when you set none. It is opt-in: an interactive run asks once, a run with nobody to ask (CI) refuses, and `--auto-cookie` is the standing yes. A session cookie is a full-account credential, strictly more powerful than any scoped key, so it gets its own page: [The Studio cookie](../cookie.md) covers which commands use it, which never will, how it is resolved, and why it is never written to disk.

## Without a config file

`--universe-id` and `--place-id` exist so a command works in a directory that has no `rbxplace.toml` at all: a one-off against a universe you never configured, somebody else's game you are helping with, or a script on a server that has no checkout:

```sh
export RBX_API_KEY="..."
rbx probe "cloud/v2/universes/{universe}" --universe-id 66778899001
```

The reconciling commands still read their own TOML for *what* to apply, and take the ids for *where*:

| Command | Ids |
| --- | --- |
| `shop` | `--universe-id`, in place of `[experience]` or `--env` |
| `rtbf` | `--universe-id`, in place of `--env` |
| `meta` | `--universe-id` and `--place-id`, in place of `[experience]` or `--env` |
| `place upload`, `place rollback` | `--universe-id` and `--place-id` |
| `place download`, `place versions` | `--place-id` |
| `config` | `--universe-id`, with `--env` still naming the section of `rbxconfig.toml` |
| `place promote` | none: it copies an env to an env |

**A write to a bare id always asks first**, or needs `--yes`: no env says whether it should, the way `confirm = true` does. A `--json` receipt for it has no `env`.

## Built-in help

```text
rbx -h                # compact summary: every subcommand and the global flags
rbx --help            # verbose: full flag descriptions
rbx shop --help       # one subcommand
rbx shop sync --help  # one verb
```
