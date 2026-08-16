//! Types for `cloud/v2` memory store sorted map items.

use serde::{Deserialize, Serialize};

/// An item as Roblox returns it.
///
/// `value` stays a raw JSON value: what a game caches is its own business, and
/// the API accepts anything JSON can express.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortedMapItem {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub etag: Option<String>,
    /// Server-computed, from the `ttl` sent on the write. Write-only on the
    /// way in, read-only on the way out, which is why `ItemWrite` has `ttl`
    /// and this has `expire_time`.
    #[serde(default)]
    pub expire_time: Option<String>,
    #[serde(default)]
    pub string_sort_key: Option<String>,
    #[serde(default)]
    pub numeric_sort_key: Option<f64>,
}

/// One page of items.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemList {
    #[serde(default)]
    pub items: Vec<SortedMapItem>,
    /// Empty string and `null` both mean "no more pages"; Roblox has been seen
    /// returning either, so both are normalised to `None` by
    /// [`ItemList::next_page`].
    #[serde(default)]
    pub next_page_token: Option<String>,
}

impl ItemList {
    pub fn next_page(&self) -> Option<&str> {
        self.next_page_token.as_deref().filter(|t| !t.is_empty())
    }
}

/// What we send on a write.
///
/// Only the fields Roblox accepts. `id` is deliberately absent: it travels as
/// a query parameter, and putting it in the body answers
/// `400 INVALID_ARGUMENT "The id field is required."` — the error names the
/// field you just sent, which is why it costs a request to work out.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemWrite {
    pub value: serde_json::Value,
    /// Serialised as a protobuf duration, e.g. `"300s"`. Omitted entirely when
    /// the item should not expire on its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string_sort_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numeric_sort_key: Option<f64>,
}
