# Stability and support

## What will not move under you

At 0.x, this is what anyone putting `rbx` in CI is betting on:

- **Config file formats are stable.** `rbxplace.toml`, `rbxshop.toml`, `rbxmeta.toml` and friends keep working. Keys get added; existing keys do not change meaning or disappear before 1.0. An unrecognised key is a warning on stderr, never a failure, so a file written for a newer release still loads on an older binary.
- **Exit codes are stable.** `0` is success, non-zero is failure, and a command changes which is which only in a release whose changelog says so. The one exception so far: the `check` commands, which reported drift on the screen and exited 0, now exit `2`, so a pipeline that passed on a drifting repository starts failing. See [Exit codes and `--json`](exit-codes-and-json.md).
- **`--json` documents carry `schema_version`.** A field may be added without a bump; a field that changes meaning or disappears bumps it.
- **Lockfile formats may migrate.** `rbxshop.lock.toml`, `rbxmeta.lock.toml` and `rbxconfig.lock.toml` are tool-owned state, not input you typed, and may change shape between releases. A newer format is refused by an older binary rather than misread.
- **Command flags may change, with a changelog entry.** A renamed or removed flag is noted in the [changelog](https://github.com/rbx-forge/rbx-deploy/blob/main/CHANGELOG.md) for the release that does it, and gets an alias where one is possible, as `--universe` did for `--universe-id`.

**MSRV: Rust 1.88.** Declared in the workspace manifest, so `cargo` refuses with a clear message rather than failing three dependencies deep. It moves when a dependency moves it. The precompiled binaries in each [release](https://github.com/rbx-forge/rbx-deploy/releases) need no toolchain at all.

## Support tiers

**Core.** A bug here blocks a release: `place`, `shop`, `meta`, `config`, `rtbf`, `env`, `init`, `import`, `check` and `status`. These are the commands people put in CI and point at production.

**Tier 2.** A bug here never blocks a release: `probe` and `completions`. They get fixed, just not on the critical path.

## Maintenance

One maintainer, maintained on their own schedule. **Issues are triaged, not promised**: a bug in a Core command gets attention first, everything else gets looked at when it gets looked at. Reproducible bug reports and PRs with tests are the fastest path to a fix. Nothing here is a support contract, and it is better to say so before anyone finds out the hard way.

Security reports go through [SECURITY.md](https://github.com/rbx-forge/rbx-deploy/blob/main/SECURITY.md), never the public tracker.

Written with AI assistance. Every line is reviewed, tested and shipped by the maintainer, who is responsible for it; the tests and the CI gates in the repository are the actual answer to "was this checked", and they are the same answer either way.
