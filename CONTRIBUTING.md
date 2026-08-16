# Contributing

Thanks for looking. This is a solo-maintained project, so before writing code:
**open an issue first for anything larger than a bug fix.** A PR that arrives
without one may be turned down for reasons that have nothing to do with its
quality — see [What this tool does not do](./README.md#what-this-tool-does-not-do)
in the README, which is where scope decisions are recorded.

Bug fixes with a test need no preamble. Send them.

## Setup

```sh
git clone https://github.com/rbx-forge/rbx-cli
cd rbx-cli
lefthook install          # one-time: pre-commit runs cargo fmt + clippy
cargo test --workspace
```

**MSRV is Rust 1.88**, declared in the workspace manifest. It is
dependency-driven, not a target of its own — our sources need 1.82.

No Roblox credentials are needed to build or test: every HTTP path is tested
against `wiremock`, never against the live API.

## Where code goes

The workspace is one binary crate, one crate per domain, one shared core.
[ARCHITECTURE.md](./ARCHITECTURE.md) is the map; read it before adding files.
The short version, which every domain crate follows:

- `lib.rs` — the clap `Args`/`Subcommand` for that tool plus `run(...)`
  dispatch, and nothing else.
- `api/` — a thin client newtype over `reqwest`, one method per endpoint, built
  through `rbx_core::api::build_client*`.
- `commands/` — one module per subcommand. The logic lives here.
- `config.rs` / `lockfile.rs` — the declarative `*.toml` model and, for
  stateful tools, the record of what was last synced to Roblox.

Retries belong to `rbx_core::api` — there is exactly one retry loop in the
workspace, and a new crate does not get its own.

## What makes a PR mergeable

- **`cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings`
  are clean.** The pre-commit hook runs both; CI runs them again.
- **`cargo test --workspace` passes**, and a behavior change comes with a test
  that fails without the change. HTTP behavior is tested with `wiremock`, and
  the test asserts the *request* that was emitted, not only the response that
  was parsed — a client that sends the wrong body against a permissive mock is
  the failure mode that matters.
- **Generated output changes go through `insta`.** `rbx shop codegen` and
  `rbx env gen-module` snapshot the whole emitted file; accept intentional
  changes with `cargo insta review` and let the diff be reviewed.
- **Docs move with the code.** A user-visible change updates the relevant
  `docs/*.md` page and adds a `## [Unreleased]` line to
  [CHANGELOG.md](./CHANGELOG.md). Features documented in `docs/` get a
  `**(X.Y.Z+)**` tag at release time; `docs/` describes `main`, and readers
  land there from search engines with no version context.
- **No new dead code.** Dead code is denied crate-wide; something deliberately
  unused carries a narrow `#[allow(dead_code)]` with a reason.

### Comment style

Comments state constraints the code cannot express: why this order, why not the
obvious alternative, what breaks if it changes. They do not narrate what the
next line does. **Prefer silence to narration** — an unnecessary comment is a
review comment here.

### Commit messages

[Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`,
`chore:`, `feat!:` for breaking). It keeps the hand-written changelog readable.
Nothing enforces it, so it is a convention kept by discipline.

## Developer Certificate of Origin

Every commit must be signed off. Add `-s` to your commit command:

```sh
git commit -s -m "fix: ..."
```

which appends:

```
Signed-off-by: Your Name <your.email@example.com>
```

That line is your statement that you wrote the patch or otherwise have the
right to contribute it under this project's license — the full text is at
<https://developercertificate.org/>. The `dco` GitHub app checks it on every
PR; a missing sign-off is fixable with
`git commit --amend -s` (or `git rebase --signoff` for several commits) and a
force-push.

No copyright assignment is asked for, and none is implied. You keep your
copyright; the sign-off is only the provenance statement.

## Licensing

The project is [MPL-2.0](./LICENSE), file-level copyleft: modified source files
stay MPL-2.0, new files added downstream may be licensed independently.

If you adapt code from another project, say so in the PR and add its notice to
[THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md). A doc-comment credit is not
a license notice. MIT and Apache-2.0 sources are fine with the notice;
GPL-family sources are not compatible with the distribution and will be
declined.

## Reporting bugs and security issues

Bugs: the issue templates ask for `rbx --version`, your OS, and redacted
command output. Redact ids and keys — the shape of the failure is what matters.

Security vulnerabilities do **not** go in the issue tracker. See
[SECURITY.md](./SECURITY.md) for the private advisory channel.
