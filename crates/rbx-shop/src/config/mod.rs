use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use colored::Colorize;
use serde::{Deserialize, Serialize};

pub use crate::toml_write::KeyRename;

/// Every table `rbxshop.toml` gives a meaning to at the top level.
///
mod include;
mod overlay;
mod resolved;
mod resources;

// Re-exported flat, so every `crate::config::PassConfig` outside this module
// keeps working. The split is an arrangement of this file, not a change to the
// shape callers see.
pub use self::{include::*, overlay::*, resolved::*, resources::*};

// Serde defaults, needed by this file's own structs and not part of the shape
// callers see, so imported by name rather than swept up in the glob above.
use self::resources::{default_icon_dir, default_true};
/// A key outside this list is kept: `save_in_place` never deletes what it
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

    /// Who owns this project: global to the config, not env-scoped. The one
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
    /// time via `Config::load_merged`. Only meaningful on the main file:
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
    /// modified by this: see `capitalize_key` for the one exception, which
    /// only affects the copy used in this derived string.
    #[serde(default = "default_gift_key_prefix")]
    pub key_prefix: String,

    /// When true, the source key's first letter is uppercased in the
    /// *derived* gift key only (never in the source's own TOML key): e.g.
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
