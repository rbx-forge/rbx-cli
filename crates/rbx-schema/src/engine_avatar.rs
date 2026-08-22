//! A schema for `engineAvatarSettings`, the modern avatar rules.
//!
//! # Why this one is different from every other schema here
//!
//! The rest of this crate derives schemas from the serde models the CLI parses
//! with, and the module documentation gives the reason: a hand-written schema
//! is a second description of the same format, and the two drift the moment
//! somebody adds a field.
//!
//! That argument does not apply here, and it is worth saying why rather than
//! quietly making an exception. There is no model to derive from: `rbx meta`
//! reads the avatar settings file, checks it parses, and sends it: it does not
//! model the contents, deliberately, because Roblox types the field as an
//! opaque JSON string and annotates it *"experimental which may be changed or
//! removed in future"*. So this is not a second description competing with a
//! first. It is the only one, and Roblox publishes none.
//!
//! The structs below therefore exist **for the schema alone**. Nothing
//! deserialises into them, nothing in the shipped binary references them, and
//! they live in this dev-only crate so that stays true.
//!
//! # What that changes about the risk
//!
//! A schema that drifts from Roblox costs an autocomplete suggestion that did
//! not appear. It cannot cost a rejected valid file, because
//! `additionalProperties` stays open everywhere: the rule this crate already
//! holds itself to, and the one that matters most here: a key Roblox adds
//! tomorrow is a key `rbx meta` already passes through today, so the schema
//! must not paint it red.
//!
//! # Where the field names come from
//!
//! Roblox documents none of this. The key names and the meaning of each
//! numeric mode are taken from the worked example in
//! [Phoenix-CLI](https://github.com/PhoenixEntertainment/Phoenix-CLI)'s
//! `Test/ConfigToFile.luau`, which spells out every key with a comment. That is
//! a community-derived source and it is cited so the next reader can judge it
//! rather than trust it.
//!
//! # Why the modes are plain integers
//!
//! Kept as integers after a live probe on 2026-08-17 rather than on taste. Two
//! documents were sent to a real universe, one with `AnimationClipsMode = 1` and
//! one with `"PlayerChoice"`; both returned 200, which proved nothing, because
//! the endpoint treats the whole document as an opaque string and validates
//! none of it. Roblox echoed nothing back and the setting was not visible in
//! the Creator Hub, so the experiment could not be finished.
//!
//! That absence cuts both ways, and it is why the integers stay: there is no
//! evidence a string form works, and no example of one existing. The only known
//! good document: Phoenix-CLI's, itself derived from what the Roblox dashboard
//! produces: is integers throughout. Widening to accept strings would trade a
//! real check (somebody writing `AvatarType = "R15"`, the name instead of the
//! number) for coverage of a form nothing attests to.
//!
//! Each `*Mode` field below is a small integer with a fixed set of meanings,
//! and modelling one as a Rust enum would emit a closed `enum` in the schema.
//! That would be stricter than the tool, which sends whatever the file holds.
//! The meanings live in the doc comments instead, where an editor shows them on
//! hover: guidance without a gate.

// Nothing reads these fields, and nothing should: they exist so the derive can
// see their names, types and doc comments. That is the whole contribution of
// this module, and `dead_code` firing on all two hundred of them is the
// compiler correctly describing a file whose point is to be described rather
// than executed.
#![allow(dead_code)]

use schemars::JsonSchema;

/// The document `game.engine_avatar_settings` points at.
///
/// Every field is optional: Roblox fills in what the document leaves out, and
/// a partial document is a normal thing to write.
#[derive(JsonSchema)]
#[schemars(rename_all = "PascalCase")]
pub struct EngineAvatarSettings {
    pub avatar_rules: Option<AvatarRules>,
    pub avatar_animation_rules: Option<AvatarAnimationRules>,
    pub avatar_clothing_rules: Option<AvatarClothingRules>,
    pub avatar_accessory_rules: Option<AvatarAccessoryRules>,
    pub avatar_collision_rules: Option<AvatarCollisionRules>,
    pub avatar_body_rules: Option<AvatarBodyRules>,

    /// Schema version of the document itself. `1` is the only value seen so
    /// far.
    #[schemars(rename = "version")]
    pub version: Option<i64>,
}

/// Which rig players get.
#[derive(JsonSchema)]
#[schemars(rename_all = "PascalCase")]
pub struct AvatarRules {
    /// `0` = R6, `1` = R15, `2` = both.
    ///
    /// **Not the same numbering as `[game.avatar] type`**, which is Roblox's
    /// older `universeAvatarType` and runs `1` = R6, `2` = player choice,
    /// `3` = R15. The two fields describe the same idea through different
    /// endpoints and disagree on the integers; setting one from the other's
    /// value silently picks the wrong rig.
    pub avatar_type: Option<i64>,
}

/// Which animations play, and whether players may bring their own.
#[derive(JsonSchema)]
#[schemars(rename_all = "PascalCase")]
pub struct AvatarAnimationRules {
    /// `0` = player choice, `1` = the custom clips named in this table.
    pub animation_clips_mode: Option<i64>,
    /// `0` = player choice, `1` = standard R15, `2` = standard R6.
    pub animation_packs_mode: Option<i64>,

    pub custom_climb_animation_enabled: Option<bool>,
    pub custom_climb_animation_id: Option<i64>,
    pub custom_fall_animation_enabled: Option<bool>,
    pub custom_fall_animation_id: Option<i64>,
    pub custom_idle_animation_enabled: Option<bool>,
    pub custom_idle_animation_id: Option<i64>,
    /// First alternate idle. Roblox cycles between the alternates.
    pub custom_idle_alt1_animation_enabled: Option<bool>,
    pub custom_idle_alt1_animation_id: Option<i64>,
    pub custom_idle_alt2_animation_enabled: Option<bool>,
    pub custom_idle_alt2_animation_id: Option<i64>,
    pub custom_jump_animation_enabled: Option<bool>,
    pub custom_jump_animation_id: Option<i64>,
    pub custom_run_animation_enabled: Option<bool>,
    pub custom_run_animation_id: Option<i64>,
    pub custom_swim_animation_enabled: Option<bool>,
    pub custom_swim_animation_id: Option<i64>,
    pub custom_swim_idle_animation_enabled: Option<bool>,
    pub custom_swim_idle_animation_id: Option<i64>,
    pub custom_walk_animation_enabled: Option<bool>,
    pub custom_walk_animation_id: Option<i64>,
}

/// What players wear, and how far it may stick out.
#[derive(JsonSchema)]
#[schemars(rename_all = "PascalCase")]
pub struct AvatarClothingRules {
    /// `0` = player choice, `1` = replace with the custom clothing below.
    pub custom_clothing_mode: Option<i64>,
    /// `0` = no limit bounds, `1` = apply `LimitBounds`.
    pub clothing_mode: Option<i64>,
    /// Padding around the avatar, as a percentage, beyond which clothing is
    /// removed or scaled. Three numbers.
    pub limit_bounds: Option<[f64; 3]>,

    pub custom_classic_pants_accessory_enabled: Option<bool>,
    pub custom_classic_pants_accessory_id: Option<i64>,
    pub custom_classic_shirts_accessory_enabled: Option<bool>,
    pub custom_classic_shirts_accessory_id: Option<i64>,
    pub custom_classic_t_shirts_accessory_enabled: Option<bool>,
    pub custom_classic_t_shirts_accessory_id: Option<i64>,
    pub custom_dress_skirt_accessory_enabled: Option<bool>,
    pub custom_dress_skirt_accessory_id: Option<i64>,
    pub custom_jacket_accessory_enabled: Option<bool>,
    pub custom_jacket_accessory_id: Option<i64>,
    pub custom_left_shoes_accessory_enabled: Option<bool>,
    pub custom_left_shoes_accessory_id: Option<i64>,
    pub custom_pants_accessory_enabled: Option<bool>,
    pub custom_pants_accessory_id: Option<i64>,
    pub custom_right_shoes_accessory_enabled: Option<bool>,
    pub custom_right_shoes_accessory_id: Option<i64>,
    pub custom_shirt_accessory_enabled: Option<bool>,
    pub custom_shirt_accessory_id: Option<i64>,
    pub custom_shorts_accessory_enabled: Option<bool>,
    pub custom_shorts_accessory_id: Option<i64>,
    pub custom_sweater_accessory_enabled: Option<bool>,
    pub custom_sweater_accessory_id: Option<i64>,
    pub custom_t_shirt_accessory_enabled: Option<bool>,
    pub custom_t_shirt_accessory_id: Option<i64>,
}

/// Accessories: which ones, and what happens to the oversized ones.
#[derive(JsonSchema)]
#[schemars(rename_all = "PascalCase")]
pub struct AvatarAccessoryRules {
    /// Whether accessory sounds play.
    pub enable_sound: Option<bool>,
    /// Whether accessory particle effects play.
    #[schemars(rename = "EnableVFX")]
    pub enable_vfx: Option<bool>,

    /// `0` = player choice, `1` = replace with the custom accessories below.
    pub custom_accessory_mode: Option<i64>,
    /// `0` = no limit bounds, `1` = apply `LimitBounds`.
    pub accessory_mode: Option<i64>,
    /// What happens to an accessory past the bounds: `0` = scaled down,
    /// `1` = removed.
    pub limit_method: Option<i64>,
    /// Padding around the avatar, as a percentage. Three numbers.
    pub limit_bounds: Option<[f64; 3]>,

    pub custom_back_accessory_enabled: Option<bool>,
    pub custom_back_accessory_id: Option<i64>,
    pub custom_face_accessory_enabled: Option<bool>,
    pub custom_face_accessory_id: Option<i64>,
    pub custom_front_accessory_enabled: Option<bool>,
    pub custom_front_accessory_id: Option<i64>,
    pub custom_hair_accessory_enabled: Option<bool>,
    pub custom_hair_accessory_id: Option<i64>,
    pub custom_head_accessory_enabled: Option<bool>,
    pub custom_head_accessory_id: Option<i64>,
    pub custom_neck_accessory_enabled: Option<bool>,
    pub custom_neck_accessory_id: Option<i64>,
    pub custom_shoulder_accessory_enabled: Option<bool>,
    pub custom_shoulder_accessory_id: Option<i64>,
    pub custom_waist_accessory_enabled: Option<bool>,
    pub custom_waist_accessory_id: Option<i64>,
}

/// How avatars collide with the world and with each other.
#[derive(JsonSchema)]
#[schemars(rename_all = "PascalCase")]
pub struct AvatarCollisionRules {
    /// `0` = default (outer box), `1` = single collider, `2` = legacy (inner
    /// box).
    ///
    /// A third numbering for collisions, and again not the one
    /// `[game.avatar] collision` uses: that field is the older
    /// `universeCollisionType`, where `1` = inner box and `2` = outer box.
    pub collision_mode: Option<i64>,
    /// What touch events fire against: `0` = avatar geometry, `1` = colliders.
    pub hit_and_touch_detection_mode: Option<i64>,
    /// Purpose undocumented; observed as `1` in every example seen. Left in the
    /// schema because omitting a key people will find in their own file is how
    /// a schema teaches them to ignore it.
    pub legacy_collision_mode: Option<i64>,
    /// Size of the collider used when `CollisionMode` is single collider.
    /// Three numbers.
    pub single_collider_size: Option<[f64; 3]>,
}

/// Body shape, scale, and which parts are forced.
///
/// The `Custom*Scale` fields are `[min, max]` pairs, not single values.
#[derive(JsonSchema)]
#[schemars(rename_all = "PascalCase")]
pub struct AvatarBodyRules {
    /// `0` = player choice, `1` = forced non-uniform scale.
    pub build_mode: Option<i64>,
    /// `0` = player choice, `1` = forced height.
    pub scale_mode: Option<i64>,
    /// `0` = player choice, `1` = the custom body parts below.
    pub appearance_mode: Option<i64>,

    /// Min and max "rthro" scale, each `0` to `1`.
    pub custom_body_type_scale: Option<[f64; 2]>,
    /// Min and max head scale, each `0` to `1`.
    pub custom_head_scale: Option<[f64; 2]>,
    /// Min and max height scale, each `0` to `1`.
    pub custom_height_scale: Option<[f64; 2]>,
    /// Min and max proportions scale, each `0` to `1`.
    pub custom_proportions_scale: Option<[f64; 2]>,
    /// Min and max width scale, each `0` to `1`.
    pub custom_width_scale: Option<[f64; 2]>,
    /// Min and max height in studs, used when `ScaleMode` is forced height.
    pub custom_height: Option<[f64; 2]>,

    /// Whether the player keeps their own head when body parts are forced.
    pub keep_player_head: Option<bool>,
    pub custom_body_type: Option<i64>,
    pub custom_body_bundle_id: Option<i64>,

    pub custom_eyebrow_enabled: Option<bool>,
    pub custom_eyebrow_id: Option<i64>,
    pub custom_eyelash_enabled: Option<bool>,
    pub custom_eyelash_id: Option<i64>,
    pub custom_face_enabled: Option<bool>,
    pub custom_face_id: Option<i64>,
    pub custom_head_enabled: Option<bool>,
    pub custom_head_id: Option<i64>,
    pub custom_left_arm_enabled: Option<bool>,
    pub custom_left_arm_id: Option<i64>,
    pub custom_left_leg_enabled: Option<bool>,
    pub custom_left_leg_id: Option<i64>,
    pub custom_mood_enabled: Option<bool>,
    pub custom_mood_id: Option<i64>,
    pub custom_right_arm_enabled: Option<bool>,
    pub custom_right_arm_id: Option<i64>,
    pub custom_right_leg_enabled: Option<bool>,
    pub custom_right_leg_id: Option<i64>,
    pub custom_torso_enabled: Option<bool>,
    pub custom_torso_id: Option<i64>,
}
