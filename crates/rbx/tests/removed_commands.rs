//! Nothing may tell a reader to run a command `rbx` no longer has.
//!
//! The live operations, API key management, the Studio launcher and the asset
//! downloader run in tools of their own. A help text, an error, a comment or a
//! page still saying `rbx data get` sends the reader to `unrecognized
//! subcommand`, which reads as a broken install rather than as a stale hint.
//!
//! # What is checked
//!
//! Every `.rs`, `.md` and `.toml` file under `crates/`, every page under
//! `docs/`, the README, and the `[Unreleased]` section of the changelog. The
//! released sections of the changelog are history and are not read.
//!
//! The one place these commands may still be named is the `### Removed` entry
//! of `[Unreleased]`, which exists to say what each one became.
//!
//! `crates/rbx-core` is skipped: it is a copy of the rbx-core library, kept
//! identical to that library's own sources until the workspace takes it as a
//! git dependency, so its wording is that library's to change.

use std::fs;
use std::path::{Path, PathBuf};

/// The top-level commands that left.
const REMOVED: &[&str] = &[
    "apikey",
    "doctor",
    "open",
    "download",
    "secret",
    "user",
    "servers",
    "analytics",
    "ban",
    "group",
    "restart",
    "data",
    "memorystore",
    "message",
    "ads",
];

/// This file's own name: it spells the commands out, which is the point.
const SELF: &str = "removed_commands.rs";

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <root>/crates/rbx.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the rbx crate should live two levels below the workspace root")
        .to_path_buf()
}

fn collect(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            collect(&path, extensions, out);
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| extensions.contains(&ext))
        {
            out.push(path);
        }
    }
}

/// The `[Unreleased]` section of the changelog, minus its `### Removed` entry.
fn unreleased_without_removed(changelog: &str) -> String {
    let Some(start) = changelog.find("## [Unreleased]") else {
        return String::new();
    };
    let rest = &changelog[start..];
    let end = rest[1..].find("\n## [").map_or(rest.len(), |at| at + 1);
    let section = &rest[..end];

    let mut kept = String::new();
    let mut in_removed = false;
    for line in section.lines() {
        if line.starts_with("### ") {
            in_removed = line.trim() == "### Removed";
        }
        if !in_removed {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    kept
}

/// Every `rbx <removed>` in `text`: `rbx` on its own, a space, then one of the
/// removed names as a whole word. `rbx-ops data` and `rbxapikey.toml` are not
/// matches.
fn mentions(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.';
    let mut found = Vec::new();
    for (at, _) in text.match_indices("rbx ") {
        if at > 0 && word(bytes[at - 1]) {
            continue;
        }
        let after = &text[at + 4..];
        for name in REMOVED {
            if let Some(tail) = after.strip_prefix(name) {
                if tail.as_bytes().first().is_none_or(|b| !word(*b)) {
                    found.push(format!("rbx {name}"));
                }
            }
        }
    }
    found
}

#[test]
fn no_file_names_a_removed_command() {
    let root = workspace_root();
    let mut files = Vec::new();
    collect(&root.join("crates"), &["rs", "md", "toml"], &mut files);
    collect(&root.join("docs"), &["md"], &mut files);
    files.push(root.join("README.md"));
    let core = root.join("crates").join("rbx-core");

    let mut offenders = Vec::new();
    for path in &files {
        if path.starts_with(&core) || path.file_name().is_some_and(|name| name == SELF) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            for hit in mentions(line) {
                let shown = path.strip_prefix(&root).unwrap_or(path).display();
                offenders.push(format!("  {shown}:{}: {hit}", number + 1));
            }
        }
    }

    let changelog = fs::read_to_string(root.join("CHANGELOG.md")).expect("CHANGELOG.md exists");
    for line in unreleased_without_removed(&changelog).lines() {
        for hit in mentions(line) {
            offenders.push(format!("  CHANGELOG.md [Unreleased]: {hit}"));
        }
    }

    assert!(
        offenders.is_empty(),
        "these name a command rbx no longer has. Name the tool that runs it now \
         (rbx-ops, rbx-apikey, rbx-open, rbx-download), or drop the mention:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_matcher_finds_a_command_and_spares_the_other_tools() {
    assert_eq!(mentions("run `rbx data get --datastore X`"), ["rbx data"]);
    assert_eq!(mentions("rbx ban list"), ["rbx ban"]);
    assert!(mentions("run `rbx-ops data get`").is_empty());
    assert!(mentions("rbx-apikey create, rbxapikey.toml").is_empty());
    assert!(mentions("rbx database").is_empty());
    assert!(mentions("rbx shop sync").is_empty());
}

#[test]
fn only_the_removed_entry_of_unreleased_is_exempt() {
    let changelog = "\
## [Unreleased]

### Removed

- `rbx data` is `rbx-ops data`.

### Fixed

- `rbx ban` was here.

## [0.1.0]

- `rbx open` shipped.
";
    let kept = unreleased_without_removed(changelog);
    assert_eq!(mentions(&kept), ["rbx ban"]);
}

/// The walk has to reach the sources, or the first test passes on nothing.
#[test]
fn the_walk_reaches_the_crates_and_the_docs() {
    let root = workspace_root();
    let mut files = Vec::new();
    collect(&root.join("crates"), &["rs", "md", "toml"], &mut files);
    collect(&root.join("docs"), &["md"], &mut files);
    assert!(
        files.iter().any(|p| p.ends_with("crates/rbx/src/main.rs")),
        "the walk missed crates/rbx/src/main.rs"
    );
    assert!(
        files.iter().any(|p| p.ends_with("docs/index.md")),
        "the walk missed docs/index.md"
    );
}
