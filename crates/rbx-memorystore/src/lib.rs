//! `rbx-ops memorystore` : read and write memory store sorted map items.
//!
//! The one Open Cloud surface a game server can read without an HTTP call.
//! A value written here is visible to `MemoryStoreService:GetSortedMap()`
//! in-experience, which is the point: something outside Roblox — a VPS, a cron
//! job, a dashboard — can publish a value that every server picks up without
//! paying a data store round trip for it.
//!
//! Sorted maps only, for now. Queues are the other half of the memory store
//! and a different shape of problem (ordering, claim, discard); nothing has
//! needed them yet, and guessing at a queue CLI before there is a queue to
//! drive is how you ship the wrong verbs.
//!
//! ## Two things the specification does not make obvious
//!
//! **The item id is a query parameter.** `POST .../items` with `{"id": ...}`
//! in the body answers `400 INVALID_ARGUMENT "The id field is required."` —
//! an error naming the field you just sent. It belongs in `?id=`, and the body
//! carries only the value, the TTL and the sort keys.
//!
//! **The map is created implicitly.** There is no "create sorted map" call.
//! Reading a map that has never existed answers `200` with an empty list, not
//! `404`, and the first write brings it into being. So `set` on a fresh name
//! is a normal write rather than a special case.
//!
//! ## Why writes here are not guarded the way `data` writes are
//!
//! `rbx-ops data` prompts before overwriting, and writes a backup file first,
//! because a player profile is irreplaceable and an overwrite destroys the
//! previous value. None of that holds here. These items are a cache: they
//! carry a TTL, they are rebuilt from whatever produced them, and the thing
//! that writes them is a script on a schedule rather than a person at a
//! terminal. A prompt would be an obstacle exactly where it cannot be answered.
//!
//! Writes still need `--apply`, because that is the rule everywhere in
//! `rbx-ops` and a rule with exceptions is not a rule you can rely on when you
//! are tired.

pub mod json;
pub mod model;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use colored::Colorize;
use reqwest::{Client, StatusCode};

use rbx_core::api::{
    build_client, encode_query_value, execute_json, execute_with_retry, explain_missing_scope,
    is_api_status, require_api_key, ApiBase,
};
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

use crate::json::{GetDocument, ListDocument};
use crate::model::{ItemList, ItemWrite, SortedMapItem};

/// Roblox caps a page at 100 items.
const MAX_PAGE_SIZE: u32 = 100;

#[derive(Args, Debug)]
pub struct MemoryStoreCli {
    #[command(subcommand)]
    command: Command,

    /// Sorted map name, as the game passes to `GetSortedMap`.
    ///
    /// The map is created by its first write; there is no separate create
    /// step, and naming one that does not exist yet is not an error.
    #[arg(long, global = true)]
    map: Option<String>,

    /// Override the API host. For testing against a mock server.
    #[arg(long, hide = true, global = true)]
    base_url: Option<String>,
}

impl MemoryStoreCli {
    /// Tests only.
    #[doc(hidden)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = Some(url);
        self
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print one item
    Get {
        /// Item id.
        item: String,

        /// Write the value to a file instead of printing it.
        #[arg(long)]
        out: Option<PathBuf>,

        /// Write the result to stdout as one JSON document.
        ///
        /// The cached value nested under `value`, plus the expiry the human
        /// form prints on stderr. A missing item stays an error rather than
        /// becoming a document: a script reading a key that was supposed to be
        /// there should stop. Field names are documented in
        /// docs/ops/memorystore.md.
        #[arg(long)]
        json: bool,
    },

    /// Write one item, creating it if it is not there
    ///
    /// One call, not a read followed by a write: the underlying `PATCH` takes
    /// `allowMissing`, so create and update are the same request. Two callers
    /// racing therefore both land as writes rather than one of them failing on
    /// a missing item.
    Set {
        /// Item id.
        item: String,

        /// Value, as JSON on the command line.
        #[arg(long, conflicts_with = "file")]
        value: Option<String>,

        /// Value, read from a file. Easier than quoting JSON in a shell.
        #[arg(long)]
        file: Option<PathBuf>,

        /// How long the item lives, as a duration like `300s`, `10m` or `2h`.
        ///
        /// Omit it and the item stays until something removes it. For a cache
        /// that is rarely what you want: a TTL is what stops a stale value
        /// outliving whatever was producing it.
        #[arg(long)]
        ttl: Option<String>,

        /// Numeric sort key, for ordering in `list`.
        #[arg(
            long,
            conflicts_with = "string_sort_key",
            allow_negative_numbers = true
        )]
        sort_key: Option<f64>,

        /// String sort key, ordered lexicographically.
        #[arg(long)]
        string_sort_key: Option<String>,

        /// Actually write it.
        #[arg(long)]
        apply: bool,
    },

    /// Remove one item before its TTL expires
    Delete {
        /// Item id.
        item: String,

        /// Actually delete it.
        #[arg(long)]
        apply: bool,
    },

    /// List items in the map
    List {
        /// Maximum items to fetch, across as many pages as it takes.
        #[arg(long, default_value_t = 100)]
        limit: u32,

        /// Print each item's value too, rather than ids and sort keys only.
        #[arg(long)]
        values: bool,

        /// Write the listing to stdout as one JSON document.
        ///
        /// Ids, sort keys and expiries, plus the values when --values asks for
        /// them: the same flag decides that in both formats. stdout carries
        /// the document and nothing else. Field names are documented in
        /// docs/ops/memorystore.md.
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(cli: MemoryStoreCli, global: &GlobalFlags) -> Result<()> {
    let Some(map) = cli.map.clone() else {
        bail!(
            "`rbx-ops memorystore` needs --map <name>, the name the game passes to GetSortedMap."
        );
    };

    let base = match &cli.base_url {
        Some(url) => ApiBase::new(url.clone()),
        None => ApiBase::default(),
    };
    let api = Api {
        client: build_client(),
        base,
        api_key: require_api_key(global.api_key.as_deref())?.to_string(),
        universe_id: global.single_universe()?,
        map,
    };

    match cli.command {
        Command::Get { item, out, json } => {
            let format = OutputFormat::from_json_flag(json);
            let Some(found) = api.get(&item).await? else {
                bail!("no item \"{item}\" in sorted map \"{}\".", api.map);
            };
            let value = found.value.clone().unwrap_or(serde_json::Value::Null);
            let rendered = serde_json::to_string_pretty(&value)?;

            match out {
                Some(path) => {
                    std::fs::write(&path, &rendered)
                        .with_context(|| format!("writing {}", path.display()))?;
                    if format.is_json() {
                        output::emit(&GetDocument::new(&api.map, &item, &found, Some(&path)))?;
                    } else {
                        println!("{}", format!("✓ wrote {}", path.display()).green());
                    }
                }
                None => {
                    if format.is_json() {
                        output::emit(&GetDocument::new(&api.map, &item, &found, None))?;
                    } else {
                        println!("{rendered}");
                    }
                }
            }
            // stderr in both formats, as it always has been, and in the
            // document too: a consumer that never reads stderr still gets the
            // expiry.
            if let Some(expires) = found.expire_time.as_deref() {
                eprintln!("{}", format!("expires {expires}").dimmed());
            }
            Ok(())
        }

        Command::Set {
            item,
            value,
            file,
            ttl,
            sort_key,
            string_sort_key,
            apply,
        } => {
            let raw = match (value, file) {
                (Some(inline), _) => inline,
                (None, Some(path)) => std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?,
                (None, None) => bail!("`set` needs --value <json> or --file <path>."),
            };
            let parsed: serde_json::Value =
                serde_json::from_str(&raw).context("the value must be valid JSON")?;

            let body = ItemWrite {
                value: parsed,
                ttl,
                string_sort_key,
                numeric_sort_key: sort_key,
            };

            if !apply {
                println!(
                    "would write \"{item}\" in sorted map \"{}\" of universe {}",
                    api.map, api.universe_id
                );
                println!("{}", serde_json::to_string_pretty(&body)?);
                println!("{}", "Nothing sent. Re-run with --apply.".dimmed());
                return Ok(());
            }

            let written = api.set(&item, &body).await?;
            println!("{}", format!("✓ wrote \"{item}\"").green());
            if let Some(expires) = written.expire_time.as_deref() {
                println!("{}", format!("  expires {expires}").dimmed());
            } else {
                println!(
                    "{}",
                    "  no TTL: it stays until something removes it".dimmed()
                );
            }
            Ok(())
        }

        Command::Delete { item, apply } => {
            if !apply {
                println!(
                    "would delete \"{item}\" from sorted map \"{}\" of universe {}",
                    api.map, api.universe_id
                );
                println!("{}", "Nothing sent. Re-run with --apply.".dimmed());
                return Ok(());
            }
            api.delete(&item).await?;
            println!("{}", format!("✓ deleted \"{item}\"").green());
            Ok(())
        }

        Command::List {
            limit,
            values,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);
            let items = api.list(limit).await?;
            if items.is_empty() {
                // Not an error, and not necessarily an empty map: a map that
                // has never been written to answers exactly the same way.
                // Under `--json` the same non-event is an empty `items` array
                // rather than no document, so `.count` answers either way.
                format.note(format!("sorted map \"{}\" is empty", api.map).dimmed());
            }
            if format.is_json() {
                return output::emit(&ListDocument::new(&api.map, limit, values, &items));
            }
            if items.is_empty() {
                return Ok(());
            }
            for item in &items {
                let id = item.id.as_deref().unwrap_or("<no id>");
                let sort = match (item.numeric_sort_key, item.string_sort_key.as_deref()) {
                    (Some(n), _) => format!(" [{n}]"),
                    (None, Some(s)) => format!(" [{s}]"),
                    (None, None) => String::new(),
                };
                let expires = item
                    .expire_time
                    .as_deref()
                    .map(|e| format!("  expires {e}"))
                    .unwrap_or_default();
                println!("{id}{}{}", sort.dimmed(), expires.dimmed());
                if values {
                    let value = item.value.clone().unwrap_or(serde_json::Value::Null);
                    println!("{}", serde_json::to_string_pretty(&value)?);
                }
            }
            println!("{}", format!("{} item(s)", items.len()).dimmed());
            Ok(())
        }
    }
}

struct Api {
    client: Client,
    base: ApiBase,
    api_key: String,
    universe_id: u64,
    map: String,
}

impl Api {
    fn items_url(&self) -> String {
        self.base.join(&format!(
            "/cloud/v2/universes/{}/memory-store/sorted-maps/{}/items",
            self.universe_id,
            encode_query_value(&self.map),
        ))
    }

    fn item_url(&self, item: &str) -> String {
        format!("{}/{}", self.items_url(), encode_query_value(item))
    }

    async fn get(&self, item: &str) -> Result<Option<SortedMapItem>> {
        let url = self.item_url(item);
        let result: Result<SortedMapItem> = execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await;

        match result {
            Ok(item) => Ok(Some(item)),
            // Matched on the typed status rather than the rendered message,
            // which embeds the response body: a cached value containing "404"
            // would otherwise read as a missing item.
            Err(error) if is_api_status(&error, StatusCode::NOT_FOUND) => Ok(None),
            Err(error) => Err(explain_missing_scope(error)),
        }
    }

    /// Create or replace, in one request.
    ///
    /// `PATCH` with `allowMissing=true` rather than `POST`: `POST` is a create
    /// and fails on an item that is already there, which would make every
    /// `set` a read followed by a branch, and would lose a race between two
    /// writers.
    async fn set(&self, item: &str, body: &ItemWrite) -> Result<SortedMapItem> {
        let url = format!("{}?allowMissing=true", self.item_url(item));
        execute_json(|| {
            let request = self
                .client
                .patch(&url)
                .header("x-api-key", &self.api_key)
                .json(body);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)
    }

    async fn delete(&self, item: &str) -> Result<()> {
        let url = self.item_url(item);
        let response = execute_with_retry(|| {
            let request = self.client.delete(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("delete failed: {status} {body}");
        }
        Ok(())
    }

    /// Up to `limit` items, following pages as far as needed.
    async fn list(&self, limit: u32) -> Result<Vec<SortedMapItem>> {
        let mut collected = Vec::new();
        let mut page_token: Option<String> = None;

        // Fixed for the whole walk, not recomputed per page. The endpoint says
        // so itself: "When paginating, all other parameters provided to the
        // subsequent call must match the call that provided the page token."
        // Shrinking `maxPageSize` as the remaining count falls sends page two
        // with a different value than the call that issued its token, which
        // Roblox is entitled to reject — and only on listings long enough to
        // page, which are the ones nobody tries by hand.
        //
        // Overshooting on the last page costs nothing: the truncate below
        // discards the surplus. Keeping the two paired is the point — a fixed
        // page size without it returns more rows than `--limit` asked for.
        let page_size = 1u32.max(limit.min(MAX_PAGE_SIZE));

        while (collected.len() as u32) < limit {
            let mut url = format!("{}?maxPageSize={}", self.items_url(), page_size);
            if let Some(token) = &page_token {
                url.push_str("&pageToken=");
                url.push_str(&encode_query_value(token));
            }

            let page: ItemList = execute_json(|| {
                let request = self.client.get(&url).header("x-api-key", &self.api_key);
                async move { request.send().await.map_err(Into::into) }
            })
            .await
            .map_err(explain_missing_scope)?;

            let empty = page.items.is_empty();
            collected.extend(page.items.iter().cloned());

            match page.next_page() {
                // An empty page with a token would otherwise spin forever.
                Some(_) if empty => break,
                Some(token) => page_token = Some(token.to_string()),
                None => break,
            }
        }

        collected.truncate(limit as usize);
        Ok(collected)
    }
}
