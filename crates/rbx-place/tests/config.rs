#![allow(clippy::unwrap_used)]

use rbx_place::config::{Environment, PlacesConfig};
use std::collections::HashMap;
use tempfile::tempdir;

fn write_config(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rbxplace.toml");
    std::fs::write(&path, content).unwrap();
    (dir, path)
}

fn env_with_places(places: &[(&str, u64)]) -> Environment {
    Environment {
        universe_id: 100,
        env: None,
        confirm: false,
        codegen: true,
        places: places
            .iter()
            .map(|(n, id)| ((*n).to_string(), *id))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Load / save / lookup
// ---------------------------------------------------------------------------

#[test]
fn loads_multi_env_config() {
    let (_d, path) = write_config(
        r#"
[dev]
universe_id = 100
[dev.places]
main = 1001

[prod]
universe_id = 200
confirm = true
[prod.places]
main = 2001
lobby = 2002
"#,
    );
    let config = PlacesConfig::load(&path).unwrap();
    assert_eq!(config.environments.len(), 2);
    assert_eq!(config.environments["dev"].universe_id, 100);
    assert!(!config.environments["dev"].confirm);
    assert!(config.environments["prod"].confirm);
    assert_eq!(config.environments["prod"].places["lobby"], 2002);
}

#[test]
fn get_env_returns_existing() {
    let mut envs = HashMap::new();
    envs.insert("dev".into(), env_with_places(&[("main", 1)]));
    let config = PlacesConfig {
        environments: envs,
        ..Default::default()
    };
    let env = config.get_env("dev").unwrap();
    assert_eq!(env.universe_id, 100);
}

#[test]
fn get_env_errors_with_available_list() {
    let mut envs = HashMap::new();
    envs.insert("dev".into(), env_with_places(&[]));
    envs.insert("prod".into(), env_with_places(&[]));
    let config = PlacesConfig {
        environments: envs,
        ..Default::default()
    };
    let err = config.get_env("staging").unwrap_err().to_string();
    assert!(err.contains("staging"));
    assert!(err.contains("dev"));
    assert!(err.contains("prod"));
}

#[test]
fn save_round_trip_preserves_content() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rbxplace.toml");
    let mut envs = HashMap::new();
    envs.insert(
        "dev".into(),
        Environment {
            universe_id: 42,
            env: None,
            confirm: true,
            codegen: true,
            places: HashMap::from([("main".into(), 1001_u64)]),
        },
    );
    let config = PlacesConfig {
        environments: envs,
        ..Default::default()
    };
    config.save(&path).unwrap();

    let loaded = PlacesConfig::load(&path).unwrap();
    assert_eq!(loaded.environments["dev"].universe_id, 42);
    assert!(loaded.environments["dev"].confirm);
    assert_eq!(loaded.environments["dev"].places["main"], 1001);
}

#[test]
fn loads_config_with_top_level_owner_block() {
    // Regression: a centralized `[owner]` block (shared with the rest of the
    // tool suite) must be consumed as a reserved key, not parsed as an env.
    let (_d, path) = write_config(
        r#"
[owner]
type = "group"
id = 1234567

[dev]
universe_id = 100
[dev.places]
main = 1001
"#,
    );
    let config = PlacesConfig::load(&path).unwrap();
    // `owner` is NOT an env.
    assert_eq!(config.environments.len(), 1);
    assert!(config.environments.contains_key("dev"));
    assert_eq!(config.environments["dev"].universe_id, 100);
    let owner = config.owner.unwrap();
    assert_eq!(owner.id, 1234567);
}

#[test]
fn save_preserves_top_level_owner_block() {
    // `rbx place fetch --write` rewrites the whole file; the owner block must
    // survive the round-trip so we don't clobber the suite-wide source of truth.
    let (_d, path) = write_config(
        r#"
[owner]
type = "user"
id = 42

[dev]
universe_id = 100
[dev.places]
main = 1001
"#,
    );
    let config = PlacesConfig::load(&path).unwrap();
    config.save(&path).unwrap();

    let reloaded = PlacesConfig::load(&path).unwrap();
    let owner = reloaded.owner.expect("owner block dropped on save");
    assert_eq!(owner.id, 42);
    assert_eq!(reloaded.environments["dev"].universe_id, 100);
}

// ---------------------------------------------------------------------------
// resolve_place: the core dispatch logic
// ---------------------------------------------------------------------------

#[test]
fn resolve_place_explicit_name() {
    let env = env_with_places(&[("main", 1), ("lobby", 2)]);
    let (name, id) = env.resolve_place(Some("lobby")).unwrap();
    assert_eq!(name, "lobby");
    assert_eq!(id, 2);
}

#[test]
fn resolve_place_errors_on_unknown_name_with_available_list() {
    let env = env_with_places(&[("main", 1), ("lobby", 2)]);
    let err = env.resolve_place(Some("missing")).unwrap_err().to_string();
    assert!(err.contains("missing"));
    assert!(err.contains("main"));
    assert!(err.contains("lobby"));
}

#[test]
fn resolve_place_auto_picks_single_place_when_no_flag() {
    let env = env_with_places(&[("solo", 99)]);
    let (name, id) = env.resolve_place(None).unwrap();
    assert_eq!(name, "solo");
    assert_eq!(id, 99);
}

#[test]
fn resolve_place_errors_when_no_places_defined() {
    let env = env_with_places(&[]);
    let err = env.resolve_place(None).unwrap_err().to_string();
    assert!(err.contains("No places"));
}

#[test]
fn resolve_place_errors_on_ambiguous_with_available_list() {
    let env = env_with_places(&[("a", 1), ("b", 2)]);
    let err = env.resolve_place(None).unwrap_err().to_string();
    assert!(err.contains("--place"));
    assert!(err.contains("a, b"));
}

// ---------------------------------------------------------------------------
// all_places_sorted
// ---------------------------------------------------------------------------

#[test]
fn all_places_sorted_returns_alphabetical() {
    let env = env_with_places(&[("zoo", 3), ("apple", 1), ("middle", 2)]);
    let sorted = env.all_places_sorted();
    let names: Vec<&str> = sorted.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["apple", "middle", "zoo"]);
    let ids: Vec<u64> = sorted.iter().map(|(_, id)| *id).collect();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn all_places_sorted_empty_when_no_places() {
    let env = env_with_places(&[]);
    assert!(env.all_places_sorted().is_empty());
}

#[test]
fn a_codegen_section_survives_a_save_round_trip() {
    // `rbx place fetch --write` rewrites rbxplace.toml wholesale. A reserved
    // key it doesn't model would be silently dropped: deleting the setting
    // that `rbx env gen-module --check` depends on.
    let dir = tempdir().unwrap();
    let path = dir.path().join("rbxplace.toml");
    std::fs::write(
        &path,
        "[codegen]\noutput = \"src/shared/Envs.luau\"\n\n[dev]\nuniverse_id = 100\n",
    )
    .unwrap();

    let config = PlacesConfig::load(&path).unwrap();
    config.save(&path).unwrap();

    let reloaded = std::fs::read_to_string(&path).unwrap();
    assert!(reloaded.contains("[codegen]"), "got:\n{reloaded}");
    assert!(
        reloaded.contains("src/shared/Envs.luau"),
        "got:\n{reloaded}"
    );
    assert!(PlacesConfig::load(&path)
        .unwrap()
        .environments
        .contains_key("dev"));
}
