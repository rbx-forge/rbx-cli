use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};

mod avatar;
mod overlay;
mod vocabulary;

// Re-exported flat, so every `crate::config::AvatarType` outside this module
// keeps working. The split is an arrangement of this file, not a change to the
// shape callers see.
pub use self::{avatar::*, overlay::*, vocabulary::*};

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
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
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

    /// What the experience lets other experiences and the client do to it.
    ///
    /// Requires cookie (universe configuration endpoint).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,

    /// Avatar rules: which rig, whose animations, how collisions are shaped,
    /// and the scale range players are held to.
    ///
    /// Requires cookie (universe configuration endpoint).
    #[serde(default, skip_serializing_if = "Avatar::is_empty")]
    pub avatar: Avatar,

    /// Whether the experience is sold, and for how much.
    ///
    /// Omit the table entirely to leave paid access unmanaged. Requires cookie
    /// (universe configuration endpoint).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid_access: Option<PaidAccess>,

    /// The legacy genre. Requires cookie (universe configuration endpoint).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genre: Option<Genre>,

    /// Path to a JSON file holding the modern avatar rules, relative to this
    /// config file.
    ///
    /// The API field is `engineAvatarSettings`, and it is a JSON *string*: a
    /// whole nested document (animation rules, clothing rules, accessory
    /// rules, collision rules, body rules) handed over as an opaque blob with
    /// no published schema for what is inside it.
    ///
    /// So this tool does not model it. It reads the file, checks it parses as
    /// JSON, and sends it. That is a deliberate limit rather than a shortcut:
    /// the spec marks the field "experimental which may be changed or removed
    /// in future", and modelling a hundred and fifty keys of something Roblox
    /// reserves the right to redefine would be inventing a contract nobody
    /// offered. A file you control, versioned next to the rest of the config,
    /// keeps working whatever Roblox does to the inside of it.
    ///
    /// Roblox's own semantics line up with this file's: an absent or empty
    /// value is not written, so omitting the key leaves the settings alone. A
    /// file containing `{}` is how you clear them.
    ///
    /// Requires cookie, and write-only like the rest of `[game.avatar]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_avatar_settings: Option<PathBuf>,
}

/// Drop the `?` segments `serde_ignored` emits for a layer it cannot name.
///
/// An `Option<T>` field contributes one, so a typo inside
/// `[game.avatar.min_scale]` arrives as `game.avatar.min_scale.?.hieght`. The
/// `?` is an artefact of how the value is reached, not part of any path a user
/// could type, and printing it in a warning invites them to go looking for a
/// table that does not exist.
fn clean_path(path: &str) -> String {
    path.split('.')
        .filter(|segment| *segment != "?")
        .collect::<Vec<_>>()
        .join(".")
}

/// Name the keys this build read nothing from, on stderr. Never fails.
///
/// # The failure this exists for
///
/// A released `rbx` reading a `rbxmeta.toml` written for a newer one answered
/// **"Nothing to do, everything is in sync."** The file declared
/// `[game.permissions]`, `[game.avatar]`, two scale tables, `genre` and
/// `engine_avatar_settings`; the binary understood none of them, discarded them
/// all, compared what was left against the lockfile, and reported agreement.
///
/// That is the worst answer a tool whose job is reporting drift can give. An
/// error would have been fine. Silence would have been survivable. A confident
/// green is what sends somebody away believing their config is applied.
///
/// # Why it warns rather than rejects
///
/// The same reason `rbxplace.toml` and `rbxshop.toml` warn: a config written
/// for a newer release has to stay loadable by an older one, or upgrading the
/// file becomes a flag day for everyone sharing the repository. What the older
/// binary owes them is to say what it skipped.
///
/// # Why the whole path and not just the root
///
/// `rbx-shop`'s equivalent checks top-level keys only, and would not have
/// caught any of the keys above: every one of them is nested inside `[game]`.
/// `serde_ignored` reports the full dotted path, so `game.permissions` and
/// `game.avatar.min_scale.hieght` are both named where they are.
///
/// # The blind spot, named
///
/// **Two tables still swallow silently: `[game.server_fill]` and
/// `[game.paid_access]`**, along with their env-overlay copies. Both are
/// internally-tagged enums (`#[serde(tag = "mode")]`), and serde buffers the
/// content of one into an intermediate value before deserializing from it,
/// which loses the ignored-key callback on the way through. It is a serde
/// limitation, not something this function can reach.
///
/// That is worth stating precisely because it is exactly where this bug was
/// first noticed: `genre` appended to the wrong place in the file landed inside
/// `[game.server_fill]` and vanished. The remedy would be a hand-written key
/// list for those two tables, which is the drift this whole approach was
/// chosen to avoid, so it is a deliberate 90% rather than an oversight. See
/// TODO.md.
fn warn_ignored_keys(path: &Path, ignored: &[String]) {
    if ignored.is_empty() {
        return;
    }
    eprintln!(
        "{} {}: {} key{} this build reads nothing from, ignored by rbx {}:
",
        "warning:".yellow().bold(),
        path.display(),
        ignored.len(),
        if ignored.len() == 1 { "" } else { "s" },
        env!("CARGO_PKG_VERSION"),
    );
    for key in ignored {
        eprintln!("  {}", key.yellow());
    }
    eprintln!(
        "\nEither one is misspelled, or this file was written for a newer rbx \
         than this one. Nothing is deleted: `rbx meta pull` writes the file \
         back with these keys intact."
    );
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
            && self.permissions.is_none()
            && self.avatar.is_empty()
            && self.paid_access.is_none()
            && self.genre.is_none()
            && self.engine_avatar_settings.is_none()
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
        // Permissions replace as a group rather than merging field by
        // field, because the group is what gets sent. An overlay that set one
        // flag and inherited three would be describing a request Roblox never
        // receives in that shape.
        if let Some(v) = overlay.permissions {
            self.permissions = Some(v);
        }
        // Avatar: field by field for the four modes; each scale table is
        // atomic, because half a scale table is not a meaningful value.
        if let Some(v) = overlay.avatar.kind {
            self.avatar.kind = Some(v);
        }
        if let Some(v) = overlay.avatar.animation {
            self.avatar.animation = Some(v);
        }
        if let Some(v) = overlay.avatar.collision {
            self.avatar.collision = Some(v);
        }
        if let Some(v) = overlay.avatar.joint_positioning {
            self.avatar.joint_positioning = Some(v);
        }
        if let Some(v) = overlay.avatar.min_scale {
            self.avatar.min_scale = Some(v);
        }
        if let Some(v) = overlay.avatar.max_scale {
            self.avatar.max_scale = Some(v);
        }
        if let Some(v) = overlay.avatar.asset_overrides {
            self.avatar.asset_overrides = Some(v);
        }
        if let Some(v) = &overlay.engine_avatar_settings {
            self.engine_avatar_settings = Some(v.clone());
        }
        if let Some(v) = &overlay.paid_access {
            self.paid_access = Some(v.clone());
        }
        if let Some(v) = overlay.genre {
            self.genre = Some(v);
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
        let (config, ignored) =
            Self::parse(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
        warn_ignored_keys(path, &ignored);
        Ok(config)
    }

    /// Parse, and report every key path the deserializer skipped.
    ///
    /// Separated from [`Self::load`] so the reporting can be tested without a
    /// file, and so a caller that wants the list rather than the warning can
    /// have it.
    pub fn parse(content: &str) -> Result<(Self, Vec<String>), toml::de::Error> {
        let mut ignored = Vec::new();
        let config = serde_ignored::deserialize(toml::Deserializer::new(content), |path| {
            ignored.push(clean_path(&path.to_string()))
        })?;
        ignored.sort();
        Ok((config, ignored))
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
