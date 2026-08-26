use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use rbx_core::owner::Owner;
use rbx_core::places::PlacesCodegen;

fn default_true() -> bool {
    true
}

/// `skip_serializing_if` for a bool defaulting to true: keeps the default out
/// of the written file.
fn is_true(value: &bool) -> bool {
    *value
}
use serde::{Deserialize, Serialize};

pub const DEFAULT_CONFIG: &str = "rbxplace.toml";

/// Top-level rbxplace.toml: each key is an environment name.
///
/// Example:
/// ```toml
/// [owner]
/// type = "group"
/// id = 1234567
///
/// [prod]
/// universe_id = 9876543210
/// confirm = true
/// places.main = 123456789012345
/// places.lobby = 987654321
///
/// [staging]
/// universe_id = 9876543211
/// places.main = 234567890123456
/// ```
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct PlacesConfig {
    /// Reserved top-level `[owner]` block: the shared source of truth for the
    /// Roblox account/group that owns this project (see `rbx_core::owner`).
    /// rbx-place doesn't act on it, but it must be parsed as a reserved key
    /// (not as an env) and preserved across `save()` round-trips so other
    /// tools (and `rbx place fetch --write`) don't clobber it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Owner>,

    /// Reserved top-level `[codegen]` block (`rbx env gen-module`'s output
    /// path). Like `owner`: not acted on here, but it must be claimed as a
    /// known key so it isn't read as an env, and preserved across `save()`
    /// so `rbx place fetch --write` doesn't drop it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codegen: Option<PlacesCodegen>,

    /// Reserved top-level `[groups]` block: the named env subsets `--env`
    /// accepts (see `rbx_core::places::PlacesFile::groups`).
    ///
    /// Claimed as a known key rather than modelled, for the reason the two
    /// above are: `environments` is a flattened catch-all, so an unclaimed
    /// `[groups]` table is read as an env and fails the whole file on a
    /// missing `universe_id`. That made `--env <group>` unusable here while
    /// the group was resolved perfectly well in rbx-core. Kept (rather than
    /// skipped) so `rbx place fetch --write` round-trips it instead of
    /// deleting somebody's groups.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub groups: HashMap<String, Vec<String>>,

    #[serde(flatten)]
    pub environments: HashMap<String, Environment>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Environment {
    pub universe_id: u64,

    /// Environment type/name (e.g. "dev", "staging", "prod").
    #[serde(default)]
    pub env: Option<String>,

    /// Require interactive confirmation before write operations (upload, rollback).
    #[serde(default)]
    pub confirm: bool,

    /// Map of place name → place ID.
    #[serde(default)]
    pub places: HashMap<String, u64>,

    /// Keep this env out of the generated modules. Not acted on here, but
    /// modelled so `save()` round-trips it instead of deleting it.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub codegen: bool,
}

impl PlacesConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;
        let config: Self = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config: {}", path.display()))?;
        rbx_core::places::warn_unknown_keys_in(path, &content);
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write config: {}", path.display()))?;
        Ok(())
    }

    pub fn get_env(&self, env: &str) -> Result<&Environment> {
        self.environments.get(env).ok_or_else(|| {
            let mut available: Vec<&str> = self.environments.keys().map(|s| s.as_str()).collect();
            available.sort();
            anyhow::anyhow!(
                "Environment '{}' not found in config.\nAvailable: {}",
                env,
                available.join(", ")
            )
        })
    }

    pub fn get_env_mut(&mut self, env: &str) -> Result<&mut Environment> {
        if !self.environments.contains_key(env) {
            let mut available: Vec<String> = self.environments.keys().cloned().collect();
            available.sort();
            anyhow::bail!(
                "Environment '{}' not found in config.\nAvailable: {}",
                env,
                available.join(", ")
            );
        }
        Ok(self.environments.get_mut(env).unwrap())
    }
}

impl Environment {
    /// Resolve a place name + ID from the optional `--place` flag.
    /// If the env has exactly one place and `place` is None, returns that place.
    pub fn resolve_place(&self, place: Option<&str>) -> Result<(String, u64)> {
        match place {
            Some(name) => {
                let id = self.places.get(name).ok_or_else(|| {
                    let mut available: Vec<&str> = self.places.keys().map(|s| s.as_str()).collect();
                    available.sort();
                    anyhow::anyhow!(
                        "Place '{}' not found.\nAvailable: {}",
                        name,
                        available.join(", ")
                    )
                })?;
                Ok((name.to_string(), *id))
            }
            None if self.places.len() == 1 => {
                let (name, id) = self.places.iter().next().unwrap();
                Ok((name.clone(), *id))
            }
            None if self.places.is_empty() => {
                bail!("No places defined in this environment")
            }
            None => {
                let mut available: Vec<&str> = self.places.keys().map(|s| s.as_str()).collect();
                available.sort();
                bail!(
                    "Multiple places defined: specify one with --place: {}",
                    available.join(", ")
                )
            }
        }
    }

    /// Returns all places sorted by name.
    pub fn all_places_sorted(&self) -> Vec<(String, u64)> {
        let mut entries: Vec<(String, u64)> =
            self.places.iter().map(|(n, id)| (n.clone(), *id)).collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }
}
