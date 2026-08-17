#![allow(clippy::unwrap_used)]
//! The universe configuration read, against the body Roblox actually returns.
//!
//! Captured from a live universe on 2026-08-17 — ids and names replaced with
//! invented ones — after the read started failing outright. `GET
//! /v1/universes/{id}/configuration` answers with the *names* of the enum
//! fields, where the v2 `PATCH` takes their integers. Modelling only the
//! integers broke the whole struct, and with it every cookie-only field `pull`
//! and `init` read, including one that had worked for months.

use super::*;
use crate::config::{AnimationType, AvatarType, CollisionType, Genre, JointPositioningType};

/// The shape as returned, with the enum fields spelled as names.
const REAL_BODY: &str = r#"{
  "allowPrivateServers": false,
  "privateServerPrice": null,
  "isMeshTextureApiAccessAllowed": false,
  "isRewardedOnDemandAdsAllowed": false,
  "id": 1122334455,
  "name": "an experience",
  "universeAvatarType": "MorphToR15",
  "universeScaleType": "AllScales",
  "universeAnimationType": "PlayerChoice",
  "universeCollisionType": "OuterBox",
  "universeBodyType": "Standard",
  "universeJointPositioningType": "ArtistIntent",
  "isArchived": false,
  "isFriendsOnly": false,
  "genre": "All",
  "playableDevices": ["Computer", "Phone", "Tablet", "VR"],
  "isForSale": false,
  "price": 0,
  "isStudioAccessToApisAllowed": false,
  "privacyType": "Private",
  "isForSaleInFiat": false,
  "fiatBasePriceId": null,
  "fiatModerationStatus": "NotModerated",
  "audiences": [1],
  "demoModeEnabled": false
}"#;

/// Through the real parsing path, not a direct deserialize of the model —
/// nothing ever deserializes that from the wire, so a test that did would pass
/// while the endpoint stayed broken.
fn parse(body: &str) -> UniverseConfigLegacy {
    parse_v1_universe_config(body).expect("the real response body must deserialize")
}

/// The regression itself. Before the fix this returned `Err`, and the caller
/// turned that into "skipping the cookie-only universe fields" — so a field
/// that had nothing to do with avatars stopped being read.
#[test]
fn the_real_response_body_deserializes() {
    let config = parse(REAL_BODY);
    assert_eq!(config.studio_access_to_apis_allowed, Some(false));
}

/// Names resolve through the name parsers, which is what the live endpoint
/// actually sends.
#[test]
fn the_named_spelling_resolves_to_the_right_variant() {
    let config = parse(REAL_BODY);

    let avatar = config
        .universe_avatar_type
        .as_ref()
        .and_then(|v| v.resolve(AvatarType::from_legacy, AvatarType::from_api_name));
    assert_eq!(avatar, Some(AvatarType::R15));

    let animation = config
        .universe_animation_type
        .as_ref()
        .and_then(|v| v.resolve(AnimationType::from_legacy, AnimationType::from_api_name));
    assert_eq!(animation, Some(AnimationType::PlayerChoice));

    let collision = config
        .universe_collision_type
        .as_ref()
        .and_then(|v| v.resolve(CollisionType::from_legacy, CollisionType::from_api_name));
    assert_eq!(collision, Some(CollisionType::OuterBox));

    let joints = config
        .universe_joint_positioning_type
        .as_ref()
        .and_then(|v| {
            v.resolve(
                JointPositioningType::from_legacy,
                JointPositioningType::from_api_name,
            )
        });
    assert_eq!(joints, Some(JointPositioningType::ArtistIntent));

    let genre = config
        .genre
        .as_ref()
        .and_then(|v| v.resolve(Genre::from_legacy, Genre::from_api_name));
    assert_eq!(genre, Some(Genre::All));
}

/// The integer spelling still resolves. The vendored spec documents the
/// request type as an integer, so a response that used one would not be a
/// surprise — and nothing says which spelling a future response will pick.
#[test]
fn the_integer_spelling_still_resolves() {
    let config = parse(r#"{"universeAvatarType": 3, "universeAnimationType": 2, "genre": 14}"#);

    assert_eq!(
        config
            .universe_avatar_type
            .as_ref()
            .and_then(|v| v.resolve(AvatarType::from_legacy, AvatarType::from_api_name)),
        Some(AvatarType::R15)
    );
    assert_eq!(
        config
            .genre
            .as_ref()
            .and_then(|v| v.resolve(Genre::from_legacy, Genre::from_api_name)),
        Some(Genre::WildWest)
    );
}

/// A name nobody has documented is not coerced into the first variant — the
/// same contract the integer parsers hold. A pull that guessed here would write
/// a wrong value into the config, and the next sync would apply it.
#[test]
fn an_unknown_name_resolves_to_nothing() {
    let config = parse(r#"{"universeAvatarType": "MorphToR27", "genre": "Interpretive Dance"}"#);

    assert_eq!(
        config
            .universe_avatar_type
            .as_ref()
            .and_then(|v| v.resolve(AvatarType::from_legacy, AvatarType::from_api_name)),
        None
    );
    assert_eq!(
        config
            .genre
            .as_ref()
            .and_then(|v| v.resolve(Genre::from_legacy, Genre::from_api_name)),
        None
    );
}

/// Fields this build does not model must not break the ones it does. The real
/// body carries `universeScaleType`, `universeBodyType`, `audiences` and
/// `fiatModerationStatus`, none of which are read here.
#[test]
fn unmodelled_fields_do_not_break_the_read() {
    let config = parse(REAL_BODY);
    assert!(config.universe_avatar_type.is_some());
    assert!(matches!(
        config.universe_avatar_type,
        Some(LegacyEnum::Name(_))
    ));
}

/// Every documented name round-trips to the integer the write side sends. A
/// mapping right in one direction only would survive a pull and a sync, and
/// change the setting on the third run.
#[test]
fn every_documented_name_maps_to_its_write_integer() {
    for (name, expected) in [("MorphToR6", 1), ("PlayerChoice", 2), ("MorphToR15", 3)] {
        assert_eq!(
            AvatarType::from_api_name(name).map(AvatarType::to_legacy),
            Some(expected),
            "{name}"
        );
    }

    for (name, expected) in [
        ("All", 0),
        ("Tutorial", 1),
        ("TownAndCity", 3),
        ("SciFi", 8),
        ("FPS", 10),
        ("RPG", 11),
        ("WildWest", 14),
    ] {
        assert_eq!(
            Genre::from_api_name(name).map(Genre::to_legacy),
            Some(expected),
            "{name}"
        );
    }
}
