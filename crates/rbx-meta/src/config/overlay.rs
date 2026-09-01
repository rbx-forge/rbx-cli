//! Media, and the per-env overlays that replace parts of a `[game]` or
//! `[media]` block.
//!
//! An overlay is the same shape with every field optional: absent means keep
//! what the base said, present means replace it. That is the whole rule, and it
//! is why these two live together.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::*;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MediaConfig {
    /// Path to the game icon PNG (relative to the config file).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,

    /// Up to 10 thumbnail PNG paths, in display order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thumbnails: Vec<PathBuf>,

    /// Destination directory for `pull --accept-remote` downloads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,

    /// Apply alpha bleed to PNG icons/thumbnails before uploading (default: true).
    #[serde(default = "default_true")]
    pub bleed: bool,

    /// Language code used for localized icon/thumbnail upload (default: "en_us").
    #[serde(default = "default_language")]
    pub language_code: String,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            icon: None,
            thumbnails: Vec::new(),
            dir: None,
            bleed: true,
            language_code: default_language(),
        }
    }
}

impl MediaConfig {
    pub(crate) fn is_default(&self) -> bool {
        self.icon.is_none()
            && self.thumbnails.is_empty()
            && self.dir.is_none()
            && self.bleed
            && self.language_code == default_language()
    }
}

fn default_true() -> bool {
    true
}

fn default_language() -> String {
    "en_us".to_string()
}

/// Per-env overlay layered on top of `[game]` + `[media]` when the matching
/// `--env` is targeted. All fields are optional; missing fields fall through to
/// the base values.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct EnvOverlay {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_size: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_chat: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_copying: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub studio_access_to_apis_allowed: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub beta_mode: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_server: Option<PrivateServer>,

    #[serde(default, skip_serializing_if = "Devices::is_empty")]
    pub devices: Devices,

    #[serde(default, skip_serializing_if = "SocialLinks::is_empty")]
    pub social_links: SocialLinks,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_fill: Option<ServerFill>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,

    #[serde(default, skip_serializing_if = "Avatar::is_empty")]
    pub avatar: Avatar,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid_access: Option<PaidAccess>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub genre: Option<Genre>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_avatar_settings: Option<PathBuf>,

    #[serde(default, skip_serializing_if = "MediaOverlay::is_empty")]
    pub media: MediaOverlay,
}

impl EnvOverlay {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.server_size.is_none()
            && self.voice_chat.is_none()
            && self.allow_copying.is_none()
            && self.visibility.is_none()
            && self.studio_access_to_apis_allowed.is_none()
            && self.beta_mode.is_none()
            && self.private_server.is_none()
            && self.devices.is_empty()
            && self.social_links.is_empty()
            && self.server_fill.is_none()
            && self.permissions.is_none()
            && self.avatar.is_empty()
            && self.paid_access.is_none()
            && self.genre.is_none()
            && self.engine_avatar_settings.is_none()
            && self.media.is_empty()
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MediaOverlay {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,

    /// `Some(vec)` overrides base thumbnails; `None` means inherit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnails: Option<Vec<PathBuf>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bleed: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_code: Option<String>,
}

impl MediaOverlay {
    pub fn is_empty(&self) -> bool {
        self.icon.is_none()
            && self.thumbnails.is_none()
            && self.dir.is_none()
            && self.bleed.is_none()
            && self.language_code.is_none()
    }
}
