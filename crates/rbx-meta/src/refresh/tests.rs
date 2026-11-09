#![allow(clippy::unwrap_used)]

use serde_json::{json, Value};

use super::*;
use crate::config::{Permissions, PrivateServer};
use crate::diff::build_universe_patch;

/// What Roblox answers, built the way it arrives: as JSON. Nothing a cookie
/// reads is confirmed.
fn read(universe: Value, place: Value) -> RemoteRead {
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

fn named(name: &str) -> Game {
    Game {
        name: Some(name.to_string()),
        ..Default::default()
    }
}

fn locked(name: &str) -> GameLock {
    GameLock {
        name: Some(name.to_string()),
        ..Default::default()
    }
}

fn discord() -> SocialLink {
    SocialLink {
        title: "Join us".into(),
        url: "https://discord.gg/example".into(),
    }
}

fn discord_json() -> Value {
    json!({ "discordSocialLink": { "title": "Join us", "uri": "https://discord.gg/example" } })
}

// ---------------------------------------------------------------------------
// Fields Roblox confirms or does not
// ---------------------------------------------------------------------------

/// The case this module exists for: the two local files agree, and Roblox was
/// changed. The plan has to see it, and the run has to be told it is newer.
#[test]
fn a_value_changed_on_roblox_is_seen_and_reported_as_newer() {
    let refreshed = refresh(
        &named("My Game"),
        &locked("My Game"),
        &read(json!({}), json!({ "displayName": "Renamed on the site" })),
    );

    assert_eq!(
        refreshed.lock.name.as_deref(),
        Some("Renamed on the site"),
        "the plan is built against what Roblox holds"
    );
    assert_eq!(
        refreshed.newer,
        vec![r#"name: lockfile "My Game", Roblox "Renamed on the site""#]
    );
}

/// A change made in the config, with Roblox where the lockfile left it, is
/// the ordinary sync: nothing on Roblox is newer than what was recorded.
#[test]
fn a_change_made_in_the_config_is_not_newer() {
    let refreshed = refresh(
        &named("Renamed here"),
        &locked("My Game"),
        &read(json!({}), json!({ "displayName": "My Game" })),
    );

    assert_eq!(refreshed.lock, locked("My Game"));
    assert!(refreshed.newer.is_empty(), "{:?}", refreshed.newer);
}

/// Somebody made the same change on both sides. There is nothing to write, so
/// nothing can be replaced, and the lockfile catches up.
#[test]
fn roblox_already_holding_what_the_config_asks_is_not_newer() {
    let refreshed = refresh(
        &named("Renamed"),
        &locked("My Game"),
        &read(json!({}), json!({ "displayName": "Renamed" })),
    );

    assert_eq!(refreshed.lock, locked("Renamed"));
    assert!(refreshed.newer.is_empty(), "{:?}", refreshed.newer);
}

/// With no record there is nothing for Roblox's value to be newer than: a
/// first sync writes what the config declares, as it always has.
#[test]
fn a_field_the_lockfile_never_recorded_has_nothing_to_be_newer_than() {
    let refreshed = refresh(
        &named("Declared"),
        &GameLock::default(),
        &read(
            json!({}),
            json!({ "displayName": "What the place was called" }),
        ),
    );

    assert_eq!(refreshed.lock, locked("What the place was called"));
    assert!(refreshed.newer.is_empty(), "{:?}", refreshed.newer);
}

/// A field neither local file mentions is not managed. Recording what Roblox
/// has would start holding Roblox to a value nobody declared.
#[test]
fn a_field_neither_file_mentions_stays_unrecorded() {
    let refreshed = refresh(
        &Game::default(),
        &GameLock::default(),
        &read(
            json!({ "voiceChatEnabled": true, "desktopEnabled": true }),
            json!({ "displayName": "Some place", "serverSize": 30 }),
        ),
    );

    assert_eq!(refreshed.lock, GameLock::default());
    assert!(refreshed.newer.is_empty(), "{:?}", refreshed.newer);
}

/// The config stopped declaring a field. Nothing is written for it, so a
/// change on Roblox replaces nothing.
#[test]
fn a_field_the_config_no_longer_declares_follows_roblox_quietly() {
    let refreshed = refresh(
        &Game::default(),
        &locked("My Game"),
        &read(json!({}), json!({ "displayName": "Renamed on the site" })),
    );

    assert_eq!(refreshed.lock, locked("Renamed on the site"));
    assert!(refreshed.newer.is_empty(), "{:?}", refreshed.newer);
}

/// Roblox did not answer for a field: the lockfile stands, which is the plan
/// `sync` made before it read anything.
#[test]
fn a_field_roblox_did_not_confirm_keeps_the_lockfile_value() {
    let refreshed = refresh(
        &named("Renamed here"),
        &locked("My Game"),
        &read(json!({}), json!({})),
    );

    assert_eq!(refreshed.lock, locked("My Game"));
    assert!(refreshed.newer.is_empty(), "{:?}", refreshed.newer);
}

/// The cookie-only fields without a cookie are the same unconfirmed case: the
/// read carries nothing for them.
#[test]
fn a_cookie_field_without_a_cookie_keeps_the_lockfile_value() {
    let game = Game {
        beta_mode: Some(true),
        ..Default::default()
    };
    let lock = GameLock {
        beta_mode: Some(false),
        ..Default::default()
    };

    let refreshed = refresh(&game, &lock, &read(json!({}), json!({})));

    assert_eq!(refreshed.lock.beta_mode, Some(false));
    assert!(refreshed.newer.is_empty(), "{:?}", refreshed.newer);
}

#[test]
fn a_cookie_field_read_with_a_cookie_is_compared_like_any_other() {
    let game = Game {
        beta_mode: Some(false),
        ..Default::default()
    };
    let lock = GameLock {
        beta_mode: Some(false),
        ..Default::default()
    };
    let mut remote = read(json!({}), json!({}));
    remote.beta_mode = Some(true);

    let refreshed = refresh(&game, &lock, &remote);

    assert_eq!(refreshed.lock.beta_mode, Some(true));
    assert_eq!(
        refreshed.newer,
        vec!["beta_mode: lockfile false, Roblox true"]
    );
}

/// No endpoint returns these, so no read can speak for them: whatever the
/// config says, the lockfile's entry is what `sync` diffs against.
#[test]
fn the_write_only_fields_always_come_from_the_lockfile() {
    let perms = |on| {
        Some(Permissions {
            third_party_teleport: on,
            third_party_asset: on,
            third_party_purchase: on,
            client_teleport: on,
        })
    };
    let game = Game {
        permissions: perms(true),
        ..Default::default()
    };
    let lock = GameLock {
        permissions: perms(false),
        engine_avatar_settings_hash: Some("deadbeef".into()),
        ..Default::default()
    };

    let refreshed = refresh(&game, &lock, &read(json!({}), json!({})));

    assert_eq!(refreshed.lock.permissions, perms(false));
    assert_eq!(
        refreshed.lock.engine_avatar_settings_hash.as_deref(),
        Some("deadbeef")
    );
    assert!(refreshed.newer.is_empty(), "{:?}", refreshed.newer);
}

// ---------------------------------------------------------------------------
// Private servers and social links, where absence is a value
// ---------------------------------------------------------------------------

/// The trap in comparing the config with Roblox alone: a link added on the
/// Creator Hub, which the config never declared, would be planned for removal.
/// Neither local file mentions it, so it is not this repository's to remove.
#[test]
fn a_link_added_on_roblox_that_no_file_mentions_is_never_removed() {
    let game = Game::default();
    let refreshed = refresh(
        &game,
        &GameLock::default(),
        &read(discord_json(), json!({})),
    );

    assert_eq!(refreshed.lock.social_links.discord, None);
    assert!(refreshed.newer.is_empty(), "{:?}", refreshed.newer);
    assert!(
        build_universe_patch(&game, &refreshed.lock).is_none(),
        "nothing is sent for a link nobody declared"
    );
}

/// The same for private servers switched on by hand.
#[test]
fn private_servers_turned_on_on_roblox_that_no_file_mentions_stay_on() {
    let game = Game::default();
    let refreshed = refresh(
        &game,
        &GameLock::default(),
        &read(json!({ "privateServerPriceRobux": 0 }), json!({})),
    );

    assert_eq!(refreshed.lock.private_server, None);
    assert!(build_universe_patch(&game, &refreshed.lock).is_none());
}

/// A link the repository manages, removed on the Creator Hub: putting it back
/// is a write over a change made on Roblox.
#[test]
fn a_managed_link_removed_on_roblox_is_newer_and_planned_back() {
    let mut game = Game::default();
    game.social_links.discord = Some(discord());
    let mut lock = GameLock::default();
    lock.social_links.discord = Some(discord());

    let refreshed = refresh(&game, &lock, &read(json!({}), json!({})));

    assert_eq!(refreshed.lock.social_links.discord, None);
    assert_eq!(refreshed.newer.len(), 1, "{:?}", refreshed.newer);
    assert!(refreshed.newer[0].starts_with("social_links.discord: lockfile"));
    assert!(refreshed.newer[0].ends_with("Roblox none"));

    let patch = build_universe_patch(&game, &refreshed.lock).expect("the link is sent again");
    assert_eq!(patch.mask, vec!["discordSocialLink"]);
}

/// Dropped from the config, still on Roblox as the lockfile recorded it: the
/// ordinary removal, and nothing on Roblox is newer.
#[test]
fn a_link_dropped_from_the_config_is_removed_without_being_newer() {
    let game = Game::default();
    let mut lock = GameLock::default();
    lock.social_links.discord = Some(discord());

    let refreshed = refresh(&game, &lock, &read(discord_json(), json!({})));

    assert_eq!(refreshed.lock.social_links.discord, Some(discord()));
    assert!(refreshed.newer.is_empty(), "{:?}", refreshed.newer);
    let patch = build_universe_patch(&game, &refreshed.lock).expect("the removal is sent");
    assert_eq!(patch.body["discordSocialLink"], Value::Null);
}

/// Dropped from the config while somebody changed it on Roblox: removing it
/// would discard their change.
#[test]
fn a_link_dropped_from_the_config_and_changed_on_roblox_is_newer() {
    let game = Game::default();
    let mut lock = GameLock::default();
    lock.social_links.discord = Some(discord());
    let changed = json!({ "discordSocialLink": { "title": "New server", "uri": "https://discord.gg/other" } });

    let refreshed = refresh(&game, &lock, &read(changed, json!({})));

    assert_eq!(refreshed.newer.len(), 1, "{:?}", refreshed.newer);
    assert!(refreshed.newer[0].contains("New server"));
}

#[test]
fn a_private_server_price_changed_on_roblox_is_newer() {
    let game = Game {
        private_server: Some(PrivateServer { price: 100 }),
        ..Default::default()
    };
    let lock = GameLock {
        private_server: Some(PrivateServer { price: 100 }),
        ..Default::default()
    };

    let refreshed = refresh(
        &game,
        &lock,
        &read(json!({ "privateServerPriceRobux": 250 }), json!({})),
    );

    assert_eq!(
        refreshed.lock.private_server,
        Some(PrivateServer { price: 250 })
    );
    assert_eq!(refreshed.newer.len(), 1, "{:?}", refreshed.newer);
    assert!(refreshed.newer[0].starts_with("private_server:"));
}

// ---------------------------------------------------------------------------
// Visibility and audience, one setting in two keys
// ---------------------------------------------------------------------------

#[test]
fn an_audience_changed_on_roblox_is_newer_as_one_setting() {
    let limited = |audience: &[Audience]| {
        (
            Some(Visibility::Limited),
            Some(audience.iter().copied().collect::<BTreeSet<_>>()),
        )
    };
    let (visibility, audience) = limited(&[Audience::Playtesters]);
    let game = Game {
        visibility,
        audience: audience.clone(),
        ..Default::default()
    };
    let lock = GameLock {
        visibility,
        audience,
        ..Default::default()
    };
    let mut remote = read(json!({}), json!({}));
    remote.access = Some(Access::Limited([Audience::Friends].into()));

    let refreshed = refresh(&game, &lock, &remote);

    let (visibility, audience) = limited(&[Audience::Friends]);
    assert_eq!(refreshed.lock.visibility, visibility);
    assert_eq!(refreshed.lock.audience, audience);
    assert_eq!(refreshed.newer.len(), 1, "{:?}", refreshed.newer);
    assert!(refreshed.newer[0].starts_with("visibility: lockfile"));
}

/// An unanswered audience read leaves both keys exactly as the lockfile
/// spells them, so the section does not read as changed.
#[test]
fn an_unconfirmed_visibility_keeps_both_keys_as_they_were() {
    let game = Game {
        visibility: Some(Visibility::Private),
        ..Default::default()
    };
    let lock = GameLock {
        visibility: Some(Visibility::Private),
        ..Default::default()
    };

    let refreshed = refresh(&game, &lock, &read(json!({}), json!({})));

    assert_eq!(refreshed.lock, lock);
}

// ---------------------------------------------------------------------------
// The whole section
// ---------------------------------------------------------------------------

/// What `sync` records has to be what `meta/live` then finds: a lock section
/// brought up to date here, compared with the same read, is clean.
#[test]
fn a_refreshed_section_is_clean_for_the_live_check() {
    let mut game = Game {
        name: Some("Declared".into()),
        description: Some("".into()),
        server_size: Some(6),
        voice_chat: Some(true),
        visibility: Some(Visibility::Private),
        private_server: Some(PrivateServer { price: 0 }),
        ..Default::default()
    };
    game.devices.desktop = Some(true);
    game.social_links.discord = Some(discord());
    // A lockfile that agrees with nothing Roblox holds.
    let mut lock = crate::diff::config_to_lock(&game);
    lock.name = Some("Old name".into());
    lock.server_size = Some(50);
    lock.private_server = Some(PrivateServer { price: 100 });
    lock.social_links.discord = None;

    let mut remote = read(
        json!({
            "voiceChatEnabled": false,
            "desktopEnabled": false,
            "privateServerPriceRobux": 25,
            "discordSocialLink": { "title": "Other", "uri": "https://discord.gg/other" }
        }),
        json!({ "displayName": "On Roblox", "description": "set there", "serverSize": 12 }),
    );
    remote.access = Some(Access::Public);

    let refreshed = refresh(&game, &lock, &remote);
    let report = crate::live::compare_read(&refreshed.lock, &remote, false);

    assert!(report.differences.is_empty(), "{:?}", report.differences);
}
