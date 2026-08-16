//! Where the pre-write copy of an entry goes, and how many are kept.
//!
//! An overwrite is unrecoverable through the API (see the crate docs), so the
//! copy written before every write is the only way back. Two things follow
//! from that, and both are the reason this is a module rather than a
//! `format!` at the call site:
//!
//! - **A backup must never overwrite a backup.** The old default was
//!   `<entry>.backup.json` in the working directory, so resetting the same
//!   player twice replaced the copy of the value the first reset destroyed —
//!   the one case where you want the older file. Names carry a UTC timestamp,
//!   and a same-second collision gets a counter rather than clobbering.
//! - **They have to be findable months later.** Scattered next to whatever
//!   directory the operator happened to `cd` into, they are not. They now land
//!   beside `rbxplace.toml`, under `.rbx/backups/<env>/`, so they follow the
//!   project rather than the shell, and one directory holds the whole history
//!   for one environment.
//!
//! Retention exists because that history is otherwise unbounded: live ops on
//! one entry writes a file per write, forever. `--keep` prunes the oldest of
//! *that entry* in *that directory*, never anything else in it.
//!
//! `--backup <path>` stays the exact-path escape hatch: the file goes where it
//! is told, nothing is created around it and nothing is pruned.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::Colorize;

/// Project-local state directory. Player data lands here, so `docs/ops/data.md`
/// tells the reader to gitignore it.
const STATE_DIR: &str = ".rbx";

/// Backups live under `<STATE_DIR>/<BACKUPS_DIR>/<env>/`.
const BACKUPS_DIR: &str = "backups";

/// How many backups of one entry survive in the default directory.
///
/// Ten is the number of resets you can undo, which is the question the value
/// answers. It is deliberately not "one per write forever": the pile the old
/// behaviour left in the working directory is what #13 is about.
pub const DEFAULT_KEEP: u32 = 10;

/// A same-second second backup gets `-2`, then `-3`. Past this the clock is not
/// the problem and a loop that never ends would be.
const MAX_SAME_SECOND: u32 = 100;

/// Where the pre-write copy of the current value goes, or that there is none.
///
/// A bare `Option<PathBuf>` cannot say "deliberately nowhere": `None` already
/// means "the default path". Encoding the choice in the type is what stops a
/// future caller from reading a missing path as permission to skip the copy.
#[derive(Debug)]
pub enum BackupTarget {
    /// `--backup <path>`: exactly this file. No directory is created around
    /// it, nothing near it is pruned — the operator named the path, so the
    /// path is the whole instruction.
    Path(PathBuf),
    /// The default: a timestamped file in `dir`, keeping the newest `keep`
    /// backups of the entry being written.
    Managed { dir: PathBuf, keep: u32 },
    /// `--no-backup`. Nothing is written locally.
    Skip,
}

impl BackupTarget {
    /// Resolve the three flags into one target.
    ///
    /// `places` is the `rbxplace.toml` path, used only for its directory:
    /// backups belong to the project, not to the shell's working directory.
    /// `env` labels the subdirectory, so two environments' copies of the same
    /// entry key cannot land on each other.
    pub fn resolve(
        backup: Option<PathBuf>,
        no_backup: bool,
        keep: u32,
        places: &Path,
        env: &str,
    ) -> Self {
        // clap already refuses `--no-backup` with either of the others, so
        // this cannot silently prefer one over another.
        if no_backup {
            return Self::Skip;
        }
        match backup {
            Some(path) => Self::Path(path),
            None => Self::Managed {
                dir: default_dir(places, env),
                keep,
            },
        }
    }
}

/// `.rbx/backups/<env>/`, beside `rbxplace.toml`.
///
/// `--places` defaults to a bare `rbxplace.toml`, whose parent is the empty
/// path rather than `.`; joining onto that would produce `/.rbx/...` on unix.
fn default_dir(places: &Path, env: &str) -> PathBuf {
    let root = places
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    root.join(STATE_DIR)
        .join(BACKUPS_DIR)
        .join(sanitise_filename(env))
}

/// What a backup left behind, for the caller to report.
#[derive(Debug)]
pub struct Written {
    pub path: PathBuf,
    /// Older backups of the same entry removed by retention.
    pub pruned: usize,
}

/// Write `contents` as the backup of `entry`, and prune under retention.
///
/// [`BackupTarget::Skip`] is not handled here: the caller warns about it, and
/// making this return `Option<Written>` would let a future caller read `None`
/// as "nothing to say" rather than "the only way back was skipped".
pub fn write(target: &BackupTarget, entry: &str, contents: &str) -> Result<Written> {
    match target {
        BackupTarget::Path(path) => {
            write_file(path, contents)?;
            Ok(Written {
                path: path.clone(),
                pruned: 0,
            })
        }
        BackupTarget::Managed { dir, keep } => {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("creating the backup directory {}", dir.display()))?;
            let path = unique_path(dir, entry, &timestamp())?;
            write_file(&path, contents)?;
            // After the write, never before: pruning to make room for a copy
            // that then fails to land would delete a backup and keep nothing.
            let pruned = match prune(dir, entry, *keep) {
                Ok(removed) => removed,
                // Non-fatal on purpose. The value about to be destroyed is
                // already on disk; a directory that cannot be tidied is not a
                // reason to stop the operator from writing.
                Err(err) => {
                    eprintln!(
                        "{}",
                        format!(
                            "could not prune older backups in {}: {err:#}",
                            dir.display()
                        )
                        .yellow()
                    );
                    0
                }
            };
            Ok(Written { path, pruned })
        }
        BackupTarget::Skip => unreachable!("BackupTarget::Skip is handled by the caller"),
    }
}

fn write_file(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, format!("{contents}\n"))
        .with_context(|| format!("writing the backup to {}", path.display()))
}

/// `20260815T091500Z`: sorts lexicographically in chronological order, which is
/// what [`prune`] relies on and what makes `ls` readable.
fn timestamp() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

/// `<dir>/<entry>-<stamp>.json`, with `-2`, `-3`… appended if that name is
/// taken.
///
/// Two writes inside one second are rare and the pair matters exactly when it
/// happens: a script resetting a player twice in a loop is the case where the
/// first copy is the one worth having.
fn unique_path(dir: &Path, entry: &str, stamp: &str) -> Result<PathBuf> {
    let entry = sanitise_filename(entry);
    let first = dir.join(format!("{entry}-{stamp}.json"));
    if !first.exists() {
        return Ok(first);
    }
    for n in 2..MAX_SAME_SECOND {
        let path = dir.join(format!("{entry}-{stamp}-{n}.json"));
        if !path.exists() {
            return Ok(path);
        }
    }
    anyhow::bail!(
        "{MAX_SAME_SECOND} backups of `{entry}` already exist for {stamp} in {}",
        dir.display()
    )
}

/// Delete the oldest backups of `entry` in `dir` beyond the newest `keep`.
///
/// Scoped to one entry's own files: the directory holds every entry of one
/// env, and `--keep 3` on one player must not evict another player's copies.
/// Anything that is not a `<entry>-*.json` this module wrote is left alone.
fn prune(dir: &Path, entry: &str, keep: u32) -> Result<usize> {
    let prefix = format!("{}-", sanitise_filename(entry));
    let mut ours: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".json"))
        })
        .collect();

    let keep = keep as usize;
    if ours.len() <= keep {
        return Ok(0);
    }
    // The timestamp is the tail of the name, so sorting by name sorts by age
    // without stat-ing anything — mtime would lie the moment a directory is
    // copied or restored from a backup of its own.
    ours.sort();
    let doomed = ours.len() - keep;
    for path in ours.into_iter().take(doomed) {
        std::fs::remove_file(&path)
            .with_context(|| format!("removing the old backup {}", path.display()))?;
    }
    Ok(doomed)
}

/// Entry keys and env names are arbitrary strings and end up in a backup path.
pub fn sanitise_filename(entry: &str) -> String {
    entry
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), "{}\n").unwrap();
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut found: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        found.sort();
        found
    }

    #[test]
    fn a_key_with_path_characters_cannot_escape_into_a_directory() {
        assert_eq!(sanitise_filename("Player_156"), "Player_156");
        assert_eq!(sanitise_filename("../../etc/passwd"), "______etc_passwd");
        assert_eq!(sanitise_filename("a b:c/d\\e"), "a_b_c_d_e");
    }

    /// The default lands beside `rbxplace.toml`, not beside the shell.
    #[test]
    fn the_default_directory_follows_the_places_file() {
        let dir = default_dir(Path::new("/srv/game/rbxplace.toml"), "prod");
        assert_eq!(
            dir,
            Path::new("/srv/game")
                .join(STATE_DIR)
                .join(BACKUPS_DIR)
                .join("prod")
        );
    }

    /// `--places rbxplace.toml` has an empty parent; joining onto it would
    /// escape to the filesystem root.
    #[test]
    fn a_bare_places_filename_stays_in_the_working_directory() {
        let dir = default_dir(Path::new("rbxplace.toml"), "prod");
        assert_eq!(
            dir,
            Path::new(".")
                .join(STATE_DIR)
                .join(BACKUPS_DIR)
                .join("prod")
        );
    }

    /// Env names reach the filesystem the same way entry keys do.
    #[test]
    fn an_env_name_cannot_escape_the_backup_directory() {
        let dir = default_dir(Path::new("rbxplace.toml"), "../../etc");
        assert!(dir.ends_with("______etc"), "got {}", dir.display());
    }

    #[test]
    fn a_second_backup_in_the_same_second_does_not_replace_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let first = unique_path(dir.path(), "Player_156", "20260815T091500Z").unwrap();
        std::fs::write(&first, "{}").unwrap();
        let second = unique_path(dir.path(), "Player_156", "20260815T091500Z").unwrap();

        assert_ne!(first, second);
        assert!(
            second.ends_with("Player_156-20260815T091500Z-2.json"),
            "got {}",
            second.display()
        );
    }

    #[test]
    fn retention_removes_the_oldest_and_keeps_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        for stamp in ["20260101T000000Z", "20260102T000000Z", "20260103T000000Z"] {
            touch(dir.path(), &format!("Player_156-{stamp}.json"));
        }

        assert_eq!(prune(dir.path(), "Player_156", 2).unwrap(), 1);
        assert_eq!(
            names(dir.path()),
            vec![
                "Player_156-20260102T000000Z.json",
                "Player_156-20260103T000000Z.json"
            ]
        );
    }

    /// One directory holds every entry of one env, so retention on one key
    /// must not evict another key's history.
    #[test]
    fn retention_only_counts_the_entry_it_was_asked_about() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Player_156-20260101T000000Z.json");
        touch(dir.path(), "Player_156-20260102T000000Z.json");
        touch(dir.path(), "Player_999-20260101T000000Z.json");
        touch(dir.path(), "notes.txt");

        assert_eq!(prune(dir.path(), "Player_156", 1).unwrap(), 1);
        assert_eq!(
            names(dir.path()),
            vec![
                "Player_156-20260102T000000Z.json",
                "Player_999-20260101T000000Z.json",
                "notes.txt"
            ]
        );
    }

    /// `Player_1` is a prefix of `Player_15`; only the separator tells them
    /// apart, which is why the prefix carries the `-`.
    #[test]
    fn retention_does_not_confuse_one_key_with_a_longer_one() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Player_1-20260101T000000Z.json");
        touch(dir.path(), "Player_15-20260101T000000Z.json");
        touch(dir.path(), "Player_15-20260102T000000Z.json");

        assert_eq!(prune(dir.path(), "Player_1", 1).unwrap(), 0);
        assert_eq!(names(dir.path()).len(), 3);
    }

    #[test]
    fn nothing_is_removed_while_under_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Player_156-20260101T000000Z.json");

        assert_eq!(prune(dir.path(), "Player_156", DEFAULT_KEEP).unwrap(), 0);
        assert_eq!(names(dir.path()).len(), 1);
    }

    /// The whole point of the managed directory: it creates itself, names by
    /// time, and reports what it removed.
    #[test]
    fn a_managed_write_creates_the_directory_and_prunes() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(".rbx/backups/prod");
        let target = BackupTarget::Managed {
            dir: dir.clone(),
            keep: 1,
        };
        touch_dir(&dir);
        touch(&dir, "Player_156-20260101T000000Z.json");

        let written = write(&target, "Player_156", "{\"coins\":1}").unwrap();

        assert!(written.path.starts_with(&dir));
        assert_eq!(written.pruned, 1);
        assert_eq!(
            std::fs::read_to_string(&written.path).unwrap(),
            "{\"coins\":1}\n"
        );
    }

    fn touch_dir(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
    }

    /// `--backup <path>` is an exact instruction: no directory around it, no
    /// pruning near it.
    #[test]
    fn an_explicit_path_is_written_as_given_and_prunes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Player_156-20260101T000000Z.json");
        let path = dir.path().join("before.json");

        let written = write(&BackupTarget::Path(path.clone()), "Player_156", "{}").unwrap();

        assert_eq!(written.path, path);
        assert_eq!(written.pruned, 0);
        assert_eq!(names(dir.path()).len(), 2);
    }
}
