//! What `rbx env list --json` and `rbx env get --json` write to stdout.
//!
//! These types mirror `rbxplace.toml` rather than re-derive it: an env's
//! `owner` here is the per-env override exactly as the file spells it, not the
//! effective owner after fallback, so `.envs[].owner // .owner` reproduces
//! `resolve_owner` and nothing is lost. Reporting only the resolved value
//! would make the two documents unable to say "this env inherits".
//!
//! The envelope follows `rbx check --json`: `schema_version` first, then named
//! objects all the way down. Field names are documented in `docs/env.md` and
//! are the compatibility surface.

use std::collections::BTreeMap;

use serde::Serialize;

use rbx_core::output::SCHEMA_VERSION;
use rbx_core::owner::Owner;
use rbx_core::places::{Environment, PlacesFile};

/// One `env list` invocation.
#[derive(Debug, Serialize)]
pub struct ListDocument {
    pub schema_version: u32,
    /// The `rbxplace.toml` these envs were read from, as given on the command
    /// line or defaulted. Present because a `--places` mistake otherwise looks
    /// like an empty repo.
    pub places_file: String,
    /// The top-level `owner`. **Absent** when the file sets none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Owner>,
    /// One object per env, in name order — the same order the human listing
    /// uses, so two runs of the same file produce the same bytes. Narrowed to
    /// one entry by `--env <name>`.
    pub envs: Vec<Env>,
}

/// One env, as `rbxplace.toml` declares it.
#[derive(Debug, Serialize)]
pub struct Env {
    /// The section name: what `--env` takes.
    pub name: String,
    pub universe_id: u64,
    /// The `env` rename, when the section name is not what game code matches
    /// on. **Absent** when unset, in which case `name` is the answer.
    #[serde(rename = "env", skip_serializing_if = "Option::is_none")]
    pub env_type: Option<String>,
    /// The per-env `owner` override. **Absent** when this env inherits the
    /// top-level one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Owner>,
    /// Whether writes to this env prompt first.
    pub confirm: bool,
    /// Whether `rbx env gen-module` emits this env. False marks a universe
    /// that only ever receives uploads.
    pub codegen: bool,
    /// Place name to place id. An object, not an array: `--place` names an
    /// entry, so a consumer looks one up rather than walking a list. Empty
    /// for tools that work at universe scope.
    pub places: BTreeMap<String, u64>,
}

impl Env {
    fn new(name: &str, env: &Environment) -> Self {
        Self {
            name: name.to_string(),
            universe_id: env.universe_id,
            env_type: env.env.clone(),
            owner: env.owner,
            confirm: env.confirm(),
            codegen: env.codegen,
            places: env
                .places
                .iter()
                .map(|(place, id)| (place.clone(), *id))
                .collect(),
        }
    }
}

impl ListDocument {
    /// Build the document for `names`, which the caller has already resolved
    /// and ordered.
    ///
    /// Returns `Err` on a name the file does not define, so `--json` reports
    /// the same "Available: ..." error the human form does rather than an
    /// empty list.
    pub fn new(
        places_path: &std::path::Path,
        places: &PlacesFile,
        names: &[String],
    ) -> anyhow::Result<Self> {
        let mut envs = Vec::with_capacity(names.len());
        for name in names {
            envs.push(Env::new(name, places.get(name)?));
        }
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            places_file: places_path.display().to_string(),
            owner: places.owner,
            envs,
        })
    }
}

/// One `env get` invocation.
///
/// `results` is always present and always an array, so one filter reads both
/// forms; `value` is the single-target shortcut. Which of the two you get is
/// decided by the invocation and never by the data — `--env all` omits `value`
/// even against a file with exactly one env, so a script cannot start working
/// by accident and stop when a second env is added.
#[derive(Debug, Serialize)]
pub struct GetDocument {
    pub schema_version: u32,
    /// The field asked for, in its canonical CLI spelling: `universe-id`,
    /// `place-id`, `owner-id`, `owner-type`.
    pub field: String,
    /// The answer, when one env was targeted. **Absent** under `--env all`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// One entry per env answered, in the same order the human form prints.
    pub results: Vec<GetResult>,
}

#[derive(Debug, Serialize)]
pub struct GetResult {
    /// The env this answer is for. **Absent** when the lookup did not need one
    /// — the owner fields resolve from the top-level `owner` block without an
    /// `--env`. Same omission rule `rbx check --json` uses for its own `env`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    /// Always a string: the exact text the human form prints on its own line.
    /// Ids stay strings rather than becoming JSON numbers, so one filter reads
    /// `owner-type` and `universe-id` alike and nothing is rounded.
    pub value: String,
}

impl GetDocument {
    /// The single-target form: `--env <name>`, or no `--env` for an owner
    /// field. Carries both `value` and a one-entry `results`.
    pub fn single(field: &str, env: Option<&str>, value: String) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            field: field.to_string(),
            value: Some(value.clone()),
            results: vec![GetResult {
                env: env.map(str::to_string),
                value,
            }],
        }
    }

    /// The `--env all` form: `results` only, one entry per env.
    pub fn every(field: &str, results: Vec<(String, String)>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            field: field.to_string(),
            value: None,
            results: results
                .into_iter()
                .map(|(env, value)| GetResult {
                    env: Some(env),
                    value,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(document: &impl Serialize) -> serde_json::Value {
        let mut buf = Vec::new();
        rbx_core::output::write_json(&mut buf, document).expect("write");
        serde_json::from_slice(&buf).expect("the document must be valid JSON")
    }

    const SAMPLE: &str = r#"
[owner]
type = "group"
id = 1234567

[dev]
universe_id = 100
[dev.places]
main = 1001
lobby = 1002

[prod]
universe_id = 200
env = "production"
confirm = true
owner = { type = "user", id = 42 }
[prod.places]
only = 2001
"#;

    fn sample() -> PlacesFile {
        toml::from_str(SAMPLE).expect("sample must parse")
    }

    fn list() -> serde_json::Value {
        let places = sample();
        let names = places.env_names();
        parsed(
            &ListDocument::new(std::path::Path::new("rbxplace.toml"), &places, &names)
                .expect("every name comes from the file"),
        )
    }

    #[test]
    fn the_list_envelope_carries_the_documented_fields() {
        let doc = list();

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["places_file"], "rbxplace.toml");
        assert_eq!(doc["owner"]["type"], "group");
        assert_eq!(doc["owner"]["id"], 1_234_567);
        // Name order, so the same file always renders the same bytes.
        assert_eq!(doc["envs"][0]["name"], "dev");
        assert_eq!(doc["envs"][1]["name"], "prod");
        assert_eq!(doc["envs"][0]["universe_id"], 100);
        assert_eq!(doc["envs"][0]["confirm"], false);
        assert_eq!(doc["envs"][0]["codegen"], true);
        assert_eq!(doc["envs"][1]["env"], "production");
        assert_eq!(doc["envs"][1]["confirm"], true);
    }

    /// An inherited owner is absent rather than copied down, so
    /// `.envs[].owner // .owner` reproduces the fallback and a consumer can
    /// still tell "overridden" from "inherited".
    #[test]
    fn a_per_env_owner_is_the_override_and_never_the_inherited_one() {
        let doc = list();

        assert!(doc["envs"][0].get("owner").is_none());
        assert_eq!(doc["envs"][1]["owner"]["type"], "user");
        assert_eq!(doc["envs"][1]["owner"]["id"], 42);
    }

    /// Places are keyed by name because `--place` names one. A positional
    /// array would put the lookup key in a column.
    #[test]
    fn places_are_an_object_keyed_by_place_name() {
        let doc = list();

        assert_eq!(doc["envs"][0]["places"]["main"], 1001);
        assert_eq!(doc["envs"][0]["places"]["lobby"], 1002);
        assert_eq!(doc["envs"][1]["places"]["only"], 2001);
    }

    #[test]
    fn an_unknown_env_errors_rather_than_listing_nothing() {
        let places = sample();
        let err = ListDocument::new(
            std::path::Path::new("rbxplace.toml"),
            &places,
            &["nope".to_string()],
        )
        .expect_err("an unknown env must not render an empty document");

        assert!(format!("{err:#}").contains("Available"), "{err:#}");
    }

    #[test]
    fn a_single_get_carries_both_the_value_and_a_one_entry_results() {
        let doc = parsed(&GetDocument::single(
            "universe-id",
            Some("prod"),
            "200".to_string(),
        ));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["field"], "universe-id");
        assert_eq!(doc["value"], "200");
        assert_eq!(doc["results"][0]["env"], "prod");
        assert_eq!(doc["results"][0]["value"], "200");
    }

    /// The owner fields answer without an env, and say so by omitting the key
    /// rather than by inventing a name for "no env".
    #[test]
    fn an_env_less_lookup_omits_the_env_key() {
        let doc = parsed(&GetDocument::single(
            "owner-type",
            None,
            "group".to_string(),
        ));

        assert_eq!(doc["value"], "group");
        assert!(doc["results"][0].get("env").is_none());
    }

    /// Shape follows the invocation, not the data: `--env all` never emits
    /// `value`, so a filter written against a one-env file keeps working when
    /// a second env lands.
    #[test]
    fn env_all_omits_the_single_value_even_for_one_env() {
        let doc = parsed(&GetDocument::every(
            "universe-id",
            vec![("dev".to_string(), "100".to_string())],
        ));

        assert!(doc.get("value").is_none());
        assert_eq!(doc["results"].as_array().map(Vec::len), Some(1));
        assert_eq!(doc["results"][0]["env"], "dev");
        assert_eq!(doc["results"][0]["value"], "100");
    }

    /// Ids stay strings so one filter reads every field, and a 64-bit id is
    /// never handed to a consumer that would round it.
    #[test]
    fn a_numeric_id_is_reported_as_a_string() {
        let doc = parsed(&GetDocument::single(
            "place-id",
            Some("dev"),
            "1001".to_string(),
        ));

        assert!(doc["value"].is_string(), "{doc}");
    }
}
