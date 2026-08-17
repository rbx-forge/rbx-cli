//! `rbx-ops data` : read and overwrite one data store entry.
//!
//! Deliberately narrow. The interactive work on data stores, browsing, diffing
//! revisions, restoring one, is better done in DataStoria's editor than in a
//! terminal, and this does not try to replace it.
//!
//! What it exists for is the one thing a delete does not do. Measured against a
//! live universe on 2026-08-03 rather than read off the specification, and the
//! two operations are not symmetric:
//!
//! | | after `set` / `reset` | after `DELETE` |
//! | --- | --- | --- |
//! | a normal read | the new value | nothing, 404 |
//! | the entry in a listing | present | present with `showDeleted=true` |
//! | the previous value | **gone at once** | **readable for 30 days** |
//!
//! The surprising half is the last row, and it is the reason this crate writes
//! a backup file. Overwriting an entry four times leaves only the fourth:
//! `listRevisions` returns one row, and asking for an earlier revision by id
//! answers `404 Entry not found at revision`, even though the revision counter
//! did increment. Deleting is the opposite: the value before the delete stays
//! fetchable at `entries/{id}@{revisionId}` until Roblox purges it.
//!
//! So an overwrite is destructive, and by default unrecoverable through the
//! API: the local copy written before every write here is not a convenience.
//!
//! The exception is `data snapshot`. After one, the next write to every key
//! keeps the value it replaced as a revision, readable for 30 days — so the
//! state as of the snapshot survives one overwrite per key. It is capped at one
//! per experience per UTC day, which makes it something you run deliberately
//! before a migration rather than a standing safety net. Absent one, the backup
//! file is still the only way back.
//!
//! Overwriting is still the right move for resetting a player: it is immediate,
//! the game reads a real profile rather than nothing, and it destroys the old
//! value rather than leaving it readable for a month.

pub mod backup;
pub mod json;
pub mod model;
pub mod ordered;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use colored::Colorize;
use reqwest::{Client, StatusCode};

use rbx_core::api::{
    build_client, encode_query_value, execute_json, execute_with_retry, explain_missing_scope,
    is_api_status, require_api_key, ApiBase,
};
use rbx_core::confirm::confirm_always;
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

use crate::backup::{sanitise_filename, BackupTarget};
use crate::json::{
    DiffDocument, DiffSide, DiffSource, GetDocument, ListDocument, RevisionDocument,
    RevisionsDocument, Store,
};
use crate::model::{DataStoreEntry, EntryList, EntryUpdate, SnapshotResult};

/// Roblox's own default scope. Every game that has never set one is here.
const DEFAULT_SCOPE: &str = "global";

/// Looked for by `data reset` when `--template` is not given, so the common
/// case is one short command in a project that keeps its default profile next
/// to the code that writes it.
const DEFAULT_TEMPLATE: &str = "playerdata.template.json";

#[derive(Args, Debug)]
pub struct DataCli {
    #[command(subcommand)]
    command: Command,

    /// Data store name, as the game passes to `GetDataStore`.
    #[arg(long, global = true)]
    datastore: Option<String>,

    /// Data store scope.
    #[arg(long, global = true, default_value = DEFAULT_SCOPE)]
    scope: String,

    /// Override the API host. For testing against a mock server.
    #[arg(long, hide = true, global = true)]
    base_url: Option<String>,
}

impl DataCli {
    /// Tests only.
    #[doc(hidden)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = Some(url);
        self
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print one entry
    Get {
        /// Entry key, e.g. `Player_156`.
        entry: String,

        /// Write the value to a file instead of printing it.
        #[arg(long)]
        out: Option<PathBuf>,

        /// Write the result to stdout as one JSON document.
        ///
        /// The stored value nested under `value`, plus the revision and
        /// whether the entry is soft-deleted. Nothing else about the player:
        /// the entry's user association and attributes stay out, as they do
        /// in the human form. stdout carries the document and nothing else;
        /// diagnostics stay on stderr. Field names are documented in
        /// docs/ops/data.md.
        #[arg(long)]
        json: bool,
    },

    /// Put a player back to a fresh profile
    ///
    /// The same write as `set`, named for what it is used for. Deleting is the
    /// obvious way to reset somebody and the wrong one: it leaves the game
    /// reading nothing, which behaves however that game's own wrapper decides,
    /// and it erases nothing anyway since the old value stays readable through
    /// its revision. Writing the template you ship with is immediate and
    /// unambiguous.
    Reset {
        /// Entry key.
        entry: String,

        /// The fresh profile to write. Defaults to `playerdata.template.json`
        /// in the working directory.
        #[arg(long)]
        template: Option<PathBuf>,

        /// Write the current value here first. Defaults to a timestamped file
        /// in `.rbx/backups/<env>/`, beside `rbxplace.toml`.
        #[arg(long)]
        backup: Option<PathBuf>,

        /// How many backups of this entry to keep in the default directory.
        #[arg(long, default_value_t = backup::DEFAULT_KEEP,
              value_parser = clap::value_parser!(u32).range(1..),
              conflicts_with_all = ["backup", "no_backup"])]
        keep: u32,

        /// Do not write the local copy at all.
        ///
        /// The copy exists because an overwrite is otherwise unrecoverable
        /// through the API. Skip it when the previous value is already
        /// recoverable — after `data snapshot`, Roblox keeps it as a revision
        /// for 30 days — or when there is nowhere to write, which is the case
        /// in a container with a read-only working directory. Without one of
        /// those, this throws away the only way back.
        #[arg(long, conflicts_with = "backup")]
        no_backup: bool,

        /// Actually write it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Force the next write to every key to keep a backup
    ///
    /// The answer to the warning the rest of this crate carries. Normally an
    /// overwrite is unrecoverable: Roblox keeps the current revision and drops
    /// the one it replaced. After a snapshot, the *next* write to every key in
    /// the experience creates a versioned backup of the previous value first,
    /// and that backup is guaranteed readable for 30 days. So the state as of
    /// the snapshot survives one overwrite per key.
    ///
    /// Run it before a migration or a bulk edit, not on a schedule: Roblox
    /// allows one per experience per UTC day. A second call the same day is a
    /// no-op that reports the standing snapshot's time rather than failing.
    ///
    /// This is why it needs `--apply` despite only ever *adding*
    /// recoverability. Spending the day's snapshot early is not free: a
    /// snapshot at 09:00 protects the values as of 09:00, and a key written at
    /// 10:00 and again at 17:00 keeps the 09:00 value, not the 10:00 one. When
    /// you want the state right before a risky write, you want the snapshot
    /// right before it too.
    ///
    /// Experience-wide, so it needs neither `--datastore` nor `--scope`.
    /// Needs `universe-datastores.control:snapshot`.
    Snapshot {
        /// Actually take it.
        #[arg(long)]
        apply: bool,
    },

    /// List entry keys
    List {
        /// Only keys starting with this.
        #[arg(long)]
        prefix: Option<String>,

        /// Include entries that have been deleted but not yet purged.
        #[arg(long)]
        show_deleted: bool,

        /// Maximum keys to fetch.
        #[arg(long, default_value_t = 100)]
        limit: u32,

        /// Write the keys to stdout as one JSON document.
        ///
        /// The keys, plus what the human form prints around them: the filter
        /// in force, the count, and whether the run stopped at --limit rather
        /// than at the end of the store. stdout carries the document and
        /// nothing else. Field names are documented in docs/ops/data.md.
        #[arg(long)]
        json: bool,
    },

    /// List the revisions of one entry
    ///
    /// Expect fewer than you wrote. Roblox keeps the current revision of a
    /// live entry and discards the ones an overwrite replaced, so this is
    /// mostly useful after a delete, where the value from before the delete
    /// survives. To undo an overwrite, use the backup file the write left
    /// behind.
    ///
    /// Needs `universe-datastores.versions:list`, and reading one needs
    /// `versions:read`, both separate from a plain read.
    Revisions {
        /// Entry key.
        entry: String,

        /// Print this revision's value instead of the list.
        #[arg(long)]
        revision: Option<String>,

        /// Write the result to stdout as one JSON document.
        ///
        /// Two documents, and --revision is what picks between them: without
        /// it, the revision list; with it, that revision's value. Field names
        /// are documented in docs/ops/data.md.
        #[arg(long)]
        json: bool,
    },

    /// Put a past revision back as the current value
    ///
    /// Works when there is a past revision to put back, which after a delete
    /// there is. After an overwrite there usually is not: see `data revisions`.
    Restore {
        /// Entry key.
        entry: String,

        /// Revision id, from `data revisions`.
        #[arg(long)]
        revision: String,

        /// Write the current value here first. Defaults to a timestamped file
        /// in `.rbx/backups/<env>/`, beside `rbxplace.toml`.
        #[arg(long)]
        backup: Option<PathBuf>,

        /// How many backups of this entry to keep in the default directory.
        #[arg(long, default_value_t = backup::DEFAULT_KEEP,
              value_parser = clap::value_parser!(u32).range(1..),
              conflicts_with_all = ["backup", "no_backup"])]
        keep: u32,

        /// Do not write the local copy at all.
        ///
        /// The copy exists because an overwrite is otherwise unrecoverable
        /// through the API. Skip it when the previous value is already
        /// recoverable — after `data snapshot`, Roblox keeps it as a revision
        /// for 30 days — or when there is nowhere to write, which is the case
        /// in a container with a read-only working directory. Without one of
        /// those, this throws away the only way back.
        #[arg(long, conflicts_with = "backup")]
        no_backup: bool,

        /// Actually write it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Copy an entry to another env, another key, or both
    ///
    /// The thing a single-universe tool cannot do: take a profile from
    /// production into staging to reproduce a bug on real data, or onto a test
    /// account. Source and destination are named explicitly; neither defaults
    /// to `--env`, so no copy happens because a flag was forgotten.
    Copy {
        /// Entry key to read.
        entry: String,

        /// Env to read from.
        #[arg(long)]
        from: String,

        /// Env to write to. May be the same as `--from`.
        #[arg(long)]
        to: String,

        /// Key to write. Defaults to the same key.
        #[arg(long)]
        to_entry: Option<String>,

        /// Write the destination's current value here first. Defaults to a
        /// timestamped file in `.rbx/backups/<to>/`, the destination env.
        #[arg(long)]
        backup: Option<PathBuf>,

        /// How many backups of this entry to keep in the default directory.
        #[arg(long, default_value_t = backup::DEFAULT_KEEP,
              value_parser = clap::value_parser!(u32).range(1..),
              conflicts_with_all = ["backup", "no_backup"])]
        keep: u32,

        /// Do not write the local copy at all.
        ///
        /// The copy exists because an overwrite is otherwise unrecoverable
        /// through the API. Skip it when the previous value is already
        /// recoverable — after `data snapshot`, Roblox keeps it as a revision
        /// for 30 days — or when there is nowhere to write, which is the case
        /// in a container with a read-only working directory. Without one of
        /// those, this throws away the only way back.
        #[arg(long, conflicts_with = "backup")]
        no_backup: bool,

        /// Actually write it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Add to a numeric entry, atomically
    ///
    /// Not the same as reading and writing back: two people granting currency
    /// at the same time both land, where `set` would lose one of them.
    Increment {
        /// Entry key.
        entry: String,

        /// Amount to add. Negative subtracts.
        #[arg(long, allow_negative_numbers = true)]
        by: i64,

        /// Actually write it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Compare two revisions, or the same key across two envs
    ///
    /// Writes both sides to files and hands them to a diff tool rather than
    /// rendering a diff here. `code --diff`, `git diff --no-index` and anything
    /// else already knows how to show this better than a terminal can.
    Diff {
        /// Entry key.
        entry: String,

        /// Two revision ids, comma separated.
        #[arg(long, conflicts_with = "between")]
        revisions: Option<String>,

        /// Two env names, comma separated.
        #[arg(long)]
        between: Option<String>,

        /// Open the pair in a diff tool.
        ///
        /// Uses `$RBX_DIFF_TOOL` if set, otherwise `code --diff`, otherwise
        /// `git diff --no-index`. Without this the two file paths are printed
        /// for you to open however you like.
        #[arg(long)]
        open: bool,

        /// Write the two paths to stdout as one JSON document.
        ///
        /// The paths and what each side is, never the two values: they are on
        /// disk, which is where the human form leaves them too. Rejected
        /// together with --open, which hands stdout to a diff tool and would
        /// write somebody else's output into the document. Field names are
        /// documented in docs/ops/data.md.
        #[arg(long, conflicts_with = "open")]
        json: bool,
    },

    /// Overwrite one entry with an arbitrary value
    Set {
        /// Entry key.
        entry: String,

        /// New value, as JSON on the command line.
        #[arg(long, conflicts_with = "file")]
        value: Option<String>,

        /// New value, read from a file. Easier than quoting JSON in a shell.
        #[arg(long)]
        file: Option<PathBuf>,

        /// Write the current value here before overwriting it.
        ///
        /// Defaults to a timestamped file in `.rbx/backups/<env>/`, beside
        /// `rbxplace.toml`. Roblox keeps revisions too, but a local copy needs
        /// no API call and no scope to read back.
        #[arg(long)]
        backup: Option<PathBuf>,

        /// How many backups of this entry to keep in the default directory.
        ///
        /// The oldest beyond this are deleted after the new one lands. Only
        /// this entry's own backups are counted, and only in the default
        /// directory: `--backup <path>` writes where it is told and prunes
        /// nothing.
        #[arg(long, default_value_t = backup::DEFAULT_KEEP,
              value_parser = clap::value_parser!(u32).range(1..),
              conflicts_with_all = ["backup", "no_backup"])]
        keep: u32,

        /// Do not write the local copy at all.
        ///
        /// The copy exists because an overwrite is otherwise unrecoverable
        /// through the API. Skip it when the previous value is already
        /// recoverable — after `data snapshot`, Roblox keeps it as a revision
        /// for 30 days — or when there is nowhere to write, which is the case
        /// in a container with a read-only working directory. Without one of
        /// those, this throws away the only way back.
        #[arg(long, conflicts_with = "backup")]
        no_backup: bool,

        /// Do not keep the entry's `users` and `attributes`.
        ///
        /// They are preserved by default. `users` is the association Roblox
        /// uses to answer a player's data request, so dropping it should be a
        /// decision rather than a side effect.
        #[arg(long)]
        drop_metadata: bool,

        /// Actually write it.
        #[arg(long)]
        apply: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Ordered data stores: the leaderboard resource.
    ///
    /// A different Open Cloud resource from the verbs above, not a mode of
    /// them: integer values, server-side ordering, and no revision history at
    /// all. `--datastore` names it the same way, and `--scope` applies.
    ///
    /// Nothing here writes a backup file, because there is nothing to back
    /// up: an ordered entry is one integer with no revision history behind it,
    /// so there would be nothing to reconstruct.
    Ordered {
        #[command(subcommand)]
        command: ordered::OrderedCommand,
    },
}

struct Api {
    client: Client,
    base: ApiBase,
    api_key: String,
    universe_id: u64,
    datastore: String,
    scope: String,
}

impl Api {
    fn entry_url(&self, entry: &str) -> String {
        self.base.join(&format!(
            "/cloud/v2/universes/{}/data-stores/{}/scopes/{}/entries/{}",
            self.universe_id,
            encode_query_value(&self.datastore),
            encode_query_value(&self.scope),
            encode_query_value(entry),
        ))
    }

    async fn get(&self, entry: &str) -> Result<Option<DataStoreEntry>> {
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
            // contain "404" — that read as a missing entry and made `get`
            // deny a key that was sitting right there.
            Err(error) if is_api_status(&error, StatusCode::NOT_FOUND) => Ok(None),
            Err(error) => Err(explain_missing_scope(error)),
        }
    }

    /// One page of entry ids. Only `id` and `path` come back; reading a value
    /// takes a second call, which is why listing is cheap and dumping is not.
    async fn list(
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

    async fn revisions(&self, entry: &str) -> Result<EntryList> {
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
    async fn get_revision(&self, entry: &str, revision: &str) -> Result<DataStoreEntry> {
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
    async fn increment(&self, entry: &str, by: i64) -> Result<DataStoreEntry> {
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
    async fn snapshot(&self) -> Result<SnapshotResult> {
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

    async fn set(&self, entry: &str, update: &EntryUpdate) -> Result<DataStoreEntry> {
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

pub async fn run(cli: DataCli, global: &GlobalFlags) -> Result<()> {
    if global.env.as_deref() == Some("all") {
        bail!(
            "`--env all` is refused here. Overwriting player data in every environment because \
             a glob matched is not something anybody means to do."
        );
    }
    // `snapshot` is experience-wide, so it is the one subcommand that has no
    // store to name. Demanding `--datastore` for it would be asking for a
    // value the call does not use.
    let datastore = match (&cli.command, cli.datastore.clone()) {
        (Command::Snapshot { .. }, store) => store.unwrap_or_default(),
        // `ordered` raises its own error, naming `GetOrderedDataStore` rather
        // than `GetDataStore`. Sending somebody to the wrong Luau function is
        // a small thing that costs a real detour.
        (Command::Ordered { .. }, store) => store.unwrap_or_default(),
        (_, Some(store)) => store,
        (_, None) => bail!(
            "`rbx-ops data` needs --datastore <name>, the name the game passes to GetDataStore."
        ),
    };

    let base = match &cli.base_url {
        Some(url) => ApiBase::new(url.clone()),
        None => ApiBase::default(),
    };
    let api_key = require_api_key(global.api_key.as_deref())?.to_string();
    let build = |universe_id: u64| Api {
        client: build_client(),
        base: base.clone(),
        api_key: api_key.clone(),
        universe_id,
        datastore: datastore.clone(),
        scope: cli.scope.clone(),
    };

    let universe_id = global.single_universe()?;
    let api = build(universe_id);
    // What every `--json` document here says it is a document of. Both halves
    // come off the command line, and both decide which keys are even visible,
    // so a saved file says which store and scope it was read from.
    let store = Store {
        datastore: datastore.clone(),
        scope: cli.scope.clone(),
    };

    match cli.command {
        Command::Ordered { command } => {
            ordered::run(
                command,
                base,
                api_key,
                universe_id,
                datastore,
                cli.scope.clone(),
            )
            .await
        }

        Command::Snapshot { apply } => {
            if !apply {
                println!(
                    "Would take a data store snapshot for universe {universe_id}.\n\
                     Every key's next write would then keep the current value as a revision, \
                     readable for 30 days.\n\
                     One snapshot is allowed per experience per UTC day."
                );
                println!("{}", "Nothing sent. Re-run with --apply.".dimmed());
                return Ok(());
            }
            let result = api.snapshot().await?;
            let when = result.latest_snapshot_time.as_deref().unwrap_or("unknown");
            if result.new_snapshot_taken {
                println!("{}", format!("✓ Snapshot taken at {when}.").green());
                println!(
                    "{}",
                    "The next write to each key now keeps the current value as a revision."
                        .dimmed()
                );
            } else {
                // Not a failure: Roblox allows one per UTC day and reports the
                // standing one instead of erroring.
                println!(
                    "{}",
                    format!("Already snapshotted today; the standing snapshot is from {when}.")
                        .yellow()
                );
            }
            Ok(())
        }

        Command::Get { entry, out, json } => {
            let format = OutputFormat::from_json_flag(json);
            let Some(found) = api.get(&entry).await? else {
                // A key that was never written is not an error in either
                // format. Under `--json` it is a document saying `found:
                // false` rather than silence, so a script can tell "no such
                // key" from "the command printed nothing".
                format.note(format!("No entry `{entry}`.").dimmed());
                if format.is_json() {
                    output::emit(&GetDocument::missing(&store, &entry))?;
                }
                return Ok(());
            };
            let value = found.value.clone().unwrap_or(serde_json::Value::Null);
            let pretty = serde_json::to_string_pretty(&value)?;

            if found.is_deleted() {
                // Still readable, because a delete only marks it. Saying so
                // avoids the conclusion that deleting worked.
                eprintln!(
                    "{} this entry is marked deleted and still readable. Roblox removes it \
                     permanently thirty days after the delete.",
                    "note:".yellow().bold()
                );
            }
            if let Some(revision) = &found.revision_id {
                eprintln!("{}", format!("revision {revision}").dimmed());
            }

            match out {
                Some(path) => {
                    std::fs::write(&path, format!("{pretty}\n"))
                        .with_context(|| format!("writing {}", path.display()))?;
                    if format.is_json() {
                        output::emit(&GetDocument::found(&store, &entry, &found, Some(&path)))?;
                    } else {
                        println!("wrote {}", path.display());
                    }
                }
                None => {
                    if format.is_json() {
                        output::emit(&GetDocument::found(&store, &entry, &found, None))?;
                    } else {
                        println!("{pretty}");
                    }
                }
            }
            Ok(())
        }

        Command::List {
            prefix,
            show_deleted,
            limit,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);
            let mut ids = Vec::new();
            let mut token: Option<String> = None;
            while (ids.len() as u32) < limit {
                let page = api
                    .list(prefix.as_deref(), show_deleted, token.as_deref())
                    .await?;
                let next = page.next_token().map(str::to_string);
                for entry in page.data_store_entries {
                    if let Some(id) = entry.base_id() {
                        ids.push(id.to_string());
                    }
                }
                match next {
                    Some(value) => token = Some(value),
                    None => break,
                }
            }
            ids.truncate(limit as usize);

            if ids.is_empty() {
                // Said in both formats, on the stream that cannot corrupt the
                // document. A prefix matching nothing is a normal answer.
                format.note("No entries.".dimmed());
            }
            if format.is_json() {
                return output::emit(&ListDocument::new(
                    &store,
                    prefix.as_deref(),
                    show_deleted,
                    limit,
                    &ids,
                ));
            }
            if ids.is_empty() {
                return Ok(());
            }
            for id in &ids {
                println!("{id}");
            }
            eprintln!();
            eprintln!("{}", format!("{} key(s)", ids.len()).dimmed());
            Ok(())
        }

        Command::Revisions {
            entry,
            revision,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);
            if let Some(revision) = revision {
                let found = api.get_revision(&entry, &revision).await?;
                if format.is_json() {
                    return output::emit(&RevisionDocument::new(
                        &store,
                        &entry,
                        &revision,
                        found.value,
                    ));
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&found.value.unwrap_or(serde_json::Value::Null))?
                );
                return Ok(());
            }

            let list = api.revisions(&entry).await?;
            if list.data_store_entries.is_empty() {
                format.note(format!("No revisions for `{entry}`.").dimmed());
            }
            if format.is_json() {
                return output::emit(&RevisionsDocument::new(
                    &store,
                    &entry,
                    &list.data_store_entries,
                ));
            }
            if list.data_store_entries.is_empty() {
                return Ok(());
            }
            println!(
                "{:<24}  {:<10}  {}",
                "WHEN".bold(),
                "STATE".bold(),
                "REVISION".bold()
            );
            for found in &list.data_store_entries {
                let when = found
                    .revision_create_time
                    .as_deref()
                    .map(|stamp| {
                        stamp
                            .replace('T', " ")
                            .split('.')
                            .next()
                            .unwrap_or(stamp)
                            .to_string()
                    })
                    .unwrap_or_else(|| "-".into());
                let state = found.state.as_deref().unwrap_or("?");
                let coloured = if found.is_deleted() {
                    state.yellow()
                } else {
                    state.normal()
                };
                println!(
                    "{when:<24}  {coloured:<10}  {}",
                    found.revision_id.as_deref().unwrap_or("-")
                );
            }
            println!();
            println!(
                "{}",
                "Read one with `data revisions <entry> --revision <id>`, put it back with \
                 `data restore`."
                    .dimmed()
            );
            Ok(())
        }

        Command::Restore {
            entry,
            revision,
            backup,
            keep,
            no_backup,
            apply,
            yes,
        } => {
            let found = api.get_revision(&entry, &revision).await?;
            let raw = serde_json::to_string(&found.value.unwrap_or(serde_json::Value::Null))?;
            println!(
                "{}",
                format!("restoring `{entry}` from revision {revision}").dimmed()
            );
            write_entry(
                &api,
                &entry,
                &raw,
                WriteOptions {
                    backup: backup_target(
                        BackupFlags {
                            backup,
                            no_backup,
                            keep,
                        },
                        global,
                        global.env.as_deref(),
                        universe_id,
                    ),
                    drop_metadata: false,
                    apply,
                    yes,
                },
                universe_id,
            )
            .await
        }

        Command::Copy {
            entry,
            from,
            to,
            to_entry,
            backup,
            keep,
            no_backup,
            apply,
            yes,
        } => {
            let source_universe = rbx_core::places::resolve_universe_id(&global.places, &from)
                .with_context(|| format!("resolving --from env `{from}`"))?;
            let target_universe = rbx_core::places::resolve_universe_id(&global.places, &to)
                .with_context(|| format!("resolving --to env `{to}`"))?;
            let target_entry = to_entry.unwrap_or_else(|| entry.clone());

            if source_universe == target_universe && entry == target_entry {
                bail!("source and destination are the same entry in the same universe");
            }

            let source = build(source_universe);
            let Some(found) = source.get(&entry).await? else {
                bail!("no entry `{entry}` in env `{from}`, nothing to copy");
            };
            let raw = serde_json::to_string(&found.value.unwrap_or(serde_json::Value::Null))?;

            println!(
                "{}",
                format!(
                    "copying {from}/{entry} ({source_universe}) into {to}/{target_entry} ({target_universe})"
                )
                .dimmed()
            );
            // The destination's `users` is what matters, not the source's: the
            // copy belongs to whoever owns the destination key, and carrying
            // the source's association across would attach one player's profile
            // to another player's record.
            let target = build(target_universe);
            write_entry(
                &target,
                &target_entry,
                &raw,
                WriteOptions {
                    // The destination env names the directory: the value being
                    // replaced is the destination's, so that is where its copy
                    // belongs.
                    backup: backup_target(
                        BackupFlags {
                            backup,
                            no_backup,
                            keep,
                        },
                        global,
                        Some(&to),
                        target_universe,
                    ),
                    drop_metadata: false,
                    apply,
                    yes,
                },
                target_universe,
            )
            .await
        }

        Command::Increment {
            entry,
            by,
            apply,
            yes,
        } => {
            let existing = api.get(&entry).await?;
            let current = existing
                .as_ref()
                .and_then(|e| e.value.as_ref())
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            println!("{}", format!("entry {entry}").bold());
            println!("  current    {current}");
            println!("  change     {by:+}");

            if !apply {
                println!();
                println!("{}", "Nothing written. Re-run with --apply.".yellow());
                return Ok(());
            }
            confirm_always(
                &format!("Change `{entry}` by {by:+} in universe {universe_id}?"),
                yes,
            )?;
            let written = api.increment(&entry, by).await?;
            println!(
                "{} {entry} is now {}",
                "done".green().bold(),
                written.value.unwrap_or(serde_json::Value::Null)
            );
            Ok(())
        }

        Command::Diff {
            entry,
            revisions,
            between,
            open,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);
            let (left, right) = match (&revisions, &between) {
                (Some(pair), _) => {
                    let (a, b) = split_pair(pair, "--revisions")?;
                    (
                        Side {
                            label: format!("{entry}@{a}"),
                            value: api.get_revision(&entry, &a).await?.value,
                            source: DiffSource::Revision(a),
                        },
                        Side {
                            label: format!("{entry}@{b}"),
                            value: api.get_revision(&entry, &b).await?.value,
                            source: DiffSource::Revision(b),
                        },
                    )
                }
                (None, Some(pair)) => {
                    let (a, b) = split_pair(pair, "--between")?;
                    let left_universe = rbx_core::places::resolve_universe_id(&global.places, &a)?;
                    let right_universe = rbx_core::places::resolve_universe_id(&global.places, &b)?;
                    // Two different env names can point at one universe, which
                    // would write the same file twice and diff it against
                    // itself. Comparing the names is not enough.
                    if left_universe == right_universe {
                        bail!(
                            "envs `{a}` and `{b}` are both universe {left_universe}, so there is                              nothing to compare"
                        );
                    }
                    let left_api = build(left_universe);
                    let right_api = build(right_universe);
                    (
                        Side {
                            label: format!("{a}-{entry}"),
                            value: left_api.get(&entry).await?.and_then(|e| e.value),
                            source: DiffSource::Env(a),
                        },
                        Side {
                            label: format!("{b}-{entry}"),
                            value: right_api.get(&entry).await?.and_then(|e| e.value),
                            source: DiffSource::Env(b),
                        },
                    )
                }
                (None, None) => bail!("pass --revisions <a,b> or --between <env1,env2>"),
            };

            let dir = std::env::temp_dir();
            let left_path = dir.join(format!("{}.json", sanitise_filename(&left.label)));
            let right_path = dir.join(format!("{}.json", sanitise_filename(&right.label)));
            for (path, value) in [(&left_path, &left.value), (&right_path, &right.value)] {
                let pretty = serde_json::to_string_pretty(
                    value.as_ref().unwrap_or(&serde_json::Value::Null),
                )?;
                std::fs::write(path, format!("{pretty}\n"))
                    .with_context(|| format!("writing {}", path.display()))?;
            }

            if format.is_json() {
                // The paths, not the values. Both files are already on disk;
                // putting two player profiles through the pipe as well would
                // say more than the human form ever has.
                return output::emit(&DiffDocument::new(
                    &store,
                    &entry,
                    DiffSide::new(&left.label, &left_path, &left.source),
                    DiffSide::new(&right.label, &right_path, &right.source),
                ));
            }
            if open {
                open_diff(&left_path, &right_path)?;
            } else {
                println!("{}", left_path.display());
                println!("{}", right_path.display());
                println!();
                println!(
                    "{}",
                    "Pass --open to hand these to a diff tool, or open them yourself.".dimmed()
                );
            }
            Ok(())
        }

        Command::Reset {
            entry,
            template,
            backup,
            keep,
            no_backup,
            apply,
            yes,
        } => {
            let path = template.unwrap_or_else(|| PathBuf::from(DEFAULT_TEMPLATE));
            if !path.exists() {
                bail!(
                    "no template at {}. Point --template at the default profile your game                      writes for a new player, or put one at {DEFAULT_TEMPLATE}.",
                    path.display()
                );
            }
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            println!("{}", format!("resetting from {}", path.display()).dimmed());
            write_entry(
                &api,
                &entry,
                &raw,
                WriteOptions {
                    backup: backup_target(
                        BackupFlags {
                            backup,
                            no_backup,
                            keep,
                        },
                        global,
                        global.env.as_deref(),
                        universe_id,
                    ),
                    drop_metadata: false,
                    apply,
                    yes,
                },
                universe_id,
            )
            .await
        }

        Command::Set {
            entry,
            value,
            file,
            backup,
            keep,
            no_backup,
            drop_metadata,
            apply,
            yes,
        } => {
            let raw = match (&value, &file) {
                (Some(inline), _) => inline.clone(),
                (None, Some(path)) => std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?,
                (None, None) => bail!("pass --value '<json>' or --file <path>"),
            };
            write_entry(
                &api,
                &entry,
                &raw,
                WriteOptions {
                    backup: backup_target(
                        BackupFlags {
                            backup,
                            no_backup,
                            keep,
                        },
                        global,
                        global.env.as_deref(),
                        universe_id,
                    ),
                    drop_metadata,
                    apply,
                    yes,
                },
                universe_id,
            )
            .await
        }
    }
}

/// One side of `data diff`, before it reaches a file.
///
/// The three facts travel together: what to call the file, what goes in it,
/// and which comparison the side came from. Keeping the last one rather than
/// re-deriving it later is what lets the JSON document say `revision` or `env`
/// without parsing the label back apart.
struct Side {
    label: String,
    value: Option<serde_json::Value>,
    source: DiffSource,
}

/// The four decisions a write carries, which four commands each pass through
/// unchanged. Grouped because they travel together and always have: as separate
/// parameters they were three consecutive bools at one call site, which is a
/// swap waiting to happen.
struct WriteOptions {
    backup: BackupTarget,
    drop_metadata: bool,
    apply: bool,
    yes: bool,
}

/// The three backup flags, resolved against the env the write lands in.
///
/// Kept as a struct rather than four positional arguments because the two
/// paths (`--backup`, `--places`) and the two names (env, entry) are all
/// strings and paths that read alike at a call site.
struct BackupFlags {
    backup: Option<PathBuf>,
    no_backup: bool,
    keep: u32,
}

/// Which subdirectory of `.rbx/backups/` this write belongs to.
///
/// `--env` is the operator's own name for the target and the one they will
/// look under months later. Without it — `--universe-id` on its own — the
/// universe is the only name there is, and it is still better than one shared
/// pile.
fn env_label(env: Option<&str>, universe_id: u64) -> String {
    match env {
        Some(name) => name.to_string(),
        None => format!("universe-{universe_id}"),
    }
}

/// Resolve the backup flags for a write landing in `env` / `universe_id`.
///
/// `global.places` is read for its directory only: the default backup
/// directory sits beside `rbxplace.toml`, so a copy of a player profile is
/// where the project is rather than where the shell was.
fn backup_target(
    flags: BackupFlags,
    global: &GlobalFlags,
    env: Option<&str>,
    universe_id: u64,
) -> BackupTarget {
    BackupTarget::resolve(
        flags.backup,
        flags.no_backup,
        flags.keep,
        &global.places,
        &env_label(env, universe_id),
    )
}

/// The shared write path behind `set`, `reset`, `restore` and `copy`.
async fn write_entry(
    api: &Api,
    entry: &str,
    raw: &str,
    options: WriteOptions,
    universe_id: u64,
) -> Result<()> {
    let WriteOptions {
        backup,
        drop_metadata,
        apply,
        yes,
    } = options;
    let new_value: serde_json::Value =
        serde_json::from_str(raw).context("the new value must be valid JSON")?;

    let existing = api.get(entry).await?;

    println!("{}", format!("entry {entry}").bold());
    match &existing {
        Some(found) => {
            let current = found.value.clone().unwrap_or(serde_json::Value::Null);
            println!("{}", "  current".dimmed());
            println!("{}", indent(&serde_json::to_string_pretty(&current)?));
            if let Some(users) = &found.users {
                println!("  users      {}", users.join(", ").dimmed());
            }
        }
        None => println!("{}", "  does not exist yet, it will be created".dimmed()),
    }
    println!("{}", "  new".dimmed());
    println!("{}", indent(&serde_json::to_string_pretty(&new_value)?));

    let update = if drop_metadata {
        EntryUpdate::bare(new_value)
    } else {
        EntryUpdate::preserving(new_value, existing.as_ref())
    };
    if drop_metadata && existing.as_ref().and_then(|e| e.users.as_ref()).is_some() {
        println!(
            "{}",
            "  --drop-metadata: the user association will be removed".yellow()
        );
    }
    println!();

    if !apply {
        println!(
            "{}",
            "Nothing written. Re-run with --apply to overwrite.".yellow()
        );
        return Ok(());
    }

    // Written before the request, not after: if the write succeeds and the
    // value was wrong, this file is what gets you back, and asking Roblox for
    // the old revision needs another scope and another call.
    match (&existing, &backup) {
        (Some(found), BackupTarget::Path(_) | BackupTarget::Managed { .. }) => {
            let contents = serde_json::to_string_pretty(
                &found.value.clone().unwrap_or(serde_json::Value::Null),
            )?;
            let written = backup::write(&backup, entry, &contents)?;
            println!("backup written to {}", written.path.display());
            if written.pruned > 0 {
                println!(
                    "{}",
                    format!(
                        "  {} older backup(s) of {entry} removed by --keep",
                        written.pruned
                    )
                    .dimmed()
                );
            }
        }
        // Said out loud every time. The prompt below is the last chance to
        // stop, and it should not be the first place you learn that the value
        // about to be replaced is not being kept anywhere.
        (Some(_), BackupTarget::Skip) => {
            println!(
                "{}",
                "--no-backup: no local copy. Unless this experience has been snapshotted today, \
                 the current value is gone the moment this write lands."
                    .yellow()
            );
        }
        // Nothing to copy: the entry does not exist yet, so the write creates
        // it and there is no previous value to lose.
        (None, _) => {}
    }

    confirm_always(
        &format!("Overwrite `{entry}` in universe {universe_id}?"),
        yes,
    )?;

    let written = api.set(entry, &update).await?;
    println!(
        "{} {entry} is now revision {}",
        "done".green().bold(),
        written.revision_id.as_deref().unwrap_or("(unknown)")
    );
    Ok(())
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indentation_keeps_every_line_of_a_json_block() {
        assert_eq!(indent("{\n  \"a\": 1\n}"), "    {\n      \"a\": 1\n    }");
    }
}

/// Where `--json` is allowed to appear, and why that is a rule rather than an
/// arrangement.
#[cfg(test)]
mod json_flag_tests {
    use super::*;

    #[derive(clap::Parser)]
    struct Wrapper {
        #[command(flatten)]
        data: DataCli,
    }

    fn parses(args: &[&str]) -> bool {
        let mut argv = vec!["data", "--datastore", "PlayerData"];
        argv.extend_from_slice(args);
        <Wrapper as clap::Parser>::try_parse_from(argv).is_ok()
    }

    /// A format that owns stdout may not stop to ask a question: that is
    /// `OutputFormat::may_prompt`, and it is false for `Json` whatever the
    /// terminal looks like. Every writing subcommand here asks one, through
    /// `confirm_always`, so none of them carries the flag — the guarantee is
    /// structural rather than a check somebody has to remember to write.
    ///
    /// This pins it. Adding `--json` to `set` would fail here, before it could
    /// make `dialoguer` draw a prompt into somebody's pipe.
    #[test]
    fn json_is_confined_to_the_subcommands_that_never_prompt() {
        assert!(!OutputFormat::Json.may_prompt());

        for reading in [
            vec!["get", "Player_156", "--json"],
            vec!["list", "--json"],
            vec!["revisions", "Player_156", "--json"],
            vec!["diff", "Player_156", "--revisions", "a,b", "--json"],
        ] {
            assert!(parses(&reading), "{reading:?} should take --json");
        }

        for writing in [
            vec!["set", "Player_156", "--value", "1", "--json"],
            vec!["reset", "Player_156", "--json"],
            vec!["restore", "Player_156", "--revision", "r1", "--json"],
            vec!["copy", "Player_156", "--from", "a", "--to", "b", "--json"],
            vec!["increment", "Player_156", "--by", "1", "--json"],
            vec!["snapshot", "--json"],
        ] {
            assert!(!parses(&writing), "{writing:?} must not take --json");
        }
    }

    /// `--open` hands stdout to `git diff --no-index` and the terminal to
    /// `code --diff`. Under `--json` that ruins the document exactly as a
    /// prompt would, so the pair is refused at parse time rather than one
    /// quietly winning.
    #[test]
    fn diff_refuses_open_and_json_together() {
        assert!(parses(&["diff", "E", "--revisions", "a,b", "--open"]));
        assert!(parses(&["diff", "E", "--revisions", "a,b", "--json"]));
        assert!(!parses(&[
            "diff",
            "E",
            "--revisions",
            "a,b",
            "--open",
            "--json"
        ]));
    }
}

/// Split `a,b` into its two halves, rejecting anything else.
fn split_pair(raw: &str, flag: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    match parts.as_slice() {
        // Both sides equal would write one file twice and diff it against
        // itself, which looks like "no differences" and means nothing.
        [a, b] if a == b => bail!("{flag} needs two different values, got `{a}` twice"),
        [a, b] => Ok((a.to_string(), b.to_string())),
        _ => bail!("{flag} takes exactly two values separated by a comma, got `{raw}`"),
    }
}

/// Hand two files to whatever diff tool is available.
///
/// Nothing is rendered here on purpose. `code --diff` is the same side-by-side
/// view DataStoria provides, `git diff --no-index` works everywhere a
/// repository does, and either beats a diff written for a terminal. The
/// override exists because the right answer is whatever the reader already
/// uses.
fn open_diff(left: &std::path::Path, right: &std::path::Path) -> Result<()> {
    let candidates: Vec<Vec<String>> = match std::env::var("RBX_DIFF_TOOL") {
        Ok(custom) if !custom.trim().is_empty() => {
            vec![custom.split_whitespace().map(str::to_string).collect()]
        }
        _ => vec![
            vec!["code".into(), "--diff".into()],
            vec!["git".into(), "diff".into(), "--no-index".into()],
        ],
    };

    for command in &candidates {
        let (program, leading) = command.split_first().expect("no empty candidate");
        let status = std::process::Command::new(program)
            .args(leading)
            .arg(left)
            .arg(right)
            .status();
        match status {
            // `git diff --no-index` exits 1 when the files differ, which is the
            // normal outcome here and not a failure.
            Ok(_) => return Ok(()),
            Err(_) => continue,
        }
    }

    bail!(
        "no diff tool found. Set RBX_DIFF_TOOL, or open these yourself:\n  {}\n  {}",
        left.display(),
        right.display()
    )
}

#[cfg(test)]
mod pair_tests {
    use super::*;

    #[test]
    fn a_pair_splits_on_the_comma_and_trims() {
        assert_eq!(
            split_pair(" prod , staging ", "--between").unwrap(),
            ("prod".to_string(), "staging".to_string())
        );
    }

    #[test]
    fn anything_that_is_not_exactly_two_values_is_rejected() {
        for bad in ["", "one", "a,b,c", ",", "a,"] {
            assert!(
                split_pair(bad, "--between").is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn the_same_value_twice_is_rejected() {
        // Both sides equal produced one file written twice, so the diff
        // compared it against itself and always reported no change.
        let error = split_pair("ops,ops", "--between").unwrap_err().to_string();
        assert!(error.contains("two different"), "got: {error}");
        assert!(split_pair(" prod , prod ", "--between").is_err());
    }

    #[test]
    fn the_error_names_the_flag_that_was_wrong() {
        let error = split_pair("nope", "--revisions").unwrap_err().to_string();
        assert!(error.contains("--revisions"), "got: {error}");
    }
}
