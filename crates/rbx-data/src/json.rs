//! What `rbx data get`, `list`, `revisions` and `diff` write to stdout under
//! `--json`.
//!
//! The envelope follows `rbx check --json`: `schema_version` first, then named
//! objects all the way down, optional fields omitted rather than emitted as
//! `null`. Field names are documented in `docs/ops/data.md` and are the
//! compatibility surface.
//!
//! ## The stored value is nested, not stringified
//!
//! `data get` answers with whatever the game stored, and that is already JSON.
//! It goes into the document as JSON, under a `value` key of our own, rather
//! than as a string carrying an escaped copy of itself.
//!
//! The objection to nesting is real, but it is an objection to *spreading*.
//! Keys nobody here controls have no business at the top level of a document
//! whose other keys are a promise. Under a dedicated `value` they cannot
//! collide with anything, because nothing of ours lives inside it: an entry
//! holding `{"schema_version": 9}` reads back as `.value.schema_version` and
//! touches nothing. Stringifying would buy back a namespace that was never at
//! risk, and would cost the thing the flag exists for: `jq .value.coins`
//! becomes `jq -r .value | jq .coins`, and a profile stored as `500` comes back
//! as `"500"`, which is a different fact.
//!
//! It is also what the human form already does. `data get` prints the value
//! pretty-printed as JSON on stdout and nothing else, so `--json` wraps that
//! output rather than re-encoding it.
//!
//! ## What is deliberately absent
//!
//! These commands read real player data, so a document says no more than the
//! human form already says out loud.
//!
//! `users`: the `users/156` association Roblox answers a player's data request
//! from, and `attributes` are on every entry this crate fetches, and are in
//! none of these documents. `data get` has never printed them, and a second
//! player id landing in whatever aggregator eats this output is not a field
//! anybody asked for. `path`, `etag` and `create_time` are absent for the
//! duller version of the same reason: unprinted today, so unpromised today.
//!
//! `data diff` carries the two file paths it already prints and neither side's
//! value: the values are on disk, where the human form leaves them.

use serde::Serialize;

use rbx_core::output::SCHEMA_VERSION;

use crate::model::{DataStore, DataStoreEntry};

/// The store a document is about.
///
/// Both come straight off the command line, and both decide which keys are
/// even visible: a key written under one scope cannot be read from another.
/// Naming them back makes a saved document say what it is a document of,
/// which a file called `dump.json` otherwise does not.
#[derive(Debug, Clone)]
pub struct Store {
    pub datastore: String,
    pub scope: String,
}

/// One `data list` invocation.
#[derive(Debug, Serialize)]
pub struct ListDocument {
    pub schema_version: u32,
    pub datastore: String,
    pub scope: String,
    /// The `--prefix` in force. **Absent** when the listing was unfiltered,
    /// which is not the same as a prefix of `""`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    /// Whether soft-deleted keys were included.
    pub show_deleted: bool,
    /// The `--limit` in force for this run.
    pub limit: u32,
    /// True when the run stopped because it hit `--limit` rather than because
    /// it ran out of keys. Raise `--limit` to see the rest.
    pub limit_reached: bool,
    /// Rows in `entries`.
    pub count: usize,
    /// One object per key, in the order Roblox returned them.
    pub entries: Vec<ListEntry>,
}

/// One key in a listing.
///
/// An object rather than a bare string, because a listing carries more than an
/// id upstream and this is where that would land. A consumer reads each
/// entry's `id` today and keeps working the day a second field appears.
#[derive(Debug, Serialize)]
pub struct ListEntry {
    /// The entry key, with any `@revision` suffix stripped: what `data get`
    /// takes.
    pub id: String,
}

impl ListDocument {
    /// Build the document from the ids `list` gathered, already truncated to
    /// `limit`.
    pub fn new(
        store: &Store,
        prefix: Option<&str>,
        show_deleted: bool,
        limit: u32,
        ids: &[String],
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            datastore: store.datastore.clone(),
            scope: store.scope.clone(),
            prefix: prefix.map(str::to_string),
            show_deleted,
            limit,
            limit_reached: ids.len() as u32 >= limit,
            count: ids.len(),
            entries: ids.iter().map(|id| ListEntry { id: id.clone() }).collect(),
        }
    }
}

/// One invocation of a command that changes an entry: `set`, `reset`,
/// `restore`, `copy` or `delete`.
///
/// The point of it is `revision_id`. Without a document these commands say
/// what happened in prose on stdout, so a caller driving them had two ways to
/// know and both were bad: parse that sentence, or read the exit code and
/// learn nothing beyond success. A revision id is the one fact a caller wants
/// afterwards, because it is what `data revisions --revision` takes.
///
/// `applied` is false for a dry run, which is a success that changed nothing.
/// Reading it wrong is the mistake this field exists to prevent: exit code 0
/// covers both.
#[derive(Debug, Serialize)]
pub struct WriteDocument {
    pub schema_version: u32,
    pub datastore: String,
    pub scope: String,
    /// The key that was acted on, as given.
    pub entry: String,
    /// `set`, `reset`, `restore`, `copy` or `delete`.
    pub action: String,
    /// False without `--apply`: nothing was sent.
    pub applied: bool,
    /// Whether the entry was there before this ran. False means `set` created
    /// it, and means `delete` found nothing to do.
    pub existed: bool,
    /// The revision the entry is at now. **Absent** on a dry run, on a delete,
    /// and when Roblox did not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    /// Where the previous value was copied. **Absent** under `--no-backup`,
    /// and when there was no previous value to copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
}

impl WriteDocument {
    pub fn new(store: &Store, entry: &str, action: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            datastore: store.datastore.clone(),
            scope: store.scope.clone(),
            entry: entry.to_string(),
            action: action.to_string(),
            applied: false,
            existed: false,
            revision_id: None,
            backup: None,
        }
    }
}

/// What one `data delete-store` or `data restore-store` did.
///
/// A separate document from [`WriteDocument`] rather than that one with empty
/// fields. These act on a store, so there is no entry and no scope to name,
/// and a receipt carrying `"entry": ""` would invite a consumer to read it as
/// an entry that happened to be unnamed.
///
/// `applied` carries the same meaning it does there, and for the same reason:
/// a dry run is a success that changed nothing, and exit code 0 covers both.
#[derive(Debug, Serialize)]
pub struct StoreWriteDocument {
    pub schema_version: u32,
    /// The store that was acted on, as given.
    pub datastore: String,
    /// `delete-store` or `restore-store`.
    pub action: String,
    /// False without `--apply`: nothing was sent.
    pub applied: bool,
}

impl StoreWriteDocument {
    pub fn new(store: &str, action: &str, applied: bool) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            datastore: store.to_string(),
            action: action.to_string(),
            applied,
        }
    }
}

/// One `data stores` invocation.
///
/// Experience-wide, so it names neither a store nor a scope: this is the
/// document you read *before* you know what to put in `--datastore`.
#[derive(Debug, Serialize)]
pub struct StoresDocument {
    pub schema_version: u32,
    /// Whether soft-deleted stores were included.
    pub show_deleted: bool,
    /// The `--limit` in force for this run.
    pub limit: u32,
    /// True when the run stopped because it hit `--limit` rather than because
    /// it ran out of stores. Raise `--limit` to see the rest.
    pub limit_reached: bool,
    /// Rows in `stores`.
    pub count: usize,
    /// One object per store, in the order Roblox returned them.
    pub stores: Vec<StoresEntry>,
}

/// One store in a listing.
#[derive(Debug, Serialize)]
pub struct StoresEntry {
    /// The store name, which is what every other subcommand takes as
    /// `--datastore`.
    pub id: String,
    /// When Roblox created the store. **Absent** when the response omitted it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
    /// True for a store soft-deleted and not yet purged. Only ever true with
    /// `--show-deleted`, since nothing else returns one.
    pub deleted: bool,
}

impl StoresDocument {
    /// Build the document from the stores the listing gathered, already
    /// truncated to `limit`.
    pub fn new(show_deleted: bool, limit: u32, stores: &[DataStore]) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            show_deleted,
            limit,
            limit_reached: stores.len() as u32 >= limit,
            count: stores.len(),
            stores: stores
                .iter()
                .map(|store| StoresEntry {
                    id: store.name().unwrap_or_default().to_string(),
                    create_time: store.create_time.clone(),
                    deleted: store.is_deleted(),
                })
                .collect(),
        }
    }
}

/// One `data get` invocation.
///
/// Two shapes, and which one you get is decided by the invocation rather than
/// by the data: `--out` puts the value in a file and the path in `out`, so
/// `value` is absent exactly when the human form would not have printed it
/// either.
#[derive(Debug, Serialize)]
pub struct GetDocument {
    pub schema_version: u32,
    pub datastore: String,
    pub scope: String,
    /// The key that was asked for, as given.
    pub entry: String,
    /// False when there is no such key: never written, or deleted more than
    /// thirty days ago and purged. Exit code is 0 either way, the same
    /// non-event the human form reports as "No entry".
    pub found: bool,
    /// True when the entry is soft-deleted and still readable. Roblox removes
    /// it permanently thirty days after the delete, and a normal read answers
    /// until then, so this is the difference between "gone" and "going".
    pub deleted: bool,
    /// The revision the value came from. **Absent** when Roblox did not say,
    /// and when there is no entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    /// The stored value, nested as JSON rather than escaped into a string.
    ///
    /// **Absent** under `--out`, where the value went to a file, and when
    /// there is no entry. A present `null` is a real answer: serde cannot tell
    /// a stored `null` from an entry with no value at all, and neither can the
    /// game reading it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// Where `--out` wrote the value. **Absent** without `--out`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
}

impl GetDocument {
    /// The answer for a key that is not there.
    pub fn missing(store: &Store, entry: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            datastore: store.datastore.clone(),
            scope: store.scope.clone(),
            entry: entry.to_string(),
            found: false,
            deleted: false,
            revision_id: None,
            value: None,
            out: None,
        }
    }

    /// The answer for a key that is there. `out` is the `--out` path, when one
    /// was given; the value is then in that file and not in the document.
    pub fn found(
        store: &Store,
        entry: &str,
        found: &DataStoreEntry,
        out: Option<&std::path::Path>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            datastore: store.datastore.clone(),
            scope: store.scope.clone(),
            entry: entry.to_string(),
            found: true,
            deleted: found.is_deleted(),
            revision_id: found.revision_id.clone(),
            value: match out {
                Some(_) => None,
                None => Some(found.value.clone().unwrap_or(serde_json::Value::Null)),
            },
            out: out.map(|path| path.display().to_string()),
        }
    }
}

/// One `data revisions <entry>` invocation, without `--revision`.
#[derive(Debug, Serialize)]
pub struct RevisionsDocument {
    pub schema_version: u32,
    pub datastore: String,
    pub scope: String,
    pub entry: String,
    /// Rows in `revisions`.
    pub count: usize,
    /// One object per revision, in the order Roblox returned them, newest
    /// first. Expect fewer than you wrote: an overwrite discards the revision
    /// it replaced unless the experience was snapshotted first.
    pub revisions: Vec<Revision>,
}

/// One revision of one entry.
#[derive(Debug, Serialize)]
pub struct Revision {
    /// What `--revision` and `data restore` take. **Absent** when Roblox sent
    /// none, which is the `-` the human table prints.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    /// When this revision was written, as Roblox sends it. The human table
    /// shortens it to the second; this does not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
    /// `ACTIVE` or `DELETED`, raw. **Absent** when Roblox sent none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// True for the state that means this revision records a delete, so a
    /// consumer does not have to match on the spelling.
    pub deleted: bool,
}

impl From<&DataStoreEntry> for Revision {
    fn from(entry: &DataStoreEntry) -> Self {
        Self {
            revision_id: entry.revision_id.clone(),
            create_time: entry.revision_create_time.clone(),
            state: entry.state.clone(),
            deleted: entry.is_deleted(),
        }
    }
}

impl RevisionsDocument {
    pub fn new(store: &Store, entry: &str, rows: &[DataStoreEntry]) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            datastore: store.datastore.clone(),
            scope: store.scope.clone(),
            entry: entry.to_string(),
            count: rows.len(),
            revisions: rows.iter().map(Revision::from).collect(),
        }
    }
}

/// One `data revisions <entry> --revision <id>` invocation.
///
/// A different document from the listing, and `--revision` is what decides
/// which one you get. The flag is the switch, never the data, so a filter
/// written against one form cannot be handed the other by an entry that
/// happens to have one revision.
#[derive(Debug, Serialize)]
pub struct RevisionDocument {
    pub schema_version: u32,
    pub datastore: String,
    pub scope: String,
    pub entry: String,
    /// The revision that was asked for, as given.
    pub revision_id: String,
    /// That revision's value, nested as JSON. Always present, `null` included:
    /// reading a revision that has no value is what the game would have read.
    pub value: serde_json::Value,
}

impl RevisionDocument {
    pub fn new(
        store: &Store,
        entry: &str,
        revision_id: &str,
        value: Option<serde_json::Value>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            datastore: store.datastore.clone(),
            scope: store.scope.clone(),
            entry: entry.to_string(),
            revision_id: revision_id.to_string(),
            value: value.unwrap_or(serde_json::Value::Null),
        }
    }
}

/// One `data diff` invocation.
///
/// The paths, not the values. `data diff` writes both sides to files and hands
/// them to a diff tool; the document reports where they went, which is exactly
/// what the human form prints, and leaves two player profiles on disk rather
/// than putting them through a pipe.
#[derive(Debug, Serialize)]
pub struct DiffDocument {
    pub schema_version: u32,
    pub datastore: String,
    pub scope: String,
    pub entry: String,
    pub left: DiffSide,
    pub right: DiffSide,
}

/// One side of a comparison.
///
/// `revision` and `env` name what this side is, and exactly one of them is
/// present: `--revisions` fills the first, `--between` the second. So
/// `.left.env` tells a consumer which comparison it is looking at without
/// parsing `label`.
#[derive(Debug, Serialize)]
pub struct DiffSide {
    /// The name this side is filed under, and the basename of `path`.
    pub label: String,
    /// The file this side was written to.
    pub path: String,
    /// The revision id, under `--revisions`. **Absent** under `--between`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// The env name, under `--between`. **Absent** under `--revisions`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
}

/// Which of the two comparisons a side came from.
///
/// Carried rather than re-derived from the label: `--revisions` and
/// `--between` build labels that look alike enough that parsing one back would
/// be a guess, and a key named after an env is not a hypothetical.
#[derive(Debug, Clone)]
pub enum DiffSource {
    /// `--revisions`: this side is a revision id.
    Revision(String),
    /// `--between`: this side is an env name.
    Env(String),
}

impl DiffSide {
    pub fn new(label: &str, path: &std::path::Path, source: &DiffSource) -> Self {
        let (revision, env) = match source {
            DiffSource::Revision(id) => (Some(id.clone()), None),
            DiffSource::Env(name) => (None, Some(name.clone())),
        };
        Self {
            label: label.to_string(),
            path: path.display().to_string(),
            revision,
            env,
        }
    }
}

impl DiffDocument {
    pub fn new(store: &Store, entry: &str, left: DiffSide, right: DiffSide) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            datastore: store.datastore.clone(),
            scope: store.scope.clone(),
            entry: entry.to_string(),
            left,
            right,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store {
            datastore: "PlayerData".into(),
            scope: "global".into(),
        }
    }

    fn parsed(document: &impl Serialize) -> serde_json::Value {
        let mut buf = Vec::new();
        rbx_core::output::write_json(&mut buf, document).expect("write");
        serde_json::from_slice(&buf).expect("the document must be valid JSON")
    }

    fn entry(json: &str) -> DataStoreEntry {
        serde_json::from_str(json).expect("fixture")
    }

    #[test]
    fn a_listing_carries_the_documented_fields() {
        let doc = parsed(&ListDocument::new(
            &store(),
            Some("Player_"),
            false,
            100,
            &["Player_1".to_string(), "Player_2".to_string()],
        ));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["datastore"], "PlayerData");
        assert_eq!(doc["scope"], "global");
        assert_eq!(doc["prefix"], "Player_");
        assert_eq!(doc["show_deleted"], false);
        assert_eq!(doc["limit"], 100);
        assert_eq!(doc["limit_reached"], false);
        assert_eq!(doc["count"], 2);
        assert_eq!(doc["entries"][0]["id"], "Player_1");
        assert_eq!(doc["entries"][1]["id"], "Player_2");
    }

    /// An unfiltered listing has no prefix rather than an empty one: a prefix
    /// of `""` is a filter somebody wrote, and this was not.
    #[test]
    fn an_unfiltered_listing_omits_the_prefix() {
        let doc = parsed(&ListDocument::new(&store(), None, true, 100, &[]));

        assert!(doc.get("prefix").is_none(), "{doc}");
        assert_eq!(doc["show_deleted"], true);
    }

    /// A prefix matching nothing is a normal answer, so it is an empty array
    /// and exit 0 rather than silence: `.count` answers either way.
    #[test]
    fn no_keys_is_an_empty_list_not_an_absent_one() {
        let doc = parsed(&ListDocument::new(&store(), None, false, 100, &[]));

        assert_eq!(doc["count"], 0);
        assert_eq!(doc["entries"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn hitting_the_limit_is_reported_rather_than_left_to_be_inferred() {
        let ids = vec!["a".to_string(), "b".to_string()];
        assert!(ListDocument::new(&store(), None, false, 2, &ids).limit_reached);
        assert!(!ListDocument::new(&store(), None, false, 3, &ids).limit_reached);
    }

    /// The arbitration this module exists for: the profile is nested as JSON
    /// under a key of ours, so `jq` reaches into it and a number stays a
    /// number.
    #[test]
    fn a_stored_profile_is_nested_as_json_rather_than_escaped_into_a_string() {
        let found = entry(r#"{"value":{"coins":500,"items":["hat"]},"revisionId":"r7"}"#);
        let doc = parsed(&GetDocument::found(&store(), "Player_156", &found, None));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["entry"], "Player_156");
        assert_eq!(doc["found"], true);
        assert_eq!(doc["deleted"], false);
        assert_eq!(doc["revision_id"], "r7");
        assert!(doc["value"].is_object(), "{doc}");
        assert_eq!(doc["value"]["coins"], 500);
        assert_eq!(doc["value"]["items"][0], "hat");
    }

    /// A profile is free to contain any key at all, including ours. Nesting
    /// keeps that harmless: nothing of ours lives inside `value`.
    #[test]
    fn a_key_of_ours_inside_a_profile_cannot_shadow_the_envelope() {
        let found = entry(r#"{"value":{"schema_version":9,"entry":"nope","found":false}}"#);
        let doc = parsed(&GetDocument::found(&store(), "Player_156", &found, None));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["entry"], "Player_156");
        assert_eq!(doc["found"], true);
        assert_eq!(doc["value"]["schema_version"], 9);
    }

    /// A scalar profile stays a scalar. This is what stringifying would cost.
    #[test]
    fn a_scalar_value_keeps_its_type() {
        for (raw, check) in [
            (r#"{"value":500}"#, "number"),
            (r#"{"value":"gold"}"#, "string"),
            (r#"{"value":false}"#, "bool"),
            (r#"{"value":[]}"#, "array"),
        ] {
            let doc = parsed(&GetDocument::found(&store(), "K", &entry(raw), None));
            let value = &doc["value"];
            let actual = match value {
                serde_json::Value::Number(_) => "number",
                serde_json::Value::String(_) => "string",
                serde_json::Value::Bool(_) => "bool",
                serde_json::Value::Array(_) => "array",
                other => panic!("unexpected {other}"),
            };
            assert_eq!(actual, check, "{raw}");
        }
    }

    /// Nothing about the player beyond the value the human form already
    /// prints. `users` is the association Roblox answers a data request from
    /// and it never reaches the document.
    #[test]
    fn the_user_association_and_attributes_never_reach_the_document() {
        let found = entry(
            r#"{"value":{"coins":1},"users":["users/156"],"attributes":{"tier":"gold"},
                "etag":"e1","path":"universes/1/data-stores/x","createTime":"2026-01-01T00:00:00Z"}"#,
        );
        let doc = parsed(&GetDocument::found(&store(), "Player_156", &found, None));

        let rendered = doc.to_string();
        for absent in ["users", "attributes", "etag", "path", "create_time"] {
            assert!(!rendered.contains(absent), "{absent} leaked: {rendered}");
        }
    }

    /// A stored `null` is a real answer and stays in the document; the key
    /// only disappears when the value went somewhere else.
    #[test]
    fn a_null_value_is_emitted_and_an_out_path_replaces_it() {
        let doc = parsed(&GetDocument::found(&store(), "K", &entry("{}"), None));
        assert!(doc["value"].is_null(), "{doc}");

        let out = std::path::Path::new("profile.json");
        let doc = parsed(&GetDocument::found(&store(), "K", &entry("{}"), Some(out)));
        assert!(doc.get("value").is_none(), "{doc}");
        assert_eq!(doc["out"], "profile.json");
    }

    /// A key that is not there is a document saying so, not an error and not
    /// silence.
    #[test]
    fn a_missing_entry_says_so_rather_than_emitting_nothing() {
        let doc = parsed(&GetDocument::missing(&store(), "Player_999"));

        assert_eq!(doc["found"], false);
        assert_eq!(doc["deleted"], false);
        assert!(doc.get("value").is_none(), "{doc}");
        assert!(doc.get("revision_id").is_none(), "{doc}");
    }

    /// A soft-deleted entry still answers, and the document says which of the
    /// two it is rather than leaving "readable" to imply "alive".
    #[test]
    fn a_soft_deleted_entry_is_found_and_flagged() {
        let found = entry(r#"{"value":1,"state":"DELETED"}"#);
        let doc = parsed(&GetDocument::found(&store(), "K", &found, None));

        assert_eq!(doc["found"], true);
        assert_eq!(doc["deleted"], true);
    }

    #[test]
    fn a_revision_listing_carries_the_columns_the_table_prints() {
        let rows = vec![
            entry(
                r#"{"id":"K@r2","revisionId":"r2","state":"DELETED",
                    "revisionCreateTime":"2026-08-15T09:15:00.123Z"}"#,
            ),
            entry(r#"{"id":"K@r1","revisionId":"r1","state":"ACTIVE"}"#),
        ];
        let doc = parsed(&RevisionsDocument::new(&store(), "K", &rows));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["entry"], "K");
        assert_eq!(doc["count"], 2);
        assert_eq!(doc["revisions"][0]["revision_id"], "r2");
        assert_eq!(doc["revisions"][0]["state"], "DELETED");
        assert_eq!(doc["revisions"][0]["deleted"], true);
        // Full precision, where the table shortens to the second.
        assert_eq!(
            doc["revisions"][0]["create_time"],
            "2026-08-15T09:15:00.123Z"
        );
        assert_eq!(doc["revisions"][1]["deleted"], false);
        assert!(doc["revisions"][1].get("create_time").is_none(), "{doc}");
    }

    /// `--revision` reads one value, and the document is the value's, not the
    /// listing's. Which one you get is the flag's decision.
    #[test]
    fn reading_one_revision_is_a_value_document() {
        let doc = parsed(&RevisionDocument::new(
            &store(),
            "K",
            "r2",
            Some(serde_json::json!({ "coins": 3 })),
        ));

        assert_eq!(doc["revision_id"], "r2");
        assert_eq!(doc["value"]["coins"], 3);
        assert!(doc.get("revisions").is_none(), "{doc}");
    }

    #[test]
    fn a_revision_with_no_value_reads_back_as_null() {
        let doc = parsed(&RevisionDocument::new(&store(), "K", "r2", None));
        assert!(doc["value"].is_null(), "{doc}");
    }

    /// `diff` reports where the two files are and nothing about what is in
    /// them, which is what the human form prints too.
    #[test]
    fn a_diff_carries_paths_and_never_the_two_values() {
        let left = std::path::Path::new("/tmp/K@r1.json");
        let right = std::path::Path::new("/tmp/K@r2.json");
        let doc = parsed(&DiffDocument::new(
            &store(),
            "K",
            DiffSide::new("K@r1", left, &DiffSource::Revision("r1".into())),
            DiffSide::new("K@r2", right, &DiffSource::Revision("r2".into())),
        ));

        assert_eq!(doc["entry"], "K");
        assert_eq!(doc["left"]["label"], "K@r1");
        assert_eq!(doc["left"]["revision"], "r1");
        assert_eq!(doc["right"]["revision"], "r2");
        assert!(doc["left"].get("env").is_none(), "{doc}");
        assert!(!doc.to_string().contains("value"), "{doc}");
    }

    /// The other comparison names envs instead, so a consumer reads which one
    /// it got rather than parsing the label.
    #[test]
    fn comparing_two_envs_names_them_instead_of_revisions() {
        let path = std::path::Path::new("/tmp/prod-K.json");
        let doc = parsed(&DiffDocument::new(
            &store(),
            "K",
            DiffSide::new("prod-K", path, &DiffSource::Env("prod".into())),
            DiffSide::new("staging-K", path, &DiffSource::Env("staging".into())),
        ));

        assert_eq!(doc["left"]["env"], "prod");
        assert_eq!(doc["right"]["env"], "staging");
        assert!(doc["left"].get("revision").is_none(), "{doc}");
    }
}
