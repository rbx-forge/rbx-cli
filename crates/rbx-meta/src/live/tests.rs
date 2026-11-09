#![allow(clippy::unwrap_used)]

use serde_json::json;

use super::*;
use crate::config::{Access, Audience};

/// What Roblox answers, built the way it arrives: as JSON.
fn read(universe: serde_json::Value, place: serde_json::Value) -> RemoteRead {
    RemoteRead {
        universe: serde_json::from_value(universe).unwrap(),
        place: serde_json::from_value(place).unwrap(),
        server_fill: None,
        allow_copying: None,
        universe_config: Default::default(),
        beta_mode: None,
        access: None,
    }
}

fn lock(name: &str) -> GameLock {
    GameLock {
        name: Some(name.to_string()),
        ..Default::default()
    }
}

#[test]
fn a_field_changed_on_roblox_is_a_difference() {
    let report = compare_read(
        &lock("My Game"),
        &read(json!({}), json!({ "displayName": "Renamed on the site" })),
        false,
    );

    assert_eq!(
        report.differences,
        vec![r#"name: lockfile "My Game", Roblox "Renamed on the site""#]
    );
}

#[test]
fn the_same_values_are_clean() {
    let report = compare_read(
        &lock("My Game"),
        &read(json!({}), json!({ "displayName": "My Game" })),
        false,
    );

    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

/// The lockfile records a field only when the config manages it.
#[test]
fn a_field_the_lockfile_does_not_record_is_not_held_to_anything() {
    let report = compare_read(
        &GameLock::default(),
        &read(
            json!({ "voiceChatEnabled": true }),
            json!({ "serverSize": 30 }),
        ),
        false,
    );

    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

/// The case measured on a live game: a link added on Roblox, for one env.
#[test]
fn a_social_link_added_or_removed_on_roblox_is_a_difference() {
    let added = compare_read(
        &GameLock::default(),
        &read(
            json!({ "discordSocialLink": { "title": "Join us", "uri": "https://discord.gg/example" } }),
            json!({}),
        ),
        false,
    );
    assert_eq!(added.differences.len(), 1, "{:?}", added.differences);
    assert!(added.differences[0].starts_with("social_links.discord: lockfile none, Roblox"));

    let mut with_link = GameLock::default();
    with_link.social_links.discord = Some(SocialLink {
        title: "Join us".into(),
        url: "https://discord.gg/example".into(),
    });
    let removed = compare_read(&with_link, &read(json!({}), json!({})), false);
    assert_eq!(removed.differences.len(), 1, "{:?}", removed.differences);
    assert!(removed.differences[0].ends_with("Roblox none"));
}

#[test]
fn private_servers_turned_on_on_roblox_are_a_difference() {
    let report = compare_read(
        &GameLock::default(),
        &read(json!({ "privateServerPriceRobux": 0 }), json!({})),
        false,
    );

    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].starts_with("private_server:"));
}

/// A green run in CI must not read as "the cookie fields are verified".
#[test]
fn without_a_cookie_the_cookie_fields_are_listed_and_never_compared() {
    let locked = GameLock {
        beta_mode: Some(true),
        ..Default::default()
    };
    let mut remote = read(json!({}), json!({}));
    remote.beta_mode = Some(false);

    let report = compare_read(&locked, &remote, false);

    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .not_compared
            .iter()
            .any(|line| line.starts_with("needs the Studio cookie:") && line.contains("beta_mode")),
        "{:?}",
        report.not_compared
    );
}

#[test]
fn with_a_cookie_the_cookie_fields_are_compared() {
    let locked = GameLock {
        beta_mode: Some(true),
        ..Default::default()
    };
    let mut remote = read(json!({}), json!({}));
    remote.beta_mode = Some(false);

    let report = compare_read(&locked, &remote, true);

    assert_eq!(
        report.differences,
        vec!["beta_mode: lockfile true, Roblox false"]
    );
    assert!(
        !report
            .not_compared
            .iter()
            .any(|l| l.contains("Studio cookie")),
        "{:?}",
        report.not_compared
    );
}

#[test]
fn the_write_only_fields_and_the_media_are_always_listed() {
    let report = compare_read(&GameLock::default(), &read(json!({}), json!({})), true);

    assert!(report
        .not_compared
        .iter()
        .any(|l| l.contains("permissions")));
    assert!(report.not_compared.iter().any(|l| l.starts_with("media")));
}

/// A recorded field Roblox did not send is neither clean nor different.
#[test]
fn a_recorded_field_roblox_did_not_confirm_is_listed() {
    let report = compare_read(&lock("My Game"), &read(json!({}), json!({})), false);

    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .not_compared
            .iter()
            .any(|l| l == "not confirmed by Roblox: name"),
        "{:?}",
        report.not_compared
    );
}

#[test]
fn a_changed_audience_is_compared_when_both_sides_are_limited() {
    let locked = GameLock {
        visibility: Some(Visibility::Limited),
        audience: Some([Audience::Playtesters].into()),
        ..Default::default()
    };
    let mut remote = read(json!({}), json!({}));
    remote.access = Some(Access::Limited([Audience::Friends].into()));

    let report = compare_read(&locked, &remote, false);

    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].starts_with("audience:"));
}
