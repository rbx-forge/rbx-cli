# Contributing

Thanks for looking. This is a solo-maintained project, so before writing code:
**open an issue first for anything larger than a bug fix.** A PR that arrives
without one may be turned down for reasons that have nothing to do with its
quality: see [What rbx does not do](./docs/reference/non-goals.md),
which is where scope decisions are recorded.

Bug fixes with a test need no preamble. Send them.

## Setup

```sh
git clone https://github.com/rbx-forge/rbx-deploy
cd rbx-deploy
lefthook install          # one-time: pre-commit runs cargo fmt + clippy
cargo test --workspace
```

**MSRV is Rust 1.88**, declared in the workspace manifest. It is
dependency-driven, not a target of its own: our sources need 1.82.

No Roblox credentials are needed to build or test: every HTTP path is tested
against `wiremock`, never against the live API.

## Where code goes

The workspace is one binary crate, one crate per domain, one shared core.
[ARCHITECTURE.md](./ARCHITECTURE.md) is the map; read it before adding files.
The short version, which every domain crate follows:

- `lib.rs`: the clap `Args`/`Subcommand` for that tool plus `run(...)`
  dispatch, and nothing else.
- `api/`: a thin client newtype over `reqwest`, one method per endpoint, built
  through `rbx_core::api::build_client*`.
- `commands/`: one module per subcommand. The logic lives here.
- `config.rs` / `lockfile.rs`: the declarative `*.toml` model and, for
  stateful tools, the record of what was last synced to Roblox.

Retries belong to `rbx_core::api`: there is exactly one retry loop in the
workspace, and a new crate does not get its own.

## What makes a PR mergeable

- **`cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings`
  are clean.** The pre-commit hook runs both; CI runs them again.
- **`cargo test --workspace` passes**, and a behavior change comes with a test
  that fails without the change. HTTP behavior is tested with `wiremock`, and
  the test asserts the *request* that was emitted, not only the response that
  was parsed: a client that sends the wrong body against a permissive mock is
  the failure mode that matters.
- **Generated output changes go through `insta`.** `rbx shop codegen` and
  `rbx env codegen` snapshot the whole emitted file; accept intentional
  changes with `cargo insta review` and let the diff be reviewed.
- **Docs move with the code.** A user-visible change updates the relevant
  `docs/*.md` page and adds a `## [Unreleased]` line to
  [CHANGELOG.md](./CHANGELOG.md). That entry is what the release notes are
  built from, so write it for somebody deciding whether to upgrade rather than
  as a summary of the commit. `docs/` describes `main`.
- **No new dead code.** Dead code is denied crate-wide; something deliberately
  unused carries a narrow `#[allow(dead_code)]` with a reason.

### Comment style

Comments state constraints the code cannot express: why this order, why not the
obvious alternative, what breaks if it changes. They do not narrate what the
next line does. **Prefer silence to narration**: an unnecessary comment is a
review comment here.

### Punctuation

No em dash (U+2014, the long one). A colon, a comma, a full stop or a pair of
parentheses says the same thing, and one of them is always right: a colon where
the second half explains the first, parentheses around an aside, a comma before
a connective, and a full stop when the sentence was really two.

This is enforced by CI rather than left to discipline, because a single one
slipping into a doc page or a `--help` string is invisible in review and
permanent once released. `spec/`, `schemas/` and `THIRD-PARTY-NOTICES.md` are
exempt: they are vendored, generated, or somebody else's text.

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
right to contribute it under this project's license: the full text is at
<https://developercertificate.org/>. No automated check enforces it: it is
looked for in review, and a missing sign-off is fixable with
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

## What runs automatically

Eight things, and nothing else. There is no release-plz and no auto-merge:
every version bump is a deliberate act.

| What | Trigger | Does |
| --- | --- | --- |
| `lefthook` (local) | `git commit` | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo doc` with warnings denied |
| `.github/workflows/ci.yml` | push / PR on `main` | eleven jobs: Test, Rustfmt, Punctuation, Clippy, No unused dependencies, Schemas up to date, Doc, MSRV (1.88), Shell completions, plus a Windows leg (platform-dependent crates) and a cross-target job (macOS check, musl build) |
| `.github/workflows/supply-chain.yml` | push / PR on `main` touching the dependency graph, **and weekly** (Monday 06:00 UTC) | `cargo deny check` for advisories, licenses, bans and sources |
| `.github/workflows/release.yml` | pushing a `v*` tag | builds four targets (Linux x86-64 gnu and musl, Windows x86-64, macOS aarch64), zips them, generates `SHA256SUMS`, attests build provenance, creates the GitHub Release |
| `.github/workflows/update-openapi.yml` | daily, 06:17 UTC | refreshes the vendored Roblox OpenAPI document, runs the drift check against it, and opens a PR if it changed |
| `.github/workflows/docs.yml` | push on `main`, PRs touching `docs/` or `mkdocs.yml` | builds the MkDocs site from `docs/`; deploys to GitHub Pages on `main` only |
| `.github/workflows/scheduled-failure.yml` | a scheduled run of Supply chain or Update OpenAPI spec failing | opens an issue for that workflow, or comments on the one already open |
| Dependabot | monthly, 06:00 Paris | one grouped PR bumping GitHub Actions versions |

The heavy jobs run on Linux, where runners bill at 1x; macOS is a
`cargo-zigbuild check` of the target the release ships rather than a rented
mac, and Windows runs only the crates whose code is platform-dependent.
Everything else is checked once. Full cross-platform *compilation* is proven at
tag time by `release.yml`, which builds all four targets before publishing
anything.

musl is the one cross target that is *built* rather than checked, and the
difference is the point: what breaks a static artifact is linking, which
`cargo check` never reaches. The job also asserts the result is actually static,
so a dependency quietly reintroducing a shared libc fails rather than producing
a musl binary with the same portability problem as the gnu one.

The weekly `supply-chain.yml` run is not redundant with its PR trigger. An
advisory is published against a version already in `Cargo.lock`: nothing in this
repository moves, and a check that only ran on push would stay green until
somebody happened to touch a manifest.

Dependabot handles **GitHub Actions only**. Cargo version updates are off
deliberately. Dependabot Security Alerts still open a PR when an advisory hits
`Cargo.lock`; `cargo-deny` is the second half of that story, because it also
answers the three questions an advisory feed does not, whether a license
reached a statically linked artifact it should not have, whether a crate came
from an unexpected registry, and how much duplication the graph has grown.

**That is not the same as nothing being wrong.** Alerts cover what GitHub's
database knows and rates as worth interrupting for; `cargo audit` reads the
RustSec advisory database and reports more, including unsound-but-unrated
crates. Nothing in CI runs it today, which is why an external audit found
advisories the Security tab was quiet about. Removing default features from
`image` and `rbx_cookie` took the dependency count from 424 to 354 and removed
two of those advisories by removing the crates that carried them.

## Open Cloud API drift check

When Roblox renames a field or moves an endpoint, we used to find out from a
user hitting `Failed to parse response` at runtime. CI now tells us first.

`spec/openapi.json` is a byte-for-byte snapshot of the OpenAPI document Roblox
publishes in [`Roblox/creator-docs`][creator-docs] (`content/en-us/reference/cloud/openapi.json`).
`spec/source.json` records the exact upstream commit it came from, and
`spec/NOTICE.md` carries the attribution: the document is CC BY 4.0, unlike
the rest of this repository.

The test in `crates/rbx-spec-drift/tests/openapi_drift/main.rs` reads every Roblox
URL the workspace builds and asserts each path still exists in that snapshot.
Matching is structural and host-aware: literal segments must match exactly,
`{...}` segments are wildcards (our Rust variable names never have to agree
with Roblox's parameter names), and a path only counts as present if the
document describes it on the same host we call it on. It runs as part of
`cargo test --workspace`, or on its own:

```sh
cargo test -p rbx-spec-drift --test openapi_drift
```

When it fails it names the missing path, every `file:line` that calls it, and
the closest paths Roblox still documents, which usually identifies the rename
outright.

**Its limits are documented in that file's module comment, and worth reading
before trusting it.** In short: it checks that endpoint *paths* exist, the fields
`rbx shop` sends, and the fields it reads back. Elsewhere it cannot see a
renamed response field, and everywhere it compares against what Roblox
documents rather than what Roblox does, so it narrows the blast radius rather
than eliminating it. Endpoints Roblox has never documented (the
`experience-releases` v1beta1 API, legacy `universes/v1` creation calls) are
listed in `KNOWN_UNDOCUMENTED` with a reason each: that list is where we admit
we have no early warning for something.

**Refreshing the snapshot** is the `update-openapi` workflow's job. It runs
daily, and can be triggered by hand from the Actions tab. It re-fetches the
document, rewrites `spec/source.json`, runs the drift check, and opens a PR
with the result quoted in the body. It never pushes to `main`: whether a Roblox
change breaks us is a judgement call, so a human always reviews it. A refresh
that changes nothing opens no PR.

Do not hand-edit `spec/openapi.json` or reformat it: diffs between refreshes
have to show real Roblox changes and nothing else.

[creator-docs]: https://github.com/Roblox/creator-docs

## Cutting a release

1. Run the live checks against a test universe. CI never talks to Roblox ([#91](https://github.com/rbx-forge/rbx-deploy/issues/91)), so this is what proves the build against the real API: the read commands and an upload of an unchanged place. Nothing that cannot be deleted gets created. The script stays out of the repository because it holds a real universe id, and it has to run locally because the key it uses is limited to one IP.
2. Bump `[workspace.package].version` in `Cargo.toml`, then `cargo check` to refresh `Cargo.lock`
3. Move `## [Unreleased]` content to `## [X.Y.Z] - YYYY-MM-DD` in `CHANGELOG.md` (no `v` in the heading)

   The heading spelling is load-bearing: `release.yml` reads the section back
   out of this file by matching `## [X.Y.Z]` against the tag it was pushed
   with, and publishes it as the release body, with GitHub's generated pull
   request list appended under it. So `CHANGELOG.md` is the release notes, and
   the two cannot drift apart into two wordings of the same change.

   A tag whose version has no section here fails the release job before
   anything is published, rather than putting a release on the page with an
   empty body.
4. Commit, then tag and push: `git tag -a vX.Y.Z -m "vX.Y.Z" && git push origin main --follow-tags`

   The `-a` is not optional. `--follow-tags` pushes **annotated** tags only, so a
   plain `git tag vX.Y.Z` is created locally, silently skipped by the push, and
   `release.yml` never fires: the commit lands on `main` and nothing else
   happens. Every release from v0.2.0 on is annotated.
5. `release.yml` picks up the tag and publishes the binaries

If you delete a release, delete its tag too (`git push origin :vX.Y.Z`).
Re-pushing an existing tag re-triggers `release.yml` against stale code: that
is how the cancelled v0.5.3 run of 2026-06-15 happened.

## Reporting bugs and security issues

Bugs: the issue templates ask for `rbx --version`, your OS, and redacted
command output. Redact ids and keys: the shape of the failure is what matters.

Security vulnerabilities do **not** go in the issue tracker. See
[SECURITY.md](./SECURITY.md) for the private advisory channel.
