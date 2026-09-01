//! The HTTP client for the Open Cloud data store API.
//!
//! One method per endpoint, each returning the parsed model, with no printing
//! and no policy: whether a write needs `--apply`, a confirmation or a backup
//! is decided by the caller. That separation is what lets the dispatch read as
//! a sequence of decisions rather than as a sequence of requests.
//!
//! URLs are written as literals here rather than composed from a helper, so
//! `crates/rbx-spec-drift` can extract them and check them against the vendored
//! Roblox specification. A path assembled from a method call is a runtime value
//! and that check cannot see it.

use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};

use rbx_core::api::{
    encode_query_value, execute_json, execute_with_retry, explain_missing_scope, is_api_status,
    ApiBase,
};

use crate::model::{DataStoreEntry, EntryList, EntryUpdate, SnapshotResult, StoreList};
/// Everything one request needs, resolved once per run.
///
/// The fields are `pub(crate)` because the dispatch builds this, once per env,
/// after resolving the store and the universe. They stay crate-visible and no
/// wider: nothing outside this crate constructs one.
pub(crate) struct Api {
    pub(crate) client: Client,
    pub(crate) base: ApiBase,
    pub(crate) api_key: String,
    pub(crate) universe_id: u64,
    pub(crate) datastore: String,
    pub(crate) scope: String,
}

impl Api {
    pub(crate) fn entry_url(&self, entry: &str) -> String {
        self.base.join(&format!(
            "/cloud/v2/universes/{}/data-stores/{}/scopes/{}/entries/{}",
            self.universe_id,
            encode_query_value(&self.datastore),
            encode_query_value(&self.scope),
            encode_query_value(entry),
        ))
    }

    pub(crate) async fn get(&self, entry: &str) -> Result<Option<DataStoreEntry>> {
        let url = self.entry_url(entry);
        let result: Result<DataStoreEntry> = execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await;

        match result {
            Ok(entry) => Ok(Some(entry)),
            // A key that has never existed is not an error for any caller here:
            // `get` reports it, and `set` creates it.
            //
            // Matched on the typed status, not on the rendered message. The
            // message embeds the response body, and a stored value is free to
            // contain "404": that read as a missing entry and made `get`
            // deny a key that was sitting right there.
            Err(error) if is_api_status(&error, StatusCode::NOT_FOUND) => Ok(None),
            Err(error) => Err(explain_missing_scope(error)),
        }
    }

    pub(crate) async fn delete(&self, entry: &str) -> Result<()> {
        let url = self.entry_url(entry);

        execute_with_retry(|| {
            let request = self.client.delete(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map(|_| ())
        .map_err(explain_missing_scope)
    }

    /// Remove a whole data store.
    ///
    /// The URL is built here rather than by appending to a helper, and that is
    /// on purpose: the drift check reads string literals, so a path assembled
    /// from a method call is one it cannot see and cannot protect against a
    /// Roblox rename. `crates/rbx-spec-drift` keeps the list of the ones that
    /// already got away.
    pub(crate) async fn delete_store(&self, store: &str) -> Result<()> {
        let url = self.base.join(&format!(
            "/cloud/v2/universes/{}/data-stores/{}",
            self.universe_id,
            encode_query_value(store),
        ));

        execute_with_retry(|| {
            let request = self.client.delete(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map(|_| ())
        .map_err(explain_missing_scope)
    }

    /// Bring back a store removed with [`Self::delete_store`].
    pub(crate) async fn restore_store(&self, store: &str) -> Result<()> {
        let url = self.base.join(&format!(
            "/cloud/v2/universes/{}/data-stores/{}:undelete",
            self.universe_id,
            encode_query_value(store),
        ));

        execute_with_retry(|| {
            let request = self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                // Roblox answers 400 on an absent body for this one, where the
                // path already carries everything it needs.
                .json(&serde_json::json!({}));
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map(|_| ())
        .map_err(explain_missing_scope)
    }

    /// One page of data store names.
    ///
    /// Experience-wide, so it uses neither `self.datastore` nor `self.scope`:
    /// this is the call that tells you what to put in the first of them.
    pub(crate) async fn stores(
        &self,
        show_deleted: bool,
        page_token: Option<&str>,
    ) -> Result<StoreList> {
        let mut url = self.base.join(&format!(
            "/cloud/v2/universes/{}/data-stores?maxPageSize=100",
            self.universe_id,
        ));
        if show_deleted {
            url.push_str("&showDeleted=true");
        }
        if let Some(token) = page_token {
            url.push_str("&pageToken=");
            url.push_str(&encode_query_value(token));
        }

        execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)
    }

    /// One page of entry ids. Only `id` and `path` come back; reading a value
    /// takes a second call, which is why listing is cheap and dumping is not.
    pub(crate) async fn list(
        &self,
        prefix: Option<&str>,
        show_deleted: bool,
        page_token: Option<&str>,
    ) -> Result<EntryList> {
        let mut url = self.base.join(&format!(
            "/cloud/v2/universes/{}/data-stores/{}/scopes/{}/entries?maxPageSize=100",
            self.universe_id,
            encode_query_value(&self.datastore),
            encode_query_value(&self.scope),
        ));
        if let Some(prefix) = prefix {
            url.push_str("&filter=");
            url.push_str(&encode_query_value(&format!("id.startsWith(\"{prefix}\")")));
        }
        if show_deleted {
            url.push_str("&showDeleted=true");
        }
        if let Some(token) = page_token {
            url.push_str("&pageToken=");
            url.push_str(&encode_query_value(token));
        }

        execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)
    }

    pub(crate) async fn revisions(&self, entry: &str) -> Result<EntryList> {
        let url = format!("{}:listRevisions?maxPageSize=100", self.entry_url(entry));
        execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)
    }

    /// Read one revision.
    ///
    /// The revision is addressed by appending `@<revisionId>` to the entry id,
    /// and this is the only way to see a value that has been overwritten or
    /// deleted. Needs `universe-datastores.versions:read`, a different scope
    /// from a plain read.
    pub(crate) async fn get_revision(&self, entry: &str, revision: &str) -> Result<DataStoreEntry> {
        let url = self.entry_url(&format!("{entry}@{revision}"));
        execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)
    }

    /// Add to a numeric entry without reading it first.
    ///
    /// Atomic on Roblox's side, which a read-then-write is not: two supporters
    /// granting currency at the same time both land here, and one of them would
    /// be lost with `set`.
    pub(crate) async fn increment(&self, entry: &str, by: i64) -> Result<DataStoreEntry> {
        let url = format!("{}:increment", self.entry_url(entry));
        let response = execute_with_retry(|| {
            let request = self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .json(&serde_json::json!({ "amount": by }));
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)?;
        let text = response.text().await?;
        serde_json::from_str(&text)
            .with_context(|| format!("parsing the incremented entry: {text}"))
    }

    /// Take an experience-wide snapshot.
    ///
    /// Universe-scoped, not store-scoped: it covers every data store in the
    /// experience, which is why it is the one call here that ignores
    /// `--datastore` and `--scope`.
    pub(crate) async fn snapshot(&self) -> Result<SnapshotResult> {
        let url = self.base.join(&format!(
            "/cloud/v2/universes/{}/data-stores:snapshot",
            self.universe_id
        ));
        let response = execute_with_retry(|| {
            let request = self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .json(&serde_json::json!({}));
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)?;
        let text = response.text().await?;
        serde_json::from_str(&text).with_context(|| format!("parsing the snapshot result: {text}"))
    }

    pub(crate) async fn set(&self, entry: &str, update: &EntryUpdate) -> Result<DataStoreEntry> {
        let url = format!("{}?allowMissing=true", self.entry_url(entry));
        let response = execute_with_retry(|| {
            let request = self
                .client
                .patch(&url)
                .header("x-api-key", &self.api_key)
                .json(update);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)?;

        let text = response.text().await?;
        serde_json::from_str(&text).with_context(|| format!("parsing the written entry: {text}"))
    }
}
