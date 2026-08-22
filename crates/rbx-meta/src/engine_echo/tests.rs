//! What the echo comparison reports, and what it stays quiet about.

use super::*;

fn body(document: &str) -> String {
    serde_json::json!({ "engineAvatarSettings": document }).to_string()
}

/// The case this module exists for. A key Roblox did not understand comes back
/// missing, and today nothing anywhere would have said so.
#[test]
fn a_key_roblox_dropped_is_reported() {
    let echo = compare(
        r#"{"AvatarRules":{"AvatarType":1,"AvatarTpye":2}}"#,
        &body(r#"{"AvatarRules":{"AvatarType":1}}"#),
    )
    .expect("a comparable echo");

    assert_eq!(echo.dropped, vec!["AvatarRules.AvatarTpye"]);
    assert!(echo.added.is_empty());
}

/// The ordinary case: a partial document, completed by Roblox. Reported, but
/// it is not a problem: it is how somebody learns the full shape.
#[test]
fn a_default_roblox_filled_in_is_reported_separately() {
    let echo = compare(
        r#"{"AvatarRules":{"AvatarType":1}}"#,
        &body(r#"{"AvatarRules":{"AvatarType":1},"version":1}"#),
    )
    .expect("a comparable echo");

    assert!(echo.dropped.is_empty());
    assert_eq!(echo.added, vec!["version"]);
}

/// The common outcome, and the one that must print nothing.
#[test]
fn an_exact_echo_is_clean() {
    let document = r#"{"AvatarRules":{"AvatarType":1},"version":1}"#;
    let echo = compare(document, &body(document)).expect("a comparable echo");

    assert!(echo.is_clean(), "{echo:?}");
}

/// A value Roblox normalised is not a dropped key. Only the *paths* are
/// compared, because a changed number is Roblox clamping something it accepted
/// and reporting it would be noise.
#[test]
fn a_changed_value_at_the_same_path_is_not_a_difference() {
    let echo = compare(
        r#"{"AvatarBodyRules":{"CustomHeight":[9.9,9.9]}}"#,
        &body(r#"{"AvatarBodyRules":{"CustomHeight":[5.5,5.5]}}"#),
    )
    .expect("a comparable echo");

    assert!(echo.is_clean(), "{echo:?}");
}

/// Arrays are leaves. A vector whose contents differ is one path, not three.
#[test]
fn an_array_is_one_path_not_one_per_element() {
    let echo = compare(
        r#"{"A":{"Bounds":[0,0,0]}}"#,
        &body(r#"{"A":{"Bounds":[1,2]}}"#),
    )
    .expect("a comparable echo");

    assert!(echo.is_clean(), "{echo:?}");
}

/// `{}` is how Roblox documents clearing the settings, so it has to survive as
/// a value rather than flattening to nothing.
#[test]
fn an_empty_object_is_a_leaf() {
    let echo = compare(r#"{"A":{}}"#, &body(r#"{"A":{}}"#)).expect("a comparable echo");
    assert!(echo.is_clean(), "{echo:?}");

    let echo = compare(r#"{"A":{}}"#, &body("{}")).expect("a comparable echo");
    assert_eq!(echo.dropped, vec!["A"]);
}

// ── nothing to compare ──

/// Three ordinary shapes that are not failures of the write that already
/// happened: no body, a body that is not JSON, and a body that carries no
/// avatar settings because the patch touched something else.
#[test]
fn an_unusable_response_is_not_an_error() {
    assert!(compare("{}", "").is_none());
    assert!(compare("{}", "not json").is_none());
    assert!(compare("{}", r#"{"genre":7}"#).is_none());
}

/// A response whose `engineAvatarSettings` is not itself parseable JSON. The
/// field is typed as a string, so this is a shape Roblox could return and a
/// panic here would turn a successful sync into a crash.
#[test]
fn an_echo_that_is_not_json_is_not_an_error() {
    assert!(compare("{}", &body("not json either")).is_none());
}
