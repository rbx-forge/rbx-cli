use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::Select;

use crate::config::ConfigsFile;
use crate::ctx::ConfigCtx;
use crate::lock;

use super::{make_client, resolve_message};
use rbx_core::confirm::confirm_destructive;

pub async fn run(
    ctx: &ConfigCtx,
    revision_id: Option<String>,
    count: usize,
    message: Option<&str>,
    no_message: bool,
    yes: bool,
) -> Result<()> {
    let (env, universe_id, confirm) = ctx.resolve_target()?;
    // A rollback publishes, so the file gets to name the repository it
    // publishes into: restoring a revision of the wrong one replaces a live
    // config nobody touched. It reads no entries though, so a directory with
    // no `rbxconfig.toml` rolls back on the flag or the default, as it always
    // did.
    let declared = if ctx.config.exists() {
        ConfigsFile::load(&ctx.config)?.declared_repository()?
    } else {
        None
    };
    let client = make_client(ctx, ctx.resolve_repository(declared)?)?;

    lock::check_drift_beside(&ctx.config, &env, universe_id)?;

    println!("Rollback {} ({})", env.bold(), universe_id);
    println!();

    let target_revision = match revision_id {
        Some(rev_id) => rev_id,
        None => {
            // Before the listing call rather than after it: a run that cannot
            // reach the picker should not spend a request on the revisions it
            // is about to have nowhere to show.
            if !rbx_core::output::is_interactive() {
                bail!(
                    "no revision id given, and there is nobody to show the picker to. \
                     Pass one as an argument: `rbx config versions` lists them."
                );
            }
            print!("  Fetching {} recent revisions ... ", count);
            let revisions = client.list_revisions(universe_id, count).await?;
            println!("{}", "ok".green());
            println!();

            if revisions.is_empty() {
                bail!("No revisions found for universe {}", universe_id);
            }

            let labels: Vec<String> = revisions
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let tag = if i == 0 { "  [published]" } else { "" };
                    format!(
                        "{:<8}  v{:<4}  {}  {}{}",
                        &r.revision_id[..std::cmp::min(8, r.revision_id.len())],
                        r.version,
                        r.time.replace('T', " ").trim_end_matches('Z'),
                        r.message.as_deref().unwrap_or("(no message)"),
                        tag
                    )
                })
                .collect();

            let idx = Select::new()
                .with_prompt("Select a revision to roll back to")
                .items(&labels)
                .default(0)
                .interact()
                .map_err(|e| anyhow::anyhow!("Prompt error: {}", e))?;

            revisions[idx].revision_id.clone()
        }
    };

    println!();
    confirm_destructive(
        &format!(
            "Roll back {} to revision {}? This will publish a new version.",
            env,
            &target_revision[..std::cmp::min(8, target_revision.len())]
        ),
        confirm,
        yes,
    )?;

    // Resolved before the restore, not after. `restore_revision` stages a
    // draft, and a draft is not nothing: it replaces whatever was staged
    // there. Failing on "no publish message" once that has happened would
    // leave the universe holding a draft nobody asked to stage, from a command
    // that then reported failure. Everything that can refuse this run refuses
    // it while the universe is untouched.
    let message = resolve_message(message, no_message, yes)?;

    print!("  Restoring revision {} ... ", target_revision);
    let draft_hash = client
        .restore_revision(universe_id, &target_revision)
        .await?;
    println!("{}", "ok".green());

    print!("  Publishing ... ");
    match client
        .publish(universe_id, &message, "Immediate", Some(&draft_hash))
        .await
    {
        Ok(result) => {
            println!("{}", format!("done → v{}", result.config_version).green());
            println!();
            println!("{}", "Rollback complete.".green());
        }
        Err(e) => {
            println!("{}", "failed".red());
            return Err(e);
        }
    }

    Ok(())
}
