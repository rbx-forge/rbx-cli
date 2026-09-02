//! The handoff: what `import` writes, the other tools have to be able to read.
//!
//! `import` does not compute a single lockfile entry: each domain writes its
//! own, through the command that already knows how. What `import` owns is the
//! `rbxplace.toml` every one of those commands resolves `--env` against, so
//! this asserts that file against the *real* resolver in `rbx_core::places`,
//! the one `rbx shop`, `rbx meta` and `rbx config` all call.
//!
//! A note on what is not here: the end-to-end `import` then `check` assertion
//! the issue asks for needs the three domain crates pointed at a mock server,
//! and each of them injects its API host per-client behind `cfg(test)`: a
//! deliberate choice, documented in `rbx_core::api::base`, that leaves no way
//! to redirect `rbx_shop::run` from outside its own crate. See the PR for the
//! one-line seam each crate would need.

#![allow(clippy::unwrap_used)]

use rbx_core::places::{self, PlacesFile};
use rbx_import::discover::{Owner, OwnerType, Place, Universe};
use rbx_import::places_file::write_env;

fn universe(id: u64, places: Vec<(&str, u64)>, owner: Option<Owner>) -> Universe {
    Universe {
        id,
        display_name: Some("Tower Defence".into()),
        owner,
        places: places
            .into_iter()
            .map(|(key, id)| Place {
                key: key.to_string(),
                id,
                display_name: key.to_string(),
            })
            .collect(),
    }
}

/// Every place-scoped command resolves `(universe_id, place_id)` through this
/// function. An import whose output it cannot answer for is an import that
/// looks fine and then fails on the next command.
#[test]
fn the_written_env_resolves_through_the_shared_resolver() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rbxplace.toml");

    write_env(
        &path,
        "prod",
        &universe(
            99887766554,
            vec![("main", 55501), ("lobby", 77702)],
            Some(Owner {
                kind: OwnerType::Group,
                id: 456,
            }),
        ),
    )
    .unwrap();

    assert_eq!(
        places::resolve_universe_id(&path, "prod").unwrap(),
        99887766554
    );
    // No `--place`: defaults to `main`, which is why the root place takes that
    // key regardless of its Roblox name.
    assert_eq!(
        places::resolve(&path, "prod", None).unwrap(),
        (99887766554, 55501)
    );
    assert_eq!(
        places::resolve(&path, "prod", Some("lobby")).unwrap(),
        (99887766554, 77702)
    );

    // And the owner badge creation falls back to.
    let file = PlacesFile::load(&path).unwrap();
    let owner = file.resolve_owner("prod").unwrap();
    assert_eq!(owner.id, 456);
}

/// Two imports into one file: both envs have to resolve independently
/// afterwards. This is the layout the differential overlays in `rbxshop.toml`
/// and `rbxmeta.toml` are built on.
#[test]
fn two_imported_envs_both_resolve() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rbxplace.toml");

    write_env(&path, "prod", &universe(111, vec![("main", 1001)], None)).unwrap();
    write_env(&path, "staging", &universe(222, vec![("main", 2001)], None)).unwrap();

    assert_eq!(places::resolve(&path, "prod", None).unwrap(), (111, 1001));
    assert_eq!(
        places::resolve(&path, "staging", None).unwrap(),
        (222, 2001)
    );
    assert_eq!(
        PlacesFile::load(&path).unwrap().env_names(),
        vec!["prod", "staging"]
    );
}

/// The file must not pick up keys `rbx env` would then report as
/// unrecognised: an import that makes every later command print a warning is
/// an import nobody trusts.
#[test]
fn the_written_file_has_no_unrecognised_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rbxplace.toml");

    write_env(
        &path,
        "prod",
        &universe(
            111,
            vec![("main", 1001), ("arena", 1002)],
            Some(Owner {
                kind: OwnerType::User,
                id: 42,
            }),
        ),
    )
    .unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    let unknown = places::unknown_keys(&content);
    assert!(
        unknown.is_empty(),
        "import wrote keys rbx ignores: {unknown:?}"
    );
}

/// An import into a file somebody else wrote leaves their `[codegen]` block
/// alone: `rbx env gen-module --check` compares against a path declared
/// there, and losing it silently turns that check into a no-op.
#[test]
fn an_existing_codegen_block_survives_an_import() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rbxplace.toml");
    std::fs::write(
        &path,
        "[codegen]\noutput = \"src/shared/Envs.luau\"\n\n[prod]\nuniverse_id = 999\nplaces.main = 888\n",
    )
    .unwrap();

    write_env(&path, "staging", &universe(111, vec![("main", 1001)], None)).unwrap();

    let file = PlacesFile::load(&path).unwrap();
    assert_eq!(
        file.codegen.as_ref().unwrap().output.as_deref(),
        Some(std::path::Path::new("src/shared/Envs.luau"))
    );
    assert_eq!(file.env_names(), vec!["prod", "staging"]);
}
