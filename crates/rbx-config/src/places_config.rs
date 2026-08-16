/// Minimal rbxplace.toml reader — only needs universe_id per env.
/// Shares the same file format as rbxplace.
use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rbx_core::owner::Owner;
use rbx_core::places::PlacesCodegen;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct PlacesConfig {
    /// Reserved top-level `[owner]` block (shared source of truth for the
    /// project owner; see `rbx_core::owner`). Consumed here as a reserved key
    /// so it isn't mistaken for an env — rbx-config itself doesn't use it.
    #[serde(default)]
    #[allow(dead_code)]
    pub owner: Option<Owner>,

    /// Reserved top-level `[codegen]` block (`rbx env gen-module`'s output
    /// path). Same deal as `owner`: unused here, but it has to be claimed as a
    /// known key or the flattened map below would try to read it as an env and
    /// fail on the missing `universe_id`.
    #[serde(default)]
    #[allow(dead_code)]
    pub codegen: Option<PlacesCodegen>,

    #[serde(flatten)]
    pub environments: HashMap<String, EnvEntry>,
}

#[derive(Debug, Deserialize)]
pub struct EnvEntry {
    pub universe_id: u64,
    #[serde(default)]
    pub confirm: bool,
    // Other fields (places) are ignored here
    #[serde(flatten)]
    _extra: HashMap<String, toml::Value>,
}

impl PlacesConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        // This struct models three keys of a file that has six; the shared
        // lint is what knows the difference between "rbx-config ignores it"
        // and "nothing reads it".
        rbx_core::places::warn_unknown_keys_in(path, &content);
        Ok(config)
    }

    pub fn get_env(&self, env: &str) -> Result<&EnvEntry> {
        self.environments.get(env).ok_or_else(|| {
            let mut available: Vec<&str> = self.environments.keys().map(|s| s.as_str()).collect();
            available.sort();
            anyhow::anyhow!(
                "Environment '{}' not found in rbxplace.toml.\nAvailable: {}",
                env,
                available.join(", ")
            )
        })
    }

    pub fn universe_id(&self, env: &str) -> Result<u64> {
        self.environments
            .get(env)
            .map(|e| e.universe_id)
            .ok_or_else(|| {
                let mut available: Vec<&str> =
                    self.environments.keys().map(|s| s.as_str()).collect();
                available.sort();
                anyhow::anyhow!(
                    "Environment '{}' not found in rbxplace.toml.\nAvailable: {}",
                    env,
                    available.join(", ")
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_owner_block_is_not_parsed_as_env() {
        // Regression: the suite-wide `[owner]` block must be consumed as a
        // reserved key, otherwise it gets routed into the env map and fails
        // with "missing field `universe_id`".
        let cfg: PlacesConfig = toml::from_str(
            r#"
[owner]
type = "group"
id = 1234567

[dev]
universe_id = 100

[prod]
universe_id = 200
"#,
        )
        .expect("[owner] block must parse");
        assert_eq!(cfg.environments.len(), 2);
        assert!(!cfg.environments.contains_key("owner"));
        assert_eq!(cfg.universe_id("dev").unwrap(), 100);
        assert_eq!(cfg.owner.unwrap().id, 1234567);
    }

    #[test]
    fn parses_without_owner_block() {
        let cfg: PlacesConfig = toml::from_str("[dev]\nuniverse_id = 100\n").unwrap();
        assert!(cfg.owner.is_none());
        assert_eq!(cfg.universe_id("dev").unwrap(), 100);
    }
}
