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
    #[serde(default)]
    pub regional_pricing: bool,
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
    #[serde(default)]
    pub regional_pricing: bool,
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
