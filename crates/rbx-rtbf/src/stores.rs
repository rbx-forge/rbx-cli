//! Listing the data stores a universe actually has.
//!
//! One endpoint, `Cloud_ListDataStores`, and it exists here rather than in
//! `rbx-core` because `rbx rtbf verify` is its only consumer today. If
//! `rbx data` grows a `stores` listing it should move; sharing it before there
//! is a second reader would be guessing at the shape the second reader wants.

use anyhow::Result;
use serde::Deserialize;

use rbx_core::api::{execute_json, ApiBase};

/// Roblox's own ceiling on this endpoint's page size.
const PAGE_SIZE: usize = 100;

/// A guard on the walk, not a user-facing limit.
///
/// A universe with more than this many stores is not a project whose templates
/// a listing can usefully check by hand, and an unbounded loop against a
/// paginated endpoint is how a hung token spins forever.
const MAX_PAGES: usize = 50;

#[derive(Debug, Deserialize)]
struct ListResponse {
    #[serde(default, rename = "dataStores")]
    data_stores: Vec<DataStore>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DataStore {
    #[serde(default)]
    path: String,
}

impl DataStore {
    /// The store's name, out of `universes/{id}/data-stores/{name}`.
    ///
    /// The API returns a resource path rather than a name, and the name is what
    /// a template's `store` field holds, so the split happens once here rather
    /// than at each comparison.
    fn name(&self) -> Option<&str> {
        self.path.rsplit_once("/data-stores/").map(|(_, name)| name)
    }
}

/// Every standard data store name in the universe, sorted.
///
/// Ordered data stores are **not** listed: this endpoint covers standard stores
/// only, which is a real limit on what `verify` can prove and is reported rather
/// than hidden. A key template marked `ordered = true` names a store this walk
/// cannot see, and saying "not found" for it would be a false alarm.
pub async fn list_standard(
    client: &reqwest::Client,
    base: &ApiBase,
    api_key: &str,
    universe_id: u64,
) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let mut token: Option<String> = None;

    for _ in 0..MAX_PAGES {
        let mut url = base.join(&format!(
            "/cloud/v2/universes/{universe_id}/data-stores?maxPageSize={PAGE_SIZE}"
        ));
        if let Some(page) = &token {
            url.push_str(&format!(
                "&pageToken={}",
                rbx_core::api::encode_query_value(page)
            ));
        }

        let response: ListResponse = execute_json(|| async {
            Ok(client.get(&url).header("x-api-key", api_key).send().await?)
        })
        .await?;

        names.extend(
            response
                .data_stores
                .iter()
                .filter_map(|store| store.name().map(str::to_string)),
        );

        // An empty token and an absent one both mean "no more pages". Roblox
        // has been seen to send the empty string, and treating that as a page
        // to fetch is how a walk spins.
        match response.next_page_token {
            Some(next) if !next.is_empty() => token = Some(next),
            _ => break,
        }
    }

    names.sort();
    names.dedup();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(path: &str) -> DataStore {
        DataStore {
            path: path.to_string(),
        }
    }

    #[test]
    fn a_name_is_taken_off_the_resource_path() {
        assert_eq!(
            store("universes/109876543210987/data-stores/PlayerInventory").name(),
            Some("PlayerInventory")
        );
    }

    /// A store name may contain a slash, and the split has to take the last
    /// separator rather than the first or the name comes back truncated.
    #[test]
    fn a_name_containing_a_slash_survives() {
        assert_eq!(
            store("universes/1/data-stores/team/scores").name(),
            Some("team/scores")
        );
    }

    #[test]
    fn a_path_in_a_shape_this_build_does_not_know_is_skipped_not_guessed() {
        assert_eq!(store("something/else").name(), None);
        assert_eq!(store("").name(), None);
    }
}
