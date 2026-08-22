//! `rbx data ordered`: the leaderboard half of data stores.
//!
//! ## Why it is here and not its own command
//!
//! Ordered data stores are a different Open Cloud resource from standard ones,
//! not a mode of them: the values are integers rather than JSON documents,
//! there are no versions and no revisions, and the whole point of the resource
//! is that a listing comes back sorted. `TODO.md` declined them once for that
//! reason and left the door open: "would fit under `data` as a sibling mode
//! […] revisit on demand".
//!
//! Under `data` is where they landed, because the thing a reader needs to know
//! is which store they are talking to, and `--datastore` / `--scope` already
//! mean that here. What does *not* carry over is everything in this crate's
//! module documentation about backups: an ordered entry has no revision
//! history to lose, so an overwrite here destroys exactly one integer, and
//! writing a backup file per write would be ceremony around a number.
//!
//! ## What is deliberately missing
//!
//! No `snapshot`, no `revisions`, no `restore`, no `diff`. Roblox offers none
//! of them on this resource, and a command that answered "not supported" for
//! four of its verbs would be worse than not having them.
//!
//! Scope: `universe.ordered-data-store.scope.entry:read` for `list` and `get`,
//! `:write` for the rest.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use colored::Colorize;
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use rbx_core::api::{
    build_client, encode_query_value, execute_json, execute_with_retry, explain_missing_scope,
    is_api_status, ApiBase,
};
use rbx_core::confirm::confirm_always;
use rbx_core::output::{self, OutputFormat};

use crate::json::Store;

/// Roblox caps a page at 100 and defaults to 10. Asking for the cap keeps the
/// number of round trips down for the listing everybody actually wants (the
/// top N) without changing what comes back.
const MAX_PAGE_SIZE: u32 = 100;

#[derive(Subcommand, Debug)]
pub enum OrderedCommand {
    /// Print the leaderboard: entries in value order.
    ///
    /// Descending by default, because "the top players" is what an ordered
    /// data store is for and ascending would make the common case the one that
    /// needs a flag.
    List {
        /// How many entries to print. Fetches as many pages as it takes.
        #[arg(long, default_value_t = 10)]
        limit: u32,

        /// Order ascending instead, lowest value first.
        #[arg(long)]
        asc: bool,

        /// Only entries whose value is at least this.
        #[arg(long)]
        min: Option<i64>,

        /// Only entries whose value is at most this.
        #[arg(long)]
        max: Option<i64>,

        /// Write the listing to stdout as one JSON document.
        #[arg(long)]
        json: bool,
    },

    /// Print one entry's value.
    Get {
        /// Entry id, as the game passes to `SetAsync`.
        entry: String,

        /// Write the answer to stdout as one JSON document.
        #[arg(long)]
        json: bool,
    },

    /// Set one entry to an exact value, creating it if absent.
    ///
    /// Overwrites. Unlike a standard data store there is no revision history
    /// to fall back on, so the previous value is gone the moment this returns.
    Set {
        /// Entry id.
        entry: String,

        /// The new value. Ordered data stores hold integers only.
        value: i64,

        /// Fail instead of creating the entry when it does not exist.
        #[arg(long)]
        no_create: bool,

        /// Skip the confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// Add to one entry's value, atomically.
    ///
    /// The operation to reach for over `set` when several writers touch the
    /// same key: a read-then-set from two places loses one of the two updates,
    /// and this does not.
    Increment {
        /// Entry id.
        entry: String,

        /// How much to add. Negative subtracts.
        ///
        /// `allow_negative_numbers` because without it clap reads `-5` as a
        /// flag it does not know, and the error names the argument rather than
        /// the sign.
        #[arg(allow_negative_numbers = true)]
        amount: i64,

        /// Skip the confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// Remove one entry from the leaderboard.
    Delete {
        /// Entry id.
        entry: String,

        /// Skip the confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },
}

/// One entry as Roblox returns it.
///
/// `value` is documented as a double that is "always rounded to the nearest
/// integer", which is a JSON-shaped statement rather than a semantic one: the
/// store holds integers. Deserialised as `f64` because that is what arrives on
/// the wire, and reported as `i64` because that is what it means.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    value: Option<f64>,
}

impl Entry {
    fn value_i64(&self) -> i64 {
        self.value.unwrap_or(0.0).round() as i64
    }

    fn id_or_unknown(&self) -> &str {
        self.id.as_deref().unwrap_or("(unnamed)")
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntryPage {
    #[serde(default)]
    ordered_data_store_entries: Vec<Entry>,
    #[serde(default)]
    next_page_token: Option<String>,
}

// ---------------------------------------------------------------------------
// JSON documents
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryRow {
    /// 1-based position in the listing as printed, not a global rank: a
    /// filtered or paged listing has no way to know what precedes it.
    position: usize,
    id: String,
    value: i64,
}

/// The two fields every document here repeats, spelled out rather than
/// flattened from `Store`: the sibling documents in `json.rs` name them the
/// same way, and a `--json` consumer should not have to know which command
/// produced the file to find the store it describes.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ListDoc<'a> {
    datastore: &'a str,
    scope: &'a str,
    descending: bool,
    entries: Vec<EntryRow>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GetDoc<'a> {
    datastore: &'a str,
    scope: &'a str,
    entry: &'a str,
    found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<i64>,
}

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------

struct Api {
    client: Client,
    base: ApiBase,
    api_key: String,
    universe_id: u64,
    datastore: String,
    scope: String,
}

impl Api {
    fn entries_path(&self) -> String {
        format!(
            "/cloud/v2/universes/{}/ordered-data-stores/{}/scopes/{}/entries",
            self.universe_id,
            encode_query_value(&self.datastore),
            encode_query_value(&self.scope),
        )
    }

    fn entry_path(&self, entry: &str) -> String {
        format!("{}/{}", self.entries_path(), encode_query_value(entry))
    }

    /// Fetch up to `limit` entries, following pages until it has them.
    ///
    /// `orderBy` and `filter` are both server-side, which matters: sorting or
    /// filtering after the fact would give the top ten of whatever page one
    /// happened to hold rather than the top ten of the store.
    async fn list(
        &self,
        limit: u32,
        descending: bool,
        min: Option<i64>,
        max: Option<i64>,
    ) -> Result<Vec<Entry>> {
        let mut out: Vec<Entry> = Vec::new();
        let mut page_token: Option<String> = None;

        // The filter grammar accepts `>=` and `<=` on the value, joined by
        // `&&`. Nothing else, which is why there is no general `--filter`.
        let filter = match (min, max) {
            (Some(lo), Some(hi)) => Some(format!("entry >= {lo} && entry <= {hi}")),
            (Some(lo), None) => Some(format!("entry >= {lo}")),
            (None, Some(hi)) => Some(format!("entry <= {hi}")),
            (None, None) => None,
        };

        // Fixed for the whole walk, not recomputed per page.
        //
        // The spec is explicit about it: "When paginating, all other parameters
        // provided to the subsequent call must match the call that provided the
        // page token." Shrinking `maxPageSize` as the remaining count fell would
        // send page two with a different value than the call that issued its
        // token, which Roblox is entitled to reject outright.
        //
        // Asking for more rows than `--limit` on the last page costs nothing:
        // `out.truncate` below discards the surplus, and the first page is
        // still sized to the limit so a small `--limit` does not fetch a
        // hundred rows to print three.
        let page_size = 1u32.max(limit.min(MAX_PAGE_SIZE));

        loop {
            if out.len() as u32 >= limit {
                break;
            }

            let mut url = self
                .base
                .join(&format!("{}?maxPageSize={page_size}", self.entries_path()));
            if descending {
                url.push_str("&orderBy=");
                url.push_str(&encode_query_value("value desc"));
            }
            if let Some(filter) = &filter {
                url.push_str("&filter=");
                url.push_str(&encode_query_value(filter));
            }
            if let Some(token) = &page_token {
                // Encoded, not pasted: the token is opaque, and a `+` or `&`
                // in it would silently re-request page one for ever.
                url.push_str("&pageToken=");
                url.push_str(&encode_query_value(token));
            }

            let page: EntryPage = execute_json(|| async {
                Ok(self
                    .client
                    .get(&url)
                    .header("x-api-key", &self.api_key)
                    .send()
                    .await?)
            })
            .await
            .map_err(explain_missing_scope)?;

            let empty = page.ordered_data_store_entries.is_empty();
            out.extend(page.ordered_data_store_entries);

            match page.next_page_token {
                // An empty page with a token would loop for ever otherwise;
                // Roblox has been seen to return one at the end of a filtered
                // listing.
                Some(token) if !token.is_empty() && !empty => page_token = Some(token),
                _ => break,
            }
        }

        out.truncate(limit as usize);
        Ok(out)
    }

    /// One entry, or `None` when it has never been written.
    async fn get(&self, entry: &str) -> Result<Option<Entry>> {
        let url = self.base.join(&self.entry_path(entry));
        let result: Result<Entry> = execute_json(|| async {
            Ok(self
                .client
                .get(&url)
                .header("x-api-key", &self.api_key)
                .send()
                .await?)
        })
        .await;

        match result {
            Ok(entry) => Ok(Some(entry)),
            // A key nobody has written is not an error, here or in the
            // standard-store `get` this mirrors.
            Err(error) if is_api_status(&error, StatusCode::NOT_FOUND) => Ok(None),
            Err(error) => Err(explain_missing_scope(error)),
        }
    }

    async fn set(&self, entry: &str, value: i64, allow_missing: bool) -> Result<Entry> {
        let url = self.base.join(&format!(
            "{}?allowMissing={allow_missing}",
            self.entry_path(entry)
        ));
        execute_json(|| async {
            Ok(self
                .client
                .patch(&url)
                .header("x-api-key", &self.api_key)
                .json(&serde_json::json!({ "value": value }))
                .send()
                .await?)
        })
        .await
        .map_err(explain_missing_scope)
    }

    async fn increment(&self, entry: &str, amount: i64) -> Result<Entry> {
        let url = self
            .base
            .join(&format!("{}:increment", self.entry_path(entry)));
        execute_json(|| async {
            Ok(self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .json(&serde_json::json!({ "amount": amount }))
                .send()
                .await?)
        })
        .await
        .map_err(explain_missing_scope)
    }

    async fn delete(&self, entry: &str) -> Result<()> {
        let url = self.base.join(&self.entry_path(entry));
        execute_with_retry(|| async {
            Ok(self
                .client
                .delete(&url)
                .header("x-api-key", &self.api_key)
                .send()
                .await?)
        })
        .await
        .map_err(explain_missing_scope)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub async fn run(
    command: OrderedCommand,
    base: ApiBase,
    api_key: String,
    universe_id: u64,
    datastore: String,
    scope: String,
) -> Result<()> {
    if datastore.is_empty() {
        bail!(
            "`rbx data ordered` needs --datastore <name>, the name the game passes to \
             GetOrderedDataStore."
        );
    }

    let store = Store {
        datastore: datastore.clone(),
        scope: scope.clone(),
    };
    let api = Api {
        client: build_client(),
        base,
        api_key,
        universe_id,
        datastore,
        scope,
    };

    match command {
        OrderedCommand::List {
            limit,
            asc,
            min,
            max,
            json,
        } => {
            if let (Some(lo), Some(hi)) = (min, max) {
                if lo > hi {
                    bail!("--min {lo} is above --max {hi}; no entry can match both.");
                }
            }
            let format = OutputFormat::from_json_flag(json);
            let entries = api.list(limit, !asc, min, max).await?;

            if entries.is_empty() {
                format.note("No entries.".dimmed());
                if format.is_json() {
                    output::emit(&ListDoc {
                        datastore: &store.datastore,
                        scope: &store.scope,
                        descending: !asc,
                        entries: Vec::new(),
                    })?;
                }
                return Ok(());
            }

            let rows: Vec<EntryRow> = entries
                .iter()
                .enumerate()
                .map(|(i, e)| EntryRow {
                    position: i + 1,
                    id: e.id_or_unknown().to_string(),
                    value: e.value_i64(),
                })
                .collect();

            if format.is_json() {
                output::emit(&ListDoc {
                    datastore: &store.datastore,
                    scope: &store.scope,
                    descending: !asc,
                    entries: rows,
                })?;
                return Ok(());
            }

            let width = rows
                .iter()
                .map(|r| r.value.to_string().len())
                .max()
                .unwrap_or(1);
            for row in &rows {
                println!(
                    "{:>4}  {:>width$}  {}",
                    row.position.to_string().dimmed(),
                    row.value.to_string().bold(),
                    row.id,
                    width = width
                );
            }
            Ok(())
        }

        OrderedCommand::Get { entry, json } => {
            let format = OutputFormat::from_json_flag(json);
            let found = api.get(&entry).await?;

            match found {
                Some(found) => {
                    if format.is_json() {
                        output::emit(&GetDoc {
                            datastore: &store.datastore,
                            scope: &store.scope,
                            entry: &entry,
                            found: true,
                            value: Some(found.value_i64()),
                        })?;
                    } else {
                        println!("{}", found.value_i64());
                    }
                }
                None => {
                    // Same contract as the standard-store `get`: a missing key
                    // is a document saying so, not silence, so a script can
                    // tell it apart from a command that printed nothing.
                    format.note(format!("No entry `{entry}`.").dimmed());
                    if format.is_json() {
                        output::emit(&GetDoc {
                            datastore: &store.datastore,
                            scope: &store.scope,
                            entry: &entry,
                            found: false,
                            value: None,
                        })?;
                    }
                }
            }
            Ok(())
        }

        OrderedCommand::Set {
            entry,
            value,
            no_create,
            yes,
        } => {
            // The previous value is named in the prompt rather than left to be
            // looked up: this write destroys it, and there is no revision
            // history on this resource to get it back from.
            let before = api.get(&entry).await?;
            let was = match &before {
                Some(e) => e.value_i64().to_string(),
                None => "absent".to_string(),
            };
            if before.is_none() && no_create {
                bail!("No entry `{entry}`, and --no-create was given.");
            }
            confirm_always(&format!("Set `{entry}` to {value}? (currently {was})"), yes)?;

            let written = api.set(&entry, value, !no_create).await?;
            println!(
                "{} {} = {}",
                "✓".green(),
                entry,
                written.value_i64().to_string().bold()
            );
            Ok(())
        }

        OrderedCommand::Increment { entry, amount, yes } => {
            confirm_always(&format!("Add {amount} to `{entry}`?"), yes)?;
            let written = api
                .increment(&entry, amount)
                .await
                .with_context(|| format!("incrementing `{entry}`"))?;
            println!(
                "{} {} = {}",
                "✓".green(),
                entry,
                written.value_i64().to_string().bold()
            );
            Ok(())
        }

        OrderedCommand::Delete { entry, yes } => {
            let before = api.get(&entry).await?;
            let Some(before) = before else {
                // Nothing to delete is not a failure, and reporting it as one
                // would make a cleanup script that runs twice fail the second
                // time.
                println!("{}", format!("No entry `{entry}`; nothing to do.").dimmed());
                return Ok(());
            };
            confirm_always(
                &format!(
                    "Delete `{entry}` (currently {})? This is not recoverable.",
                    before.value_i64()
                ),
                yes,
            )?;
            api.delete(&entry).await?;
            println!("{} deleted {}", "✓".green(), entry);
            Ok(())
        }
    }
}
