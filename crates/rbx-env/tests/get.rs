//! Field resolution for `rbx env get`. The contract that matters here is that
//! these values match what every other subcommand resolves for the same
//! `--env` / `--place`, so the tests pin the defaulting rules explicitly.

#![allow(clippy::unwrap_used)]

use rbx_env::{resolve_field, Field};
use tempfile::tempdir;

fn write_places(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rbxplace.toml");
    std::fs::write(&path, content).unwrap();
    (dir, path)
}

const SAMPLE: &str = r#"
[owner]
type = "group"
id = 1234567

[dev]
universe_id = 100
[dev.places]
main = 1001
lobby = 1002

[prod]
universe_id = 200
owner = { type = "user", id = 42 }
[prod.places]
only = 2001
"#;

#[test]
fn reads_universe_id() {
    let (_d, path) = write_places(SAMPLE);
    assert_eq!(
        resolve_field(&path, "dev", None, Field::UniverseId).unwrap(),
        "100"
    );
    assert_eq!(
        resolve_field(&path, "prod", None, Field::UniverseId).unwrap(),
        "200"
    );
}

#[test]
fn place_id_defaults_to_main_when_several_places_exist() {
    let (_d, path) = write_places(SAMPLE);
    assert_eq!(
        resolve_field(&path, "dev", None, Field::PlaceId).unwrap(),
        "1001"
    );
}

#[test]
fn place_id_honors_explicit_place_name() {
    let (_d, path) = write_places(SAMPLE);
    assert_eq!(
        resolve_field(&path, "dev", Some("lobby"), Field::PlaceId).unwrap(),
        "1002"
    );
}

#[test]
fn place_id_falls_back_to_the_only_entry_without_main() {
    let (_d, path) = write_places(SAMPLE);
    assert_eq!(
        resolve_field(&path, "prod", None, Field::PlaceId).unwrap(),
        "2001"
    );
}

#[test]
fn owner_falls_back_to_the_top_level_block() {
    let (_d, path) = write_places(SAMPLE);
    assert_eq!(
        resolve_field(&path, "dev", None, Field::OwnerId).unwrap(),
        "1234567"
    );
    assert_eq!(
        resolve_field(&path, "dev", None, Field::OwnerType).unwrap(),
        "group"
    );
}

#[test]
fn per_env_owner_override_wins() {
    let (_d, path) = write_places(SAMPLE);
    assert_eq!(
        resolve_field(&path, "prod", None, Field::OwnerId).unwrap(),
        "42"
    );
    assert_eq!(
        resolve_field(&path, "prod", None, Field::OwnerType).unwrap(),
        "user"
    );
}

#[test]
fn unknown_env_lists_the_available_ones() {
    let (_d, path) = write_places(SAMPLE);
    let err = resolve_field(&path, "nope", None, Field::UniverseId)
        .unwrap_err()
        .to_string();
    assert!(err.contains("dev"), "got {err}");
    assert!(err.contains("prod"), "got {err}");
}

#[test]
fn unknown_env_errors_on_owner_lookup_too() {
    // `resolve_owner` tolerates unknown env names by design (it falls through
    // to the top-level block); `get` must not silently answer for a typo.
    let (_d, path) = write_places(SAMPLE);
    let err = resolve_field(&path, "nope", None, Field::OwnerId)
        .unwrap_err()
        .to_string();
    assert!(err.contains("not found"), "got {err}");
}

#[test]
fn missing_owner_is_an_error_not_an_empty_value() {
    let (_d, path) = write_places("[dev]\nuniverse_id = 100\n");
    let err = resolve_field(&path, "dev", None, Field::OwnerId)
        .unwrap_err()
        .to_string();
    assert!(err.contains("No owner"), "got {err}");
}

#[test]
fn ambiguous_place_asks_for_an_explicit_name() {
    let (_d, path) = write_places(
        r#"
[dev]
universe_id = 100
[dev.places]
lobby = 1002
arena = 1003
"#,
    );
    let err = resolve_field(&path, "dev", None, Field::PlaceId)
        .unwrap_err()
        .to_string();
    assert!(err.contains("--place"), "got {err}");
}

#[test]
fn field_names_round_trip_to_their_cli_spelling() {
    assert_eq!(Field::UniverseId.as_str(), "universe-id");
    assert_eq!(Field::PlaceId.as_str(), "place-id");
    assert_eq!(Field::OwnerId.as_str(), "owner-id");
    assert_eq!(Field::OwnerType.as_str(), "owner-type");
}

#[test]
fn only_the_id_fields_require_an_env() {
    assert!(Field::UniverseId.needs_env());
    assert!(Field::PlaceId.needs_env());
    assert!(!Field::OwnerId.needs_env());
    assert!(!Field::OwnerType.needs_env());
}
