//! A config split across files by `[include]`, and the bookkeeping that
//! needs.
//!
//! Once a resource can live in any of several files, every write has to find
//! the one that owns it: `find_owner` and `find_overlay_owner` are that
//! question, and `ResourceKind` is what makes it askable without three copies
//! of each function.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Every table `rbxshop.toml` gives a meaning to at the top level.
///
use super::*;

/// One physical file backing part of a (possibly split) config, alongside
/// its raw, unmerged contents. Produced by `Config::load_all`: index 0 in
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

    /// Singular, lowercase: the word used in progress lines and errors.
    pub fn label(self) -> &'static str {
        match self {
            ResourceKind::Pass => "pass",
            ResourceKind::Badge => "badge",
            ResourceKind::Product => "product",
        }
    }

    /// Plural, lowercase, for prose naming a whole collection.
    ///
    /// Spelled out rather than `format!("{}s", label())`, which produces
    /// "passs" and shipped to users in the preflight scope error before anybody
    /// read it aloud.
    pub fn plural(self) -> &'static str {
        // Delegates rather than repeating the three strings. `section()` is
        // already the plural (it names the TOML table) and two independent
        // matches over the same variants returning the same words drift the
        // first time a kind is renamed in one and not the other.
        self.section()
    }

    /// Singular, capitalized, for a message that starts with the kind.
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
