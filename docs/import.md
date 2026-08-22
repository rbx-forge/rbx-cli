# rbx import

Adopt an existing universe in one command. `rbx import` resolves a universe you already have on Roblox, writes the env into `rbxplace.toml`, and brings its passes, badges, products, metadata and live config into the TOML files each tool manages, with the lockfiles that make the immediately-following `check` green.

It is the command for a game that predates this toolkit, which is nearly every game. Without it, adoption means copying ids out of the Creator Hub by hand: the class of transcription where one wrong digit points a `sync` at somebody else's universe.

## Features

- **One gesture** - Universe, places, owner, monetization, metadata and live config, in one pass
- **Composition, not a second implementation** - Each domain is imported by the command that already owns it, so nothing here can disagree with what `sync` and `check` expect
- **Safe on an existing file** - Other envs, `[owner]`, `[codegen]` and every comment in `rbxplace.toml` survive an import verbatim
- **Repeatable** - A second `--env` layers a second universe onto the same files, in the differential overlay layout the tools already use
- **Nothing hidden** - What could not be imported is named at the end, with the reason and the fix

## Usage

```bash
rbx import --universe-id 123456789 --env prod       # Adopt a universe as "prod"
rbx import --universe-id 987654321 --env staging    # Add a second, beside the first
rbx import --universe-id 123456789 --env prod --dry-run    # Resolve and report, write nothing
rbx import --universe-id 123456789 --env prod --only shop  # One domain
rbx import --universe-id 123456789 --env prod --dir ./game # Write somewhere other than .
rbx import --universe-id 123456789 --env prod --strict     # Fail instead of skipping a domain
```

## What it writes

| File | Written by | Contains |
| --- | --- | --- |
| `rbxplace.toml` | `import` itself | `[<env>]` with `universe_id` and every place, plus `[owner]` if the file had none |
| `rbxshop.toml` + `.lock` | `rbx shop init --from-remote` / `rbx shop pull` | Game passes, badges, developer products, with icons downloaded |
| `rbxmeta.toml` + `.lock` | `rbx meta init --from-remote` then `rbx meta pull --accept-remote` | Name, description, devices, social links, private servers, icon and thumbnails |
| `rbxconfig.toml` + `.lock` | `rbx config pull` | The live in-experience config, live being authoritative |

`import` computes none of those lockfile entries. It runs each tool's own import path, which is what makes the zero-drift guarantee reachable at all: the lockfile is written by the same code that later reads it.

## The acceptance criterion

**`import` then `check` is green, with nothing in between.**

```bash
rbx import --universe-id 123456789 --env prod
rbx check --env prod
```

`rbx check` discovers the tools from the files the import just wrote and returns one exit code for all of them: `0` clean, `2` drift, `1` a check that could not answer. See [`docs/check.md`](check.md).

If there is drift straight after an import, the import is wrong. `import` prints that exact command line when it finishes, `--dir` included when it was passed.

## Running it twice

The second import is what decides whether the command is usable, and it is the case the implementation is shaped around.

```bash
rbx import --universe-id 111 --env prod
rbx import --universe-id 222 --env staging
```

The second run:

- **appends** `[staging]` to `rbxplace.toml`; `[prod]`, `[owner]`, `[codegen]` and every comment are untouched, byte for byte;
- **pulls** rather than re-initialises each domain, so `rbxshop.toml` gains an `[envs.staging.*]` overlay for the fields that differ from base rather than being overwritten.

Which of `init --from-remote` and `pull` runs is decided by whether the config file exists, not by whether this is your first import: a directory can already be under management for reasons that have nothing to do with this command.

Those pulls run with `--accept-remote --yes`, because the live game is what an import is adopting. The consequence is worth knowing before you run it a second time: **a local edit you have not synced is resolved to the remote value without a prompt.** `import` names the files it is about to layer onto, on the real run and under `--dry-run` alike:

```
! rbxshop.toml, rbxmeta.toml already exist: this env is layered onto them, and a
  local edit that disagrees with Roblox is resolved to the remote value without
  asking. Commit or sync local edits first if you have any.
```

Commit first, or run the domain's own `sync`, if you have edits you meant to keep.

An env that is already in `rbxplace.toml` is left exactly as it is. If its `universe_id` disagrees with the one you passed, `import` says so and keeps the file's:

```
! [prod] already points at universe 999: kept. Nothing was retargeted; use a
  different --env if you meant to add this universe.
```

## What it cannot import

Reported at the end of every run, because a directory that looks adopted but quietly omits something is worse than one that failed.

```
! 1 thing could not be imported:
  meta server fill, copying permission, beta mode: these live only on legacy
      endpoints that need a Roblox session cookie
    -> re-run with --cookie, or set them by hand in rbxmeta.toml
```

Two categories:

- **Cookie-only metadata.** `server_fill`, `allow_copying` and `beta_mode` have no Open Cloud endpoint. `rbx meta` models them and `sync` can write them, but nothing can read them back without a session cookie, so an import that resolves no cookie at all leaves them unset and says so. The test is what the `meta` step resolves, not what you typed: it auto-detects like any other `rbx meta` run, so on a machine with Studio signed in these fields are read and this line does not appear. Without that line, the first `meta sync` after an import looks like it is inventing changes.
- **A domain that failed.** By default a domain that errors (usually a key missing one scope) is skipped and reported rather than aborting the run, because a half-written directory with no explanation is the worse outcome. `--strict` inverts that, which is what you want in CI.

## Flags

| Flag | Default | Meaning |
| --- | --- | --- |
| `--universe-id <id>` | required | The universe to adopt |
| `--env <name>` | required | Name for its env in `rbxplace.toml`. `all`, `owner` and `codegen` are reserved and refused |
| `--dir <path>` | `.` | Where to write the config files |
| `--dry-run` | off | Resolve and report; write nothing |
| `--strict` | off | Fail on a domain error instead of skipping and reporting it |
| `--only <domains>` | all | Comma-separated: `shop`, `meta`, `config` |

`--places <path>` (global) points at a shared `rbxplace.toml` outside `--dir`; otherwise the file is written next to everything else. `rbx check --dir` resolves the same default the same way, so `rbx check --dir <path>` reads back exactly what `rbx import --dir <path>` wrote.

## Required API scopes

The import calls each tool's own read paths, so it needs the read half of what those tools need:

| Step | Scopes |
| --- | --- |
| Resolve the universe | `universe:read` |
| List places | none: the legacy `develop` host answers without any credential |
| Shop | `game-pass:read`, `developer-product:read`, `legacy-badge:manage` to list badges, `legacy-asset:manage` for icon downloads. Without that last one icons still arrive, from the public thumbnail service, but rescaled rather than as stored |
| Meta | `universe:read`, `universe.place:read`. Icons and thumbnails need nothing: `pull` reads them from `thumbnails.roblox.com`, the public service, with no key attached |
| Config | `universe:read` |

`rbx doctor --universe-id <id>` answers whether the loaded key carries them, before you run this.

## Where the place list comes from

Open Cloud can read one place (`/cloud/v2/universes/{id}/places/{p}`) but cannot enumerate them, so the place list is fetched from `develop.roblox.com`, the same host `rbx init list-places` uses.

**That listing needs no credential**, and a private universe is no exception: it answers in full to an anonymous caller. So this step never fails for want of a cookie, and passing one widens nothing. See [the listings need no credential](./init.md#the-listings-need-no-credential) for what was measured.

The call does not auto-detect a Studio cookie, and now that the listing is known to be open there is nothing for auto-detection to buy here. The `meta` step that runs afterwards is an ordinary `rbx meta` invocation and resolves the cookie the usual way, which is where a cookie genuinely changes the outcome. [docs/cookie.md](./cookie.md) has the full order.

The root place is always written as `places.main`, whatever Roblox calls it, because `main` is the key the rest of the toolkit resolves to when `--place` is omitted. Other places take a slugified form of their display name, suffixed if two collide.

## Related

- [docs/place.md](./place.md): the `rbxplace.toml` this writes, and what reads it
- [docs/shop.md](./shop.md): what lands in `rbxshop.toml`, and the overlay layout a second import produces
- [docs/meta.md](./meta.md): the metadata fields, including the cookie-only ones
- [docs/cookie.md](./cookie.md), the trust model: what the cookie is for, how it is resolved, and why it never reaches disk
- [docs/config.md](./config.md): live config, where live is authoritative
- [docs/doctor.md](./doctor.md): check the key before importing with it
