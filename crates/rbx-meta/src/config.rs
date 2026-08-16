use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    /// Optional fallback experience IDs. Only consulted when no `--env` is passed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experience: Option<Experience>,

    #[serde(default, skip_serializing_if = "Game::is_empty")]
    pub game: Game,

    #[serde(default, skip_serializing_if = "MediaConfig::is_default")]
    pub media: MediaConfig,

    /// Per-env overrides, layered on top of `[game]` and `[media]` when
    /// the corresponding `--env` is targeted.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub envs: BTreeMap<String, EnvOverlay>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Experience {
    pub universe_id: u64,
    /// Root place ID. Used to write displayName/description/serverSize.
    pub place_id: u64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Game {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Max concurrent players per server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_size: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_chat: Option<bool>,

    /// Private server settings. Omit the entire table to disable private servers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_server: Option<PrivateServer>,

    #[serde(default, skip_serializing_if = "Devices::is_empty")]
    pub devices: Devices,

    #[serde(default, skip_serializing_if = "SocialLinks::is_empty")]
    pub social_links: SocialLinks,

    /// Server fill mode. Requires cookie (not exposed by Open Cloud).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_fill: Option<ServerFill>,

    /// Allow other users to copy this place. Requires cookie (not exposed by Open Cloud).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_copying: Option<bool>,

    /// Public vs private visibility. Read via Open Cloud, write requires cookie.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,

    /// Studio API access toggle. Requires cookie (universe configuration endpoint).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub studio_access_to_apis_allowed: Option<bool>,

    /// Beta mode toggle: limits the experience's reach by hiding it from
    /// Home Recommendations. Requires cookie (experience-releases endpoint).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beta_mode: Option<bool>,
}

impl Game {
    fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.server_size.is_none()
            && self.voice_chat.is_none()
            && self.private_server.is_none()
            && self.devices.is_empty()
            && self.social_links.is_empty()
            && self.server_fill.is_none()
            && self.allow_copying.is_none()
            && self.visibility.is_none()
            && self.studio_access_to_apis_allowed.is_none()
            && self.beta_mode.is_none()
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
    fn is_default(&self) -> bool {
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
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
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

impl Game {
    /// Apply an env overlay in place: every non-None field in `overlay` replaces
    /// the corresponding field on `self`.
    pub fn apply_overlay(&mut self, overlay: &EnvOverlay) {
        if let Some(v) = &overlay.name {
            self.name = Some(v.clone());
        }
        if let Some(v) = &overlay.description {
            self.description = Some(v.clone());
        }
        if let Some(v) = overlay.server_size {
            self.server_size = Some(v);
        }
        if let Some(v) = overlay.voice_chat {
            self.voice_chat = Some(v);
        }
        if let Some(v) = overlay.allow_copying {
            self.allow_copying = Some(v);
        }
        if let Some(v) = overlay.visibility {
            self.visibility = Some(v);
        }
        if let Some(v) = overlay.studio_access_to_apis_allowed {
            self.studio_access_to_apis_allowed = Some(v);
        }
        if let Some(v) = overlay.beta_mode {
            self.beta_mode = Some(v);
        }
        if let Some(ps) = &overlay.private_server {
            self.private_server = Some(ps.clone());
        }
        if let Some(sf) = &overlay.server_fill {
            self.server_fill = Some(sf.clone());
        }
        // Devices: merge field by field.
        if let Some(v) = overlay.devices.desktop {
            self.devices.desktop = Some(v);
        }
        if let Some(v) = overlay.devices.mobile {
            self.devices.mobile = Some(v);
        }
        if let Some(v) = overlay.devices.tablet {
            self.devices.tablet = Some(v);
        }
        if let Some(v) = overlay.devices.console {
            self.devices.console = Some(v);
        }
        if let Some(v) = overlay.devices.vr {
            self.devices.vr = Some(v);
        }
        // Social links: per platform atomic.
        if let Some(s) = &overlay.social_links.facebook {
            self.social_links.facebook = Some(s.clone());
        }
        if let Some(s) = &overlay.social_links.twitter {
            self.social_links.twitter = Some(s.clone());
        }
        if let Some(s) = &overlay.social_links.youtube {
            self.social_links.youtube = Some(s.clone());
        }
        if let Some(s) = &overlay.social_links.twitch {
            self.social_links.twitch = Some(s.clone());
        }
        if let Some(s) = &overlay.social_links.discord {
            self.social_links.discord = Some(s.clone());
        }
        if let Some(s) = &overlay.social_links.roblox_group {
            self.social_links.roblox_group = Some(s.clone());
        }
        if let Some(s) = &overlay.social_links.guilded {
            self.social_links.guilded = Some(s.clone());
        }
    }
}

impl MediaConfig {
    pub fn apply_overlay(&mut self, overlay: &MediaOverlay) {
        if let Some(v) = &overlay.icon {
            self.icon = Some(v.clone());
        }
        if let Some(v) = &overlay.thumbnails {
            self.thumbnails = v.clone();
        }
        if let Some(v) = &overlay.dir {
            self.dir = Some(v.clone());
        }
        if let Some(v) = overlay.bleed {
            self.bleed = v;
        }
        if let Some(v) = &overlay.language_code {
            self.language_code = v.clone();
        }
    }
}

impl Config {
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(config)
    }

    /// Resolve effective `(Game, MediaConfig)` for a target env. When `env` is
    /// `None`, returns the base `[game]` / `[media]` untouched.
    pub fn resolve_env(&self, env: Option<&str>) -> (Game, MediaConfig) {
        let mut game = self.game.clone();
        let mut media = self.media.clone();
        if let Some(name) = env {
            if let Some(overlay) = self.envs.get(name) {
                game.apply_overlay(overlay);
                media.apply_overlay(&overlay.media);
            }
        }
        (game, media)
    }

    /// Validate Roblox-imposed invariants on the resolved game state. Called
    /// before sending any PATCH so we can surface a clear error instead of the
    /// generic 500 INTERNAL that Roblox returns for invalid combinations.
    pub fn validate_invariants(game: &Game) -> Result<()> {
        // Roblox enforces a minimum of 10 Robux for paid private servers.
        // price = 0 (free) and absent table (disabled) are both fine.
        if let Some(price) = game.private_server.as_ref().map(|p| p.price) {
            if price > 0 && price < 10 {
                bail!(
                    "Invalid private_server.price = {} Robux.\n  \
                     Roblox requires a minimum of 10 Robux for paid private servers.\n  \
                     Fix: set price = 0 (free), price >= 10, or remove the [private_server] table to disable.",
                    price
                );
            }
        }

        let is_private = matches!(game.visibility, Some(Visibility::Private));
        let has_paid_private_server = game
            .private_server
            .as_ref()
            .map(|p| p.price > 0)
            .unwrap_or(false);
        if is_private && has_paid_private_server {
            bail!(
                "Invalid combination: visibility=\"private\" with private_server.price > 0.\n  \
                 Roblox requires the experience to be PUBLIC to enable paid private servers.\n  \
                 Fix: set visibility = \"public\", or set private_server.price = 0 (free)."
            );
        }
        Ok(())
    }

    /// Validate that referenced icon/thumbnail paths exist on disk. Call after
    /// resolving the env so per-env media overrides are honored.
    pub fn validate_media_paths(media: &MediaConfig, config_dir: &Path) -> Result<()> {
        if let Some(icon) = &media.icon {
            let full = config_dir.join(icon);
            if !full.exists() {
                bail!("Icon path does not exist: {}", full.display());
            }
        }
        for (idx, thumb) in media.thumbnails.iter().enumerate() {
            let full = config_dir.join(thumb);
            if !full.exists() {
                bail!(
                    "Thumbnail #{}: path does not exist: {}",
                    idx + 1,
                    full.display()
                );
            }
        }
        if media.thumbnails.len() > 10 {
            bail!(
                "Roblox allows at most 10 thumbnails per experience (found {})",
                media.thumbnails.len()
            );
        }
        Ok(())
    }

    pub fn default_template() -> String {
        r#"# rbx meta configuration
# Manages Roblox game/universe metadata via Open Cloud API.
#
# Two ways to point at an experience:
#   1. Multi-env (recommended): pass `--env <name>`. universe_id/place_id are
#      resolved from rbxplace.toml [<env>] + [<env>.places.<place>].
#   2. Standalone: keep the [experience] block below.
# Per-env overrides go under [envs.<name>] (see bottom of file).

[experience]
universe_id = 0       # Your Roblox universe ID (omit if you use --env)
place_id = 0          # Your root place ID (omit if you use --env)

[game]
# Scalar [game] fields must be declared here, BEFORE any [game.*] sub-table.
# name = "My Game"
# description = "A really fun game"
# server_size = 50          # Max concurrent players per server
# voice_chat = false
# allow_copying = false              # REQUIRES cookie
# visibility = "public"              # "public" | "private", REQUIRES cookie to change
# studio_access_to_apis_allowed = false  # REQUIRES cookie. Lets Studio scripts call Open Cloud / data store APIs
# beta_mode = false                      # REQUIRES cookie. true = hides from Home Recommendations (Experience Beta)

# Private servers. Omit this table to disable private servers entirely.
# [game.private_server]
# price = 0                 # 0 = free private servers, > 0 = paid (Robux)

# Playable devices. Omit a field to leave it unchanged on Roblox.
# [game.devices]
# desktop = true
# mobile = true
# tablet = true
# console = false
# vr = false

# Server fill mode. REQUIRES .ROBLOSECURITY cookie (not exposed by Open Cloud).
# Auto-detected from Roblox Studio if installed; otherwise pass --cookie.
# [game.server_fill]
# mode = "automatic"        # "automatic" | "empty"
# (or for custom)
# [game.server_fill]
# mode = "custom"
# reserved_slots = 5

# Social links. Omit a section to remove that link.
# [game.social_links.discord]
# title = "Join our Discord"
# url = "https://discord.gg/example"
#
# [game.social_links.twitter]
# title = "Follow us on X"
# url = "https://x.com/example"
#
# Other platforms: facebook, youtube, twitch, roblox_group, guilded

# Media: icon, thumbnails, and upload options.
# [media]
# icon = "assets/icon.png"                                  # Path to a PNG icon
# thumbnails = ["assets/thumb1.png", "assets/thumb2.png"]   # Up to 10
# dir = "assets"                                            # Destination for `pull --accept-remote`
# bleed = true                                              # Apply alpha bleed to PNGs (default: true)
# language_code = "en_us"

# Per-env overrides. Layered on top of [game] / [media] when --env <name> is
# passed. Pull writes here automatically when a remote value diverges from base.
# [envs.dev]
# visibility = "private"
#
# [envs.prod]
# visibility = "public"
# allow_copying = false
#
# [envs.dev.devices]
# desktop = false           # override a single device toggle for dev only
"#
        .to_string()
    }
}
