//! The avatar vocabulary: the enums Roblox spells one way and a config file
//! another, plus the shapes that carry them.
//!
//! Every one of these carries its own parse and render, because a value that
//! round-trips through the config file has to come back spelled exactly as it
//! went in.

use serde::{Deserialize, Serialize};

/// Which rig players get.
///
/// The API takes an integer, and the integers are not in the order the names
/// suggest: `PlayerChoice` sits between the two rigs. Hence the explicit
/// mapping rather than a `#[repr]` cast.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarType {
    R6,
    PlayerChoice,
    R15,
}

impl AvatarType {
    pub fn to_legacy(self) -> u8 {
        match self {
            AvatarType::R6 => 1,
            AvatarType::PlayerChoice => 2,
            AvatarType::R15 => 3,
        }
    }

    pub fn from_legacy(value: u8) -> Option<Self> {
        match value {
            1 => Some(AvatarType::R6),
            2 => Some(AvatarType::PlayerChoice),
            3 => Some(AvatarType::R15),
            _ => None,
        }
    }

    /// The name `GET /v1/universes/{id}/configuration` answers with.
    ///
    /// Measured against a live universe on 2026-08-17: the v1 read returns
    /// `"MorphToR15"` where the v2 write takes `3`. Both spellings are in the
    /// vendored spec (the integers as the request type, the names inside the
    /// response field's own description) and this tool has to speak both.
    pub fn from_api_name(name: &str) -> Option<Self> {
        match name {
            "MorphToR6" => Some(AvatarType::R6),
            "PlayerChoice" => Some(AvatarType::PlayerChoice),
            "MorphToR15" => Some(AvatarType::R15),
            _ => None,
        }
    }
}

/// Whether players keep their own animations.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationType {
    Standard,
    PlayerChoice,
}

impl AnimationType {
    pub fn to_legacy(self) -> u8 {
        match self {
            AnimationType::Standard => 1,
            AnimationType::PlayerChoice => 2,
        }
    }

    pub fn from_legacy(value: u8) -> Option<Self> {
        match value {
            1 => Some(AnimationType::Standard),
            2 => Some(AnimationType::PlayerChoice),
            _ => None,
        }
    }

    pub fn from_api_name(name: &str) -> Option<Self> {
        match name {
            "Standard" => Some(AnimationType::Standard),
            "PlayerChoice" => Some(AnimationType::PlayerChoice),
            _ => None,
        }
    }
}

/// The shape of an avatar's collision box.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CollisionType {
    InnerBox,
    OuterBox,
}

impl CollisionType {
    pub fn to_legacy(self) -> u8 {
        match self {
            CollisionType::InnerBox => 1,
            CollisionType::OuterBox => 2,
        }
    }

    pub fn from_legacy(value: u8) -> Option<Self> {
        match value {
            1 => Some(CollisionType::InnerBox),
            2 => Some(CollisionType::OuterBox),
            _ => None,
        }
    }

    pub fn from_api_name(name: &str) -> Option<Self> {
        match name {
            "InnerBox" => Some(CollisionType::InnerBox),
            "OuterBox" => Some(CollisionType::OuterBox),
            _ => None,
        }
    }
}

/// How avatar joints are positioned.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JointPositioningType {
    Standard,
    ArtistIntent,
}

impl JointPositioningType {
    pub fn to_legacy(self) -> u8 {
        match self {
            JointPositioningType::Standard => 1,
            JointPositioningType::ArtistIntent => 2,
        }
    }

    pub fn from_legacy(value: u8) -> Option<Self> {
        match value {
            1 => Some(JointPositioningType::Standard),
            2 => Some(JointPositioningType::ArtistIntent),
            _ => None,
        }
    }

    pub fn from_api_name(name: &str) -> Option<Self> {
        match name {
            "Standard" => Some(JointPositioningType::Standard),
            "ArtistIntent" => Some(JointPositioningType::ArtistIntent),
            _ => None,
        }
    }
}

/// One end of the avatar scale range.
///
/// Every field is required rather than optional, and that is the point: Roblox
/// takes the scales as a single object, so a table with three of the five keys
/// would send an object Roblox reads as "the other two are zero". Requiring
/// all five makes a half-written table a load error instead of a silently
/// squashed avatar.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct AvatarScales {
    pub height: f64,
    pub width: f64,
    pub head: f64,
    pub body_type: f64,
    pub proportion: f64,
}

/// Avatar rules, as a group.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Avatar {
    /// `r6`, `r15`, or `player_choice`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<AvatarType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub animation: Option<AnimationType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub collision: Option<CollisionType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub joint_positioning: Option<JointPositioningType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_scale: Option<AvatarScales>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_scale: Option<AvatarScales>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_overrides: Option<AssetOverrides>,
}

impl Avatar {
    pub fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.animation.is_none()
            && self.collision.is_none()
            && self.joint_positioning.is_none()
            && self.min_scale.is_none()
            && self.max_scale.is_none()
            && self.asset_overrides.is_none()
    }
}

/// What one avatar slot is forced to, or `player_choice` to leave it alone.
///
/// Written in TOML as either an asset id or the literal string
/// `"player_choice"`:
///
/// ```toml
/// [game.avatar.asset_overrides]
/// pants = 12345678
/// shirt = "player_choice"
/// ```
///
/// An untagged enum rather than a bare `Option<u64>` where `None` means player
/// choice: absent and "explicitly the player's choice" have to stay different,
/// because the table requires every slot (see [`AssetOverrides`]) and there is
/// no third state left to spell "not managed" with.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AssetOverride {
    /// A specific asset every player wears in this slot.
    Asset(u64),
    /// The player keeps whatever they are wearing.
    PlayerChoice(PlayerChoiceMarker),
}

/// The one string [`AssetOverride`] accepts in place of an id.
///
/// A single-variant enum rather than a `String`, so `"playerchoice"` is a load
/// error naming the valid value instead of a silently ignored slot.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum PlayerChoiceMarker {
    #[serde(rename = "player_choice")]
    PlayerChoice,
}

impl AssetOverride {
    /// `(isPlayerChoice, assetID)` as the API wants them.
    pub fn to_legacy(self) -> (bool, u64) {
        match self {
            AssetOverride::Asset(id) => (false, id),
            AssetOverride::PlayerChoice(_) => (true, 0),
        }
    }
}

/// The ten avatar slots Roblox lets an experience override.
///
/// **Every slot is required**, for the third time in this file and for the
/// same reason: Roblox takes `universeAvatarAssetOverrides` as one array and
/// replaces it wholesale, so a table naming three slots is a request to reset
/// the other seven. Since there is no endpoint that returns the array either,
/// nothing could fill in the missing seven, not Roblox, not the lockfile.
/// Requiring all ten makes a partial table a load error rather than a silent
/// reset.
///
/// The `assetTypeID` each slot maps to is Roblox's global asset-type
/// numbering, which is neither contiguous nor ordered the way this list reads:
/// `Head` is 17 and `Torso` is 27. Hence the explicit table in
/// [`AssetOverrides::to_legacy`].
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct AssetOverrides {
    pub face: AssetOverride,
    pub head: AssetOverride,
    pub torso: AssetOverride,
    pub left_arm: AssetOverride,
    pub right_arm: AssetOverride,
    pub left_leg: AssetOverride,
    pub right_leg: AssetOverride,
    pub t_shirt: AssetOverride,
    pub shirt: AssetOverride,
    pub pants: AssetOverride,
}

impl AssetOverrides {
    /// The slots paired with their Roblox `assetTypeID`, in the order the API
    /// documentation lists them.
    pub fn to_legacy(&self) -> Vec<(u32, bool, u64)> {
        [
            (18, self.face),
            (17, self.head),
            (27, self.torso),
            (29, self.left_arm),
            (28, self.right_arm),
            (30, self.left_leg),
            (31, self.right_leg),
            (2, self.t_shirt),
            (11, self.shirt),
            (12, self.pants),
        ]
        .into_iter()
        .map(|(type_id, slot)| {
            let (is_player_choice, asset_id) = slot.to_legacy();
            (type_id, is_player_choice, asset_id)
        })
        .collect()
    }
}
