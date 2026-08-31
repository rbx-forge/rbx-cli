//! Every `rbx …` in `docs/` has to still parse.
//!
//! The site cannot drift from the repository: `mkdocs.yml` renders `docs/` in
//! place, nothing is copied, and `strict: true` refuses a nav entry or an
//! anchor that does not resolve. None of that says the prose describes the
//! code. These pages are written by hand, on purpose, because a generated
//! reference would not explain why an overwrite is unrecoverable or why
//! deleting is the gentler of the two. What a generator *would* have kept
//! correct is the mechanical half, and that is what this pins instead.
//!
//! So: pull every command line out of the fenced blocks and hand it to clap.
//! `try_parse_from` never runs anything, touches no network and needs no key,
//! which is why this can be an ordinary unit test.
//!
//! It catches the three ways a page goes stale without anybody noticing: a
//! flag renamed, a subcommand gone, and a combination that used to be legal.
//! The third is not hypothetical. `--json` on a write started requiring
//! `--yes`, and every documented example without it became wrong the same
//! minute; the docs were fixed by attention rather than by anything that would
//! have failed.
//!
//! What it deliberately does not check is whether the *prose* is true. Nothing
//! can.

use clap::Parser;

use crate::Cli;

/// Where the pages live, relative to this crate's manifest directory
/// (`crates/rbx`), which is what `CARGO_MANIFEST_DIR` gives.
const DOCS: &str = "../../docs";

/// Fences whose contents are commands. Everything else in a page is output, a
/// usage synopsis or a config file, and all three are shaped enough like a
/// command line to be mistaken for one: `rbx download [IDS]... [OPTIONS]` is a
/// synopsis, and `rbx = "rbx-forge/rbx-cli"` is a line of `rokit.toml`.
const SHELL_FENCES: &[&str] = &["sh", "bash", "shell", "zsh", "console"];

/// Lines clap refuses today, each with the reason it is listed.
///
/// Two kinds end up here and they are not the same thing. An example that is
/// *meant* to be refused belongs here permanently. A page that documents
/// behaviour the CLI does not have belongs here only until one of the two is
/// fixed, and saying which is which is the whole point of writing the reason
/// down: an unexplained allowlist is how a known bug becomes a forgotten one.
/// Empty, and it has been non-empty exactly once. The first run of this test
/// caught `place versions --place-id` and `place download --place-id`, both
/// documented under "Working without rbxplace.toml" and both refused by clap
/// for a missing `--env`. The docs described the behaviour that was designed
/// and the CLI had the other one, so the CLI was fixed
/// (`required_unless_present = "place_id"` on those two reads) and the entries
/// came out in the same commit. Keeping the list is the point: the next
/// disagreement gets written down here with its reason rather than quietly
/// deleted from a page.
const KNOWN_MISMATCHES: &[(&str, &str)] = &[];

fn known_mismatch(line: &str) -> Option<&'static str> {
    KNOWN_MISMATCHES
        .iter()
        .find(|(text, _)| *text == line)
        .map(|(_, why)| *why)
}

/// Splits a shell line into arguments, respecting quotes, and stops at the
/// first thing that is shell rather than argument.
///
/// A pipe, a redirect, `&&` or a trailing `#` comment all end the invocation:
/// `rbx env list --json | jq -r '.envs[].name'` is one command and one filter,
/// and handing clap the filter is how a good example fails a bad test.
fn arguments(line: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;

    let mut characters = line.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '\'' | '"' if quote.is_none() => quote = Some(character),
            c if Some(c) == quote => quote = None,

            // Shell from here on, whatever follows.
            '|' | '>' | ';' if quote.is_none() => break,
            '&' if quote.is_none() && characters.peek() == Some(&'&') => break,
            '#' if quote.is_none() && !started => break,
            '#' if quote.is_none() && current.is_empty() => break,

            // A trailing backslash is a continuation, and this reads one line
            // at a time, so the example is incomplete rather than wrong.
            '\\' if quote.is_none() && characters.peek().is_none() => return None,

            c if c.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                    started = true;
                }
            }

            c => current.push(c),
        }
    }

    if !current.is_empty() {
        args.push(current);
    }

    // An unbalanced quote means the example spans lines. Not this test's to
    // judge.
    if quote.is_some() {
        return None;
    }

    Some(args)
}

/// One command line found in a page, kept with where it came from so a failure
/// names the file and line rather than the string alone.
struct Invocation {
    file: String,
    line: usize,
    text: String,
    args: Vec<String>,
}

fn invocations() -> Vec<Invocation> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(DOCS);
    let mut found = Vec::new();
    let mut pages = Vec::new();

    collect_markdown(&root, &mut pages);
    pages.sort();

    for page in pages {
        let Ok(text) = std::fs::read_to_string(&page) else {
            continue;
        };

        let name = page
            .strip_prefix(&root)
            .unwrap_or(&page)
            .display()
            .to_string()
            .replace('\\', "/");

        let mut shell = false;
        let mut fenced = false;

        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim();

            if let Some(info) = trimmed.strip_prefix("```") {
                if fenced {
                    fenced = false;
                    shell = false;
                } else {
                    fenced = true;
                    shell = SHELL_FENCES.contains(&info.trim());
                }
                continue;
            }

            // Only inside a shell fence. `rbx` written mid-sentence is prose,
            // and prose is allowed to be shaped like a command without being
            // one.
            if !shell {
                continue;
            }

            if !trimmed.starts_with("rbx ") {
                continue;
            }

            let Some(args) = arguments(trimmed) else {
                continue;
            };

            if args.len() < 2 {
                continue;
            }

            found.push(Invocation {
                file: name.clone(),
                line: index + 1,
                text: trimmed.to_string(),
                args,
            });
        }
    }

    found
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

#[test]
fn the_docs_contain_commands_to_check() {
    // A test that silently stops finding anything passes forever. This is the
    // guard against the extractor breaking rather than the docs improving.
    let count = invocations().len();

    assert!(
        count > 100,
        "only {count} `rbx …` lines found in docs/. Either the pages lost their \
         examples or the extractor stopped seeing fenced blocks."
    );
}

#[test]
fn every_documented_invocation_still_parses() {
    let mut broken = Vec::new();

    for invocation in invocations() {
        let listed = known_mismatch(&invocation.text);
        let outcome = Cli::try_parse_from(&invocation.args);

        match (outcome, listed) {
            // Parses and was not expected to: the ordinary case.
            (Ok(_), None) => {}

            // Refused, and nobody said it would be.
            (Err(error), None) => {
                let first = error
                    .to_string()
                    .lines()
                    .find(|line| line.starts_with("error:"))
                    .unwrap_or("")
                    .to_string();

                broken.push(format!(
                    "  docs/{}:{}  no longer parses\n    {}\n    {first}",
                    invocation.file, invocation.line, invocation.text
                ));
            }

            // Refused and listed: recorded, and left alone.
            (Err(_), Some(_)) => {}

            // Listed as refused but accepted, so whatever it recorded is
            // settled and the entry is now describing nothing.
            (Ok(_), Some(why)) => broken.push(format!(
                "  docs/{}:{}  parses now, so its KNOWN_MISMATCHES entry is stale\n    {}\n    it \
                 was listed as: {why}",
                invocation.file, invocation.line, invocation.text
            )),
        }
    }

    assert!(
        broken.is_empty(),
        "{} documented command(s) disagree with the CLI:\n{}\n\nEither a flag moved \
         and the page needs updating, or the disagreement is real and belongs in \
         KNOWN_MISMATCHES with the reason written out.",
        broken.len(),
        broken.join("\n")
    );
}
