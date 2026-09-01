//! Every flag `rbx` accepts has to be written down somewhere.
//!
//! This is the dual of [`crate::docs_drift`], and the two fail in opposite
//! directions on purpose. That one hands the pages to clap and catches prose
//! describing a CLI that no longer exists. It cannot catch the reverse: a flag
//! added without a documentation line is invisible to a test that only reads
//! what the pages already say.
//!
//! The reverse is the one that rots quietly. A stale page is caught the first
//! time somebody follows it and it does not work. An undocumented flag is never
//! caught at all, because nobody goes looking for a thing they were not told
//! about, and the tool ends up with capabilities only its author knows.
//!
//! "Documented" here means the literal string `--flag` appears somewhere in the
//! page. Nothing stricter, deliberately: requiring a table row or a named
//! section would prescribe how a page is laid out, and these pages are prose
//! with tables rather than generated reference. The test's business is whether
//! a reader can find the flag at all, not where.

use std::collections::BTreeSet;

use clap::CommandFactory;

use crate::Cli;

/// Where the pages live, relative to this crate's manifest directory.
const DOCS: &str = "../../docs";

/// Which page answers for which top-level subcommand.
///
/// Mirrors the `nav` in `mkdocs.yml`, which is where a reader is sent from, so
/// the mapping is read off the site rather than invented here. It is spelled
/// out rather than parsed because `nav` is prose in places (`rbx check and rbx
/// status` covers two commands under one page, `Live operations` covers none),
/// and a parser for that would be guessing at exactly the entries that need a
/// decision.
///
/// Every top-level subcommand must appear, `probe` included: it is hidden from
/// the command list but fully supported and documented, so being hidden buys it
/// no exemption from being written down.
const PAGES: &[(&str, &str)] = &[
    ("init", "init.md"),
    ("import", "import.md"),
    ("env", "env.md"),
    ("apikey", "apikey.md"),
    ("doctor", "doctor.md"),
    ("check", "check.md"),
    ("status", "check.md"),
    ("place", "place.md"),
    ("meta", "meta.md"),
    ("config", "config.md"),
    ("secret", "secret.md"),
    ("rtbf", "rtbf.md"),
    ("shop", "shop.md"),
    ("open", "open.md"),
    ("download", "download.md"),
    ("servers", "ops/servers.md"),
    ("analytics", "ops/analytics.md"),
    ("ban", "ops/ban.md"),
    ("restart", "ops/restart.md"),
    ("data", "ops/data.md"),
    ("memorystore", "ops/memorystore.md"),
    ("message", "ops/message.md"),
    ("ads", "ops/ads.md"),
    ("probe", "ops/probe.md"),
    ("completions", "completions.md"),
];

/// Flags that exist and are deliberately written down nowhere, each with the
/// reason.
///
/// The same device as `docs_drift`'s `KNOWN_MISMATCHES`, and it earns its
/// keep the same way: an entry here is a decision somebody made and can be
/// argued with later, where a silently skipped flag is a decision nobody
/// recorded. Adding one should feel like writing a sentence, because it is.
const UNDOCUMENTED_ON_PURPOSE: &[(&str, &str)] = &[
    (
        "--base-url",
        "Hidden test seam. It points the client at a mock server so the \
         integration tests can drive real commands, and it is not a thing to do \
         to a live Roblox account. Documenting it would advertise an override \
         with no supported use.",
    ),
    (
        "--users-url",
        "The same seam as --base-url, for the second host `rbx ban` talks to. \
         `users.roblox.com` resolves a username to an id, which is a different \
         service from the one the bans go to, so the mock server needs to be \
         named twice. Listed separately rather than folded in, because a reader \
         of this list should not have to know that `ban` has two hosts to \
         understand why two flags are here.",
    ),
];

fn deliberate(flag: &str) -> Option<&'static str> {
    UNDOCUMENTED_ON_PURPOSE
        .iter()
        .find(|(name, _)| *name == flag)
        .map(|(_, why)| *why)
}

fn docs_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(DOCS)
}

/// One page's text, or the empty string when the file is missing. A missing
/// page is reported by [`every_documented_page_exists`] rather than here, so a
/// typo in `PAGES` fails once with its own message instead of once per flag.
fn page_text(relative: &str) -> String {
    std::fs::read_to_string(docs_root().join(relative)).unwrap_or_default()
}

/// Every page's text joined together, for the flags that are not any one
/// command's to document.
fn all_pages() -> String {
    let mut pages = Vec::new();
    collect_markdown(&docs_root(), &mut pages);
    pages.sort();
    pages
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

fn collect_markdown(directory: &std::path::Path, into: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            collect_markdown(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            into.push(path);
        }
    }
}

/// One long flag, and the command path it was reached through.
///
/// The path is for the failure message: `place upload --yes` sends a reader
/// somewhere, where `--yes` on its own sends them to fourteen commands that
/// have one.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Flag {
    path: String,
    long: String,
    /// A `global = true` argument, which clap propagates into every
    /// subcommand. It belongs to no single page, so it is checked against all
    /// of them.
    global: bool,
}

/// Walk a command and everything under it, collecting long flags.
///
/// `--help` and `--version` are clap's own and are skipped: they are not this
/// tool's to document, and no page should have to say so. A `hide = true`
/// argument is *not* skipped, because hiding a flag from `--help` is a
/// statement about the help output rather than about whether the flag exists;
/// one that should also go undocumented belongs in
/// [`UNDOCUMENTED_ON_PURPOSE`] with the reason written out.
fn flags(command: &clap::Command, path: &str, into: &mut BTreeSet<Flag>) {
    for argument in command.get_arguments() {
        let id = argument.get_id().as_str();
        if id == "help" || id == "version" {
            continue;
        }
        let Some(long) = argument.get_long() else {
            continue;
        };
        into.insert(Flag {
            path: path.to_string(),
            long: format!("--{long}"),
            global: argument.is_global_set(),
        });
    }

    for sub in command.get_subcommands() {
        // clap synthesises a `help` subcommand. It has no page and needs none.
        if sub.get_name() == "help" {
            continue;
        }
        let child = if path.is_empty() {
            sub.get_name().to_string()
        } else {
            format!("{path} {}", sub.get_name())
        };
        flags(sub, &child, into);
    }
}

/// The page answering for a flag reached through `path`, by its first segment.
fn page_for(path: &str) -> Option<&'static str> {
    let top = path.split_whitespace().next()?;
    PAGES
        .iter()
        .find(|(name, _)| *name == top)
        .map(|(_, page)| *page)
}

#[test]
fn every_top_level_command_has_a_page() {
    let command = Cli::command();
    let mut missing = Vec::new();

    for sub in command.get_subcommands() {
        let name = sub.get_name();
        if name == "help" {
            continue;
        }
        if page_for(name).is_none() {
            missing.push(name.to_string());
        }
    }

    assert!(
        missing.is_empty(),
        "no page mapped for: {}\n\nAdd it to PAGES here and to the nav in \
         mkdocs.yml. A subcommand nobody can be sent to a page for is one \
         nobody can be told about.",
        missing.join(", ")
    );
}

#[test]
fn every_mapped_page_exists() {
    let root = docs_root();
    let missing: Vec<&str> = PAGES
        .iter()
        .map(|(_, page)| *page)
        .filter(|page| !root.join(page).is_file())
        .collect();

    assert!(
        missing.is_empty(),
        "PAGES names {} file(s) that do not exist: {}",
        missing.len(),
        missing.join(", ")
    );
}

#[test]
fn the_command_tree_has_flags_to_check() {
    // The guard against this test quietly measuring nothing, the same one
    // `docs_drift` carries. A walk that stops finding arguments passes forever.
    let mut found = BTreeSet::new();
    flags(&Cli::command(), "", &mut found);

    let count = found.len();
    assert!(
        count > 100,
        "only {count} long flags found on the command tree. Either the CLI lost \
         most of its arguments or the walk stopped descending."
    );
}

#[test]
fn every_flag_is_written_down_somewhere() {
    let mut found = BTreeSet::new();
    flags(&Cli::command(), "", &mut found);

    let everything = all_pages();
    let mut undocumented = Vec::new();

    for flag in &found {
        if deliberate(&flag.long).is_some() {
            continue;
        }

        // A global argument is propagated into every subcommand, so asking its
        // own page to mention it would ask twenty pages to repeat one flag.
        // Anywhere in the docs is the honest bar for those.
        let (haystack, where_looked) = if flag.global {
            (everything.clone(), "anywhere in docs/".to_string())
        } else {
            match page_for(&flag.path) {
                Some(page) => (page_text(page), format!("docs/{page}")),
                // Reported by `every_top_level_command_has_a_page`, which says
                // it once rather than once per flag.
                None => continue,
            }
        };

        if !haystack.contains(&flag.long) {
            undocumented.push(format!(
                "  rbx {} {}  is in no page\n    looked in: {}",
                flag.path, flag.long, where_looked
            ));
        }
    }

    assert!(
        undocumented.is_empty(),
        "{} flag(s) exist and are documented nowhere:\n{}\n\nEither the page \
         needs a line, or the omission is deliberate and belongs in \
         UNDOCUMENTED_ON_PURPOSE with the reason written out.",
        undocumented.len(),
        undocumented.join("\n")
    );
}

/// An entry that stops describing anything has to come out, the same rule
/// `docs_drift` applies to its own list. A flag documented after all is the
/// good outcome, and leaving the entry behind would exempt it again the day
/// somebody deletes that line.
#[test]
fn nothing_is_listed_as_undocumented_while_being_documented() {
    let everything = all_pages();
    let stale: Vec<String> = UNDOCUMENTED_ON_PURPOSE
        .iter()
        .filter(|(flag, _)| everything.contains(*flag))
        .map(|(flag, why)| format!("  {flag}  is documented now\n    it was listed as: {why}"))
        .collect();

    assert!(
        stale.is_empty(),
        "{} UNDOCUMENTED_ON_PURPOSE entry/entries describe nothing:\n{}",
        stale.len(),
        stale.join("\n")
    );
}
