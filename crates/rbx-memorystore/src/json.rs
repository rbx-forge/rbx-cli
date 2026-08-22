//! What `rbx memorystore get` and `list` write to stdout under `--json`.
//!
//! The envelope follows `rbx check --json`: `schema_version` first, then named
//! objects all the way down, optional fields omitted rather than emitted as
//! `null`. Field names are documented in `docs/ops/memorystore.md` and are the
//! compatibility surface.
//!
//! The cached value is nested as JSON under a `value` key of our own, for the
//! reasons `rbx_data::json` sets out at length: a value under a key of ours
//! cannot collide with the envelope, escaping it into a string would cost
//! `jq .value.map` and turn a stored number into a quoted one, and the human
//! form already prints it as JSON.
//!
//! `etag` and `path` are on every item Roblox returns and in none of these
//! documents. The human form has never printed them, and a field nobody asked
//! for is not worth promising to keep.

use serde::Serialize;

use rbx_core::output::SCHEMA_VERSION;

use crate::model::SortedMapItem;

/// One `memorystore get` invocation.
///
/// A missing item is an error here rather than a document saying so, exactly
/// as in the human form: a script reading a key that was supposed to be there
/// should stop, not carry on with `null`. So `found` has no counterpart to
/// `rbx data get`'s: a document at all means the item was there.
#[derive(Debug, Serialize)]
pub struct GetDocument {
    pub schema_version: u32,
    /// The sorted map, as `--map` named it.
    pub map: String,
    /// The item id, as given.
    pub item: String,
    /// When Roblox will drop the item, computed server-side from the `--ttl`
    /// of the write that put it there. **Absent** when the item has no TTL, in
    /// which case it stays until something removes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expire_time: Option<String>,
    /// The cached value, nested as JSON rather than escaped into a string.
    /// **Absent** under `--out`, where the value went to a file. A present
    /// `null` is a real answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// Where `--out` wrote the value. **Absent** without `--out`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
}

impl GetDocument {
    pub fn new(
        map: &str,
        item: &str,
        found: &SortedMapItem,
        out: Option<&std::path::Path>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            map: map.to_string(),
            item: item.to_string(),
            expire_time: found.expire_time.clone(),
            value: match out {
                Some(_) => None,
                None => Some(found.value.clone().unwrap_or(serde_json::Value::Null)),
            },
            out: out.map(|path| path.display().to_string()),
        }
    }
}

/// One `memorystore list` invocation.
#[derive(Debug, Serialize)]
pub struct ListDocument {
    pub schema_version: u32,
    pub map: String,
    /// The `--limit` in force for this run.
    pub limit: u32,
    /// True when the run stopped because it hit `--limit` rather than because
    /// the map ran out. Raise `--limit` to see the rest.
    pub limit_reached: bool,
    /// Rows in `items`.
    pub count: usize,
    /// One object per item, in the order the map returned them, which is sort
    /// key order. Empty for a map whose items have all expired **and** for one
    /// that has never been written to: Roblox does not distinguish the two,
    /// and neither can this.
    pub items: Vec<Item>,
}

/// One sorted map item.
#[derive(Debug, Serialize)]
pub struct Item {
    /// **Absent** when Roblox sent none, which is the `<no id>` the human
    /// listing prints.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The numeric sort key, when the item has one. At most one of the two
    /// sort keys is set: `--sort-key` and `--string-sort-key` are exclusive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numeric_sort_key: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string_sort_key: Option<String>,
    /// **Absent** when the item has no TTL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expire_time: Option<String>,
    /// The cached value. **Absent** without `--values`, which is the flag that
    /// decides whether the human listing prints values too. Shape follows the
    /// invocation, so a filter cannot start working because one item happened
    /// to be fetched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}

impl Item {
    fn new(item: &SortedMapItem, values: bool) -> Self {
        Self {
            id: item.id.clone(),
            numeric_sort_key: item.numeric_sort_key,
            string_sort_key: item.string_sort_key.clone(),
            expire_time: item.expire_time.clone(),
            value: values.then(|| item.value.clone().unwrap_or(serde_json::Value::Null)),
        }
    }
}

impl ListDocument {
    pub fn new(map: &str, limit: u32, values: bool, items: &[SortedMapItem]) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            map: map.to_string(),
            limit,
            limit_reached: items.len() as u32 >= limit,
            count: items.len(),
            items: items.iter().map(|item| Item::new(item, values)).collect(),
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

    fn item(json: &str) -> SortedMapItem {
        serde_json::from_str(json).expect("fixture")
    }

    #[test]
    fn a_get_carries_the_documented_fields_and_nests_the_value() {
        let found = item(
            r#"{"id":"rotation","value":{"map":"desert","weight":3},
                "expireTime":"2026-08-15T09:43:49Z","etag":"e1","path":"universes/1/x"}"#,
        );
        let doc = parsed(&GetDocument::new("Cache", "rotation", &found, None));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["map"], "Cache");
        assert_eq!(doc["item"], "rotation");
        assert_eq!(doc["expire_time"], "2026-08-15T09:43:49Z");
        assert_eq!(doc["value"]["map"], "desert");
        assert_eq!(doc["value"]["weight"], 3);
        // Never printed by the human form, so never promised here.
        let rendered = doc.to_string();
        for absent in ["etag", "path"] {
            assert!(!rendered.contains(absent), "{absent} leaked: {rendered}");
        }
    }

    /// No TTL is an absent key, not a null one: the item stays until something
    /// removes it, which is a different fact from "expires at null".
    #[test]
    fn an_item_with_no_ttl_omits_the_expiry() {
        let doc = parsed(&GetDocument::new(
            "Cache",
            "k",
            &item(r#"{"value":1}"#),
            None,
        ));

        assert!(doc.get("expire_time").is_none(), "{doc}");
        assert_eq!(doc["value"], 1);
    }

    #[test]
    fn an_out_path_replaces_the_value_rather_than_doubling_it() {
        let path = std::path::Path::new("value.json");
        let doc = parsed(&GetDocument::new(
            "Cache",
            "k",
            &item(r#"{"value":{"a":1}}"#),
            Some(path),
        ));

        assert!(doc.get("value").is_none(), "{doc}");
        assert_eq!(doc["out"], "value.json");
    }

    #[test]
    fn a_listing_carries_the_columns_the_human_form_prints() {
        let items = vec![
            item(r#"{"id":"a","numericSortKey":3.5,"expireTime":"2026-08-15T09:43:49Z"}"#),
            item(r#"{"id":"b","stringSortKey":"zulu","value":{"n":1}}"#),
        ];
        let doc = parsed(&ListDocument::new("Cache", 100, false, &items));

        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        assert_eq!(doc["map"], "Cache");
        assert_eq!(doc["limit"], 100);
        assert_eq!(doc["limit_reached"], false);
        assert_eq!(doc["count"], 2);
        assert_eq!(doc["items"][0]["id"], "a");
        assert_eq!(doc["items"][0]["numeric_sort_key"], 3.5);
        assert_eq!(doc["items"][0]["expire_time"], "2026-08-15T09:43:49Z");
        assert_eq!(doc["items"][1]["string_sort_key"], "zulu");
        assert!(doc["items"][1].get("numeric_sort_key").is_none(), "{doc}");
    }

    /// `--values` is what decides whether values are printed, in both formats.
    /// Without it the key is absent, so nothing reads a value it did not ask
    /// for.
    #[test]
    fn values_appear_only_when_they_were_asked_for() {
        let items = vec![item(r#"{"id":"a","value":{"n":1}}"#)];

        let without = parsed(&ListDocument::new("Cache", 100, false, &items));
        assert!(without["items"][0].get("value").is_none(), "{without}");

        let with = parsed(&ListDocument::new("Cache", 100, true, &items));
        assert_eq!(with["items"][0]["value"]["n"], 1);
    }

    /// An empty map is an empty array and exit 0, not silence: a map that has
    /// never been written to answers the same way as one whose items expired,
    /// and `.count` reads both.
    #[test]
    fn an_empty_map_is_an_empty_list_not_an_absent_one() {
        let doc = parsed(&ListDocument::new("Cache", 100, false, &[]));

        assert_eq!(doc["count"], 0);
        assert_eq!(doc["items"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn hitting_the_limit_is_reported_rather_than_left_to_be_inferred() {
        let items = vec![item(r#"{"id":"a"}"#), item(r#"{"id":"b"}"#)];
        assert!(ListDocument::new("Cache", 2, false, &items).limit_reached);
        assert!(!ListDocument::new("Cache", 3, false, &items).limit_reached);
    }
}
