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
use std::str::FromStr;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use rbx_core::api::Repository;

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
    /// Which configs repository these entries belong to.
    ///
    /// Absent means `InExperienceConfig`, so every file written before this
    /// field existed keeps meaning what it meant. Held as text rather than a
    /// parsed `Repository` because `Repository` is not a serde type, and
    /// parsing it here would trade the name the reader typed for a serde error
    /// that lists nothing: `declared_repository` does the parse and answers
    /// with the eight names the API exposes.
    ///
    /// Declared **before** the flattened environments on purpose. `serde`
    /// offers a key to the named fields first and only then to the flattened
    /// map, so after the flatten this field would never be reached and
    /// `repository = "..."` would be read as an environment called
    /// `repository`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
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

        // Before the first `[env.entries.*]` header, because a bare key after
        // a table header belongs to that table: written last it would be read
        // back as an entry of whichever env came first. A file that names no
        // repository still renders none, so `pull` keeps writing the bytes it
        // always did.
        if let Some(repository) = &self.repository {
            let literal = toml::Value::String(repository.clone()).to_string();
            let _ = writeln!(out, "repository = {}", literal);
            let _ = writeln!(out);
        }

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

    /// The repository this file names, parsed, or `None` when it names none.
    ///
    /// Wrong name, and this is where it is caught: `Repository::from_str`
    /// answers with the eight the public API exposes, which is more use than
    /// the 400 Roblox would send back for an unknown path segment.
    pub fn declared_repository(&self) -> Result<Option<Repository>> {
        self.repository
            .as_deref()
            .map(Repository::from_str)
            .transpose()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `rbxconfig.toml` written before the field existed. Silence is
    /// not a repository, it is the absence of one: the caller turns that into
    /// `InExperienceConfig`, and only after the flag has had its say.
    #[test]
    fn a_file_that_names_no_repository_declares_none() {
        let file: ConfigsFile = toml::from_str(
            r#"
[dev.entries."features.flag"]
value = true
"#,
        )
        .expect("parse");

        assert_eq!(file.repository, None);
        assert_eq!(file.declared_repository().unwrap(), None);
        assert!(file.environments.contains_key("dev"));
    }

    /// The field is read as a field, not as an environment called
    /// `repository`. That is what declaring it before the `serde(flatten)`
    /// buys, and it is invisible until it breaks.
    #[test]
    fn a_named_repository_is_a_field_and_not_an_environment() {
        let file: ConfigsFile = toml::from_str(
            r#"repository = "DataStoresConfig"

[dev.entries."features.flag"]
value = true
"#,
        )
        .expect("parse");

        assert_eq!(
            file.declared_repository().unwrap(),
            Some(Repository::DataStoresConfig)
        );
        assert_eq!(file.environments.len(), 1);
        assert!(file.environments.contains_key("dev"));
    }

    /// `pull` rewrites the whole file through `render`, so a field it dropped
    /// would turn the next `sync` into a publish into the default repository.
    #[test]
    fn render_round_trips_the_repository() {
        let source = r#"repository = "LeaderboardsConfig"

[dev.entries."features.flag"]
value = true
"#;
        let file: ConfigsFile = toml::from_str(source).expect("parse");

        let rendered = file.render();
        let reread: ConfigsFile = toml::from_str(&rendered).expect("reparse");

        assert_eq!(
            reread.declared_repository().unwrap(),
            Some(Repository::LeaderboardsConfig)
        );
        assert_eq!(reread.environments["dev"].entries.len(), 1);
    }

    /// A file with no repository renders none: `pull` keeps writing the bytes
    /// it always wrote.
    #[test]
    fn render_adds_no_repository_line_to_a_file_that_had_none() {
        let file: ConfigsFile = toml::from_str(
            r#"
[dev.entries."features.flag"]
value = true
"#,
        )
        .expect("parse");

        assert_eq!(
            file.render(),
            "[dev.entries.\"features.flag\"]\nvalue = true\n\n"
        );
    }

    /// A typo is caught here rather than by a 400 from Roblox naming nothing,
    /// and the answer is the list, because a name that is not one of eight is
    /// only actionable next to the eight.
    #[test]
    fn an_unknown_repository_lists_the_eight() {
        let file: ConfigsFile = toml::from_str(r#"repository = "InExperience""#).expect("parse");

        let err = file
            .declared_repository()
            .expect_err("'InExperience' is not a repository")
            .to_string();
        for repository in Repository::ALL {
            assert!(err.contains(repository.as_str()), "{err}");
        }
        assert_eq!(Repository::ALL.len(), 8);
    }
}
