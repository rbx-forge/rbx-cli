//! Types for `cloud/v2` data store entries.

use serde::{Deserialize, Serialize};

/// Answer to `data-stores:snapshot`.
///
/// `new_snapshot_taken` is false when one was already taken today: the call is
/// a no-op then, and `latest_snapshot_time` still reports when the standing
/// snapshot was made. Both cases are a success, which is why the command
/// prints the time rather than treating the second one as a failure.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotResult {
    #[serde(default)]
    pub new_snapshot_taken: bool,
    #[serde(default)]
    pub latest_snapshot_time: Option<String>,
}

/// An entry as Roblox returns it.
///
/// `value` is whatever the game stored, so it stays a raw JSON value: this tool
/// has no business knowing the shape of a player profile.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataStoreEntry {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub create_time: Option<String>,
    #[serde(default)]
    pub revision_id: Option<String>,
    #[serde(default)]
    pub revision_create_time: Option<String>,
    /// `ACTIVE` or `DELETED`. A deleted entry still answers for thirty days.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    /// Resource paths like `users/156`. This is the association Roblox uses to
    /// answer a player's data request, so an overwrite that drops it is not a
    /// cosmetic loss.
    #[serde(default)]
    pub users: Option<Vec<String>>,
    #[serde(default)]
    pub attributes: Option<serde_json::Value>,
}

impl DataStoreEntry {
    pub fn is_deleted(&self) -> bool {
        self.state.as_deref() == Some("DELETED")
    }
}

/// What we send back. Only the three writable fields; everything else on the
/// entry is server-owned and rejected or ignored.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntryUpdate {
    pub value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub users: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<serde_json::Value>,
}

impl EntryUpdate {
    /// Build an overwrite that keeps the association and attributes the entry
    /// already had.
    ///
    /// This is the default on purpose. Sending only `value` would replace the
    /// entry with one that has no `users`, quietly severing the link Roblox
    /// relies on for a player's data request, and nothing in the response would
    /// say so.
    pub fn preserving(value: serde_json::Value, existing: Option<&DataStoreEntry>) -> Self {
        Self {
            value,
            users: existing.and_then(|e| e.users.clone()),
            attributes: existing.and_then(|e| e.attributes.clone()),
        }
    }

    /// Build an overwrite that deliberately drops them.
    pub fn bare(value: serde_json::Value) -> Self {
        Self {
            value,
            users: None,
            attributes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(json: &str) -> DataStoreEntry {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn an_overwrite_keeps_the_user_association_by_default() {
        let existing = entry(r#"{"users":["users/156"],"value":{"coins":10}}"#);
        let update = EntryUpdate::preserving(serde_json::json!({ "coins": 0 }), Some(&existing));
        assert_eq!(update.users, Some(vec!["users/156".to_string()]));
    }

    #[test]
    fn an_overwrite_keeps_attributes_by_default() {
        let existing = entry(r#"{"attributes":{"tier":"gold"}}"#);
        let update = EntryUpdate::preserving(serde_json::json!(1), Some(&existing));
        assert_eq!(update.attributes, Some(serde_json::json!({"tier":"gold"})));
    }

    #[test]
    fn writing_a_brand_new_entry_carries_neither() {
        let update = EntryUpdate::preserving(serde_json::json!(1), None);
        assert_eq!(update.users, None);
        assert_eq!(update.attributes, None);
        assert_eq!(serde_json::to_string(&update).unwrap(), r#"{"value":1}"#);
    }

    #[test]
    fn bare_drops_them_deliberately() {
        let existing = entry(r#"{"users":["users/156"],"attributes":{"a":1}}"#);
        let update = EntryUpdate::bare(serde_json::json!(1));
        assert_eq!(update.users, None);
        assert_ne!(
            update,
            EntryUpdate::preserving(serde_json::json!(1), Some(&existing))
        );
    }

    #[test]
    fn absent_fields_are_omitted_rather_than_sent_as_null() {
        // A null `users` would clear the association just as surely as an empty
        // array; omitting the key leaves it alone.
        let json = serde_json::to_string(&EntryUpdate::bare(serde_json::json!({"a":1}))).unwrap();
        assert!(!json.contains("users"), "got: {json}");
        assert!(!json.contains("attributes"), "got: {json}");
    }

    #[test]
    fn a_deleted_entry_is_recognised() {
        // Roblox soft-deletes: the entry still answers for thirty days with
        // state DELETED, which is why deleting is a poor way to reset a player.
        assert!(entry(r#"{"state":"DELETED"}"#).is_deleted());
        assert!(!entry(r#"{"state":"ACTIVE"}"#).is_deleted());
        assert!(!entry("{}").is_deleted());
    }

    #[test]
    fn any_json_shape_survives_a_round_trip() {
        // The tool never interprets a profile, so anything the game stored has
        // to come back unchanged, nesting and nulls included.
        for raw in [
            r#"{"value":{"coins":10,"items":["a","b"],"nested":{"deep":[1,2,{"x":null}]}}}"#,
            r#"{"value":42}"#,
            r#"{"value":"a string"}"#,
            r#"{"value":[]}"#,
            r#"{"value":false}"#,
        ] {
            let parsed = entry(raw);
            let update = EntryUpdate::preserving(parsed.value.clone().unwrap(), None);
            assert_eq!(Some(update.value), parsed.value, "{raw}");
        }
    }

    #[test]
    fn a_value_of_null_is_indistinguishable_from_no_value_at_all() {
        // serde folds an explicit `null` into `None`, so the two cases cannot
        // be told apart here. Callers therefore treat a missing value as
        // `Value::Null` rather than as an error, which is what the game would
        // have read anyway.
        assert!(entry(r#"{"value":null}"#).value.is_none());
        assert!(entry("{}").value.is_none());
    }
}

/// A page of entries, used for both listing a store and listing one entry's
/// revisions: Roblox returns the same envelope for each, with only `id`,
/// `path`, `state` and the revision fields populated.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryList {
    #[serde(default)]
    pub data_store_entries: Vec<DataStoreEntry>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

impl EntryList {
    /// `cloud/v2` ends a listing with an empty string rather than by omitting
    /// the field. Sending `""` back asks for the same page forever.
    pub fn next_token(&self) -> Option<&str> {
        self.next_page_token
            .as_deref()
            .filter(|token| !token.is_empty())
    }
}

impl DataStoreEntry {
    /// The bare entry id, with any `@revision` suffix removed.
    ///
    /// A revision listing reports `id` as `Player_156@08DE....01`, so anything
    /// keying off the id has to strip it or it will look like a different
    /// entry on every revision.
    pub fn base_id(&self) -> Option<&str> {
        let id = self.id.as_deref()?;
        Some(id.split('@').next().unwrap_or(id))
    }
}

#[cfg(test)]
mod list_tests {
    use super::*;

    #[test]
    fn a_revision_id_is_stripped_back_to_the_entry_it_belongs_to() {
        let entry: DataStoreEntry =
            serde_json::from_str(r#"{"id":"Player_156@08DEF1A1B5E3ADA9.0000000001.x.01"}"#)
                .unwrap();
        assert_eq!(entry.base_id(), Some("Player_156"));
    }

    #[test]
    fn a_plain_id_is_left_alone() {
        let entry: DataStoreEntry = serde_json::from_str(r#"{"id":"Player_156"}"#).unwrap();
        assert_eq!(entry.base_id(), Some("Player_156"));
    }

    #[test]
    fn a_listing_ends_on_an_empty_token() {
        let list: EntryList =
            serde_json::from_str(r#"{"dataStoreEntries":[],"nextPageToken":""}"#).unwrap();
        assert_eq!(list.next_token(), None);
    }

    #[test]
    fn a_revision_listing_parses_the_state_of_each_revision() {
        // Recorded from a live universe: deleting an entry adds a revision
        // whose state is DELETED, while the one holding the value stays ACTIVE.
        let list: EntryList = serde_json::from_str(
            r#"{"dataStoreEntries":[
                {"id":"X@r2","revisionId":"r2","state":"DELETED"},
                {"id":"X@r1","revisionId":"r1","state":"ACTIVE"}
            ]}"#,
        )
        .unwrap();
        assert!(list.data_store_entries[0].is_deleted());
        assert!(!list.data_store_entries[1].is_deleted());
    }
}
