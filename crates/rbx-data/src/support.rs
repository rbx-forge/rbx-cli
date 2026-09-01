//! The pieces the subcommand modules share: the write path behind four of them,
//! the backup resolution every write goes through, and the small helpers around
//! `diff`.
//!
//! Extracted from `lib.rs` with the dispatch, and kept out of `commands/`
//! because two of the three modules there use them and neither owns them.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colored::Colorize;

use rbx_core::confirm::confirm_always;
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

use crate::api::Api;
use crate::backup::{self, BackupTarget};
use crate::json::{DiffSource, Store, WriteDocument};
use crate::model::EntryUpdate;
/// One side of `data diff`, before it reaches a file.
///
/// The three facts travel together: what to call the file, what goes in it,
/// and which comparison the side came from. Keeping the last one rather than
/// re-deriving it later is what lets the JSON document say `revision` or `env`
/// without parsing the label back apart.
pub(crate) struct Side {
    pub(crate) label: String,
    pub(crate) value: Option<serde_json::Value>,
    pub(crate) source: DiffSource,
}

/// The four decisions a write carries, which four commands each pass through
/// unchanged. Grouped because they travel together and always have: as separate
/// parameters they were three consecutive bools at one call site, which is a
/// swap waiting to happen.
pub(crate) struct WriteOptions {
    pub(crate) backup: BackupTarget,
    pub(crate) drop_metadata: bool,
    pub(crate) apply: bool,
    pub(crate) yes: bool,
    /// What the document calls this: `set`, `reset`, `restore` or `copy`.
    /// One code path, four names, and a caller reading the document wants the
    /// one it asked for rather than the one they share.
    pub(crate) action: &'static str,
    pub(crate) format: OutputFormat,
}

/// The three backup flags, resolved against the env the write lands in.
///
/// Kept as a struct rather than four positional arguments because the two
/// paths (`--backup`, `--places`) and the two names (env, entry) are all
/// strings and paths that read alike at a call site.
pub(crate) struct BackupFlags {
    pub(crate) backup: Option<PathBuf>,
    pub(crate) no_backup: bool,
    pub(crate) keep: u32,
}

/// Which subdirectory of `.rbx/backups/` this write belongs to.
///
/// `--env` is the operator's own name for the target and the one they will
/// look under months later. Without it (`--universe-id` on its own) the
/// universe is the only name there is, and it is still better than one shared
/// pile.
pub(crate) fn env_label(env: Option<&str>, universe_id: u64) -> String {
    match env {
        Some(name) => name.to_string(),
        None => format!("universe-{universe_id}"),
    }
}

/// Resolve the backup flags for a write landing in `env` / `universe_id`.
///
/// `global.places` is read for its directory only: the default backup
/// directory sits beside `rbxplace.toml`, so a copy of a player profile is
/// where the project is rather than where the shell was.
pub(crate) fn backup_target(
    flags: BackupFlags,
    global: &GlobalFlags,
    env: Option<&str>,
    universe_id: u64,
) -> BackupTarget {
    BackupTarget::resolve(
        flags.backup,
        flags.no_backup,
        flags.keep,
        &global.places,
        &env_label(env, universe_id),
    )
}

/// The shared write path behind `set`, `reset`, `restore` and `copy`.
pub(crate) async fn write_entry(
    api: &Api,
    entry: &str,
    raw: &str,
    options: WriteOptions,
    universe_id: u64,
    store: &Store,
) -> Result<()> {
    let WriteOptions {
        backup,
        drop_metadata,
        apply,
        yes,
        action,
        format,
    } = options;

    let mut document = WriteDocument::new(store, entry, action);
    let new_value: serde_json::Value =
        serde_json::from_str(raw).context("the new value must be valid JSON")?;

    let existing = api.get(entry).await?;

    document.existed = existing.is_some();

    format.note(format!("entry {entry}").bold());
    match &existing {
        Some(found) => {
            let current = found.value.clone().unwrap_or(serde_json::Value::Null);
            format.note("  current".dimmed());
            format.note(indent(&serde_json::to_string_pretty(&current)?));
            if let Some(users) = &found.users {
                format.note(format!("  users      {}", users.join(", ").dimmed()));
            }
        }
        None => format.note("  does not exist yet, it will be created".dimmed()),
    }
    format.note("  new".dimmed());
    format.note(indent(&serde_json::to_string_pretty(&new_value)?));

    let update = if drop_metadata {
        EntryUpdate::bare(new_value)
    } else {
        EntryUpdate::preserving(new_value, existing.as_ref())
    };
    if drop_metadata && existing.as_ref().and_then(|e| e.users.as_ref()).is_some() {
        format.note("  --drop-metadata: the user association will be removed".yellow());
    }
    format.note("");

    if !apply {
        format.note("Nothing written. Re-run with --apply to overwrite.".yellow());

        if format.is_json() {
            return output::emit(&document);
        }

        return Ok(());
    }

    // Written before the request, not after: if the write succeeds and the
    // value was wrong, this file is what gets you back, and asking Roblox for
    // the old revision needs another scope and another call.
    match (&existing, &backup) {
        (Some(found), BackupTarget::Path(_) | BackupTarget::Managed { .. }) => {
            let contents = serde_json::to_string_pretty(
                &found.value.clone().unwrap_or(serde_json::Value::Null),
            )?;
            let written = backup::write(&backup, entry, &contents)?;
            document.backup = Some(written.path.display().to_string());
            format.note(format!("backup written to {}", written.path.display()));
            if written.pruned > 0 {
                format.note(
                    format!(
                        "  {} older backup(s) of {entry} removed by --keep",
                        written.pruned
                    )
                    .dimmed(),
                );
            }
        }
        // Said out loud every time. The prompt below is the last chance to
        // stop, and it should not be the first place you learn that the value
        // about to be replaced is not being kept anywhere.
        (Some(_), BackupTarget::Skip) => {
            format.note(
                "--no-backup: no local copy. Unless this experience has been snapshotted today, \
                 the current value is gone the moment this write lands."
                    .yellow(),
            );
        }
        // Nothing to copy: the entry does not exist yet, so the write creates
        // it and there is no previous value to lose.
        (None, _) => {}
    }

    confirm_always(
        &format!("Overwrite `{entry}` in universe {universe_id}?"),
        yes,
    )?;

    let written = api.set(entry, &update).await?;

    document.applied = true;
    document.revision_id = written.revision_id.clone();

    format.note(format!(
        "{} {entry} is now revision {}",
        "done".green().bold(),
        written.revision_id.as_deref().unwrap_or("(unknown)")
    ));

    if format.is_json() {
        return output::emit(&document);
    }

    Ok(())
}

pub(crate) fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indentation_keeps_every_line_of_a_json_block() {
        assert_eq!(indent("{\n  \"a\": 1\n}"), "    {\n      \"a\": 1\n    }");
    }
}

/// Split `a,b` into its two halves, rejecting anything else.
pub(crate) fn split_pair(raw: &str, flag: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    match parts.as_slice() {
        // Both sides equal would write one file twice and diff it against
        // itself, which looks like "no differences" and means nothing.
        [a, b] if a == b => bail!("{flag} needs two different values, got `{a}` twice"),
        [a, b] => Ok((a.to_string(), b.to_string())),
        _ => bail!("{flag} takes exactly two values separated by a comma, got `{raw}`"),
    }
}

/// Hand two files to whatever diff tool is available.
///
/// Nothing is rendered here on purpose. `code --diff` is the same side-by-side
/// view DataStoria provides, `git diff --no-index` works everywhere a
/// repository does, and either beats a diff written for a terminal. The
/// override exists because the right answer is whatever the reader already
/// uses.
pub(crate) fn open_diff(left: &std::path::Path, right: &std::path::Path) -> Result<()> {
    let candidates: Vec<Vec<String>> = match std::env::var("RBX_DIFF_TOOL") {
        Ok(custom) if !custom.trim().is_empty() => {
            vec![custom.split_whitespace().map(str::to_string).collect()]
        }
        _ => vec![
            vec!["code".into(), "--diff".into()],
            vec!["git".into(), "diff".into(), "--no-index".into()],
        ],
    };

    for command in &candidates {
        let (program, leading) = command.split_first().expect("no empty candidate");
        let status = std::process::Command::new(program)
            .args(leading)
            .arg(left)
            .arg(right)
            .status();
        match status {
            // `git diff --no-index` exits 1 when the files differ, which is the
            // normal outcome here and not a failure.
            Ok(_) => return Ok(()),
            Err(_) => continue,
        }
    }

    bail!(
        "no diff tool found. Set RBX_DIFF_TOOL, or open these yourself:\n  {}\n  {}",
        left.display(),
        right.display()
    )
}

#[cfg(test)]
mod pair_tests {
    use super::*;

    #[test]
    fn a_pair_splits_on_the_comma_and_trims() {
        assert_eq!(
            split_pair(" prod , staging ", "--between").unwrap(),
            ("prod".to_string(), "staging".to_string())
        );
    }

    #[test]
    fn anything_that_is_not_exactly_two_values_is_rejected() {
        for bad in ["", "one", "a,b,c", ",", "a,"] {
            assert!(
                split_pair(bad, "--between").is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn the_same_value_twice_is_rejected() {
        // Both sides equal produced one file written twice, so the diff
        // compared it against itself and always reported no change.
        let error = split_pair("ops,ops", "--between").unwrap_err().to_string();
        assert!(error.contains("two different"), "got: {error}");
        assert!(split_pair(" prod , prod ", "--between").is_err());
    }

    #[test]
    fn the_error_names_the_flag_that_was_wrong() {
        let error = split_pair("nope", "--revisions").unwrap_err().to_string();
        assert!(error.contains("--revisions"), "got: {error}");
    }
}
