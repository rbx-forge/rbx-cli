//! What `rbx config get --json`, `rbx config list --json` and
//! `rbx config versions --json` write to stdout.
//!
//! All three report the **published** side: what Roblox is serving to players
//! right now, and how it got there. None of them reads `rbxconfig.toml`, and
//! that shows in the documents: there is no `config_file` field anywhere in
//! this module, because no local file was opened.
//!
//! Which matters because `rbx check --json` already has a row for this domain
//! and calls it `config/live`. That row compares the local file against the
//! published config and answers with `outcome`, `summary` and `details`.
//! Nothing here is named any of those three: a declared-versus-live verdict
//! and a snapshot of live are different documents, and a `jq` filter must not
//! be able to half-read one as the other. What is shared is deliberate:
//! `schema_version` comes from the same constant, `env` is the same env name
//! omitted under the same rule, and `live` keeps meaning what `check` already
//! made it mean.
//!
//! `get` and `list` emit the *same* type. Their human forms differ only in
//! layout, and giving them one document means a consumer writes one filter and
//! the two cannot drift apart field by field. Field names are documented in
//! `docs/config.md` and are the compatibility surface.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value as Json;

use rbx_core::output::SCHEMA_VERSION;

use crate::api::models::{ConfigSnapshot, RevisionEntry};
use crate::value::type_label;

/// One `config get` or `config list` invocation: the published config.
#[derive(Debug, Serialize)]
pub struct LiveDocument {
    pub schema_version: u32,
    /// The env named on the command line. **Absent** when the universe was
    /// given directly with `--universe-id` and no `--env` was needed: the
    /// human form prints a `<universe-id>` placeholder there, which is a label
    /// and not an env name, so the document omits the key instead. Same
    /// omission rule `rbx check --json` uses for its own `env`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    pub universe_id: u64,
    /// Roblox's `configVersion` for this snapshot, in snake case like every
    /// other field here. The number that tells two snapshots apart.
    pub config_version: u64,
    /// The key asked for. **Absent** for `config list`, and for `config get`
    /// without a key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// That key's value, raw, exactly as `config get <key>` prints it on its
    /// own. **Absent** whenever `key` is.
    ///
    /// Which of the two forms you get is decided by the invocation and never
    /// by the data: `config list` omits `value` even against a config holding
    /// exactly one entry, so a script cannot start working by accident and
    /// stop when a second key is published.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Json>,
    /// The entries answered, keyed by config key: one when `key` is set, all
    /// of them otherwise. An object, not an array, because a config key is
    /// what a consumer looks up.
    ///
    /// No `totals` object. `rbx check --json` has one and it counts outcomes;
    /// one here would count keys under the same name. `.entries | length` is
    /// the count, and it cannot be misread.
    pub entries: BTreeMap<String, Entry>,
}

/// One published entry.
///
/// Carries the type label the human listing prints, so a consumer does not
/// have to re-derive "is this an object or a string" from the value, and so
/// `get` and `list` can share one shape rather than one of them being a bare
/// value map.
#[derive(Debug, Serialize)]
pub struct Entry {
    /// `bool`, `number`, `string`, `array`, `object`, or `null`: the same
    /// words the human listing prints in its type column.
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub value: Json,
}

impl From<&Json> for Entry {
    fn from(value: &Json) -> Self {
        Self {
            kind: type_label(value),
            value: value.clone(),
        }
    }
}

impl LiveDocument {
    /// The whole published config: `config list`, and `config get` with no key.
    pub fn snapshot(env: Option<&str>, universe_id: u64, snapshot: &ConfigSnapshot) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            env: env.map(str::to_string),
            universe_id,
            config_version: snapshot.metadata.config_version,
            key: None,
            value: None,
            entries: snapshot
                .entries
                .iter()
                .map(|(key, value)| (key.clone(), Entry::from(value)))
                .collect(),
        }
    }

    /// One key: `config get <key>`. Carries both the `value` shortcut and a
    /// one-entry `entries`, so one filter reads both forms.
    pub fn single(
        env: Option<&str>,
        universe_id: u64,
        snapshot: &ConfigSnapshot,
        key: &str,
        value: &Json,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            env: env.map(str::to_string),
            universe_id,
            config_version: snapshot.metadata.config_version,
            key: Some(key.to_string()),
            value: Some(value.clone()),
            entries: [(key.to_string(), Entry::from(value))]
                .into_iter()
                .collect(),
        }
    }
}

/// One `config versions` invocation: the publish history.
///
/// History, not state. It is the one document in this module that is not a
/// snapshot of anything, which is why it shares no field with `LiveDocument`
/// beyond the envelope.
#[derive(Debug, Serialize)]
pub struct VersionsDocument {
    pub schema_version: u32,
    /// **Absent** under a bare `--universe-id`, same rule as everywhere else.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    pub universe_id: u64,
    /// The `--count` in force for this run.
    pub count: usize,
    /// True when the run stopped because it hit `--count` rather than because
    /// it ran out of revisions. Raise `--count` to see further back.
    pub count_reached: bool,
    /// Newest first, the order Roblox returns and the order the human listing
    /// prints.
    pub revisions: Vec<Revision>,
}

/// One revision.
#[derive(Debug, Serialize)]
pub struct Revision {
    pub revision_id: String,
    pub version: u64,
    /// The timestamp Roblox sent, untouched. The human listing swaps the `T`
    /// for a space and drops the `Z` to be read; a consumer wants the ISO
    /// string back.
    pub time: String,
    /// The publish message. **Absent** when the publish carried none, which is
    /// not the same fact as an empty one: the human listing renders both as
    /// `(no message)` and the document keeps them apart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The keys this revision changed, sorted so two runs against the same
    /// history produce the same bytes. The human listing prints the count;
    /// `.changed_keys | length` is that count, and the names are what a
    /// changelog actually wants. Always an array, empty included.
    pub changed_keys: Vec<String>,
    /// True for the revision currently serving players: the one the human
    /// listing tags `published`. Derived rather than left to be inferred from
    /// the position, so a consumer that sorted the array still knows.
    pub published: bool,
}

impl VersionsDocument {
    pub fn new(
        env: Option<&str>,
        universe_id: u64,
        count: usize,
        revisions: &[RevisionEntry],
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            env: env.map(str::to_string),
            universe_id,
            count,
            count_reached: revisions.len() >= count,
            revisions: revisions
                .iter()
                .enumerate()
                .map(|(index, entry)| Revision {
                    revision_id: entry.revision_id.clone(),
                    version: entry.version,
                    time: entry.time.clone(),
                    message: entry.message.clone(),
                    changed_keys: {
                        let mut keys: Vec<String> = entry.changes.keys().cloned().collect();
                        keys.sort();
                        keys
                    },
                    published: index == 0,
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

    fn snapshot() -> ConfigSnapshot {
        serde_json::from_value(serde_json::json!({
            "metadata": { "configVersion": 14 },
            "entries": {
                "features.new_xp_popup": true,
                "ops.teleport_place_id": 12345,
                "balance.speed_multipliers": { "tier_1": 1.5 }
            }
        }))
        .expect("fixture")
    }

    #[test]
    fn the_live_envelope_carries_the_documented_fields() {
        let doc = parsed(&LiveDocument::snapshot(Some("dev"), 100, &snapshot()));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["env"], "dev");
        assert_eq!(doc["universe_id"], 100);
        assert_eq!(doc["config_version"], 14);
        assert_eq!(doc["entries"]["features.new_xp_popup"]["type"], "bool");
        assert_eq!(doc["entries"]["features.new_xp_popup"]["value"], true);
        assert_eq!(doc["entries"]["ops.teleport_place_id"]["type"], "number");
        assert_eq!(
            doc["entries"]["balance.speed_multipliers"]["type"],
            "object"
        );
        assert_eq!(
            doc["entries"]["balance.speed_multipliers"]["value"]["tier_1"],
            1.5
        );
    }

    /// The words `rbx check --json` owns. A snapshot of live says nothing
    /// about whether the local file agrees with it, so a filter reaching for
    /// a verdict must find nothing rather than something plausible.
    #[test]
    fn a_live_document_carries_no_drift_vocabulary_and_no_local_file() {
        let doc = parsed(&LiveDocument::snapshot(Some("dev"), 100, &snapshot()));

        for word in [
            "outcome", "checks", "check", "tool", "summary", "details", "totals",
        ] {
            assert!(doc.get(word).is_none(), "{word} must not appear: {doc}");
        }
        // Nothing local was read, so nothing claims one was.
        assert!(doc.get("config_file").is_none(), "{doc}");
    }

    #[test]
    fn a_single_key_carries_both_the_value_and_a_one_entry_entries() {
        let snapshot = snapshot();
        let value = snapshot.entries["ops.teleport_place_id"].clone();
        let doc = parsed(&LiveDocument::single(
            Some("dev"),
            100,
            &snapshot,
            "ops.teleport_place_id",
            &value,
        ));

        assert_eq!(doc["key"], "ops.teleport_place_id");
        assert_eq!(doc["value"], 12345);
        assert_eq!(doc["entries"].as_object().map(|m| m.len()), Some(1));
        assert_eq!(doc["entries"]["ops.teleport_place_id"]["value"], 12345);
        // The envelope is the same one `list` emits, down to the version.
        assert_eq!(doc["config_version"], 14);
    }

    /// Shape follows the invocation, not the data: a whole-config read never
    /// emits `key`/`value`, so a filter written against a one-key config keeps
    /// working when a second key is published.
    #[test]
    fn a_whole_config_read_omits_the_single_value_even_for_one_entry() {
        let snapshot: ConfigSnapshot = serde_json::from_value(serde_json::json!({
            "metadata": { "configVersion": 1 },
            "entries": { "only": "one" }
        }))
        .expect("fixture");

        let doc = parsed(&LiveDocument::snapshot(None, 100, &snapshot));

        assert!(doc.get("key").is_none(), "{doc}");
        assert!(doc.get("value").is_none(), "{doc}");
        assert_eq!(doc["entries"]["only"]["value"], "one");
    }

    /// Nothing published yet is a document with no entries, not a missing
    /// document: `.entries | length == 0` has to be answerable.
    #[test]
    fn an_unpublished_config_is_an_empty_object_not_an_absent_one() {
        let doc = parsed(&LiveDocument::snapshot(
            None,
            100,
            &ConfigSnapshot::default(),
        ));

        assert_eq!(doc["entries"].as_object().map(|m| m.len()), Some(0));
        assert_eq!(doc["config_version"], 0);
        assert!(doc.get("env").is_none(), "{doc}");
    }

    fn revisions() -> Vec<RevisionEntry> {
        serde_json::from_value(serde_json::json!([
            {
                "revisionId": "aaaaaaaa-1111",
                "version": 14,
                "time": "2026-08-15T09:30:00Z",
                "message": "raise the cap",
                "changes": { "b.key": {}, "a.key": {} }
            },
            {
                "revisionId": "bbbbbbbb-2222",
                "version": 13,
                "time": "2026-08-14T09:30:00Z"
            }
        ]))
        .expect("fixture")
    }

    #[test]
    fn the_versions_envelope_carries_the_documented_fields() {
        let doc = parsed(&VersionsDocument::new(Some("dev"), 100, 20, &revisions()));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["env"], "dev");
        assert_eq!(doc["universe_id"], 100);
        assert_eq!(doc["count"], 20);
        assert_eq!(doc["count_reached"], false);
        assert_eq!(doc["revisions"][0]["revision_id"], "aaaaaaaa-1111");
        assert_eq!(doc["revisions"][0]["version"], 14);
        assert_eq!(doc["revisions"][0]["time"], "2026-08-15T09:30:00Z");
        assert_eq!(doc["revisions"][0]["message"], "raise the cap");
        assert_eq!(doc["revisions"][1]["version"], 13);
    }

    /// Sorted, so the same history renders the same bytes twice running:
    /// `changes` arrives as a hash map and would not.
    #[test]
    fn changed_keys_are_sorted_and_always_an_array() {
        let doc = parsed(&VersionsDocument::new(None, 100, 20, &revisions()));

        assert_eq!(doc["revisions"][0]["changed_keys"][0], "a.key");
        assert_eq!(doc["revisions"][0]["changed_keys"][1], "b.key");
        assert_eq!(
            doc["revisions"][1]["changed_keys"].as_array().map(Vec::len),
            Some(0)
        );
    }

    /// A publish with no message is not a publish with an empty one.
    #[test]
    fn a_missing_message_is_omitted_rather_than_rendered_as_a_placeholder() {
        let doc = parsed(&VersionsDocument::new(None, 100, 20, &revisions()));

        assert!(doc["revisions"][1].get("message").is_none(), "{doc}");
    }

    /// Which revision is live is a fact about the history, so the document
    /// states it instead of leaving it to be inferred from the position.
    #[test]
    fn only_the_newest_revision_is_marked_published() {
        let doc = parsed(&VersionsDocument::new(None, 100, 20, &revisions()));

        assert_eq!(doc["revisions"][0]["published"], true);
        assert_eq!(doc["revisions"][1]["published"], false);
    }

    #[test]
    fn hitting_the_count_is_reported_rather_than_left_to_be_inferred() {
        assert!(VersionsDocument::new(None, 100, 2, &revisions()).count_reached);
        assert!(!VersionsDocument::new(None, 100, 3, &revisions()).count_reached);
    }

    /// A universe with no publishes is an empty list, not an error.
    #[test]
    fn no_revisions_is_an_empty_list_not_an_absent_one() {
        let doc = parsed(&VersionsDocument::new(None, 100, 20, &[]));

        assert_eq!(doc["revisions"].as_array().map(Vec::len), Some(0));
        assert_eq!(doc["count_reached"], false);
    }
}
