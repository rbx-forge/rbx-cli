//! The three things this tool sells, as a config file declares them: a game
//! pass, a badge, a developer product.
//!
//! They sit together because most of what can be said about one can be said
//! about the others, and a field that exists on only two of the three should be
//! visibly missing from the third rather than filed elsewhere.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PassConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,
    #[serde(default = "default_true")]
    pub for_sale: bool,
    /// Deprecated by Roblox in favour of `managed_pricing`, which supersedes
    /// it. Kept because it is already in released configs.
    /// Not written when false: a config fresh from `init` would otherwise
    /// show the deprecated key and not its successor, which reads as pricing
    /// turned off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub regional_pricing: bool,
    /// Roblox's successor to `regional_pricing`: it bundles regional pricing
    /// with price optimization under one opt-in.
    ///
    /// Tri-state on purpose. Roblox enables managed pricing by itself on
    /// passes, so a plain `false` default would make every sync turn off
    /// something nobody asked to turn off. Unset sends no field at all and
    /// leaves whatever Roblox has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_pricing: Option<bool>,
    /// When true, an extra developer product is derived automatically at
    /// resolve time: same price/description/icon, name prefixed with
    /// `[gifts].label`. See `crate::gifts`.
    #[serde(default)]
    pub create_gift: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

// `deny_unknown_fields` (unlike Pass/ProductConfig) because `create_gift` is
// documented right next to badges and only applies to passes/products:
// silently swallowing it here would look like a no-op bug rather than an
// unsupported field.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BadgeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProductConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub price: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,
    #[serde(default = "default_true")]
    pub for_sale: bool,
    /// Deprecated by Roblox in favour of `managed_pricing`. See `PassConfig`.
    /// Not written when false: a config fresh from `init` would otherwise
    /// show the deprecated key and not its successor, which reads as pricing
    /// turned off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub regional_pricing: bool,
    /// See `PassConfig::managed_pricing`. Unlike a pass, a developer product
    /// is not opted in by Roblox on its own: enabling it also requires
    /// dynamically scripted prices and a `GetUsersPriceLevelsAsync` call in
    /// the experience, which this tool cannot check for you.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_pricing: Option<bool>,
    #[serde(default)]
    pub store_page: bool,
    /// When true, an extra developer product is derived automatically at
    /// resolve time: same price/description/icon, name prefixed with
    /// `[gifts].label`. See `crate::gifts`.
    #[serde(default)]
    pub create_gift: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn default_icon_dir() -> PathBuf {
    PathBuf::from("icons")
}

/// Resolve the display name for a resource: use the explicit `name` field if set,
/// otherwise fall back to the TOML key.
pub fn resolve_name<'a>(config_name: Option<&'a str>, key: &'a str) -> &'a str {
    config_name.unwrap_or(key)
}
