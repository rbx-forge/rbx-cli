# rbx completions

Generate a shell completion script. `--env` and `--place` complete with the names in the `rbxplace.toml` of whatever directory you are standing in, so a new env completes without regenerating anything.

```sh
rbx completions bash       -o ~/.local/share/bash-completion/completions/rbx
rbx completions zsh        -o "${fpath[1]}/_rbx"
rbx completions fish       -o ~/.config/fish/completions/rbx.fish
rbx completions powershell -o $PROFILE
```

`-o` is `--output`; without it the script goes to stdout, which is what you want for piping into a file yourself.

| Shell | Where it goes | Then |
| --- | --- | --- |
| bash | `~/.local/share/bash-completion/completions/rbx` | New shell, or `source` the file. Needs `bash-completion` installed |
| zsh | any directory on `$fpath`, named `_rbx` | New shell, or `compinit`. `${fpath[1]}` is usually writable; if not, pick your own and add it to `fpath` in `.zshrc` before `compinit` |
| fish | `~/.config/fish/completions/rbx.fish` | Picked up immediately |
| powershell | appended to `$PROFILE` | New session, or `. $PROFILE`. Use `>>` rather than `-o` if the profile already has content |

## The values are not baked in

When you press TAB the script runs [`rbx env list --names`](env.md#the-two-name-listings) or `rbx env list --place-names` in the current directory and offers what comes back. Those two listings are a supported surface for exactly this reason: one bare value per line, no colors, no headers, and they will not grow columns.

So a completion script generated once keeps working as `rbxplace.toml` changes, and it completes different names in different projects without knowing anything about either.

## What it does when there is nothing to complete

Outside a project, or with a file that does not parse, **both complete to nothing and print nothing**. The completion discards the command's stderr and ignores its exit status, so a broken `rbxplace.toml` never lands in the middle of a half-typed line.

One exception: bash 3.2, the version macOS ships, has no `compopt` and falls back to offering file names.

## `--place` is completed across every env

Unless the file's envs hold genuinely different places, in which case some of what is offered belongs to another env: the completion does not read the `--env` you already typed. Doing so would mean parsing the command line in four shell languages to save a case that most `rbxplace.toml` files do not have. The command you eventually run says so if the place is not in the env.

## Turning the dynamic part off

```sh
rbx completions bash --no-dynamic
```

A script that never starts a subprocess, and completes `--env` with file names. Worth having where a completion must not run a program: a locked-down shell, or a machine where `rbx` is slow to start because it lives on a network drive.

## See also

- [`rbx env`](env.md): the file these names come from, and the two listings the script calls
