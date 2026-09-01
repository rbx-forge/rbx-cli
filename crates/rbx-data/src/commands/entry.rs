//! The subcommands that read or write one entry: `get`, `set`, `reset`,
//! `delete-key`, `restore-key`, `copy`, `increment`.
//!
//! Four of them share one write path, [`write_entry`], and differ only in where
//! the new value comes from and what the receipt calls the action. That shared
//! path is why they are one module: a change to how a write backs up, confirms
//! or reports lands in one place and covers all four.
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colored::Colorize;

use rbx_core::confirm::confirm_always;
use rbx_core::output::{self, OutputFormat};

use crate::backup::{self, BackupTarget};
use crate::DEFAULT_TEMPLATE;

use crate::cli::Command;
use crate::commands::Ctx;
use crate::json::*;
use crate::support::*;
pub(crate) async fn run(ctx: &Ctx<'_>, command: Command) -> Result<()> {
    let api = &ctx.api;
    let store = &ctx.store;
    let universe_id = ctx.universe_id;
    let global = ctx.global;
    let build = |id: u64| ctx.build(id);
    let _ = (store, universe_id, global, &build);

    match command {
        Command::Get { entry, out, json } => {
            let format = OutputFormat::from_json_flag(json);
            let Some(found) = api.get(&entry).await? else {
                // A key that was never written is not an error in either
                // format. Under `--json` it is a document saying `found:
                // false` rather than silence, so a script can tell "no such
                // key" from "the command printed nothing".
                format.note(format!("No entry `{entry}`.").dimmed());
                if format.is_json() {
                    output::emit(&GetDocument::missing(store, &entry))?;
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
                        output::emit(&GetDocument::found(store, &entry, &found, Some(&path)))?;
                    } else {
                        println!("wrote {}", path.display());
                    }
                }
                None => {
                    if format.is_json() {
                        output::emit(&GetDocument::found(store, &entry, &found, None))?;
                    } else {
                        println!("{pretty}");
                    }
                }
            }
            Ok(())
        }

        Command::RestoreKey {
            entry,
            revision,
            backup,
            keep,
            no_backup,
            apply,
            yes,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);
            let found = api.get_revision(&entry, &revision).await?;
            let raw = serde_json::to_string(&found.value.unwrap_or(serde_json::Value::Null))?;
            format.note(format!("restoring `{entry}` from revision {revision}").dimmed());
            write_entry(
                api,
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
                    action: "restore",
                    format,
                },
                universe_id,
                store,
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
                    action: "copy",
                    format: OutputFormat::Human,
                },
                target_universe,
                // The same store and scope on the other side: `copy` moves an
                // entry between universes, never between stores. It was built
                // inline here from the two flags, which is what `ctx.store`
                // already is.
                store,
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

        Command::Reset {
            entry,
            template,
            backup,
            keep,
            no_backup,
            apply,
            yes,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);
            let path = template.unwrap_or_else(|| PathBuf::from(DEFAULT_TEMPLATE));
            if !path.exists() {
                bail!(
                    "no template at {}. Point --template at the default profile your game                      writes for a new player, or put one at {DEFAULT_TEMPLATE}.",
                    path.display()
                );
            }
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            format.note(format!("resetting from {}", path.display()).dimmed());
            write_entry(
                api,
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
                    action: "reset",
                    format,
                },
                universe_id,
                store,
            )
            .await
        }

        Command::DeleteKey {
            entry,
            backup,
            keep,
            no_backup,
            apply,
            yes,
            json,
        } => {
            let format = OutputFormat::from_json_flag(json);
            let mut document = WriteDocument::new(store, &entry, "delete");
            let target = backup_target(
                BackupFlags {
                    backup,
                    no_backup,
                    keep,
                },
                global,
                global.env.as_deref(),
                universe_id,
            );
            let existing = api.get(&entry).await?;

            document.existed = existing.is_some();
            format.note(format!("entry {entry}").bold());

            let found = match &existing {
                Some(found) => found,
                None => {
                    format.note("  no such entry, nothing to remove".dimmed());

                    if format.is_json() {
                        return output::emit(&document);
                    }

                    return Ok(());
                }
            };

            let current = found.value.clone().unwrap_or(serde_json::Value::Null);
            format.note("  current".dimmed());
            format.note(indent(&serde_json::to_string_pretty(&current)?));
            format.note("");

            if !apply {
                format.note("Nothing removed. Re-run with --apply to delete.".yellow());

                if format.is_json() {
                    return output::emit(&document);
                }

                return Ok(());
            }

            match &target {
                BackupTarget::Path(_) | BackupTarget::Managed { .. } => {
                    let written =
                        backup::write(&target, &entry, &serde_json::to_string_pretty(&current)?)?;
                    document.backup = Some(written.path.display().to_string());
                    format.note(format!("backup written to {}", written.path.display()));
                    if written.pruned > 0 {
                        format.note(
                            format!(
                                "  {} older backup(s) of {entry} removed by --keep",
                                written.pruned
                            )
                            .dimmed(),
                        );
                    }
                }
                // Unlike an overwrite, a delete leaves the value readable for
                // thirty days, so skipping the copy is not the same cliff. It
                // is still a deadline rather than a keepsake.
                BackupTarget::Skip => {
                    format.note(
                        "--no-backup: no local copy. The value stays readable through \
                         `data revisions` for thirty days, and then it does not."
                            .yellow(),
                    );
                }
            }

            confirm_always(
                &format!("Remove `{entry}` from universe {universe_id}?"),
                yes,
            )?;

            api.delete(&entry).await?;
            document.applied = true;

            format.note(format!(
                "{} {entry} is gone. A read answers nothing; `data revisions {entry}` still has it \
                 for thirty days.",
                "done".green().bold()
            ));

            if format.is_json() {
                return output::emit(&document);
            }

            Ok(())
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
            json,
        } => {
            let raw = match (&value, &file) {
                (Some(inline), _) => inline.clone(),
                (None, Some(path)) => std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?,
                (None, None) => bail!("pass --value '<json>' or --file <path>"),
            };
            write_entry(
                api,
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
                    action: "set",
                    format: OutputFormat::from_json_flag(json),
                },
                universe_id,
                store,
            )
            .await
        }
        // Every other variant is routed elsewhere by `run` in lib.rs. Reaching
        // this arm would mean the grouping there and the grouping here have
        // drifted apart, which is a bug in the dispatch rather than in a
        // command.
        other => unreachable!("{other:?} is not this module's to handle"),
    }
}
