# Exit codes and `--json`

What a script can rely on. Both are covered by the [stability policy](stability.md).

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success. For a check: everything is in sync |
| `1` | The command failed, or a check could not answer |
| `2` | A check ran and found drift: something to sync, or a generated file to regenerate |

`2` exists so a pipeline can tell "publish this" or "regenerate and commit" from "something broke". Commands that can return it:

| Command | Exits `2` when |
| --- | --- |
| [`rbx check`](../check.md) | at least one check found drift and none failed |
| `rbx shop check`, `rbx meta check` | the config differs from the lockfile, in any targeted env (offline) |
| `rbx config check`, `rbx rtbf check` | the file differs from what is published, in any targeted env |
| `rbx rtbf verify` | at least one template names no data store that exists |
| `rbx shop codegen --check`, `rbx env codegen --check` | the committed generated file differs from what would be generated |

[`rbx status`](../check.md#rbx-status) is the same engine as `rbx check` and **always exits 0**: it is for a human, and a status command that fails a script is a check with a different name.

### Generated files stay honest

Both generators write modules your game code imports, derived from files you commit, and both can verify instead of writing:

```sh
rbx shop codegen --check     # against rbxshop.toml + rbxshop.lock.toml
rbx env codegen --check      # against rbxplace.toml
```

No API key, no network: the inputs are local, so this runs in a pre-commit hook and in CI. The hook snippets are in [Guarding generated files](../shop-codegen.md#guarding-generated-files).

## `--json`

These subcommands take `--json`:

| Command | Page |
| --- | --- |
| `rbx check`, `rbx status` | [check](../check.md) |
| `rbx env list`, `rbx env get` | [env](../env.md) |
| `rbx place upload`, `promote`, `rollback`, `versions`, `places` | [place](../place.md) |
| `rbx config get`, `list`, `versions` | [config](../config.md) |
| `rbx rtbf show`, `verify` | [rtbf](../rtbf.md) |
| `rbx shop list`, `show` | [shop](../shop.md) |

Each page documents its documents field by field. The rules they share:

- **One JSON document on stdout, and nothing else.** Diagnostics, warnings and progress stay on stderr, so `jq` reads the pipe while a human still reads the terminal.
- **`schema_version` comes first**, `1` today, the same in every document. Refuse a version you do not understand.
- **The exit code does not change.** A document, the stream and the status all give the same answer.
- **Ids are strings, quantities are numbers.** A place id already exceeds 2^53, and a consumer parsing JSON with doubles would round one. Robux stays a number, so arithmetic on it keeps working.
- **Absent, not `null`.** A field that does not apply, or that Roblox did not send, is left out, so `has("field")` is a usable test. Where `null` does appear it is a real value, and the page says so.
- **No prompt under `--json`.** A command that would stop and ask fails at that point instead, with a message on stderr naming the flag that answers the question (`--yes`, most often), and writes nothing to stdout.
