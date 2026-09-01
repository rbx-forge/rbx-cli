//! The merged view for one env, built from the base plus its overlay.
//!
//! What diff, sync and codegen read, so none of them has to know the overlay
//! system exists.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};

pub use crate::toml_write::KeyRename;

/// Every table `rbxshop.toml` gives a meaning to at the top level.
///
use super::*;

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
    /// [`Config::save_in_place`] instead: this one reorders keys and drops
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
    /// unmerged `ConfigFile`: index 0 is always the main file. Included
    /// files must be "pure" resource files: `experience`/`owner`/
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
                     and their [envs.<name>.*] overlays: experience/owner/codegen/icons/\
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
    /// by cloning entries: `files` stays usable by the caller afterward.
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
                             one file (main config or {}): each (env, key) pair may only be \
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
                             one file (main config or {}): each (env, key) pair may only be \
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
                             one file (main config or {}): each (env, key) pair may only be \
                             declared once across all included files.",
                            file.path.display()
                        );
                    }
                }
            }

            for (key, value) in &file.config.passes {
                if main.passes.insert(key.clone(), value.clone()).is_some() {
                    bail!(
                        "Pass '{key}' is declared in both the main config and {}: \
                         each resource key may only be declared once across all included files.",
                        file.path.display()
                    );
                }
            }
            for (key, value) in &file.config.badges {
                if main.badges.insert(key.clone(), value.clone()).is_some() {
                    bail!(
                        "Badge '{key}' is declared in both the main config and {}: \
                         each resource key may only be declared once across all included files.",
                        file.path.display()
                    );
                }
            }
            for (key, value) in &file.config.products {
                if main.products.insert(key.clone(), value.clone()).is_some() {
                    bail!(
                        "Product '{key}' is declared in both the main config and {}: \
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

# Codegen: generate a Luau module folder with all asset IDs.
# `output` is a FOLDER path (no extension). It will contain:
#   <output>/init.luau     -- dispatcher + exported type
#   <output>/<env>.luau    -- per-env IDs (0-stubs for missing resources)
# [codegen]
# output = "src/shared/GameIds"
# typescript = false           # Also generate <output>/init.d.ts
# style = "flat"               # "flat" (default) or "nested"
#                              # flat:   GameIds.passes["VIP"]: path-like keys
#                              # nested: GameIds.passes.VIP: nested tables
#
# Custom paths: dot-separated, used as prefix (flat) or nesting (nested)
# [codegen.paths]
# passes = "player.vips"
# products = "shop.items"
#
# Extra entries: pre-existing assets injected into every env's module
# [codegen.extra]
# "passes.legacy_vip" = 1234567

# Icon settings
# [icons]
# bleed = true         # Apply alpha bleed (fixes resize artifacts)
# dir = "icons"        # Directory for downloaded icons

# Gift products: see `create_gift` below. `label` is prefixed to the
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
# name = "VIP Pass"       # optional: defaults to "VIP"
# price = 499
# description = "VIP access"
# icon = "icons/vip.png"
# for_sale = true          # optional: defaults to true
# regional_pricing = false # optional: defaults to false
# create_gift = false      # optional: derive a "GiftVIP" dev product twin
# path = "shop.specials"   # optional: override codegen path

# Badges
# [badges.Welcome]
# name = "Welcome Badge"  # optional: defaults to "Welcome"
# description = "Welcome to the game!"
# icon = "icons/welcome.png"
# enabled = true
# path = "rewards"          # optional: override codegen path

# Developer Products
# [products.Coins100]
# name = "100 Coins"      # optional: defaults to "Coins100"
# price = 99
# description = "100 coins"
# icon = "icons/coins.png"
# for_sale = true
# regional_pricing = false
# store_page = false
# create_gift = false      # optional: derive a "GiftCoins100" dev product twin
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
