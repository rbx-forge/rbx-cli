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
//! keeps the value it replaced as a revision, readable for 30 days, so the
//! state as of the snapshot survives one overwrite per key. It is capped at one
//! per experience per UTC day, which makes it something you run deliberately
//! before a migration rather than a standing safety net. Absent one, the backup
//! file is still the only way back.
//!
//! Overwriting is still the right move for resetting a player: it is immediate,
//! the game reads a real profile rather than nothing, and it destroys the old
//! value rather than leaving it readable for a month.

mod api;
pub mod backup;
mod cli;
mod commands;
pub mod json;
pub mod model;
pub mod ordered;
mod support;

pub use cli::DataCli;

use crate::api::Api;
use crate::cli::Command;

use anyhow::{bail, Result};

use rbx_core::api::{build_client, require_api_key, ApiBase};
use rbx_core::GlobalFlags;

use crate::json::Store;

/// Roblox's own default scope. Every game that has never set one is here.
const DEFAULT_SCOPE: &str = "global";

/// Looked for by `data reset` when `--template` is not given, so the common
/// case is one short command in a project that keeps its default profile next
/// to the code that writes it.
const DEFAULT_TEMPLATE: &str = "playerdata.template.json";

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
        // `stores` is what you run *because* you do not know a store name.
        // Demanding one would make the discovery command need its own answer.
        (Command::Stores { .. }, store) => store.unwrap_or_default(),
        // These two name their store positionally, and deliberately: a store
        // being destroyed should be named in the command that destroys it,
        // not inherited from a flag a shell alias may be supplying.
        (Command::DeleteStore { .. }, store) => store.unwrap_or_default(),
        (Command::RestoreStore { .. }, store) => store.unwrap_or_default(),
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

        command => {
            let ctx = commands::Ctx {
                api,
                store,
                universe_id,
                global,
            };
            match commands::group(&command) {
                commands::Group::Store => commands::store::run(&ctx, command).await,
                commands::Group::Entry => commands::entry::run(&ctx, command).await,
                commands::Group::Inspect => commands::inspect::run(&ctx, command).await,
            }
        }
    }
}
