#![allow(clippy::unwrap_used)]
//! The keys `rbxmeta.toml` reads nothing from must be named, not swallowed.
//!
//! Written after a released `rbx` read a config full of keys it had never
//! heard of: `[game.permissions]`, `[game.avatar]`, two scale tables, `genre`,
//! `engine_avatar_settings`: discarded every one of them, and reported
//! "Nothing to do, everything is in sync."

use rbx_meta::config::Config;

fn ignored(content: &str) -> Vec<String> {
    Config::parse(content).expect("valid TOML").1
}

/// The exact shape of the failure: keys nested inside `[game]`, which a
/// top-level-only check would have missed entirely.
#[test]
fn a_nested_key_this_build_does_not_know_is_named_with_its_path() {
    let found = ignored(
        r#"
[game]
name = "Test"
totalement_inconnu = 42

[game.permissions]
third_party_teleport = false
third_party_asset = false
third_party_purchase = false
client_teleport = true
"#,
    );

    assert!(
        found.contains(&"game.totalement_inconnu".to_string()),
        "{found:?}"
    );
    assert!(
        !found.iter().any(|k| k.starts_with("game.permissions")),
        "permissions is a key this build does know: {found:?}"
    );
}

/// The blind spot, pinned so it cannot be forgotten or silently widened.
///
/// `[game.server_fill]` and `[game.paid_access]` are internally-tagged enums,
/// and serde buffers their content into an intermediate value before
/// deserializing from it, which loses the ignored-key callback. So a key
/// misfiled into one of those two tables is still swallowed.
///
/// This is the case that started the whole investigation: `genre` appended to
/// the wrong place in the file landed here and vanished. The assertion is
/// deliberately of the *current* behaviour rather than the desired one, so
/// that a serde release which fixes this turns the test red and says so.
#[test]
fn an_internally_tagged_table_still_swallows_its_unknown_keys() {
    let found = ignored(
        r#"
[game]
name = "Test"

[game.server_fill]
mode = "automatic"
genre = "all"
"#,
    );

    assert!(
        found.is_empty(),
        "serde now reports through internally-tagged enums. Delete the \
         blind-spot paragraph on `warn_ignored_keys`, drop the TODO.md entry, \
         and assert `game.server_fill.genre` here instead. Got: {found:?}"
    );
}

/// Depth is not the point; the full path is. A typo three levels down has to
/// be findable from the message alone.
#[test]
fn a_deeply_nested_typo_carries_its_whole_path() {
    let found = ignored(
        r#"
[game.avatar.min_scale]
height = 0.9
width = 0.7
head = 0.95
body_type = 0.0
proportion = 0.0
hieght = 1.0
"#,
    );

    assert_eq!(found, vec!["game.avatar.min_scale.hieght"]);
}

/// The control. A config using only known keys must produce no warning at all,
/// or the warning becomes noise and stops being read.
#[test]
fn a_config_of_known_keys_reports_nothing() {
    let found = ignored(
        r#"
[game]
name = "Test"
genre = "all"
engine_avatar_settings = "rbxavatar.toml"

[game.devices]
desktop = true

[game.avatar]
type = "r15"

[envs.prod]
name = "Test (prod)"

[envs.prod.permissions]
third_party_teleport = true
third_party_asset = true
third_party_purchase = true
client_teleport = true
"#,
    );

    assert!(found.is_empty(), "{found:?}");
}

/// Unknown keys are reported, never rejected: a config written for a newer
/// release has to stay loadable by an older one, or upgrading the file becomes
/// a flag day for everyone sharing the repository.
#[test]
fn an_unknown_key_does_not_fail_the_load() {
    let (config, found) = Config::parse(
        r#"
[game]
name = "Test"
something_from_the_future = true
"#,
    )
    .expect("an unknown key must not fail the parse");

    assert_eq!(config.game.name.as_deref(), Some("Test"));
    assert_eq!(found, vec!["game.something_from_the_future"]);
}
