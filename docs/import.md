# rbx import

Adopt a universe that already exists: write its env into `rbxplace.toml`, and its passes, badges, products, metadata and live config into each tool's file and lockfile.

```sh
rbx import --universe-id 123456789 --env prod
rbx check --env prod        # clean right after an import
```

Each domain is imported by the tool that owns it (`shop init --from-remote` or `shop pull`, `meta init --from-remote` then `meta pull --accept-remote`, `config pull`), so the lockfiles are exactly what `check` and `sync` expect. Drift straight after an import is a bug.

| File | Contains |
| --- | --- |
| `rbxplace.toml` | `[<env>]` with the universe and every place, and `[owner]` if the file had none |
| `rbxshop.toml` + lock | Passes, badges, products, icons downloaded |
| `rbxmeta.toml` + lock | Metadata, icon and thumbnails |
| `rbxconfig.toml` + lock | The live config |

The start place is written as `places.main`; other places get a key from their name. An env that already lists the start place under another key keeps it, with `root` added.

## Flags

| Flag | Default | Meaning |
| --- | --- | --- |
| `--universe-id <id>` | required | The universe |
| `--env <name>` | required | Its env name. `all`, `owner`, `codegen` and `groups` are refused |
| `--dir <path>` | `.` | Where to write the files |
| `--dry-run` | | Report, write nothing |
| `--strict` | | Fail on a domain error instead of skipping it |
| `--only <domains>` | all | `shop`, `meta`, `config`, comma-separated |

`rbx check --dir <path>` reads back what `rbx import --dir <path>` wrote.

## A second import

```sh
rbx import --universe-id 987654321 --env staging
```

This appends `[staging]` (the rest of `rbxplace.toml`, comments included, is untouched) and **pulls** each domain, so the existing files gain `[envs.staging]` overlays for what differs.

!!! warning "Unsynced local edits are overwritten"
    The pulls take Roblox's values without asking. `import` names the files it will layer onto; commit or sync your edits first.

An env already in `rbxplace.toml` is left as it is, even if it points at another universe: `import` says so and changes nothing there.

## What it does not import

Listed at the end of every run:

- **Cookie-only metadata** (`server_fill`, `allow_copying`, `beta_mode`) when no [cookie](cookie.md) is available. Set them by hand, or re-run with one.
- **A domain that failed**, usually for a missing scope. Skipped and reported, or fatal with `--strict`.
- **RTBF templates.** Run `rbx rtbf pull --env <name>` after the import. Writing `rbxrtbf.toml` by hand and syncing would replace whatever the universe already declares.

## Required API scopes

| Step | Scopes |
| --- | --- |
| Universe | none: Roblox does not check `universe:read` on this read |
| Places | none (the listing is public) |
| Shop | `game-pass:read`, `developer-product:read`, `legacy-badge:manage`; `legacy-asset:manage` for icons as stored |
| Meta | as for [`meta`](meta.md#required-api-scopes), whose universe and place reads Roblox does not check |
| Config | `universe:read` |
