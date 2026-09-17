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
//! `[owner]` used to be the exception, written here because `record` had no
//! writer for it. It has one now (`rbx init create-group --record` needs to
//! write that block before any env exists), so this module states the *rule*
//! for it and delegates the bytes: append only, and never over an owner the
//! user already declared.

use std::path::Path;

use anyhow::{Context, Result};

use rbx_core::places::PlacesFile;

use crate::discover::Universe;

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
    /// `root = "<key>"` was inserted: the env had no start place, and Roblox's
    /// is on file under a key other than `main`.
    pub root_written: Option<String>,
    /// The file names a start place other than Roblox's, and it was kept.
    pub root_conflict: Option<RootConflict>,
}

/// A start place on file that is not the one Roblox reports.
///
/// Reported rather than fixed: the start place is also the default target of
/// every command run without `--place`, so rewriting it would silently change
/// where the next `rbx place upload` lands.
#[derive(Debug, PartialEq, Eq)]
pub struct RootConflict {
    /// What the file resolves as the start place, `None` when Roblox's root id
    /// is not on file at all.
    pub on_file: Option<(String, u64)>,
    /// What Roblox reports.
    pub roblox: u64,
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

    record_root(path, env, root.id, &mut result)?;

    if let Some(owner) = &universe.owner {
        let has_owner = PlacesFile::load(path)
            .ok()
            .is_some_and(|f| f.resolve_owner(env).is_some());
        if !has_owner {
            rbx_init::record::append_owner(path, *owner)?;
            result.owner_written = true;
        }
    }

    Ok(result)
}

/// Make the env's start place say what Roblox says, when that needs no
/// rewrite.
///
/// Runs after the places are written, so the root id is on file whenever it
/// could be. Only one case is written: no start place yet, and the root id
/// listed under some key. Every disagreement with an existing declaration,
/// explicit `root` or implicit `main`, is reported instead (see
/// [`RootConflict`]).
fn record_root(path: &Path, env: &str, root_id: u64, result: &mut PlacesWrite) -> Result<()> {
    // Lenient like every other read in `write_env`: the envs are already
    // written, and failing the import here would report as lost what is on
    // disk. The next command to load the file names the problem.
    let Ok(file) = PlacesFile::load(path) else {
        return Ok(());
    };
    let Some(entry) = file.environments.get(env) else {
        return Ok(());
    };

    let on_file = entry.root_place().map(|(name, id)| (name.to_string(), id));
    if on_file.as_ref().is_some_and(|(_, id)| *id == root_id) {
        return Ok(());
    }

    // Sorted so two keys holding the same id pick the same one every run.
    let mut listed: Vec<&str> = entry
        .places
        .iter()
        .filter(|(_, id)| **id == root_id)
        .map(|(key, _)| key.as_str())
        .collect();
    listed.sort();

    match (on_file, listed.first()) {
        (None, Some(key)) => {
            rbx_init::record::insert_root(path, env, key)?;
            result.root_written = Some((*key).to_string());
        }
        (on_file, _) => {
            result.root_conflict = Some(RootConflict {
                on_file,
                roblox: root_id,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::discover::{Owner, OwnerType, Place};

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
                    kind: OwnerType::Group,
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
                    kind: OwnerType::Group,
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

        // And since that key is not `main`, the file now says which place
        // starts the game, or the generated module would carry no root id.
        assert_eq!(result.root_written.as_deref(), Some("start"));
        assert_eq!(
            loaded.get("prod").unwrap().root_place(),
            Some(("start", 222))
        );
    }

    /// A fresh env keys its root `main`, so there is nothing to declare.
    #[test]
    fn a_new_env_needs_no_root_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rbxplace.toml");

        let result = write_env(
            &path,
            "prod",
            &universe(111, vec![place("main", 222), place("lobby", 333)], None),
        )
        .unwrap();

        assert_eq!(result.root_written, None);
        assert_eq!(result.root_conflict, None);
        assert!(!std::fs::read_to_string(&path).unwrap().contains("root"));
    }

    /// `main` on file points at another place. Writing `root` would move the
    /// default target of `rbx place upload`, so the disagreement is reported
    /// and the file is left as the user wrote it.
    #[test]
    fn a_main_that_is_not_the_root_is_reported_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "[prod]\nuniverse_id = 111\nplaces.main = 999\nplaces.start = 222\n",
        );
        let before = std::fs::read_to_string(&path).unwrap();

        let result = write_env(
            &path,
            "prod",
            &universe(111, vec![place("main", 222)], None),
        )
        .unwrap();

        assert_eq!(result.root_written, None);
        assert_eq!(
            result.root_conflict,
            Some(RootConflict {
                on_file: Some(("main".into(), 999)),
                roblox: 222,
            })
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }
}
