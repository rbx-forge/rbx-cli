//! The subcommands that act on a store, or on the experience, rather than on
//! one entry: `snapshot`, `stores`, `delete-store`, `restore-store`.
//!
//! Grouped because none of them reads or writes a value. What they have in
//! common is that the thing named on the command line is a container, so the
//! blast radius of a mistake is every key inside it.

use anyhow::Result;
use colored::Colorize;

use rbx_core::confirm::confirm_by_typing;
use rbx_core::output::{self, OutputFormat};

use crate::cli::Command;
use crate::commands::Ctx;
use crate::json::*;
use crate::model::*;
pub(crate) async fn run(ctx: &Ctx<'_>, command: Command) -> Result<()> {
    let api = &ctx.api;
    let store = &ctx.store;
    let universe_id = ctx.universe_id;
    let global = ctx.global;
    let build = |id: u64| ctx.build(id);
    let _ = (store, universe_id, global, &build);

    match command {
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

        Command::Stores {
            show_deleted,
            limit,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);
            let mut stores: Vec<DataStore> = Vec::new();
            let mut token: Option<String> = None;
            while (stores.len() as u32) < limit {
                let page = api.stores(show_deleted, token.as_deref()).await?;
                let next = page.next_token().map(str::to_string);
                for store in page.data_stores {
                    if store.name().is_some() {
                        stores.push(store);
                    }
                }
                match next {
                    Some(value) => token = Some(value),
                    None => break,
                }
            }
            stores.truncate(limit as usize);

            if stores.is_empty() {
                // Not an error, and worth saying plainly: a store exists from
                // its first write, so an experience nothing has written to yet
                // really does have none.
                format.note("No data stores. One exists from its first write.".dimmed());
            }
            if format.is_json() {
                return output::emit(&StoresDocument::new(show_deleted, limit, &stores));
            }
            if stores.is_empty() {
                return Ok(());
            }
            for store in &stores {
                let name = store.name().unwrap_or_default();
                if store.is_deleted() {
                    println!("{name} {}", "(deleted)".dimmed());
                } else {
                    println!("{name}");
                }
            }
            eprintln!();
            eprintln!("{}", format!("{} data store(s)", stores.len()).dimmed());
            Ok(())
        }

        Command::DeleteStore {
            store: name,
            apply,
            yes,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);

            if !apply {
                // Named before anything is sent, so a dry run reads back the
                // store it would have removed. That is the whole value of the
                // dry run here: the danger is the name, not the operation.
                format.note(
                    format!("Would remove data store `{name}` and every entry in it.").dimmed(),
                );
                format.note("Add --apply to do it.".dimmed());
                if format.is_json() {
                    output::emit(&StoreWriteDocument::new(&name, "delete-store", false))?;
                }
                return Ok(());
            }

            confirm_by_typing(
                &name,
                &format!("Removing data store `{name}` and every entry in it."),
                yes,
            )?;

            api.delete_store(&name).await?;

            if format.is_json() {
                return output::emit(&StoreWriteDocument::new(&name, "delete-store", true));
            }
            println!("{}", format!("Removed data store `{name}`.").green());
            // Said here rather than in the help alone, because this is the
            // moment somebody realises they meant a different store.
            eprintln!(
                "{}",
                format!(
                    "`rbx data restore-store {name}` brings it back, while Roblox still holds it."
                )
                .dimmed()
            );
            Ok(())
        }

        Command::RestoreStore {
            store: name,
            apply,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);

            if !apply {
                format.note(format!("Would bring data store `{name}` back.").dimmed());
                format.note("Add --apply to do it.".dimmed());
                if format.is_json() {
                    output::emit(&StoreWriteDocument::new(&name, "restore-store", false))?;
                }
                return Ok(());
            }

            api.restore_store(&name).await?;

            if format.is_json() {
                return output::emit(&StoreWriteDocument::new(&name, "restore-store", true));
            }
            println!("{}", format!("Restored data store `{name}`.").green());
            Ok(())
        }
        // Every other variant is routed elsewhere by `run` in lib.rs. Reaching
        // this arm would mean the grouping there and the grouping here have
        // drifted apart, which is a bug in the dispatch rather than in a
        // command.
        other => unreachable!("{other:?} is not this module's to handle"),
    }
}
