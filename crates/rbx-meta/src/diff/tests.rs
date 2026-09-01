//! The diff tests, moved out of the module file without a line of them
//! changing.

#![allow(clippy::unwrap_used)]

use super::*;
use crate::config::{AvatarScales, ServerFill, SocialLink};
use crate::config::{Devices, MediaConfig, PrivateServer};
use crate::lockfile::MediaLock;
use rbx_core::image::{hash_bytes, process_image};
use serde_json::json;
use std::path::Path;

// -----------------------------------------------------------------------
// Fixtures
// -----------------------------------------------------------------------

fn game() -> Game {
    Game::default()
}

fn lock() -> GameLock {
    GameLock::default()
}

fn link(title: &str, url: &str) -> SocialLink {
    SocialLink {
        title: title.to_string(),
        url: url.to_string(),
    }
}

/// `build_plan` with no media, so nothing touches the filesystem.
fn plan_for(game: &Game, lock: &GameLock) -> SyncPlan {
    build_plan(
        game,
        &MediaConfig::default(),
        lock,
        &MediaLockfile::default(),
        Path::new("."),
    )
    .expect("a plan with no media never reads a file")
}

// -----------------------------------------------------------------------
// build_plan: the entry point
// -----------------------------------------------------------------------

mod build_plan {
    use super::*;

    #[test]
    fn a_config_that_matches_the_lock_produces_an_empty_plan() {
        let mut g = game();
        g.name = Some("Game".into());
        g.server_size = Some(50);
        g.voice_chat = Some(true);
        g.visibility = Some(Visibility::Public);
        g.beta_mode = Some(false);
        g.allow_copying = Some(true);
        g.studio_access_to_apis_allowed = Some(true);
        g.private_server = Some(PrivateServer { price: 100 });
        g.devices.desktop = Some(true);
        g.social_links.discord = Some(link("Discord", "https://d.test"));
        g.server_fill = Some(ServerFill::Empty);

        let plan = plan_for(&g, &config_to_lock(&g));

        assert!(
            plan.is_empty(),
            "a lock built from the config itself leaves nothing to do: {plan:?}"
        );
    }

    /// An empty config is not a request to clear everything. Every field is
    /// `Option`, and `None` means "not declared here", not "set to nothing".
    #[test]
    fn an_empty_config_against_a_populated_lock_asks_for_nothing() {
        let mut locked = lock();
        locked.name = Some("Remote name".into());
        locked.server_size = Some(30);
        locked.voice_chat = Some(true);
        locked.devices.desktop = Some(true);
        locked.visibility = Some(Visibility::Public);
        locked.beta_mode = Some(true);
        locked.allow_copying = Some(true);
        locked.studio_access_to_apis_allowed = Some(true);
        locked.server_fill = Some(ServerFill::Automatic);

        let plan = plan_for(&game(), &locked);

        assert!(plan.universe_patch.is_none());
        assert!(plan.place_patch.is_none());
        assert!(plan.place_legacy_patch.is_none());
        assert!(plan.universe_legacy_patch.is_none());
        assert!(plan.visibility_change.is_none());
        assert!(plan.beta_mode_change.is_none());
        assert!(plan.is_empty());
    }

    /// The one exception to the rule above: a private server that the lock
    /// has and the config does not is an explicit "disable", because
    /// removing the `[game.private_server]` table is how you disable it.
    #[test]
    fn a_dropped_private_server_table_is_an_explicit_disable() {
        let mut locked = lock();
        locked.private_server = Some(PrivateServer { price: 100 });

        let plan = plan_for(&game(), &locked);
        let patch = plan.universe_patch.expect("disabling is a change");

        assert_eq!(patch.body, json!({ "privateServerPriceRobux": null }));
        assert_eq!(patch.mask, vec!["privateServerPriceRobux"]);
    }

    /// Each field lands in the patch its own API demands. Getting this
    /// wrong sends a cookie-only field to Open Cloud, which ignores it.
    #[test]
    fn each_field_is_routed_to_the_api_that_owns_it() {
        let mut g = game();
        g.name = Some("Name".into()); // place
        g.voice_chat = Some(true); // universe
        g.allow_copying = Some(true); // place, legacy
        g.studio_access_to_apis_allowed = Some(true); // universe config, legacy
        g.visibility = Some(Visibility::Public); // activate/deactivate
        g.beta_mode = Some(true); // experience releases

        let plan = plan_for(&g, &lock());

        assert_eq!(
            plan.universe_patch.expect("universe").body,
            json!({ "voiceChatEnabled": true })
        );
        assert_eq!(
            plan.place_patch.expect("place").body,
            json!({ "displayName": "Name" })
        );
        assert_eq!(
            plan.place_legacy_patch.expect("place legacy").body,
            json!({ "copyingAllowed": true })
        );
        assert_eq!(
            plan.universe_legacy_patch.expect("universe legacy").body,
            json!({ "studioAccessToApisAllowed": true })
        );
        assert_eq!(plan.visibility_change, Some(Visibility::Public));
        assert_eq!(plan.beta_mode_change, Some(true));
    }
}

// -----------------------------------------------------------------------
// build_universe_patch
// -----------------------------------------------------------------------

mod universe_patch {
    use super::*;

    #[test]
    fn no_change_means_no_patch() {
        let mut g = game();
        g.voice_chat = Some(true);
        let mut l = lock();
        l.voice_chat = Some(true);

        assert!(plan_for(&g, &l).universe_patch.is_none());
    }

    #[test]
    fn every_device_toggle_has_its_own_api_field_and_mask_entry() {
        let mut g = game();
        g.devices = Devices {
            desktop: Some(true),
            mobile: Some(false),
            tablet: Some(true),
            console: Some(false),
            vr: Some(true),
        };

        let patch = plan_for(&g, &lock()).universe_patch.expect("patch");

        assert_eq!(
            patch.body,
            json!({
                "desktopEnabled": true,
                "mobileEnabled": false,
                "tabletEnabled": true,
                "consoleEnabled": false,
                "vrEnabled": true,
            })
        );
        assert_eq!(
            patch.mask,
            vec![
                "desktopEnabled",
                "mobileEnabled",
                "tabletEnabled",
                "consoleEnabled",
                "vrEnabled"
            ]
        );
    }

    /// Only the devices that actually differ are sent. A mask naming a
    /// field the body does not carry is how you clear a field by accident.
    #[test]
    fn only_the_devices_that_differ_are_sent() {
        let mut g = game();
        g.devices.desktop = Some(true);
        g.devices.mobile = Some(true);
        let mut l = lock();
        l.devices.desktop = Some(true);

        let patch = plan_for(&g, &l).universe_patch.expect("patch");

        assert_eq!(patch.body, json!({ "mobileEnabled": true }));
        assert_eq!(patch.mask, vec!["mobileEnabled"]);
    }

    #[test]
    fn a_social_link_is_sent_as_the_api_shape() {
        let mut g = game();
        g.social_links.discord = Some(link("Join us", "https://discord.gg/x"));

        let patch = plan_for(&g, &lock()).universe_patch.expect("patch");

        assert_eq!(
            patch.body,
            json!({
                "discordSocialLink": { "title": "Join us", "uri": "https://discord.gg/x" }
            })
        );
        assert_eq!(patch.mask, vec!["discordSocialLink"]);
    }

    /// A link in the lock and not in the config is a removal, sent as an
    /// explicit null. This is the one place where "absent" does mean
    /// "clear it", because a social link has no other way to be removed.
    #[test]
    fn a_link_dropped_from_the_config_is_removed_with_an_explicit_null() {
        let mut l = lock();
        l.social_links.twitter = Some(link("Old", "https://x.test"));

        let patch = plan_for(&game(), &l).universe_patch.expect("patch");

        assert_eq!(patch.body, json!({ "twitterSocialLink": null }));
        assert_eq!(patch.mask, vec!["twitterSocialLink"]);
    }

    #[test]
    fn an_unchanged_link_is_left_alone() {
        let mut g = game();
        g.social_links.youtube = Some(link("Yt", "https://yt.test"));
        let mut l = lock();
        l.social_links.youtube = Some(link("Yt", "https://yt.test"));

        assert!(plan_for(&g, &l).universe_patch.is_none());
    }

    #[test]
    fn retitling_a_link_at_the_same_url_still_patches() {
        let mut g = game();
        g.social_links.youtube = Some(link("New title", "https://yt.test"));
        let mut l = lock();
        l.social_links.youtube = Some(link("Old title", "https://yt.test"));

        let patch = plan_for(&g, &l).universe_patch.expect("patch");

        assert_eq!(
            patch.body,
            json!({
                "youtubeSocialLink": { "title": "New title", "uri": "https://yt.test" }
            })
        );
    }

    #[test]
    fn a_private_server_price_change_carries_the_new_price() {
        let mut g = game();
        g.private_server = Some(PrivateServer { price: 250 });
        let mut l = lock();
        l.private_server = Some(PrivateServer { price: 100 });

        let patch = plan_for(&g, &l).universe_patch.expect("patch");

        assert_eq!(patch.body, json!({ "privateServerPriceRobux": 250 }));
        assert_eq!(patch.mask, vec!["privateServerPriceRobux"]);
    }

    /// Free (price 0) and disabled (no table) are different states, and
    /// `Some(0) != None` has to survive the round trip through the patch.
    #[test]
    fn free_private_servers_are_not_the_same_as_disabled_ones() {
        let mut g = game();
        g.private_server = Some(PrivateServer { price: 0 });

        let patch = plan_for(&g, &lock()).universe_patch.expect("patch");

        assert_eq!(patch.body, json!({ "privateServerPriceRobux": 0 }));
    }
}

// -----------------------------------------------------------------------
// build_place_patch
// -----------------------------------------------------------------------

mod place_patch {
    use super::*;

    #[test]
    fn name_description_and_server_size_map_to_their_api_fields() {
        let mut g = game();
        g.name = Some("My Game".into());
        g.description = Some("Fun".into());
        g.server_size = Some(50);

        let patch = plan_for(&g, &lock()).place_patch.expect("patch");

        assert_eq!(
            patch.body,
            json!({ "displayName": "My Game", "description": "Fun", "serverSize": 50 })
        );
        assert_eq!(patch.mask, vec!["displayName", "description", "serverSize"]);
    }

    #[test]
    fn only_the_fields_that_differ_are_sent() {
        let mut g = game();
        g.name = Some("Same".into());
        g.server_size = Some(60);
        let mut l = lock();
        l.name = Some("Same".into());
        l.server_size = Some(50);

        let patch = plan_for(&g, &l).place_patch.expect("patch");

        assert_eq!(patch.body, json!({ "serverSize": 60 }));
        assert_eq!(patch.mask, vec!["serverSize"]);
    }

    #[test]
    fn an_empty_string_is_a_value_not_an_absence() {
        let mut g = game();
        g.description = Some(String::new());
        let mut l = lock();
        l.description = Some("something".into());

        let patch = plan_for(&g, &l).place_patch.expect("patch");

        assert_eq!(patch.body, json!({ "description": "" }));
    }
}

// -----------------------------------------------------------------------
// Legacy (cookie-only) patches
// -----------------------------------------------------------------------

mod legacy_patches {
    use super::*;

    #[test]
    fn a_custom_server_fill_carries_its_reserved_slot_count() {
        let mut g = game();
        g.server_fill = Some(ServerFill::Custom { reserved_slots: 5 });

        let patch = plan_for(&g, &lock()).place_legacy_patch.expect("patch");

        assert_eq!(
            patch.body,
            json!({ "socialSlotType": "Custom", "customSocialSlotsCount": 5 })
        );
    }

    /// Non-custom modes must not carry a slot count: the legacy API takes
    /// the field literally, and a stale count on `Automatic` sticks.
    #[test]
    fn a_non_custom_server_fill_sends_no_slot_count() {
        let mut g = game();
        g.server_fill = Some(ServerFill::Automatic);
        let mut l = lock();
        l.server_fill = Some(ServerFill::Custom { reserved_slots: 5 });

        let patch = plan_for(&g, &l).place_legacy_patch.expect("patch");

        assert_eq!(patch.body, json!({ "socialSlotType": "Automatic" }));
    }

    #[test]
    fn changing_only_the_reserved_slot_count_is_still_a_change() {
        let mut g = game();
        g.server_fill = Some(ServerFill::Custom { reserved_slots: 8 });
        let mut l = lock();
        l.server_fill = Some(ServerFill::Custom { reserved_slots: 5 });

        let patch = plan_for(&g, &l).place_legacy_patch.expect("patch");

        assert_eq!(
            patch.body,
            json!({ "socialSlotType": "Custom", "customSocialSlotsCount": 8 })
        );
    }

    #[test]
    fn allow_copying_and_server_fill_share_one_legacy_patch() {
        let mut g = game();
        g.allow_copying = Some(false);
        g.server_fill = Some(ServerFill::Empty);

        let patch = plan_for(&g, &lock()).place_legacy_patch.expect("patch");

        assert_eq!(
            patch.body,
            json!({ "socialSlotType": "Empty", "copyingAllowed": false })
        );
    }

    /// `engineAvatarSettings` restates the legacy avatar fields rather than
    /// extending them, and neither side can be read back, so a sync that
    /// sends both is a contradiction nothing downstream can detect. Found
    /// the hard way: a test universe sent both came back as
    /// `AvatarSettings Error: Failed to deserialize properties` the next
    /// time Studio opened it.
    mod engine_avatar_overlap {
        use super::*;

        /// Reduced to what the guard reads. The real document nests its rule
        /// sections under wrappers this crate deliberately does not model,
        /// so the nesting is kept to prove the search does not depend on a
        /// fixed path.
        const WITH_BODY_RULES: &str =
            r#"{"Settings":{"AvatarBodyRules":{"CustomHeightScale":[1,1]}}}"#;
        /// A document that says nothing about the body: no overlap to find.
        const COLLISION_ONLY: &str = r#"{"Settings":{"AvatarCollisionRules":{"CollisionMode":1}}}"#;

        fn project(document: &str) -> tempfile::TempDir {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("avatar.json"), document).unwrap();
            dir
        }

        fn plan_in(dir: &Path, game: &Game, lock: &GameLock) -> Result<SyncPlan> {
            build_plan(
                game,
                &MediaConfig::default(),
                lock,
                &MediaLockfile::default(),
                dir,
            )
        }

        fn scales() -> AvatarScales {
            AvatarScales {
                height: 1.0,
                width: 1.0,
                head: 1.0,
                body_type: 0.0,
                proportion: 0.0,
            }
        }

        #[test]
        fn scales_beside_a_body_rules_document_are_refused() {
            let dir = project(WITH_BODY_RULES);
            let mut g = game();
            g.avatar.min_scale = Some(scales());
            g.engine_avatar_settings = Some(PathBuf::from("avatar.json"));

            let err = plan_in(dir.path(), &g, &lock()).unwrap_err().to_string();

            // Names the TOML the reader wrote, not the wire field.
            assert!(err.contains("game.avatar.min_scale"), "{err}");
            assert!(err.contains("AvatarBodyRules"), "{err}");
        }

        /// A second entry of the table, so the guard is not pinned to one
        /// row of it.
        #[test]
        fn the_max_scale_is_caught_as_well_as_the_min() {
            let dir = project(WITH_BODY_RULES);
            let mut g = game();
            g.avatar.max_scale = Some(scales());
            g.engine_avatar_settings = Some(PathBuf::from("avatar.json"));

            let err = plan_in(dir.path(), &g, &lock()).unwrap_err().to_string();

            assert!(err.contains("game.avatar.max_scale"), "{err}");
        }

        /// The control that keeps the guard from being "refuse whenever both
        /// keys exist". A document that describes collisions and nothing
        /// else does not overlap the scales.
        #[test]
        fn a_document_that_does_not_mention_the_body_is_allowed_beside_scales() {
            let dir = project(COLLISION_ONLY);
            let mut g = game();
            g.avatar.min_scale = Some(scales());
            g.engine_avatar_settings = Some(PathBuf::from("avatar.json"));

            let plan = plan_in(dir.path(), &g, &lock()).expect("no overlap to refuse");
            let body = &plan.universe_legacy_patch.expect("patch").body;

            assert!(body.get("universeAvatarMinScales").is_some());
            assert!(body.get("engineAvatarSettings").is_some());
        }

        /// Either channel alone is the ordinary case and must stay usable.
        #[test]
        fn the_document_alone_is_allowed() {
            let dir = project(WITH_BODY_RULES);
            let mut g = game();
            g.engine_avatar_settings = Some(PathBuf::from("avatar.json"));

            let plan = plan_in(dir.path(), &g, &lock()).expect("one channel is fine");

            assert!(plan
                .universe_legacy_patch
                .expect("patch")
                .body
                .get("engineAvatarSettings")
                .is_some());
        }

        #[test]
        fn the_scales_alone_are_allowed() {
            let dir = project(WITH_BODY_RULES);
            let mut g = game();
            g.avatar.min_scale = Some(scales());

            let plan = plan_in(dir.path(), &g, &lock()).expect("one channel is fine");

            assert!(plan
                .universe_legacy_patch
                .expect("patch")
                .body
                .get("universeAvatarMinScales")
                .is_some());
        }

        /// A scale already recorded in the lock produces no body key, so
        /// there is nothing to contradict and nothing to refuse.
        #[test]
        fn an_unchanged_scale_does_not_trip_the_guard() {
            let dir = project(WITH_BODY_RULES);
            let mut g = game();
            g.avatar.min_scale = Some(scales());
            g.engine_avatar_settings = Some(PathBuf::from("avatar.json"));
            let mut l = lock();
            l.avatar.min_scale = Some(scales());

            plan_in(dir.path(), &g, &l).expect("nothing to send, nothing to clash");
        }
    }

    #[test]
    fn studio_api_access_goes_to_the_universe_config_endpoint() {
        let mut g = game();
        g.studio_access_to_apis_allowed = Some(false);
        let mut l = lock();
        l.studio_access_to_apis_allowed = Some(true);

        let patch = plan_for(&g, &l).universe_legacy_patch.expect("patch");

        assert_eq!(patch.body, json!({ "studioAccessToApisAllowed": false }));
    }
}

// -----------------------------------------------------------------------
// visibility and beta_mode
//
// Neither is a patch body: both are separate endpoints, and visibility in
// particular decides the *order* the rest of the plan is applied in.
// `commands::sync` activates before every other call when going public
// (a paid private server cannot be set on a private universe) and
// deactivates after them all when going private.
// -----------------------------------------------------------------------

mod visibility_and_beta {
    use super::*;

    #[test]
    fn going_from_private_to_public_is_a_change() {
        let mut g = game();
        g.visibility = Some(Visibility::Public);
        let mut l = lock();
        l.visibility = Some(Visibility::Private);

        let plan = plan_for(&g, &l);

        assert_eq!(plan.visibility_change, Some(Visibility::Public));
        assert!(
            plan.visibility_change.is_some_and(|v| v.is_public()),
            "sync keys the activate-first branch off is_public()"
        );
    }

    #[test]
    fn going_from_public_to_private_is_a_change() {
        let mut g = game();
        g.visibility = Some(Visibility::Private);
        let mut l = lock();
        l.visibility = Some(Visibility::Public);

        let plan = plan_for(&g, &l);

        assert_eq!(plan.visibility_change, Some(Visibility::Private));
        assert!(
            plan.visibility_change.is_some_and(|v| !v.is_public()),
            "sync keys the deactivate-last branch off !is_public()"
        );
    }

    /// A universe that has never been synced has no locked visibility.
    /// Declaring `public` must still activate it, or every other patch in
    /// the same run is sent against a private universe.
    #[test]
    fn an_unlocked_visibility_still_produces_a_change() {
        let mut g = game();
        g.visibility = Some(Visibility::Public);

        assert_eq!(
            plan_for(&g, &lock()).visibility_change,
            Some(Visibility::Public)
        );
    }

    #[test]
    fn an_already_matching_visibility_is_not_a_change() {
        let mut g = game();
        g.visibility = Some(Visibility::Public);
        let mut l = lock();
        l.visibility = Some(Visibility::Public);

        assert_eq!(plan_for(&g, &l).visibility_change, None);
    }

    /// Going public while enabling a paid private server is the
    /// combination the ordering exists for: the price patch is rejected by
    /// Roblox unless the universe is already public. Both must be in the
    /// same plan for sync to be able to order them.
    #[test]
    fn going_public_and_pricing_a_private_server_land_in_the_same_plan() {
        let mut g = game();
        g.visibility = Some(Visibility::Public);
        g.private_server = Some(PrivateServer { price: 100 });
        let mut l = lock();
        l.visibility = Some(Visibility::Private);

        let plan = plan_for(&g, &l);

        assert_eq!(plan.visibility_change, Some(Visibility::Public));
        assert_eq!(
            plan.universe_patch.expect("price patch").body,
            json!({ "privateServerPriceRobux": 100 })
        );
    }

    #[test]
    fn beta_mode_toggles_only_when_it_differs() {
        let mut g = game();
        g.beta_mode = Some(true);
        assert_eq!(plan_for(&g, &lock()).beta_mode_change, Some(true));

        let mut l = lock();
        l.beta_mode = Some(true);
        assert_eq!(plan_for(&g, &l).beta_mode_change, None);

        l.beta_mode = Some(false);
        assert_eq!(plan_for(&g, &l).beta_mode_change, Some(true));
    }
}

// -----------------------------------------------------------------------
// Media: icon and thumbnails
// -----------------------------------------------------------------------

/// 1x1 PNGs, one per colour, so each has a distinct hash. Written to a
/// temp dir per test rather than committed as fixture files.
const RED: &str = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d4944415478da63f8cfc0f01f00050001ff56c72f0d0000000049454e44ae426082";
const GREEN: &str = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d4944415478da6360f8cff01f00040101ffaeb555f50000000049454e44ae426082";
const BLUE: &str = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d4944415478da636060f8ff1f00030201ff392919be0000000049454e44ae426082";
const WHITE: &str = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000b4944415478da63f80f040009fb03fd68fa1ccc0000000049454e44ae426082";

fn from_hex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
        .collect()
}

/// Write `name` into `dir` with the given PNG, and return the hash
/// `build_*_plan` will compute for it. Computed rather than hardcoded: the
/// hash is of the *re-encoded* bytes, so it is the image pipeline's to own.
fn put_image(dir: &Path, name: &str, hex: &str) -> String {
    std::fs::write(dir.join(name), from_hex(hex)).expect("write png");
    hash_bytes(&process_image(&dir.join(name), true).expect("process png"))
}

fn media_with(thumbnails: &[&str]) -> MediaConfig {
    MediaConfig {
        thumbnails: thumbnails.iter().map(PathBuf::from).collect(),
        ..MediaConfig::default()
    }
}

fn locked(entries: &[(&str, Option<u64>)]) -> MediaLockfile {
    MediaLockfile {
        icon: None,
        thumbnails: entries
            .iter()
            .map(|(hash, image_id)| MediaLock {
                hash: (*hash).to_string(),
                image_id: *image_id,
            })
            .collect(),
    }
}

fn thumbnail_plan(dir: &Path, media: &MediaConfig, media_lock: &MediaLockfile) -> ThumbnailPlan {
    build_plan(&game(), media, &lock(), media_lock, dir)
        .expect("plan")
        .thumbnails
}

fn slot_ids(plan: &ThumbnailPlan) -> Vec<Option<u64>> {
    plan.slots
        .iter()
        .map(|s| match s {
            ThumbSlot::Keep { image_id, .. } => Some(*image_id),
            ThumbSlot::NewUpload { .. } => None,
        })
        .collect()
}

mod icon {
    use super::*;

    #[test]
    fn an_unchanged_icon_is_not_re_uploaded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hash = put_image(dir.path(), "icon.png", RED);

        let media = MediaConfig {
            icon: Some(PathBuf::from("icon.png")),
            ..MediaConfig::default()
        };
        let media_lock = MediaLockfile {
            icon: Some(MediaLock {
                hash,
                image_id: Some(1),
            }),
            thumbnails: Vec::new(),
        };

        let plan = build_plan(&game(), &media, &lock(), &media_lock, dir.path()).expect("plan");

        assert!(matches!(plan.icon, IconPlan::None));
    }

    #[test]
    fn a_changed_icon_is_uploaded_with_its_config_relative_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        put_image(dir.path(), "icon.png", RED);
        let stale = put_image(dir.path(), "other.png", GREEN);

        let media = MediaConfig {
            icon: Some(PathBuf::from("icon.png")),
            ..MediaConfig::default()
        };
        let media_lock = MediaLockfile {
            icon: Some(MediaLock {
                hash: stale,
                image_id: Some(1),
            }),
            thumbnails: Vec::new(),
        };

        let plan = build_plan(&game(), &media, &lock(), &media_lock, dir.path()).expect("plan");

        match plan.icon {
            IconPlan::Upload { path, .. } => assert_eq!(path, PathBuf::from("icon.png")),
            IconPlan::None => panic!("a changed icon must be uploaded"),
        }
    }

    #[test]
    fn no_icon_in_the_config_is_not_a_deletion() {
        let dir = tempfile::tempdir().expect("tempdir");
        let media_lock = MediaLockfile {
            icon: Some(MediaLock {
                hash: "whatever".into(),
                image_id: Some(1),
            }),
            thumbnails: Vec::new(),
        };

        let plan = build_plan(
            &game(),
            &MediaConfig::default(),
            &lock(),
            &media_lock,
            dir.path(),
        )
        .expect("plan");

        assert!(matches!(plan.icon, IconPlan::None));
    }
}

// -----------------------------------------------------------------------
// build_thumbnail_plan
//
// The delete / upload / reorder reconciliation. Thumbnails are matched to
// lockfile entries by content hash, not by position, so the interesting
// cases are all about which lock entry a local file claims and what is
// left over.
// -----------------------------------------------------------------------

mod thumbnails {
    use super::*;

    #[test]
    fn a_first_sync_uploads_everything_in_config_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        put_image(dir.path(), "a.png", RED);
        put_image(dir.path(), "b.png", GREEN);

        let plan = thumbnail_plan(dir.path(), &media_with(&["a.png", "b.png"]), &locked(&[]));

        assert!(plan.deletes.is_empty());
        assert_eq!(
            plan.uploads
                .iter()
                .map(|u| (u.path.clone(), u.slot_index))
                .collect::<Vec<_>>(),
            vec![(PathBuf::from("a.png"), 0), (PathBuf::from("b.png"), 1)]
        );
        assert_eq!(slot_ids(&plan), vec![None, None]);
        assert!(!plan.needs_reorder, "there is nothing to reorder yet");
    }

    #[test]
    fn thumbnails_already_uploaded_in_the_same_order_are_left_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = put_image(dir.path(), "a.png", RED);
        let b = put_image(dir.path(), "b.png", GREEN);

        let plan = thumbnail_plan(
            dir.path(),
            &media_with(&["a.png", "b.png"]),
            &locked(&[(&a, Some(10)), (&b, Some(20))]),
        );

        assert!(plan.is_empty(), "nothing to do: {plan:?}");
        assert_eq!(slot_ids(&plan), vec![Some(10), Some(20)]);
    }

    #[test]
    fn a_thumbnail_dropped_from_the_config_is_deleted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = put_image(dir.path(), "a.png", RED);
        let b = put_image(dir.path(), "b.png", GREEN);

        let plan = thumbnail_plan(
            dir.path(),
            &media_with(&["a.png"]),
            &locked(&[(&a, Some(10)), (&b, Some(20))]),
        );

        assert_eq!(plan.deletes, vec![20]);
        assert!(plan.uploads.is_empty());
        assert_eq!(slot_ids(&plan), vec![Some(10)]);
    }

    #[test]
    fn clearing_the_config_deletes_every_tracked_thumbnail() {
        let dir = tempfile::tempdir().expect("tempdir");

        let plan = thumbnail_plan(
            dir.path(),
            &media_with(&[]),
            &locked(&[("h1", Some(10)), ("h2", Some(20))]),
        );

        assert_eq!(plan.deletes, vec![10, 20]);
        assert!(plan.slots.is_empty());
    }

    /// A lock entry with no `image_id` is one whose upload response never
    /// came back. There is nothing to delete on Roblox and nothing to
    /// reuse, so the local file has to be uploaded again.
    #[test]
    fn a_lock_entry_without_an_image_id_is_re_uploaded_not_deleted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = put_image(dir.path(), "a.png", RED);

        let plan = thumbnail_plan(dir.path(), &media_with(&["a.png"]), &locked(&[(&a, None)]));

        assert!(
            plan.deletes.is_empty(),
            "there is no remote image to delete"
        );
        assert_eq!(plan.uploads.len(), 1);
        assert_eq!(slot_ids(&plan), vec![None]);
    }

    /// An orphaned entry with no `image_id` is no remote work: there is
    /// nothing to delete and nothing to reorder, so the plan is empty and
    /// stays empty. That is correct here and is deliberately not where the
    /// entry gets cleaned up.
    ///
    /// It used to be nowhere. `sync` only dropped lockfile entries inside
    /// its deletes loop, which is keyed on `image_id`, so an entry without
    /// one survived every run forever (#22). `sync` now prunes those as
    /// bookkeeping, before its own "nothing to do" exit, which it has to
    /// be, because this plan being empty is exactly what that exit tests.
    #[test]
    fn an_orphaned_lock_entry_without_an_image_id_is_no_remote_work() {
        let dir = tempfile::tempdir().expect("tempdir");

        let plan = thumbnail_plan(dir.path(), &media_with(&[]), &locked(&[("gone", None)]));

        assert!(plan.deletes.is_empty());
        assert!(plan.is_empty());
    }

    /// Reordering the same files is invisible to a delete/upload diff:
    /// nothing is added and nothing is removed. Without `needs_reorder`
    /// the plan would look empty and the new order would never be sent.
    #[test]
    fn a_pure_reorder_is_detected_even_though_nothing_is_added_or_removed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = put_image(dir.path(), "a.png", RED);
        let b = put_image(dir.path(), "b.png", GREEN);

        let plan = thumbnail_plan(
            dir.path(),
            &media_with(&["b.png", "a.png"]),
            &locked(&[(&a, Some(10)), (&b, Some(20))]),
        );

        assert!(plan.deletes.is_empty());
        assert!(plan.uploads.is_empty());
        assert!(plan.needs_reorder, "the order changed and must be sent");
        assert!(!plan.is_empty(), "a pure reorder is not an empty plan");
        assert_eq!(slot_ids(&plan), vec![Some(20), Some(10)]);
    }

    /// `needs_reorder` is only computed when there is nothing else to do,
    /// because uploads and deletes change the remote order anyway and the
    /// reorder is derived from the post-op state instead.
    #[test]
    fn a_reorder_alongside_an_upload_is_left_to_the_post_upload_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = put_image(dir.path(), "a.png", RED);
        put_image(dir.path(), "c.png", BLUE);

        let plan = thumbnail_plan(
            dir.path(),
            &media_with(&["c.png", "a.png"]),
            &locked(&[(&a, Some(10))]),
        );

        assert_eq!(plan.uploads.len(), 1);
        assert!(
            !plan.needs_reorder,
            "with an upload pending, the order comes from desired_order instead"
        );
        assert!(!plan.is_empty());
    }

    /// Two identical images at different positions must consume two
    /// distinct lock entries. Matching one entry twice would leave the
    /// other unconsumed, and it would be deleted out from under a slot
    /// that still points at it.
    #[test]
    fn duplicate_images_consume_one_lock_entry_each() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = put_image(dir.path(), "a.png", RED);
        std::fs::copy(dir.path().join("a.png"), dir.path().join("copy.png")).expect("copy");

        let plan = thumbnail_plan(
            dir.path(),
            &media_with(&["a.png", "copy.png"]),
            &locked(&[(&a, Some(10)), (&a, Some(11))]),
        );

        assert!(
            plan.deletes.is_empty(),
            "both lock entries are claimed, so neither is deleted: {plan:?}"
        );
        assert!(plan.uploads.is_empty());
        assert_eq!(slot_ids(&plan), vec![Some(10), Some(11)]);
    }

    /// The same image twice locally against a single lock entry: the first
    /// slot reuses it, the second has to be a fresh upload.
    #[test]
    fn a_duplicate_beyond_the_locked_copies_is_uploaded_again() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = put_image(dir.path(), "a.png", RED);
        std::fs::copy(dir.path().join("a.png"), dir.path().join("copy.png")).expect("copy");

        let plan = thumbnail_plan(
            dir.path(),
            &media_with(&["a.png", "copy.png"]),
            &locked(&[(&a, Some(10))]),
        );

        assert_eq!(slot_ids(&plan), vec![Some(10), None]);
        assert_eq!(plan.uploads.len(), 1);
        assert_eq!(plan.uploads[0].slot_index, 1);
        assert!(plan.deletes.is_empty());
    }

    /// Replacing the middle of a list: one delete, one upload, and the
    /// untouched neighbours keep their image IDs in place.
    #[test]
    fn replacing_one_of_three_touches_only_that_slot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = put_image(dir.path(), "a.png", RED);
        let b = put_image(dir.path(), "b.png", GREEN);
        let c = put_image(dir.path(), "c.png", BLUE);
        put_image(dir.path(), "d.png", WHITE);

        let plan = thumbnail_plan(
            dir.path(),
            &media_with(&["a.png", "d.png", "c.png"]),
            &locked(&[(&a, Some(10)), (&b, Some(20)), (&c, Some(30))]),
        );

        assert_eq!(plan.deletes, vec![20]);
        assert_eq!(plan.uploads.len(), 1);
        assert_eq!(plan.uploads[0].slot_index, 1);
        assert_eq!(slot_ids(&plan), vec![Some(10), None, Some(30)]);
    }

    #[test]
    fn upload_bytes_are_the_processed_image_not_the_file_on_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let expected_hash = put_image(dir.path(), "a.png", RED);

        let plan = thumbnail_plan(dir.path(), &media_with(&["a.png"]), &locked(&[]));

        assert_eq!(plan.uploads[0].hash, expected_hash);
        assert_eq!(hash_bytes(&plan.uploads[0].bytes), expected_hash);
    }

    #[test]
    fn a_missing_thumbnail_file_is_an_error_not_a_silent_skip() {
        let dir = tempfile::tempdir().expect("tempdir");

        let err = build_plan(
            &game(),
            &media_with(&["absent.png"]),
            &lock(),
            &MediaLockfile::default(),
            dir.path(),
        )
        .expect_err("a config naming a file that is not there must fail");

        assert!(format!("{err:#}").contains("absent.png"), "{err:#}");
    }
}

// -----------------------------------------------------------------------
// desired_order
//
// The final image-ID order sent to Roblox. Kept images already have their
// IDs; new uploads only get theirs once the upload call returns, and the
// sync loop appends them to the lockfile in upload order. So the config
// order and the post-upload order are different lists, and this function
// is what reconciles them.
// -----------------------------------------------------------------------

mod ordering {
    use super::*;

    fn plan(slots: Vec<ThumbSlot>, uploads: Vec<usize>) -> ThumbnailPlan {
        ThumbnailPlan {
            deletes: Vec::new(),
            uploads: uploads
                .into_iter()
                .map(|slot_index| ThumbUpload {
                    bytes: Vec::new(),
                    hash: String::new(),
                    path: PathBuf::new(),
                    slot_index,
                })
                .collect(),
            slots,
            needs_reorder: false,
        }
    }

    fn keep(image_id: u64) -> ThumbSlot {
        ThumbSlot::Keep {
            hash: String::new(),
            image_id,
        }
    }

    fn new_upload() -> ThumbSlot {
        ThumbSlot::NewUpload {
            hash: String::new(),
        }
    }

    #[test]
    fn kept_images_come_back_in_config_order() {
        let p = plan(vec![keep(30), keep(10), keep(20)], vec![]);

        assert_eq!(desired_order(&p, &[]), vec![30, 10, 20]);
    }

    #[test]
    fn a_fresh_upload_takes_the_id_the_upload_returned() {
        let p = plan(vec![new_upload()], vec![0]);

        assert_eq!(desired_order(&p, &[Some(99)]), vec![99]);
    }

    /// The case the reorder exists for. Uploads are appended to the end of
    /// the remote list, but the config wants the new image first, so the
    /// desired order is not the post-upload order, and sending it is the
    /// only thing that puts the new thumbnail where the user asked.
    #[test]
    fn a_new_upload_is_placed_where_the_config_asks_not_where_it_landed() {
        let p = plan(vec![new_upload(), keep(10), keep(20)], vec![0]);

        assert_eq!(
            desired_order(&p, &[Some(99)]),
            vec![99, 10, 20],
            "the new image goes first, ahead of the images that already existed"
        );
    }

    #[test]
    fn uploads_interleaved_with_kept_images_land_in_their_own_slots() {
        let p = plan(
            vec![keep(10), new_upload(), keep(20), new_upload()],
            vec![1, 3],
        );

        assert_eq!(
            desired_order(&p, &[Some(98), Some(99)]),
            vec![10, 98, 20, 99]
        );
    }

    /// Uploads are indexed by their position in `uploads`, not by slot, so
    /// a plan whose uploads are not in slot order still maps correctly.
    #[test]
    fn upload_ids_are_matched_by_upload_index_not_by_slot_position() {
        let p = plan(vec![new_upload(), new_upload()], vec![1, 0]);

        assert_eq!(
            desired_order(&p, &[Some(98), Some(99)]),
            vec![99, 98],
            "uploads[0] fills slot 1, so its id belongs second"
        );
    }

    /// An upload whose response carried no image ID is skipped rather than
    /// poisoning the order with a placeholder. The rest still reorder.
    #[test]
    fn an_upload_with_no_returned_id_is_left_out_of_the_order() {
        let p = plan(vec![keep(10), new_upload(), keep(20)], vec![1]);

        assert_eq!(desired_order(&p, &[None]), vec![10, 20]);
    }

    #[test]
    fn a_short_id_list_does_not_panic() {
        let p = plan(vec![new_upload(), new_upload()], vec![0, 1]);

        assert_eq!(desired_order(&p, &[Some(98)]), vec![98]);
    }

    #[test]
    fn an_empty_plan_has_no_order_to_send() {
        assert!(desired_order(&plan(vec![], vec![]), &[]).is_empty());
    }
}

// -----------------------------------------------------------------------
// The universe configuration fields that only the cookie can write
// -----------------------------------------------------------------------

mod universe_legacy {
    use super::*;
    use crate::config::{
        AnimationType, AssetOverride, AssetOverrides, Avatar, AvatarScales, AvatarType,
        CollisionType, Genre, JointPositioningType, PaidAccess, Permissions,
    };

    fn body(g: &Game, l: &GameLock) -> Value {
        plan_for(g, l)
            .universe_legacy_patch
            .expect("a legacy patch")
            .body
    }

    fn perms() -> Permissions {
        Permissions {
            third_party_teleport: true,
            third_party_asset: false,
            third_party_purchase: true,
            client_teleport: false,
        }
    }

    /// The object goes out whole, with the PascalCase keys Roblox uses.
    /// Sending a subset would leave the unstated flags at values nothing
    /// here can read back.
    #[test]
    fn permissions_are_sent_as_one_object() {
        let mut g = game();
        g.permissions = Some(perms());

        assert_eq!(
            body(&g, &lock()),
            json!({ "permissions": {
                "IsThirdPartyTeleportAllowed": true,
                "IsThirdPartyAssetAllowed": false,
                "IsThirdPartyPurchaseAllowed": true,
                "IsClientTeleportAllowed": false,
            }})
        );
    }

    #[test]
    fn permissions_matching_the_lock_send_nothing() {
        let mut g = game();
        g.permissions = Some(perms());

        assert!(plan_for(&g, &config_to_lock(&g))
            .universe_legacy_patch
            .is_none());
    }

    /// The three avatar modes map to integers Roblox chose, and the
    /// mapping is not alphabetical: `player_choice` sits between the two
    /// rigs. Asserting the numbers is the only way this stays right.
    #[test]
    fn avatar_modes_map_to_their_legacy_integers() {
        let mut g = game();
        g.avatar = Avatar {
            kind: Some(AvatarType::R15),
            animation: Some(AnimationType::PlayerChoice),
            collision: Some(CollisionType::OuterBox),
            joint_positioning: Some(JointPositioningType::ArtistIntent),
            min_scale: None,
            max_scale: None,
            asset_overrides: None,
        };

        assert_eq!(
            body(&g, &lock()),
            json!({
                "universeAvatarType": 3,
                "universeAnimationType": 2,
                "universeCollisionType": 2,
                "universeJointPositioningType": 2,
            })
        );
    }

    #[test]
    fn r6_is_one_and_player_choice_is_two() {
        let mut g = game();
        g.avatar.kind = Some(AvatarType::R6);
        assert_eq!(body(&g, &lock()), json!({ "universeAvatarType": 1 }));

        g.avatar.kind = Some(AvatarType::PlayerChoice);
        assert_eq!(body(&g, &lock()), json!({ "universeAvatarType": 2 }));
    }

    #[test]
    fn a_scale_table_is_sent_with_roblox_key_casing() {
        let mut g = game();
        g.avatar.min_scale = Some(AvatarScales {
            height: 0.9,
            width: 0.7,
            head: 0.95,
            body_type: 0.0,
            proportion: 0.0,
        });

        assert_eq!(
            body(&g, &lock()),
            json!({ "universeAvatarMinScales": {
                "height": 0.9,
                "width": 0.7,
                "head": 0.95,
                "bodyType": 0.0,
                "proportion": 0.0,
            }})
        );
    }

    /// A price without `isForSale` is a price Roblox ignores, and
    /// `isForSale` without a price is a paid experience that is free by
    /// accident. They travel together.
    #[test]
    fn a_paid_experience_sends_both_the_flag_and_the_price() {
        let mut g = game();
        g.paid_access = Some(PaidAccess::Paid { price: 25 });

        assert_eq!(body(&g, &lock()), json!({ "isForSale": true, "price": 25 }));
    }

    /// `free` is an instruction, not an absence: it turns paid access off.
    /// No price goes with it, because there is nothing to charge.
    #[test]
    fn free_sends_the_flag_and_no_price() {
        let mut g = game();
        g.paid_access = Some(PaidAccess::Free);

        assert_eq!(body(&g, &lock()), json!({ "isForSale": false }));
    }

    /// Omitting the table entirely means "not managed", which is a third
    /// state and has to stay distinct from `free`.
    #[test]
    fn an_absent_paid_access_table_sends_nothing() {
        let mut g = game();
        g.paid_access = None;
        g.genre = None;

        assert!(plan_for(&g, &lock()).universe_legacy_patch.is_none());
    }

    #[test]
    fn genres_map_to_their_legacy_integers() {
        let mut g = game();
        g.genre = Some(Genre::All);
        assert_eq!(body(&g, &lock()), json!({ "genre": 0 }));

        g.genre = Some(Genre::WildWest);
        assert_eq!(body(&g, &lock()), json!({ "genre": 14 }));
    }

    /// Every legacy integer Roblox documents round-trips. A mapping that
    /// was wrong in one direction only would otherwise survive a pull and
    /// a sync, and change the setting on the third run.
    #[test]
    fn every_legacy_mapping_round_trips() {
        for v in 1..=3 {
            assert_eq!(
                AvatarType::from_legacy(v).map(AvatarType::to_legacy),
                Some(v)
            );
        }
        for v in 1..=2 {
            assert_eq!(
                AnimationType::from_legacy(v).map(AnimationType::to_legacy),
                Some(v)
            );
            assert_eq!(
                CollisionType::from_legacy(v).map(CollisionType::to_legacy),
                Some(v)
            );
            assert_eq!(
                JointPositioningType::from_legacy(v).map(JointPositioningType::to_legacy),
                Some(v)
            );
        }
        for v in 0..=14 {
            assert_eq!(Genre::from_legacy(v).map(Genre::to_legacy), Some(v));
        }
    }

    /// An integer Roblox has not defined is not silently coerced to the
    /// first variant: a pull that met one would otherwise write a wrong
    /// value into the config, and a sync would then apply it.
    #[test]
    fn an_unknown_legacy_integer_is_rejected() {
        assert!(AvatarType::from_legacy(0).is_none());
        assert!(AvatarType::from_legacy(4).is_none());
        assert!(AnimationType::from_legacy(3).is_none());
        assert!(Genre::from_legacy(15).is_none());
    }

    /// These are cookie-only settings. A plan that carried them without
    /// saying so would be refused at apply time with a confusing message.
    #[test]
    fn the_new_fields_make_the_plan_need_a_cookie() {
        let mut g = game();
        g.permissions = Some(perms());

        assert!(plan_for(&g, &lock()).needs_cookie());
    }

    // -------------------------------------------------------------------
    // Avatar slot overrides
    // -------------------------------------------------------------------

    fn choice() -> AssetOverride {
        AssetOverride::PlayerChoice(crate::config::PlayerChoiceMarker::PlayerChoice)
    }

    fn slots() -> AssetOverrides {
        let c = choice();
        AssetOverrides {
            face: c,
            head: c,
            torso: c,
            left_arm: c,
            right_arm: c,
            left_leg: c,
            right_leg: c,
            t_shirt: c,
            shirt: c,
            pants: c,
        }
    }

    /// The `assetTypeID` numbers are Roblox's global asset-type numbering
    /// and are neither contiguous nor in the order the slots read: `Head`
    /// is 17 while `Torso` is 27, and `TShirt` is 2 while `Shirt` is 11.
    /// Nothing but an assertion keeps that table right.
    #[test]
    fn every_slot_maps_to_its_roblox_asset_type_id() {
        let mut g = game();
        g.avatar.asset_overrides = Some(slots());

        let sent = body(&g, &lock());
        let array = sent["universeAvatarAssetOverrides"].as_array().unwrap();

        let ids: Vec<i64> = array
            .iter()
            .map(|slot| slot["assetTypeID"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, vec![18, 17, 27, 29, 28, 30, 31, 2, 11, 12]);
    }

    /// `player_choice` is `isPlayerChoice: true` with a zero id, and an
    /// override is the reverse. Sending an id alongside `isPlayerChoice:
    /// true` would be asking for two different things at once.
    #[test]
    fn a_forced_asset_and_a_player_choice_slot_differ_on_both_fields() {
        let mut g = game();
        g.avatar.asset_overrides = Some(AssetOverrides {
            pants: AssetOverride::Asset(12345),
            ..slots()
        });

        let sent = body(&g, &lock());
        let array = sent["universeAvatarAssetOverrides"].as_array().unwrap();

        let pants = array
            .iter()
            .find(|slot| slot["assetTypeID"] == 12)
            .expect("the pants slot");
        assert_eq!(pants["isPlayerChoice"], json!(false));
        assert_eq!(pants["assetID"], json!(12345));

        let shirt = array
            .iter()
            .find(|slot| slot["assetTypeID"] == 11)
            .expect("the shirt slot");
        assert_eq!(shirt["isPlayerChoice"], json!(true));
        assert_eq!(shirt["assetID"], json!(0));
    }

    /// All ten slots always travel, because Roblox replaces the array
    /// rather than merging into it.
    #[test]
    fn all_ten_slots_are_always_sent() {
        let mut g = game();
        g.avatar.asset_overrides = Some(slots());

        let sent = body(&g, &lock());
        assert_eq!(
            sent["universeAvatarAssetOverrides"]
                .as_array()
                .unwrap()
                .len(),
            10
        );
    }

    #[test]
    fn slots_matching_the_lock_send_nothing() {
        let mut g = game();
        g.avatar.asset_overrides = Some(slots());

        assert!(plan_for(&g, &config_to_lock(&g))
            .universe_legacy_patch
            .is_none());
    }

    // -------------------------------------------------------------------
    // engineAvatarSettings
    // -------------------------------------------------------------------

    /// `build_plan` with an `engine_avatar_settings` file on disk.
    ///
    /// Needs a real directory, unlike every other test here, because the
    /// whole point of the field is that the document comes from a file.
    fn plan_with_engine_settings(
        lock: &GameLock,
        contents: &str,
    ) -> (tempfile::TempDir, Result<SyncPlan>) {
        plan_with_engine_file(lock, "avatar.json", contents)
    }

    fn plan_with_engine_file(
        lock: &GameLock,
        name: &str,
        contents: &str,
    ) -> (tempfile::TempDir, Result<SyncPlan>) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(name), contents).expect("write");
        let mut g = game();
        g.engine_avatar_settings = Some(PathBuf::from(name));
        let plan = build_plan(
            &g,
            &MediaConfig::default(),
            lock,
            &MediaLockfile::default(),
            dir.path(),
        );
        (dir, plan)
    }

    /// The document this sends, whatever format it came from.
    fn engine_document(plan: Result<SyncPlan>) -> String {
        plan.unwrap().universe_legacy_patch.expect("a patch").body["engineAvatarSettings"]
            .as_str()
            .expect("a JSON string")
            .to_string()
    }

    /// The field is typed as a JSON *string* on the API, so the document is
    /// serialised and handed over as text rather than nested as an object.
    #[test]
    fn the_document_is_sent_as_a_json_string() {
        let (_dir, plan) =
            plan_with_engine_settings(&lock(), r#"{"AvatarRules":{"AvatarType":1}}"#);
        let patch = plan.unwrap().universe_legacy_patch.expect("a patch");

        let sent = patch.body["engineAvatarSettings"]
            .as_str()
            .expect("a string, not an object");
        assert_eq!(sent, r#"{"AvatarRules":{"AvatarType":1}}"#);
    }

    /// Reindenting the file is not a change to re-send. The hash is of the
    /// canonical serialisation, so whitespace never reaches the wire.
    #[test]
    fn reformatting_the_file_is_not_a_change() {
        let (_dir, plan) = plan_with_engine_settings(&lock(), r#"{"a":1}"#);
        let hash = plan
            .unwrap()
            .universe_legacy_patch
            .expect("a patch")
            .engine_avatar_settings_hash
            .expect("a hash");

        let mut locked = lock();
        locked.engine_avatar_settings_hash = Some(hash);

        let (_dir2, plan2) = plan_with_engine_settings(&locked, "{\n    \"a\" :   1\n}\n");
        assert!(
            plan2.unwrap().universe_legacy_patch.is_none(),
            "the same document, formatted differently, must not re-send"
        );
    }

    /// A real edit does re-send.
    #[test]
    fn a_changed_document_is_sent() {
        let (_dir, plan) = plan_with_engine_settings(&lock(), r#"{"a":1}"#);
        let hash = plan
            .unwrap()
            .universe_legacy_patch
            .expect("a patch")
            .engine_avatar_settings_hash
            .expect("a hash");

        let mut locked = lock();
        locked.engine_avatar_settings_hash = Some(hash);

        let (_dir2, plan2) = plan_with_engine_settings(&locked, r#"{"a":2}"#);
        assert!(plan2.unwrap().universe_legacy_patch.is_some());
    }

    /// Caught locally, before a cookie-authenticated write that would come
    /// back as an opaque 400.
    #[test]
    fn a_file_that_is_not_json_is_refused_by_name() {
        let (_dir, plan) = plan_with_engine_settings(&lock(), "AvatarType = 1");
        let error = format!("{:#}", plan.unwrap_err());

        assert!(error.contains("avatar.json"), "{error}");
        assert!(error.contains("not valid JSON"), "{error}");
    }

    #[test]
    fn a_missing_file_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut g = game();
        g.engine_avatar_settings = Some(PathBuf::from("nowhere.json"));

        let error = build_plan(
            &g,
            &MediaConfig::default(),
            &lock(),
            &MediaLockfile::default(),
            dir.path(),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("nowhere.json"));
    }

    /// `{}` is how Roblox documents clearing the settings, so it has to
    /// reach the wire rather than being treated as "nothing to send".
    #[test]
    fn an_empty_object_is_a_real_instruction() {
        let (_dir, plan) = plan_with_engine_settings(&lock(), "{}");
        let patch = plan.unwrap().universe_legacy_patch.expect("a patch");

        assert_eq!(patch.body["engineAvatarSettings"], json!("{}"));
    }

    /// A TOML document reaches Roblox as the JSON its field is typed as.
    /// The point of accepting TOML is that the project keeps one config
    /// language, not that Roblox learns a second one.
    #[test]
    fn a_toml_document_is_sent_as_json() {
        let (_dir, plan) =
            plan_with_engine_file(&lock(), "avatar.toml", "[AvatarRules]\nAvatarType = 1\n");

        assert_eq!(engine_document(plan), r#"{"AvatarRules":{"AvatarType":1}}"#);
    }

    /// The two formats are interchangeable, which is the property that
    /// makes converting a file a no-op rather than a spurious re-send.
    #[test]
    fn the_same_document_hashes_the_same_in_either_format() {
        let (_dir, json_plan) = plan_with_engine_file(
            &lock(),
            "avatar.json",
            r#"{"AvatarRules": {"AvatarType": 1}, "version": 1}"#,
        );
        let (_dir2, toml_plan) = plan_with_engine_file(
            &lock(),
            "avatar.toml",
            "version = 1\n\n[AvatarRules]\nAvatarType = 1\n",
        );

        assert_eq!(engine_document(json_plan), engine_document(toml_plan));
    }

    /// Numbers have to survive the conversion with their type intact:
    /// `AvatarType = 1` is an integer to Roblox, and a `1.0` would be a
    /// different value.
    #[test]
    fn toml_integers_and_floats_keep_their_types() {
        let (_dir, plan) = plan_with_engine_file(
            &lock(),
            "avatar.toml",
            "mode = 1\nscale = 1.5\nenabled = true\nbounds = [0, 0, 0]\n",
        );
        let sent = engine_document(plan);

        assert!(sent.contains(r#""mode":1"#), "{sent}");
        assert!(sent.contains(r#""scale":1.5"#), "{sent}");
        assert!(sent.contains(r#""enabled":true"#), "{sent}");
        assert!(sent.contains(r#""bounds":[0,0,0]"#), "{sent}");
    }

    #[test]
    fn a_file_that_is_not_toml_is_refused_by_name() {
        let (_dir, plan) = plan_with_engine_file(&lock(), "avatar.toml", "{\"not\": \"toml\"}");
        let error = format!("{:#}", plan.unwrap_err());

        assert!(error.contains("avatar.toml"), "{error}");
        assert!(error.contains("not valid TOML"), "{error}");
    }

    /// Sniffing the content instead would let a `.txt` through, and turn a
    /// typo in the path into a silent success.
    #[test]
    fn an_unrecognised_extension_is_refused_rather_than_sniffed() {
        let (_dir, plan) = plan_with_engine_file(&lock(), "avatar.yaml", "{}");
        let error = format!("{:#}", plan.unwrap_err());

        assert!(error.contains(".toml or .json"), "{error}");
        assert!(error.contains("avatar.yaml"), "{error}");
    }
}
