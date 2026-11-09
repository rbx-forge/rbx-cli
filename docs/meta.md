# rbx meta

Universe and place metadata (name, description, icon, thumbnails, devices, social links, private servers, visibility, avatar rules, paid access), declared in `rbxmeta.toml` and synced to every env. Most fields go through Open Cloud with an API key; a few need the [session cookie](cookie.md).

```sh
rbx meta init                                 # commented template
rbx meta pull --env dev --accept-remote       # fill it from dev, downloading media
rbx meta pull --env prod --accept-remote      # record where prod differs
rbx meta check --env all
rbx meta sync --env dev
```

Without `rbxplace.toml`: `rbx meta init --from-remote --universe-id 123456789 --place-id 987654321` writes the ids into `rbxmeta.toml` as `[experience]`.

Every field is on [Configuration](meta-config.md).

## Envs

`[game]` and `[media]` are the base. `[envs.<name>]` (and `[envs.<name>.devices]`, `[envs.<name>.media]`, ...) override them for one env.

`pull --env <name>` writes only differences: a field the base lacks goes to the base, a field equal to the base is removed from the overlay, a different one goes to the overlay. It never moves values from overlays into the base.

With `--env all` or a group, `check`, `sync` and `pull`:

- plan every env before writing anything, and ask for one confirmation covering all of them;
- `check` exits `2` if any env drifts;
- `pull` writes `rbxmeta.toml` once, at the end.

### One place per env

`name`, `description` and `server_size` belong to a place, and the lockfile records one place per env. `--place` picks it (default: the start place). Syncing a different place under an env that already tracks one is refused. To manage two places, give each its own env pointing at the same universe.

## Commands

### rbx meta init

Without flags, write a commented template.

| Flag | Description |
| --- | --- |
| `--from-remote` | Fill the config from the live universe and place |

With `--env`, ids come from `rbxplace.toml`; with `--universe-id` and `--place-id`, they are written as `[experience]`.

### rbx meta sync

Send the fields that differ from the lockfile, and update the lockfile after each call.

| Flag | Description |
| --- | --- |
| `--dry-run` | Show what would change |
| `--yes` / `-y` | Skip the confirmation |

Checked before anything is sent:

- a private server price of `0` or at least `10`, and a paid one only on a public experience;
- `audience` with `limited` only;
- every media file exists;
- the session, when a cookie field is in the plan.

The lockfile is saved after every successful call, so an interrupted sync resumes where it stopped. Icons and thumbnails are uploaded only when their content changed.

### rbx meta check

Compare the config with the lockfile. Exits `0` nothing pending, `2` drift, `1` error. It does not contact Roblox: `rbx check` does, in its `meta/live` row. See [Comparing with Roblox](check.md#comparing-with-roblox).

### rbx meta pull

Write live metadata into the config and lockfile, preserving comments.

| Flag | Description |
| --- | --- |
| `--dry-run` | Show what would change |
| `--accept-remote` | Download the icon and thumbnails into `media.dir` and point the config at them |
| `--accept-local` | Keep your files; the next `sync` uploads them |
| `--yes` / `-y` | Skip the confirmation |

With an env, downloads go to `<media.dir>/<env>/` and the paths are written to that env's overlay, so envs never overwrite each other's files. Downloads are what Roblox serves now (512x512 icon, 768x432 thumbnails), not necessarily the original upload. An image already downloaded and unchanged is skipped.

### Per-tool flags

| Flag | Description |
| --- | --- |
| `--config <path>` | Path to `rbxmeta.toml` (default `rbxmeta.toml`) |

## Lockfile

`rbxmeta.lock.toml` records, per env, the universe, the place, and the last state applied (media as content hashes and Roblox image ids). Without `--env` the section is `[envs.default]`. Commit it.
