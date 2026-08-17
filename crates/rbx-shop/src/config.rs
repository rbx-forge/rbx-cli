use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{bail, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};

pub use crate::toml_write::KeyRename;

/// Every table `rbxshop.toml` gives a meaning to at the top level.
///
/// A key outside this list is kept — `save_in_place` never deletes what it
/// does not model, and rejecting it would make a config written for a newer
/// rbx unloadable by an older one. But it is not kept *silently*: an ignored
/// key looks exactly like an honoured one from the outside, which is the same
/// argument `rbx env` makes for `rbxplace.toml`.
pub const ROOT_KEYS: &[&str] = &[
    "experience",
    "owner",
    "codegen",
    "icons",
    "gifts",
    "include",
    "passes",
    "badges",
    "products",
    "envs",
];

/// Top-level keys in `content` that this build reads nothing from, sorted.
///
/// Read off the raw document rather than the deserialized struct, so the
/// answer does not depend on which `#[serde]` attributes happen to be on
/// `Config` today.
pub fn unknown_root_keys(content: &str) -> Vec<String> {
    let Ok(toml::Value::Table(root)) = content.parse::<toml::Value>() else {
        return Vec::new();
    };
    let mut found: Vec<String> = root
        .keys()
        .filter(|k| !ROOT_KEYS.contains(&k.as_str()))
        .cloned()
        .collect();
    found.sort();
    found
}

/// Paths already warned about, so a command that loads the main file and then
/// its `[include]` siblings says this once per file rather than once per read.
fn warned_paths() -> &'static Mutex<HashSet<PathBuf>> {
    static WARNED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    WARNED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Name unrecognised top-level keys on stderr. Never fails the command.
fn warn_unknown_root_keys(path: &Path, content: &str) {
    let unknown = unknown_root_keys(content);
    if unknown.is_empty() {
        return;
    }
    {
        let mut warned = warned_paths()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !warned.insert(path.to_path_buf()) {
            return;
        }
    }

    eprintln!(
        "{} {}: {} unrecognised top-level key{}, ignored by rbx {}:\n",
        "warning:".yellow().bold(),
        path.display(),
        unknown.len(),
        if unknown.len() == 1 { "" } else { "s" },
        env!("CARGO_PKG_VERSION"),
    );
    for key in &unknown {
        eprintln!("  {}", key.yellow());
    }
    eprintln!(
        "    {} {}",
        "known keys:".dimmed(),
        ROOT_KEYS.join(", ").dimmed()
    );
    eprintln!(
        "\nAn ignored key changes nothing, but it is preserved: `rbx shop pull` and\n\
         `rbx shop rename` write the file back without deleting it. Either it is\n\
         misspelled, or it comes from a release newer than this one."
    );
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Optional standalone fallback universe ID. Only consulted when no
    /// `--env` is passed. With `--env <name>`, the universe id comes from
    /// rbxplace.toml.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experience: Option<Experience>,

    /// Who owns this project — global to the config, not env-scoped. The one
    /// thing it decides is the payment source for badge creation, and that is
    /// not a free choice: Roblox pays a group-owned game's badge out of group
    /// funds and a user-owned game's out of the user's, with no way to cross
    /// them. So the payer is the owner, necessarily, and this is an override
    /// of `[owner]` in `rbxplace.toml` rather than a second concept.
    ///
    /// Optional. When absent, `ShopCtx::resolve_owner_type` falls back to
    /// `rbxplace.toml`: the env's own `[<env>.owner]` first, then top-level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<rbx_core::owner::Owner>,

    #[serde(default, skip_serializing_if = "CodegenConfig::is_default")]
    pub codegen: CodegenConfig,

    #[serde(default, skip_serializing_if = "IconsConfig::is_default")]
    pub icons: IconsConfig,

    #[serde(default, skip_serializing_if = "GiftsConfig::is_default")]
    pub gifts: GiftsConfig,

    /// Extra files (relative to this one) whose `passes`/`badges`/`products`
    /// tables (and their `[envs.<name>.*]` overlays) get merged in at load
    /// time via `Config::load_merged`. Only meaningful on the main file —
    /// included files must be "pure" resource files (no
    /// `experience`/`owner`/`codegen`/`icons`/`gifts`/`include` of their
    /// own). See `docs/shop.md` for the `pull`/`rename` limitation this
    /// currently carries.
    #[serde(default, skip_serializing_if = "IncludeConfig::is_empty")]
    pub include: IncludeConfig,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub passes: BTreeMap<String, PassConfig>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub badges: BTreeMap<String, BadgeConfig>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub products: BTreeMap<String, ProductConfig>,

    /// Per-env overlays. When `--env <name>` is targeted, each overlay map is
    /// merged on top of the base. Resources defined only in `envs.<name>` are
    /// added as env-exclusive.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub envs: BTreeMap<String, EnvOverlay>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Experience {
    pub universe_id: u64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CodegenStyle {
    #[default]
    Flat,
    Nested,
}

impl CodegenStyle {
    fn is_default(&self) -> bool {
        matches!(self, CodegenStyle::Flat)
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct CodegenPaths {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badges: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub products: Option<String>,
}

impl CodegenPaths {
    pub fn is_default(&self) -> bool {
        self.passes.is_none() && self.badges.is_none() && self.products.is_none()
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct CodegenConfig {
    /// Path to the generated Luau module **folder** (no extension). The folder
    /// will contain `init.luau` (dispatcher + type alias) plus one `<env>.luau`
    /// per env. If left unset, code generation is disabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,

    /// Also generate a TypeScript definition file (`init.d.ts` next to `init.luau`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub typescript: bool,

    /// Code generation style applied within each env module: "flat" or "nested".
    #[serde(default, skip_serializing_if = "CodegenStyle::is_default")]
    pub style: CodegenStyle,

    #[serde(default, skip_serializing_if = "CodegenPaths::is_default")]
    pub paths: CodegenPaths,

    /// Extra entries injected into every env module: `"path.to.key" = asset_id`.
    /// Shared across all envs (no overlay support for extras).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, u64>,
}

impl CodegenConfig {
    fn is_default(&self) -> bool {
        self.output.is_none()
            && !self.typescript
            && self.style.is_default()
            && self.paths.is_default()
            && self.extra.is_empty()
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IconsConfig {
    /// Apply alpha bleed to icons before uploading (default: true)
    #[serde(default = "default_true")]
    pub bleed: bool,

    /// Directory for downloaded icons (default: "icons")
    #[serde(default = "default_icon_dir")]
    pub dir: PathBuf,
}

impl Default for IconsConfig {
    fn default() -> Self {
        Self {
            bleed: true,
            dir: default_icon_dir(),
        }
    }
}

impl IconsConfig {
    fn is_default(&self) -> bool {
        self.bleed && self.dir == default_icon_dir()
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GiftsConfig {
    /// Prefixed to the source's resolved display name for the derived gift
    /// product (e.g. "VIP Pass" -> `"[GIFT] VIP Pass"`). This is the Roblox
    /// display name, independent of `key_prefix` below.
    #[serde(default = "default_gift_label")]
    pub label: String,

    /// Prefixed to the source's TOML key to build the resolved/codegen key,
    /// e.g. with the default "Gift": `VIP` -> `GiftVIP`, `vip_pass` ->
    /// `Giftvip_pass`. Must be non-empty. The source's own TOML key is never
    /// modified by this — see `capitalize_key` for the one exception, which
    /// only affects the copy used in this derived string.
    #[serde(default = "default_gift_key_prefix")]
    pub key_prefix: String,

    /// When true, the source key's first letter is uppercased in the
    /// *derived* gift key only (never in the source's own TOML key) — e.g.
    /// with `key_prefix = "gift"`: `vipPass` -> `giftVipPass` instead of the
    /// default `giftvipPass`. Default: `false` (exact concatenation).
    #[serde(default)]
    pub capitalize_key: bool,
}

impl Default for GiftsConfig {
    fn default() -> Self {
        Self {
            label: default_gift_label(),
            key_prefix: default_gift_key_prefix(),
            capitalize_key: false,
        }
    }
}

impl GiftsConfig {
    fn is_default(&self) -> bool {
        self.label == default_gift_label()
            && self.key_prefix == default_gift_key_prefix()
            && !self.capitalize_key
    }
}

fn default_gift_label() -> String {
    "[GIFT] ".to_string()
}

fn default_gift_key_prefix() -> String {
    "Gift".to_string()
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct IncludeConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PathBuf>,
}

impl IncludeConfig {
    fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Resource configs (base)
// ---------------------------------------------------------------------------

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
    /// resolve time — same price/description/icon, name prefixed with
    /// `[gifts].label`. See `crate::gifts`.
    #[serde(default)]
    pub create_gift: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

// `deny_unknown_fields` (unlike Pass/ProductConfig) because `create_gift` is
// documented right next to badges and only applies to passes/products —
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
    /// resolve time — same price/description/icon, name prefixed with
    /// `[gifts].label`. See `crate::gifts`.
    #[serde(default)]
    pub create_gift: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_icon_dir() -> PathBuf {
    PathBuf::from("icons")
}

/// Resolve the display name for a resource: use the explicit `name` field if set,
/// otherwise fall back to the TOML key.
pub fn resolve_name<'a>(config_name: Option<&'a str>, key: &'a str) -> &'a str {
    config_name.unwrap_or(key)
}

// ---------------------------------------------------------------------------
// Env overlays
// ---------------------------------------------------------------------------

/// Per-env overlay grouping the three resource maps. All fields are optional —
/// merging is done resource by resource, field by field. A resource defined
/// only in the overlay (not in base) is treated as env-exclusive.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct EnvOverlay {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub passes: BTreeMap<String, PassOverlay>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub badges: BTreeMap<String, BadgeOverlay>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub products: BTreeMap<String, ProductOverlay>,
}

impl EnvOverlay {
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty() && self.badges.is_empty() && self.products.is_empty()
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct PassOverlay {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub for_sale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regional_pricing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_gift: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BadgeOverlay {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct ProductOverlay {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub for_sale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regional_pricing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_page: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_gift: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl PassOverlay {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.price.is_none()
            && self.description.is_none()
            && self.icon.is_none()
            && self.for_sale.is_none()
            && self.regional_pricing.is_none()
            && self.create_gift.is_none()
            && self.path.is_none()
    }
}

impl BadgeOverlay {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.icon.is_none()
            && self.enabled.is_none()
            && self.path.is_none()
    }
}

impl ProductOverlay {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.price.is_none()
            && self.description.is_none()
            && self.icon.is_none()
            && self.for_sale.is_none()
            && self.regional_pricing.is_none()
            && self.store_page.is_none()
            && self.create_gift.is_none()
            && self.path.is_none()
    }
}

impl PassConfig {
    pub fn apply_overlay(&mut self, ov: &PassOverlay) {
        if let Some(v) = &ov.name {
            self.name = Some(v.clone());
        }
        if let Some(v) = ov.price {
            self.price = Some(v);
        }
        if let Some(v) = &ov.description {
            self.description = Some(v.clone());
        }
        if let Some(v) = &ov.icon {
            self.icon = Some(v.clone());
        }
        if let Some(v) = ov.for_sale {
            self.for_sale = v;
        }
        if let Some(v) = ov.regional_pricing {
            self.regional_pricing = v;
        }
        if let Some(v) = ov.create_gift {
            self.create_gift = v;
        }
        if let Some(v) = &ov.path {
            self.path = Some(v.clone());
        }
    }

    /// Build from an overlay alone (when the pass exists only in `envs.<name>`).
    pub fn from_overlay(ov: &PassOverlay) -> Self {
        Self {
            name: ov.name.clone(),
            price: ov.price,
            description: ov.description.clone(),
            icon: ov.icon.clone(),
            for_sale: ov.for_sale.unwrap_or(true),
            regional_pricing: ov.regional_pricing.unwrap_or(false),
            create_gift: ov.create_gift.unwrap_or(false),
            path: ov.path.clone(),
        }
    }
}

impl BadgeConfig {
    pub fn apply_overlay(&mut self, ov: &BadgeOverlay) {
        if let Some(v) = &ov.name {
            self.name = Some(v.clone());
        }
        if let Some(v) = &ov.description {
            self.description = Some(v.clone());
        }
        if let Some(v) = &ov.icon {
            self.icon = Some(v.clone());
        }
        if let Some(v) = ov.enabled {
            self.enabled = v;
        }
        if let Some(v) = &ov.path {
            self.path = Some(v.clone());
        }
    }

    pub fn from_overlay(ov: &BadgeOverlay) -> Self {
        Self {
            name: ov.name.clone(),
            description: ov.description.clone(),
            icon: ov.icon.clone(),
            enabled: ov.enabled.unwrap_or(true),
            path: ov.path.clone(),
        }
    }
}

impl ProductConfig {
    pub fn apply_overlay(&mut self, ov: &ProductOverlay) {
        if let Some(v) = &ov.name {
            self.name = Some(v.clone());
        }
        if let Some(v) = ov.price {
            self.price = v;
        }
        if let Some(v) = &ov.description {
            self.description = Some(v.clone());
        }
        if let Some(v) = &ov.icon {
            self.icon = Some(v.clone());
        }
        if let Some(v) = ov.for_sale {
            self.for_sale = v;
        }
        if let Some(v) = ov.regional_pricing {
            self.regional_pricing = v;
        }
        if let Some(v) = ov.store_page {
            self.store_page = v;
        }
        if let Some(v) = ov.create_gift {
            self.create_gift = v;
        }
        if let Some(v) = &ov.path {
            self.path = Some(v.clone());
        }
    }

    /// Build from an overlay alone. Errors if `price` is unset (required).
    pub fn from_overlay(key: &str, ov: &ProductOverlay) -> Result<Self> {
        let price = ov.price.ok_or_else(|| {
            anyhow::anyhow!(
                "Product '{}' is defined only in an env overlay but lacks the required `price` field. \
                 Add [envs.<name>.products.{}].price or move the product into base [products.{}].",
                key, key, key
            )
        })?;
        Ok(Self {
            name: ov.name.clone(),
            price,
            description: ov.description.clone(),
            icon: ov.icon.clone(),
            for_sale: ov.for_sale.unwrap_or(true),
            regional_pricing: ov.regional_pricing.unwrap_or(false),
            store_page: ov.store_page.unwrap_or(false),
            create_gift: ov.create_gift.unwrap_or(false),
            path: ov.path.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Multi-file configs ([include])
// ---------------------------------------------------------------------------

/// One physical file backing part of a (possibly split) config, alongside
/// its raw, unmerged contents. Produced by `Config::load_all` — index 0 in
/// that `Vec` is always the main file.
#[derive(Debug, Clone)]
pub struct ConfigFile {
    pub path: PathBuf,
    pub config: Config,
}

/// The three things a shop manages.
///
/// This is the crate's only dispatch mechanism. Nothing matches on `"pass"` /
/// `"badge"` / `"product"` any more: a stringly-typed arm needs a `_` fallback,
/// and every `_ => {}` in this crate was a place a typo would have been
/// swallowed instead of failing to compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceKind {
    Pass,
    Badge,
    Product,
}

impl ResourceKind {
    /// Every kind, in the order the CLI reports them.
    ///
    /// Iterating this is what lets diff/sync/pull say a thing once. Adding a
    /// fourth kind makes every `match` on `ResourceKind` fail to compile,
    /// which is the point.
    pub const ALL: [ResourceKind; 3] = [
        ResourceKind::Pass,
        ResourceKind::Badge,
        ResourceKind::Product,
    ];

    /// Singular, lowercase — the word used in progress lines and errors.
    pub fn label(self) -> &'static str {
        match self {
            ResourceKind::Pass => "pass",
            ResourceKind::Badge => "badge",
            ResourceKind::Product => "product",
        }
    }

    /// Plural, lowercase — for prose naming a whole collection.
    ///
    /// Spelled out rather than `format!("{}s", label())`, which produces
    /// "passs" and shipped to users in the preflight scope error before anybody
    /// read it aloud.
    pub fn plural(self) -> &'static str {
        // Delegates rather than repeating the three strings. `section()` is
        // already the plural — it names the TOML table — and two independent
        // matches over the same variants returning the same words drift the
        // first time a kind is renamed in one and not the other.
        self.section()
    }

    /// Singular, capitalized — for a message that starts with the kind.
    pub fn title(self) -> &'static str {
        match self {
            ResourceKind::Pass => "Pass",
            ResourceKind::Badge => "Badge",
            ResourceKind::Product => "Product",
        }
    }

    /// The TOML table and lockfile section this kind lives in.
    pub fn section(self) -> &'static str {
        match self {
            ResourceKind::Pass => "passes",
            ResourceKind::Badge => "badges",
            ResourceKind::Product => "products",
        }
    }
}

impl std::fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl ResourceKind {
    /// The `icon` field of this kind's base entry, for `pull` to fill in after
    /// downloading one.
    ///
    /// This and [`ResourceKind::overlay_icon_mut`] are the accessors that let
    /// `pull` persist an icon once instead of three times: the only thing that
    /// differed between the three copies was which map to reach into.
    pub fn base_icon_mut<'a>(
        self,
        config: &'a mut Config,
        key: &str,
    ) -> Option<&'a mut Option<PathBuf>> {
        match self {
            ResourceKind::Pass => config.passes.get_mut(key).map(|c| &mut c.icon),
            ResourceKind::Badge => config.badges.get_mut(key).map(|c| &mut c.icon),
            ResourceKind::Product => config.products.get_mut(key).map(|c| &mut c.icon),
        }
    }

    /// Whether this kind's base table declares `key`.
    pub fn base_contains(self, config: &Config, key: &str) -> bool {
        match self {
            ResourceKind::Pass => config.passes.contains_key(key),
            ResourceKind::Badge => config.badges.contains_key(key),
            ResourceKind::Product => config.products.contains_key(key),
        }
    }

    /// Envs whose overlay declares `key`, in name order.
    pub fn overlay_envs<'a>(self, config: &'a Config, key: &str) -> Vec<&'a str> {
        config
            .envs
            .iter()
            .filter(|(_, ov)| match self {
                ResourceKind::Pass => ov.passes.contains_key(key),
                ResourceKind::Badge => ov.badges.contains_key(key),
                ResourceKind::Product => ov.products.contains_key(key),
            })
            .map(|(env, _)| env.as_str())
            .collect()
    }

    /// Move a base entry to a new key, returning whether it was there.
    ///
    /// An entry with no explicit `name` takes the old key as one: the TOML key
    /// *is* the Roblox display name when `name` is unset, so renaming the key
    /// alone would quietly rename what players see.
    pub fn rename_base(self, config: &mut Config, old: &str, new: &str) -> bool {
        fn shift<C>(
            map: &mut BTreeMap<String, C>,
            old: &str,
            new: &str,
            name: fn(&mut C) -> &mut Option<String>,
        ) -> bool {
            let Some(mut entry) = map.remove(old) else {
                return false;
            };
            let slot = name(&mut entry);
            if slot.is_none() {
                *slot = Some(old.to_string());
            }
            map.insert(new.to_string(), entry);
            true
        }

        match self {
            ResourceKind::Pass => shift(&mut config.passes, old, new, |c| &mut c.name),
            ResourceKind::Badge => shift(&mut config.badges, old, new, |c| &mut c.name),
            ResourceKind::Product => shift(&mut config.products, old, new, |c| &mut c.name),
        }
    }

    /// Move the same key in every `[envs.*]` overlay, returning whether any
    /// overlay had it. Overlays carry no display name of their own, so nothing
    /// is pinned here.
    pub fn rename_overlays(self, config: &mut Config, old: &str, new: &str) -> bool {
        fn shift<C>(map: &mut BTreeMap<String, C>, old: &str, new: &str) -> bool {
            match map.remove(old) {
                Some(entry) => {
                    map.insert(new.to_string(), entry);
                    true
                }
                None => false,
            }
        }

        let mut found = false;
        for overlay in config.envs.values_mut() {
            // `|=`, not `||`: every env has to be visited, not just up to the
            // first hit.
            found |= match self {
                ResourceKind::Pass => shift(&mut overlay.passes, old, new),
                ResourceKind::Badge => shift(&mut overlay.badges, old, new),
                ResourceKind::Product => shift(&mut overlay.products, old, new),
            };
        }
        found
    }

    /// The same field on an existing `[envs.<env>.*]` overlay entry.
    pub fn overlay_icon_mut<'a>(
        self,
        config: &'a mut Config,
        env: &str,
        key: &str,
    ) -> Option<&'a mut Option<PathBuf>> {
        let overlay = config.envs.get_mut(env)?;
        match self {
            ResourceKind::Pass => overlay.passes.get_mut(key).map(|o| &mut o.icon),
            ResourceKind::Badge => overlay.badges.get_mut(key).map(|o| &mut o.icon),
            ResourceKind::Product => overlay.products.get_mut(key).map(|o| &mut o.icon),
        }
    }
}

/// Index into `files` of the file whose *base* table (`[passes.*]` etc.)
/// declares `key`, if any. Used by `pull` to route a write to wherever the
/// entry already lives instead of always writing to the main file.
pub fn find_owner(files: &[ConfigFile], kind: ResourceKind, key: &str) -> Option<usize> {
    files.iter().position(|f| match kind {
        ResourceKind::Pass => f.config.passes.contains_key(key),
        ResourceKind::Badge => f.config.badges.contains_key(key),
        ResourceKind::Product => f.config.products.contains_key(key),
    })
}

/// Same as `find_owner`, but for an existing `[envs.<env>.*]` overlay entry.
pub fn find_overlay_owner(
    files: &[ConfigFile],
    kind: ResourceKind,
    env: &str,
    key: &str,
) -> Option<usize> {
    files.iter().position(|f| {
        f.config
            .envs
            .get(env)
            .map(|ov| match kind {
                ResourceKind::Pass => ov.passes.contains_key(key),
                ResourceKind::Badge => ov.badges.contains_key(key),
                ResourceKind::Product => ov.products.contains_key(key),
            })
            .unwrap_or(false)
    })
}

// ---------------------------------------------------------------------------
// Resolved-per-env view
// ---------------------------------------------------------------------------

/// The merged passes/badges/products effective for a target env. Built from
/// `Config` + `envs.<name>` overlay. Used by diff/sync/codegen so those
/// modules don't need to know about the overlay system.
#[derive(Debug, Default, Clone)]
pub struct ResolvedResources {
    pub passes: BTreeMap<String, PassConfig>,
    pub badges: BTreeMap<String, BadgeConfig>,
    pub products: BTreeMap<String, ProductConfig>,
}

impl Config {
    /// Resolve `(passes, badges, products)` for a target env. When `env_name`
    /// is None or no overlay exists for the env, returns the base unchanged.
    pub fn resolve_env(&self, env_name: Option<&str>) -> Result<ResolvedResources> {
        let mut resolved = ResolvedResources {
            passes: self.passes.clone(),
            badges: self.badges.clone(),
            products: self.products.clone(),
        };

        if let Some(overlay) = env_name.and_then(|name| self.envs.get(name)) {
            for (key, ov) in &overlay.passes {
                if let Some(base) = resolved.passes.get_mut(key) {
                    base.apply_overlay(ov);
                } else {
                    resolved
                        .passes
                        .insert(key.clone(), PassConfig::from_overlay(ov));
                }
            }
            for (key, ov) in &overlay.badges {
                if let Some(base) = resolved.badges.get_mut(key) {
                    base.apply_overlay(ov);
                } else {
                    resolved
                        .badges
                        .insert(key.clone(), BadgeConfig::from_overlay(ov));
                }
            }
            for (key, ov) in &overlay.products {
                if let Some(base) = resolved.products.get_mut(key) {
                    base.apply_overlay(ov);
                } else {
                    resolved
                        .products
                        .insert(key.clone(), ProductConfig::from_overlay(key, ov)?);
                }
            }
        }

        crate::gifts::apply_gifts(
            &mut resolved,
            &self.gifts.label,
            &self.gifts.key_prefix,
            self.gifts.capitalize_key,
        )?;

        Ok(resolved)
    }

    /// Serialise the whole model to a new file. For commands that *create* a
    /// config (`init`), where there is no document to preserve.
    ///
    /// Commands that write back to a config the user already owns must use
    /// [`Config::save_in_place`] instead — this one reorders keys and drops
    /// both comments and unmodeled fields.
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }

    /// Write the resource tables back into an existing `rbxshop.toml`,
    /// editing the document rather than reserialising it. Keeps comments, key
    /// order, and every key the model does not know about.
    pub fn save_in_place(&self, path: &Path) -> Result<()> {
        crate::toml_write::save_in_place(self, path, &[])
    }

    /// [`Config::save_in_place`], plus the key moves that `rename` performed,
    /// so a renamed entry keeps its comments and its place in the file.
    pub fn save_in_place_renaming(&self, path: &Path, renames: &[KeyRename]) -> Result<()> {
        crate::toml_write::save_in_place(self, path, renames)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        warn_unknown_root_keys(path, &content);
        Ok(config)
    }

    /// Load the main file plus every file listed under its `[include].files`
    /// (resolved relative to `path`'s directory), each kept as its own
    /// unmerged `ConfigFile` — index 0 is always the main file. Included
    /// files must be "pure" resource files — `experience`/`owner`/
    /// `codegen`/`icons`/`gifts`/`include` are only ever read from the main
    /// file and rejected elsewhere.
    ///
    /// Used by `pull`/`rename`, which write back to the config and need to
    /// know which physical file currently owns a given key (see
    /// `find_owner`/`find_overlay_owner`). `load_merged` below is the
    /// read-only counterpart used by commands that never write back.
    pub fn load_all(path: &Path) -> Result<Vec<ConfigFile>> {
        let main = Self::load(path)?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let include_files = main.include.files.clone();

        let mut files = vec![ConfigFile {
            path: path.to_path_buf(),
            config: main,
        }];

        for rel in include_files {
            let inc_path = dir.join(&rel);
            let inc = Self::load(&inc_path)
                .with_context(|| format!("Failed to load included file {}", inc_path.display()))?;

            if inc.experience.is_some()
                || inc.owner.is_some()
                || !inc.codegen.is_default()
                || !inc.icons.is_default()
                || !inc.gifts.is_default()
                || !inc.include.is_empty()
            {
                bail!(
                    "Included file {} may only contain [passes.*], [badges.*], [products.*], \
                     and their [envs.<name>.*] overlays — experience/owner/codegen/icons/\
                     gifts/include belong in the main config file.",
                    inc_path.display()
                );
            }

            files.push(ConfigFile {
                path: inc_path,
                config: inc,
            });
        }

        Ok(files)
    }

    /// Merge already-loaded files (see `load_all`) into one `Config` view,
    /// by cloning entries — `files` stays usable by the caller afterward.
    /// Errors if the same resource key, or the same env+key overlay, is
    /// declared in more than one file.
    pub fn merge_loaded(files: &[ConfigFile]) -> Result<Config> {
        let mut main = files[0].config.clone();
        for file in &files[1..] {
            for (env_name, inc_overlay) in &file.config.envs {
                let main_overlay = main.envs.entry(env_name.clone()).or_default();
                for (key, value) in &inc_overlay.passes {
                    if main_overlay
                        .passes
                        .insert(key.clone(), value.clone())
                        .is_some()
                    {
                        bail!(
                            "Overlay [envs.{env_name}.passes.{key}] is declared in more than \
                             one file (main config or {}) — each (env, key) pair may only be \
                             declared once across all included files.",
                            file.path.display()
                        );
                    }
                }
                for (key, value) in &inc_overlay.badges {
                    if main_overlay
                        .badges
                        .insert(key.clone(), value.clone())
                        .is_some()
                    {
                        bail!(
                            "Overlay [envs.{env_name}.badges.{key}] is declared in more than \
                             one file (main config or {}) — each (env, key) pair may only be \
                             declared once across all included files.",
                            file.path.display()
                        );
                    }
                }
                for (key, value) in &inc_overlay.products {
                    if main_overlay
                        .products
                        .insert(key.clone(), value.clone())
                        .is_some()
                    {
                        bail!(
                            "Overlay [envs.{env_name}.products.{key}] is declared in more than \
                             one file (main config or {}) — each (env, key) pair may only be \
                             declared once across all included files.",
                            file.path.display()
                        );
                    }
                }
            }

            for (key, value) in &file.config.passes {
                if main.passes.insert(key.clone(), value.clone()).is_some() {
                    bail!(
                        "Pass '{key}' is declared in both the main config and {} — \
                         each resource key may only be declared once across all included files.",
                        file.path.display()
                    );
                }
            }
            for (key, value) in &file.config.badges {
                if main.badges.insert(key.clone(), value.clone()).is_some() {
                    bail!(
                        "Badge '{key}' is declared in both the main config and {} — \
                         each resource key may only be declared once across all included files.",
                        file.path.display()
                    );
                }
            }
            for (key, value) in &file.config.products {
                if main.products.insert(key.clone(), value.clone()).is_some() {
                    bail!(
                        "Product '{key}' is declared in both the main config and {} — \
                         each resource key may only be declared once across all included files.",
                        file.path.display()
                    );
                }
            }
        }

        Ok(main)
    }

    /// Load `path` and merge in every `[include].files` entry into one
    /// `Config` view. This is the read-only path used by `sync`/`check`/
    /// `show`/`list`/codegen, which never write the config back. `pull` and
    /// `rename` use `load_all` + `merge_loaded` instead, since they need to
    /// route writes back to whichever file currently owns a key.
    pub fn load_merged(path: &Path) -> Result<Self> {
        let files = Self::load_all(path)?;
        Self::merge_loaded(&files)
    }

    /// Validate that referenced icon paths exist on disk for a resolved env.
    /// Call after `resolve_env` so per-env icon overrides are checked.
    pub fn validate_icon_paths(resources: &ResolvedResources, config_dir: &Path) -> Result<()> {
        for (name, pass) in &resources.passes {
            if let Some(icon) = &pass.icon {
                let full = config_dir.join(icon);
                if !full.exists() {
                    bail!(
                        "Pass '{}': icon path does not exist: {}",
                        name,
                        full.display()
                    );
                }
            }
        }
        for (name, badge) in &resources.badges {
            if let Some(icon) = &badge.icon {
                let full = config_dir.join(icon);
                if !full.exists() {
                    bail!(
                        "Badge '{}': icon path does not exist: {}",
                        name,
                        full.display()
                    );
                }
            }
        }
        for (name, product) in &resources.products {
            if let Some(icon) = &product.icon {
                let full = config_dir.join(icon);
                if !full.exists() {
                    bail!(
                        "Product '{}': icon path does not exist: {}",
                        name,
                        full.display()
                    );
                }
            }
        }
        Ok(())
    }

    pub fn default_template() -> String {
        r#"# rbx shop configuration
# Manages Roblox game passes, badges, and developer products via the Open Cloud API.
#
# Two ways to target a universe:
#   1. Multi-env (recommended): pass `--env <name>`. universe_id resolves from
#      rbxplace.toml [<env>].universe_id. Omit [experience] in that case.
#   2. Standalone: keep [experience] below.
# Per-env overrides go under [envs.<name>] (see bottom of file).

[experience]
universe_id = 0        # Your Roblox universe ID (omit if you always use --env)

# Owner is global (same for every env). The one thing it decides here is the
# payment source for badge creation, and that follows from ownership rather
# than being a choice: Roblox pays a group-owned game's badge from group funds
# and a user-owned game's from the user's, with no way to cross them.
# Optional: when omitted, rbx shop falls back to [owner] in rbxplace.toml
# (per-env [<env>.owner] first, then top-level [owner]).
# [owner]
# type = "user"          # "user" or "group"
# id = 0                 # Your Roblox user or group ID

# Codegen — generate a Luau module folder with all asset IDs.
# `output` is a FOLDER path (no extension). It will contain:
#   <output>/init.luau     -- dispatcher + exported type
#   <output>/<env>.luau    -- per-env IDs (0-stubs for missing resources)
# [codegen]
# output = "src/shared/GameIds"
# typescript = false           # Also generate <output>/init.d.ts
# style = "flat"               # "flat" (default) or "nested"
#                              # flat:   GameIds.passes["VIP"]   — path-like keys
#                              # nested: GameIds.passes.VIP      — nested tables
#
# Custom paths — dot-separated, used as prefix (flat) or nesting (nested)
# [codegen.paths]
# passes = "player.vips"
# products = "shop.items"
#
# Extra entries — pre-existing assets injected into every env's module
# [codegen.extra]
# "passes.legacy_vip" = 1234567

# Icon settings
# [icons]
# bleed = true         # Apply alpha bleed (fixes resize artifacts)
# dir = "icons"        # Directory for downloaded icons

# Gift products — see `create_gift` below. `label` is prefixed to the
# source's display name for the derived product (e.g. "VIP Pass" becomes
# "[GIFT] VIP Pass"). `key_prefix` does the same for the codegen/lockfile
# key (e.g. "VIP" becomes "GiftVIP"); `capitalize_key` uppercases just the
# derived copy's first letter (useful with a lowercase key_prefix).
# [gifts]
# label = "[GIFT] "
# key_prefix = "Gift"
# capitalize_key = false

# Game Passes
# [passes.VIP]
# name = "VIP Pass"       # optional — defaults to "VIP"
# price = 499
# description = "VIP access"
# icon = "icons/vip.png"
# for_sale = true          # optional — defaults to true
# regional_pricing = false # optional — defaults to false
# create_gift = false      # optional — derive a "GiftVIP" dev product twin
# path = "shop.specials"   # optional — override codegen path

# Badges
# [badges.Welcome]
# name = "Welcome Badge"  # optional — defaults to "Welcome"
# description = "Welcome to the game!"
# icon = "icons/welcome.png"
# enabled = true
# path = "rewards"          # optional — override codegen path

# Developer Products
# [products.Coins100]
# name = "100 Coins"      # optional — defaults to "Coins100"
# price = 99
# description = "100 coins"
# icon = "icons/coins.png"
# for_sale = true
# regional_pricing = false
# store_page = false
# create_gift = false      # optional — derive a "GiftCoins100" dev product twin
# path = "shop.specials"

# Per-env overrides. Layered on top of base when `--env <name>` is passed.
# Pull writes here automatically when a remote value diverges from base.
# [envs.prod.passes.VIP]
# price = 999             # prod-only price override
#
# [envs.dev.passes.BetaPass]
# price = 0               # pass exclusive to dev env
# description = "Beta tester perks"
# icon = "icons/beta.png"
"#
        .to_string()
    }
}
