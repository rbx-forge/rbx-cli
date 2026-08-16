use anyhow::Result;
use colored::Colorize;

use crate::config::ConfigsFile;
use crate::ctx::ConfigCtx;
use crate::diff::Diff;
use crate::lock::{LockFile, LOCKFILE_NAME};
use crate::Strategy;

use super::{make_client, resolve_message};
use rbx_core::confirm::confirm_destructive;

pub async fn run(
    ctx: &ConfigCtx,
    message: Option<&str>,
    no_message: bool,
    strategy: &Strategy,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    let (env, universe_id, confirm_required) = ctx.resolve_target()?;
    let client = make_client(ctx)?;
    let file = &ctx.config;

    let local = ConfigsFile::load(file)?;
    let local_entries = local.entries_as_json(&env)?;

    crate::lock::check_drift_beside(file, &env, universe_id)?;
    let lock_path = file
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(LOCKFILE_NAME);

    println!(
        "Sync {} → [{}] (universe {})",
        file.display().to_string().bold(),
        env.bold(),
        universe_id
    );
    println!("  local entries: {}", local_entries.len());

    print!("  Fetching live config ... ");
    let snapshot = client.get_config(universe_id).await?;
    println!(
        "{} (configVersion {})",
        "ok".green(),
        snapshot.metadata.config_version
    );
    println!();

    println!("Changes:");
    let diff = Diff::compute(&local_entries, &snapshot.entries);
    diff.print();

    if dry_run {
        println!();
        println!("{}", "--dry-run: no HTTP write performed".dimmed());
        return Ok(());
    }

    if diff.is_empty() {
        println!();
        println!("Nothing to push.");
        return Ok(());
    }

    println!();
    confirm_destructive(
        &format!("Publish these changes to [{}]?", env),
        confirm_required,
        yes,
    )?;

    let effective_message = resolve_message(message, no_message, yes)?;
    if effective_message.is_empty() {
        println!("  message: (none)");
    } else {
        println!("  message: {}", effective_message.dimmed());
    }
    println!();

    print!("Publishing ... ");
    let result = client
        .overwrite_and_publish(
            universe_id,
            &local_entries,
            &effective_message,
            strategy.as_api_str(),
        )
        .await?;

    println!("{} configVersion {}", "ok →".green(), result.config_version);
    println!(
        "  strategy: {} (~{} min propagation)",
        strategy.as_api_str(),
        strategy.eta_minutes()
    );

    let mut lock = LockFile::load(&lock_path).unwrap_or_default();
    let env_config = local.get_env(&env).unwrap();
    lock.update_env(
        &env,
        universe_id,
        format!("v{}", result.config_version),
        env_config
            .entries
            .iter()
            .map(|(k, entry)| (k.clone(), entry.value.clone()))
            .collect(),
    );
    lock.save(&lock_path)?;

    println!();
    println!("{}", "Sync complete.".green());
    Ok(())
}
