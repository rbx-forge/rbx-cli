/// rbxconfig.toml: the local source of truth for in-experience tunables, per environment.
///
/// Layout:
/// ```toml
/// [dev.entries."features.new_xp_popup"]
/// value = true
/// description = "Testing new popup: remove in v2"
///
/// [dev.entries."balance.speed_multipliers"]
/// value = { tier_1 = 1.5, tier_2 = 2.0 }
/// description = "Updated for new economy model"
///
/// [prod.entries."features.new_xp_popup"]
/// value = false
/// description = "Disabled in prod until stable"
/// ```
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::value::{json_to_toml, toml_to_json};

pub const CONFIG_NAME: &str = "rbxconfig.toml";

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EntryConfig {
    /// The value published for this key. Any TOML value: a bool, a number, a
    /// string, or a table for structured config.
    ///
    /// Described to the schema as an arbitrary JSON value, which is what it
    /// is: the Configs API stores JSON, and constraining the shape here would
    /// reject entries the tool publishes happily.
    #[cfg_attr(feature = "schema", schemars(with = "serde_json::Value"))]
    pub value: toml::Value,
    /// Why this entry exists, for whoever reads the diff. Not sent to Roblox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct EnvConfig {
    #[serde(default)]
    pub entries: BTreeMap<String, EntryConfig>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct ConfigsFile {
    #[serde(flatten)]
    pub environments: HashMap<String, EnvConfig>,
}

impl ConfigsFile {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = self.render();
        std::fs::write(path, content).with_context(|| format!("Failed to write {}", path.display()))
    }

    /// Render to TOML manually so that table/array values stay inline next to
    /// `value =`, rather than being promoted to nested sub-sections (which is
    /// what `toml::to_string_pretty` does for any `Value::Table` field).
    fn render(&self) -> String {
        let mut out = String::new();
        let mut envs: Vec<&String> = self.environments.keys().collect();
        envs.sort();

        for env in envs {
            let env_config = &self.environments[env];
            for (key, entry) in &env_config.entries {
                let key_lit = toml::Value::String(key.clone()).to_string();
                let _ = writeln!(out, "[{}.entries.{}]", env, key_lit);
                let _ = writeln!(out, "value = {}", entry.value);
                if let Some(desc) = &entry.description {
                    let desc_lit = toml::Value::String(desc.clone()).to_string();
                    let _ = writeln!(out, "description = {}", desc_lit);
                }
                let _ = writeln!(out);
            }
        }
        out
    }

    /// Get entries for a specific environment.
    pub fn get_env(&self, env: &str) -> Option<&EnvConfig> {
        self.environments.get(env)
    }

    /// Get mutable entries for a specific environment.
    pub fn get_env_mut(&mut self, env: &str) -> &mut EnvConfig {
        self.environments.entry(env.to_string()).or_default()
    }

    /// Convert all entries for an environment to JSON for the Roblox API.
    pub fn entries_as_json(&self, env: &str) -> Result<BTreeMap<String, Json>> {
        let env_config = self
            .get_env(env)
            .ok_or_else(|| anyhow::anyhow!("Environment '{}' not found in rbxconfig.toml", env))?;

        Ok(env_config
            .entries
            .iter()
            .map(|(k, entry_config)| (k.clone(), toml_to_json(entry_config.value.clone())))
            .collect())
    }

    /// Build a ConfigsFile from a JSON entries map (received from the API).
    pub fn from_json_entries(env: &str, entries: BTreeMap<String, Json>) -> Self {
        let mut file = ConfigsFile::default();
        file.replace_env_from_json(env, entries);
        file
    }

    /// Replace the entries of `env` with the given JSON map. Preserves any
    /// `description` annotations on keys that survive the replacement; other
    /// environments are left untouched. Returns stats for reporting.
    pub fn replace_env_from_json(
        &mut self,
        env: &str,
        entries: BTreeMap<String, Json>,
    ) -> ReplaceStats {
        let prev_descriptions: BTreeMap<String, String> = self
            .get_env(env)
            .map(|cfg| {
                cfg.entries
                    .iter()
                    .filter_map(|(k, e)| e.description.clone().map(|d| (k.clone(), d)))
                    .collect()
            })
            .unwrap_or_default();

        let prev_keys: std::collections::BTreeSet<String> = self
            .get_env(env)
            .map(|cfg| cfg.entries.keys().cloned().collect())
            .unwrap_or_default();

        let new_keys: std::collections::BTreeSet<String> = entries.keys().cloned().collect();

        let added: Vec<String> = new_keys.difference(&prev_keys).cloned().collect();
        let removed: Vec<String> = prev_keys.difference(&new_keys).cloned().collect();
        let preserved_descriptions = new_keys
            .intersection(&prev_keys)
            .filter(|k| prev_descriptions.contains_key(*k))
            .count();

        let env_config = self.get_env_mut(env);
        env_config.entries = entries
            .into_iter()
            .map(|(k, v)| {
                let description = prev_descriptions.get(&k).cloned();
                (
                    k,
                    EntryConfig {
                        value: json_to_toml(v),
                        description,
                    },
                )
            })
            .collect();

        ReplaceStats {
            total: env_config.entries.len(),
            added,
            removed,
            preserved_descriptions,
        }
    }
}

#[derive(Debug)]
pub struct ReplaceStats {
    pub total: usize,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub preserved_descriptions: usize,
}
