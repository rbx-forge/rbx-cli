//! The small closed vocabularies of a universe: who may do what, who pays,
//! what it is, where it runs.
//!
//! One enum each, with the same parse-and-render pair the avatar types carry
//! and for the same reason.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The four flags under `permissions` on the legacy universe configuration.
///
/// Worth managing in a versioned file more than most settings here: each one
/// widens what code outside the experience is allowed to do to it, they are
/// changed rarely and by hand, and nothing inside the experience shows that one
/// flipped. A diff is the only way anybody finds out.
///
/// **All four fields are required**, and that is a consequence of the API
/// rather than a style choice. Roblox takes `permissions` as a single object on
/// the PATCH body, so sending one flag means sending all four, and it exposes
/// no GET that returns them: the v1 configuration response has no `permissions`
/// field, and the v2 endpoint answers to PATCH only. There is therefore no way
/// to fill in the flags a partial table left out, not from Roblox and not from
/// a first-run lockfile. Requiring all four makes a half-written table a load
/// error instead of a write whose result nobody can predict.
///
/// The same absence of a GET means `pull` cannot adopt these: see
/// `commands::pull`. The lockfile records what this tool last wrote, which is
/// what `check` and `sync` compare against.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Permissions {
    /// Whether another experience may teleport players into this one.
    pub third_party_teleport: bool,

    /// Whether this experience may load assets it does not own.
    pub third_party_asset: bool,

    /// Whether this experience may prompt purchases for another creator's
    /// products.
    pub third_party_purchase: bool,

    /// Whether client-initiated teleports are allowed.
    pub client_teleport: bool,
}

/// Whether players pay to enter.
///
/// Tagged like [`ServerFill`] rather than modelled as a bare `Option<u64>`,
/// because "not for sale" and "not managed by this file" are different states
/// and a price of zero means neither. Omitting the table leaves paid access
/// alone; `mode = "free"` actively turns it off.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum PaidAccess {
    /// Free to play.
    Free,
    /// Sold for `price` Robux.
    Paid { price: u64 },
}

impl PaidAccess {
    pub fn is_for_sale(&self) -> bool {
        matches!(self, PaidAccess::Paid { .. })
    }

    pub fn price(&self) -> Option<u64> {
        match self {
            PaidAccess::Paid { price } => Some(*price),
            PaidAccess::Free => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Genre
// ---------------------------------------------------------------------------

/// The legacy genre field.
///
/// Legacy in Roblox's own sense: the discovery system has moved to experience
/// types and tags, and this list has not changed in years. It is here because
/// the field is still on the configuration endpoint and still round-trips, so
/// a config that does not model it silently loses whatever it was set to on
/// the next `pull`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Genre {
    All,
    Tutorial,
    Scary,
    TownAndCity,
    War,
    Funny,
    Fantasy,
    Adventure,
    SciFi,
    Pirate,
    Fps,
    Rpg,
    Sports,
    Ninja,
    WildWest,
}

impl Genre {
    pub fn to_legacy(self) -> u8 {
        match self {
            Genre::All => 0,
            Genre::Tutorial => 1,
            Genre::Scary => 2,
            Genre::TownAndCity => 3,
            Genre::War => 4,
            Genre::Funny => 5,
            Genre::Fantasy => 6,
            Genre::Adventure => 7,
            Genre::SciFi => 8,
            Genre::Pirate => 9,
            Genre::Fps => 10,
            Genre::Rpg => 11,
            Genre::Sports => 12,
            Genre::Ninja => 13,
            Genre::WildWest => 14,
        }
    }

    /// The name the v1 read answers with. `FPS` and `RPG` keep their
    /// capitalisation; the rest are the variant names.
    pub fn from_api_name(name: &str) -> Option<Self> {
        Some(match name {
            "All" => Genre::All,
            "Tutorial" => Genre::Tutorial,
            "Scary" => Genre::Scary,
            "TownAndCity" => Genre::TownAndCity,
            "War" => Genre::War,
            "Funny" => Genre::Funny,
            "Fantasy" => Genre::Fantasy,
            "Adventure" => Genre::Adventure,
            "SciFi" => Genre::SciFi,
            "Pirate" => Genre::Pirate,
            "FPS" => Genre::Fps,
            "RPG" => Genre::Rpg,
            "Sports" => Genre::Sports,
            "Ninja" => Genre::Ninja,
            "WildWest" => Genre::WildWest,
            _ => return None,
        })
    }

    pub fn from_legacy(value: u8) -> Option<Self> {
        Some(match value {
            0 => Genre::All,
            1 => Genre::Tutorial,
            2 => Genre::Scary,
            3 => Genre::TownAndCity,
            4 => Genre::War,
            5 => Genre::Funny,
            6 => Genre::Fantasy,
            7 => Genre::Adventure,
            8 => Genre::SciFi,
            9 => Genre::Pirate,
            10 => Genre::Fps,
            11 => Genre::Rpg,
            12 => Genre::Sports,
            13 => Genre::Ninja,
            14 => Genre::WildWest,
            _ => return None,
        })
    }
}

/// Who can play the experience: the Creator Hub "Audience" setting.
/// `limited` takes its groups from the `audience` key.
// How the pair goes on the wire: see `Access`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Anyone, and discoverable.
    Public,
    /// Only the groups listed in `audience`. Not discoverable.
    Limited,
    /// Only users with edit permission.
    Private,
}

/// One group a `limited` experience is open to.
// Ordered so a set of them has one spelling, whatever order the config listed
// them in: the lockfile compares sets, not lists.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Audience {
    /// Users granted playtest permission on the experience.
    Playtesters,
    /// The owner's friends, or the group's members ("Community Members") when
    /// a group owns the experience. Roblox sends both as the same value.
    Friends,
}

/// `visibility` and `audience` resolved into the one setting Roblox stores.
///
/// # The wire format, measured
///
/// Roblox stores a list of audience values, read from `GET
/// develop.roblox.com/v1/universes/{id}` and written as `audiences` on the v2
/// configuration PATCH. The values are `Editors = 1`, `PlayTesters = 2`,
/// `Friends = 3`, `Public = 4`, taken from the Creator Hub bundle and checked
/// against a live universe on 2026-10-05:
///
/// - private is `[1]`, public is `[4]`;
/// - limited is any non-empty subset of `{2, 3}`. Roblox adds `1` itself: a
///   universe set to Limited ⟩ Playtesters reads back `[1, 2]`.
///
/// Open Cloud's `Universe.visibility` cannot tell these apart. It still has
/// two values, and that same Limited universe reports `isActive: false`, which
/// is what Open Cloud calls `PRIVATE`. Reading it is how `pull` came to write
/// `private` over a limited experience.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Private,
    Limited(BTreeSet<Audience>),
    Public,
}

const AUDIENCE_EDITORS: u8 = 1;
const AUDIENCE_PLAYTESTERS: u8 = 2;
const AUDIENCE_FRIENDS: u8 = 3;
const AUDIENCE_PUBLIC: u8 = 4;

impl Access {
    /// Join the two config keys. `None` when `visibility` is unset: the
    /// setting is then not managed, whatever `audience` says.
    ///
    /// `audience` is read for `limited` only, so an env overlay can switch a
    /// limited base to `public` without having to unset a key TOML has no way
    /// to unset.
    pub fn from_parts(
        visibility: Option<Visibility>,
        audience: Option<&BTreeSet<Audience>>,
    ) -> Option<Self> {
        Some(match visibility? {
            Visibility::Private => Access::Private,
            Visibility::Public => Access::Public,
            Visibility::Limited => Access::Limited(audience.cloned().unwrap_or_default()),
        })
    }

    /// Split back into the two config keys. `audience` is `None` unless
    /// limited.
    pub fn into_parts(self) -> (Visibility, Option<BTreeSet<Audience>>) {
        match self {
            Access::Private => (Visibility::Private, None),
            Access::Public => (Visibility::Public, None),
            Access::Limited(set) => (Visibility::Limited, Some(set)),
        }
    }

    /// Parse the `audiences` list Roblox returns.
    ///
    /// `None` for a value this build does not know, rather than dropping it:
    /// a fifth audience that got ignored here would make a pull write a
    /// narrower setting than the experience really has.
    pub fn from_audiences(values: &[u8]) -> Option<Self> {
        let mut set = BTreeSet::new();
        let mut public = false;
        for &v in values {
            match v {
                AUDIENCE_EDITORS => {}
                AUDIENCE_PLAYTESTERS => {
                    set.insert(Audience::Playtesters);
                }
                AUDIENCE_FRIENDS => {
                    set.insert(Audience::Friends);
                }
                AUDIENCE_PUBLIC => public = true,
                _ => return None,
            }
        }
        Some(if public {
            Access::Public
        } else if set.is_empty() {
            Access::Private
        } else {
            Access::Limited(set)
        })
    }

    /// The `audiences` list to send, as Creator Hub sends it: `[1]` for
    /// private, `[4]` for public, and the bare subset for limited.
    pub fn to_audiences(&self) -> Vec<u8> {
        match self {
            Access::Private => vec![AUDIENCE_EDITORS],
            Access::Public => vec![AUDIENCE_PUBLIC],
            Access::Limited(set) => set
                .iter()
                .map(|a| match a {
                    Audience::Playtesters => AUDIENCE_PLAYTESTERS,
                    Audience::Friends => AUDIENCE_FRIENDS,
                })
                .collect(),
        }
    }

    /// Whether the universe has to be activated for this setting, or
    /// deactivated.
    ///
    /// Not the same line as public versus private. Creator Hub deactivates
    /// whenever every audience is editors or playtesters, so Limited ⟩
    /// Playtesters is an *inactive* universe and Limited ⟩ Friends an active
    /// one. This mirrors that rule rather than guessing a cleaner one.
    pub fn is_active(&self) -> bool {
        match self {
            Access::Private => false,
            Access::Public => true,
            Access::Limited(set) => set.contains(&Audience::Friends),
        }
    }
}

impl std::fmt::Display for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Access::Private => f.write_str("private"),
            Access::Public => f.write_str("public"),
            Access::Limited(set) => {
                let names: Vec<&str> = set
                    .iter()
                    .map(|a| match a {
                        Audience::Playtesters => "playtesters",
                        Audience::Friends => "friends",
                    })
                    .collect();
                write!(f, "limited ({})", names.join(", "))
            }
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ServerFill {
    /// Roblox decides fill behavior automatically.
    Automatic,
    /// New players go to empty servers first.
    Empty,
    /// Reserve N slots in each server for friends/invites.
    Custom { reserved_slots: u32 },
}

impl ServerFill {
    /// Roblox API value for `socialSlotType`.
    pub fn social_slot_type(&self) -> &'static str {
        match self {
            ServerFill::Automatic => "Automatic",
            ServerFill::Empty => "Empty",
            ServerFill::Custom { .. } => "Custom",
        }
    }

    pub fn custom_count(&self) -> Option<u32> {
        match self {
            ServerFill::Custom { reserved_slots } => Some(*reserved_slots),
            _ => None,
        }
    }

    /// Build a ServerFill from the legacy API's pair of fields.
    pub fn from_legacy(social_slot_type: Option<&str>, count: Option<u32>) -> Option<Self> {
        match social_slot_type? {
            "Automatic" => Some(ServerFill::Automatic),
            "Empty" => Some(ServerFill::Empty),
            "Custom" => Some(ServerFill::Custom {
                reserved_slots: count.unwrap_or(0),
            }),
            _ => None,
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PrivateServer {
    /// Price in Robux. 0 = free private servers, > 0 = paid.
    pub price: u64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Devices {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mobile: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tablet: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub console: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vr: Option<bool>,
}

impl Devices {
    pub fn is_empty(&self) -> bool {
        self.desktop.is_none()
            && self.mobile.is_none()
            && self.tablet.is_none()
            && self.console.is_none()
            && self.vr.is_none()
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SocialLinks {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facebook: Option<SocialLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twitter: Option<SocialLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub youtube: Option<SocialLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twitch: Option<SocialLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discord: Option<SocialLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roblox_group: Option<SocialLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guilded: Option<SocialLink>,
}

impl SocialLinks {
    pub fn is_empty(&self) -> bool {
        self.facebook.is_none()
            && self.twitter.is_none()
            && self.youtube.is_none()
            && self.twitch.is_none()
            && self.discord.is_none()
            && self.roblox_group.is_none()
            && self.guilded.is_none()
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SocialLink {
    pub title: String,
    pub url: String,
}
