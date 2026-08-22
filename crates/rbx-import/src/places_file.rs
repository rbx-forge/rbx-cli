//! Laying the resolved universe down in `rbxplace.toml`.
//!
//! The hard case is not the empty directory: it is the file that already has
//! two envs, an `[owner]` block, a `[codegen]` block and the comments that
//! explain them. Importing a third env must leave all of that byte-for-byte
//! intact, or `import` becomes a command nobody dares run twice.
//!
//! Writing is therefore line-level, and the env/place insertion reuses
//! `rbx-init`'s `record` helpers rather than a second implementation: they
//! already detect the file's newline style, honour an existing
//! `[<env>.places]` sub-table, and only ever insert lines. `rbx init
//! create-universe` has been appending envs that way since before this
//! command existed.
//!
//! The one thing `record` has no writer for is `[owner]`, so that is here:
//! same rule: append only, and never over an owner the user already declared.

use std::path::Path;

use anyhow::{Context, Result};

use rbx_core::places::PlacesFile;

use crate::discover::{Owner, Universe};

/// What [`write_env`] did, so the caller can report it without re-reading the
/// file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PlacesWrite {
    /// The env block was appended (false when it was already there).
    pub env_created: bool,
    /// Place keys added under the env.
    pub places_added: Vec<String>,
    /// An `[owner]` block was appended because the file had none.
    pub owner_written: bool,
    /// The env already existed, so its `universe_id` was left as the user
    /// wrote it. Carries the id on file when it disagrees with the one being
    /// imported: a mismatch worth naming rather than silently honouring.
    pub existing_universe_id: Option<u64>,
}

/// Create or complete the `[<env>]` block for `universe` in `path`.
///
/// Never rewrites what is already there: an env that exists keeps its
/// `universe_id`, and places already listed keep their keys and ids. Only
/// missing lines are inserted.
pub fn write_env(path: &Path, env: &str, universe: &Universe) -> Result<PlacesWrite> {
    let mut result = PlacesWrite::default();
    let root = universe.root_place()?;

    if !path.exists() {
        std::fs::write(path, "").with_context(|| format!("Failed to create {}", path.display()))?;
    }

    // Parsed only to decide what is missing. Every write below goes through
    // the line-level helpers, so this view is never serialized back.
    let existing = PlacesFile::load(path).ok();
    let existing_env = existing.as_ref().and_then(|f| f.environments.get(env));

    match existing_env {
        None => {
            rbx_init::record::append_env(path, env, universe.id, &root.key, root.id)?;
            result.env_created = true;
            result.places_added.push(root.key.clone());
        }
        Some(entry) => {
            if entry.universe_id != universe.id {
                result.existing_universe_id = Some(entry.universe_id);
            }
            // Same test as the loop below: a key already there, or the id
            // already listed under a name the user chose. A second key for one
            // id would make `--place` ambiguous, root place or not.
            let known = entry.places.contains_key(&root.key)
                || entry.places.values().any(|id| *id == root.id);
            if !known {
                rbx_init::record::insert_place(path, env, &root.key, root.id)?;
                result.places_added.push(root.key.clone());
            }
        }
    }

    // Re-read after each insertion rather than tracking state: `insert_place`
    // is line surgery on a file this loop is also editing.
    for place in universe.places.iter().skip(1) {
        let current = PlacesFile::load(path).ok();
        let known = current
            .as_ref()
            .and_then(|f| f.environments.get(env))
            .is_some_and(|e| {
                e.places.contains_key(&place.key) || e.places.values().any(|id| *id == place.id)
            });
        if !known {
            rbx_init::record::insert_place(path, env, &place.key, place.id)?;
            result.places_added.push(place.key.clone());
        }
    }

    if let Some(owner) = &universe.owner {
        let has_owner = PlacesFile::load(path)
            .ok()
            .is_some_and(|f| f.resolve_owner(env).is_some());
        if !has_owner {
            append_owner(path, owner)?;
            result.owner_written = true;
        }
    }

    Ok(result)
}

/// Append a top-level `[owner]` block.
///
/// Only ever called when the file resolves no owner for this env, so it cannot
/// overwrite a per-env `[<env>.owner]` or a top-level one the user set.
fn append_owner(path: &Path, owner: &Owner) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    std::fs::write(path, append_owner_str(&content, owner))
        .with_context(|| format!("Failed to write {}", path.display()))
}

/// String form of [`append_owner`], so the formatting is directly testable.
///
/// `[owner]` goes at the end like any other appended table. It reads better at
/// the top, but moving it there would mean rewriting lines that are already
/// on disk, which is the one thing this module does not do.
fn append_owner_str(content: &str, owner: &Owner) -> String {
    let newline = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut out = content.to_string();

    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(newline);
    }
    if !out.trim().is_empty() {
        out.push_str(newline);
    }

    out.push_str(&format!("[owner]{newline}"));
    out.push_str(&format!("type = \"{}\"{newline}", owner.kind));
    out.push_str(&format!("id = {}{newline}", owner.id));
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::discover::Place;

    fn universe(id: u64, places: Vec<Place>, owner: Option<Owner>) -> Universe {
        Universe {
            id,
            display_name: Some("Test Game".into()),
            owner,
            places,
        }
    }

    fn place(key: &str, id: u64) -> Place {
        Place {
            key: key.into(),
            id,
            display_name: key.into(),
        }
    }

    fn write(dir: &Path, contents: &str) -> std::path::PathBuf {
        let path = dir.join("rbxplace.toml");
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn a_missing_file_is_created_with_the_env_and_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rbxplace.toml");

        let result = write_env(
            &path,
            "prod",
            &universe(
                111,
                vec![place("main", 222)],
                Some(Owner {
                    kind: "group",
                    id: 7,
                }),
            ),
        )
        .unwrap();

        assert!(result.env_created);
        assert!(result.owner_written);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[prod]"), "{text}");
        assert!(text.contains("universe_id = 111"), "{text}");
        assert!(text.contains("places.main = 222"), "{text}");
        assert!(text.contains("[owner]"), "{text}");
        assert!(text.contains("type = \"group\""), "{text}");

        // And it round-trips through the loader every other command uses.
        let loaded = PlacesFile::load(&path).unwrap();
        assert_eq!(loaded.get("prod").unwrap().universe_id, 111);
    }

    /// The case that decides whether the command is usable twice.
    #[test]
    fn an_existing_file_keeps_its_other_envs_owner_codegen_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let before = "\
# Our envs. Ask before touching prod.

[owner]
type = \"user\"
id = 42

[codegen]
output = \"src/shared/Envs.luau\"

# The live game.
[prod]
universe_id = 999
places.main = 888
";
        let path = write(dir.path(), before);

        let result = write_env(
            &path,
            "staging",
            &universe(
                111,
                vec![place("main", 222)],
                Some(Owner {
                    kind: "group",
                    id: 7,
                }),
            ),
        )
        .unwrap();

        assert!(result.env_created);
        assert!(
            !result.owner_written,
            "an owner the user already declared must not be replaced"
        );

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.starts_with(before),
            "everything already on disk must survive verbatim:\n{after}"
        );
        assert!(after.contains("[staging]"));
        assert!(after.contains("universe_id = 111"));
        // The owner on file wins, untouched.
        let loaded = PlacesFile::load(&path).unwrap();
        assert_eq!(loaded.owner.as_ref().unwrap().id, 42);
        assert_eq!(loaded.env_names(), vec!["prod", "staging"]);
    }

    /// Re-importing the same env must be a no-op rather than a second block.
    #[test]
    fn re_importing_the_same_env_adds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rbxplace.toml");
        let u = universe(111, vec![place("main", 222)], None);

        write_env(&path, "prod", &u).unwrap();
        let once = std::fs::read_to_string(&path).unwrap();

        let result = write_env(&path, "prod", &u).unwrap();
        assert!(!result.env_created);
        assert!(result.places_added.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), once);
    }

    /// A universe id that disagrees with the file is reported, not applied:
    /// rewriting it would silently retarget every command that reads this env.
    #[test]
    fn a_conflicting_universe_id_is_reported_and_the_file_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "[prod]\nuniverse_id = 999\nplaces.main = 888\n");

        let result = write_env(
            &path,
            "prod",
            &universe(111, vec![place("main", 222)], None),
        )
        .unwrap();

        assert_eq!(result.existing_universe_id, Some(999));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("universe_id = 999"));
    }

    #[test]
    fn extra_places_are_added_next_to_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rbxplace.toml");

        let result = write_env(
            &path,
            "prod",
            &universe(
                111,
                vec![place("main", 222), place("lobby", 333), place("arena", 444)],
                None,
            ),
        )
        .unwrap();

        assert_eq!(result.places_added, ["main", "lobby", "arena"]);
        let loaded = PlacesFile::load(&path).unwrap();
        let env = loaded.get("prod").unwrap();
        assert_eq!(env.places.get("lobby"), Some(&333));
        assert_eq!(env.places.get("arena"), Some(&444));
    }

    /// A place already listed under another key is left alone: the user's
    /// naming is theirs, and adding a second key for the same id would make
    /// `--place` ambiguous.
    #[test]
    fn a_place_already_listed_under_another_key_is_not_duplicated() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "[prod]\nuniverse_id = 111\nplaces.main = 222\nplaces.the_lobby = 333\n",
        );

        let result = write_env(
            &path,
            "prod",
            &universe(111, vec![place("main", 222), place("lobby", 333)], None),
        )
        .unwrap();

        assert!(result.places_added.is_empty());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("places.lobby"), "{text}");
    }

    /// The root place gets the same treatment: a file that already names it
    /// `start` must not gain a second `main` key pointing at the same id.
    #[test]
    fn a_root_place_already_listed_under_another_key_is_not_duplicated() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "[prod]\nuniverse_id = 111\nplaces.start = 222\n",
        );

        let result = write_env(
            &path,
            "prod",
            &universe(111, vec![place("main", 222), place("lobby", 333)], None),
        )
        .unwrap();

        assert_eq!(result.places_added, ["lobby"]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("places.main"), "{text}");
        let loaded = PlacesFile::load(&path).unwrap();
        assert_eq!(loaded.get("prod").unwrap().places.get("start"), Some(&222));
    }

    #[test]
    fn an_owner_block_keeps_the_files_line_endings() {
        let crlf = "[prod]\r\nuniverse_id = 1\r\n";
        let out = append_owner_str(
            crlf,
            &Owner {
                kind: "user",
                id: 5,
            },
        );
        assert!(out.ends_with("id = 5\r\n"), "{out:?}");
        assert!(!out.contains("\n\n"), "a CRLF file must not gain LF lines");
    }
}
