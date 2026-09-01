//! The small closed vocabularies of a universe: who may do what, who pays,
//! what it is, where it runs.
//!
//! One enum each, with the same parse-and-render pair the avatar types carry
//! and for the same reason.

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

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Private,
}

impl Visibility {
    /// Parse the Open Cloud Universe.visibility enum value.
    pub fn from_open_cloud(value: &str) -> Option<Self> {
        match value {
            "PUBLIC" => Some(Visibility::Public),
            "PRIVATE" => Some(Visibility::Private),
            _ => None,
        }
    }

    pub fn is_public(&self) -> bool {
        matches!(self, Visibility::Public)
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
