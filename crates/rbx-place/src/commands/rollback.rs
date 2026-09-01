use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::Select;

use crate::config::PlacesConfig;
use crate::json::{WriteCommand, WriteDocument};
use rbx_core::confirm::confirm_destructive;
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

use super::{cannot_ask, make_client};

// See upload.rs: one argument per clap arg, deliberately.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    global: &GlobalFlags,
    base_url: Option<&str>,
    env: &str,
    place: Option<&str>,
    version: Option<u64>,
    count: usize,
    yes: bool,
    json: bool,
) -> Result<()> {
    let format = OutputFormat::from_json_flag(json);
    let config = PlacesConfig::load(&global.places)?;
    let env_config = config.get_env(env)?;
    let client = make_client(global, base_url)?;

    let (place_name, place_id) = env_config.resolve_place(place)?;

    if !format.is_json() {
        println!(
            "Rollback {}/{} ({})",
            env.bold(),
            place_name.bold(),
            place_id
        );
        println!();
    }

    let target_version = match version {
        Some(v) => v,
        None => {
            // The selector is the whole command when `--version` is missing.
            // Refused up front rather than after the fetch: a run that cannot
            // choose has no reason to spend a request finding out.
            if !format.may_prompt() {
                return Err(cannot_ask(
                    format,
                    "which version to roll back to",
                    "--version <n>",
                ));
            }

            print!("  Fetching {} recent versions ... ", count);
            let versions = client.list_versions(place_id, count).await?;
            println!("{}", "ok".green());
            println!();

            if versions.is_empty() {
                bail!("No versions found for place {}", place_id);
            }

            let labels: Vec<String> = versions
                .iter()
                .map(|v| {
                    let tag = if v.published { "  [published]" } else { "" };
                    format!("v{}{:<4}  {}", v.version_number, tag, v.display_time())
                })
                .collect();

            let idx = Select::new()
                .with_prompt("Select a version to roll back to")
                .items(&labels)
                .default(0)
                .interact()
                .map_err(|e| anyhow::anyhow!("Prompt error: {}", e))?;

            versions[idx].version_number
        }
    };

    if !format.is_json() {
        println!();
    }
    if env_config.confirm && !yes && !format.may_prompt() {
        return Err(cannot_ask(format, "for confirmation", "--yes"));
    }
    confirm_destructive(
        &format!(
            "Roll back {}/{} to v{}? This will publish a new version live.",
            env, place_name, target_version
        ),
        env_config.confirm,
        yes,
    )?;

    // `published: true`: a rollback republishes the old bytes live; there is
    // no draft form of it.
    let mut receipt = WriteDocument::new(
        WriteCommand::Rollback,
        env,
        env_config.universe_id,
        true,
        true,
    )
    .rolled_back_to(target_version);

    if !format.is_json() {
        print!("  Rolling back to v{} ... ", target_version);
    }
    match client.rollback_place(place_id, target_version).await {
        Ok(new_version) => {
            // `None`, not `Some(true)`: rolling back goes through a different
            // endpoint from an upload, and whether it too declines to make a
            // version when the target is already the current one has not been
            // measured. An unmeasured `true` here would be the same claim this
            // field exists to stop.
            receipt.landed(&place_name, place_id, new_version, None);
            if format.is_json() {
                return output::emit(&receipt);
            }
            println!("{}", format!("done → v{}", new_version).green());
            println!();
            println!("{}", "Rollback complete.".green());
        }
        Err(e) => {
            if format.is_json() {
                output::emit(&receipt.failed(&e))?;
            } else {
                println!("{}", "failed".red());
            }
            return Err(e);
        }
    }

    Ok(())
}
