//! The subcommands that answer a question without changing anything: `list`,
//! `revisions`, `diff`.
//!
//! Read-only by construction, which is what separates them from the entry
//! commands next door. None takes `--apply`, none asks for a confirmation, and
//! none writes a backup, because there is nothing to undo.

use anyhow::{bail, Context, Result};
use colored::Colorize;

use rbx_core::output::{self, OutputFormat};

use crate::backup::sanitise_filename;

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
                    store,
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
                        store,
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
                    store,
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
                    store,
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
        // Every other variant is routed elsewhere by `run` in lib.rs. Reaching
        // this arm would mean the grouping there and the grouping here have
        // drifted apart, which is a bug in the dispatch rather than in a
        // command.
        other => unreachable!("{other:?} is not this module's to handle"),
    }
}
