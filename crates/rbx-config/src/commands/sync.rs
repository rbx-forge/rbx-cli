use anyhow::Result;
use colored::Colorize;

use crate::config::ConfigsFile;
use crate::ctx::ConfigCtx;
use crate::diff::Diff;
use crate::lock::{LockFile, LOCKFILE_NAME};
use crate::Strategy;

use super::{make_client, resolve_message};
use rbx_core::api::validate_entries;
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
    let file = &ctx.config;

    let local = ConfigsFile::load(file)?;
    let local_entries = local.entries_as_json(&env)?;
    // The file names the repository these entries are published into, and the
    // flag may only agree with it: pushing a file's entries into a repository
    // it does not describe is the one mistake here that cannot be undone.
    let client = make_client(ctx, ctx.resolve_repository(local.declared_repository()?)?)?;

    // `overwrite_and_publish` validates what it is about to write, so a real
    // publish is already covered. A dry run makes no such call, and a plan
    // reported clean for a publish Roblox would refuse is worse than no plan
    // at all, so the same bounds are checked before anything is printed.
    if dry_run {
        validate_entries(&local_entries)?;
    }

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
    let (result, replaced) = client
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
    // Said rather than refused: a pipeline that publishes on every merge
    // legitimately overwrites whatever the Creator Hub had staged, and
    // stopping it would be the wrong end of the trade. What it must not do is
    // discard somebody's staged edit in silence, so the keys are named.
    if !replaced.is_empty() {
        println!(
            "  replaced a staged draft holding: {}",
            replaced.keys.join(", ").dimmed()
        );
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_core::api::MAX_KEYS_PER_REPOSITORY;

    /// A dry run reports what a publish would do, so it has to refuse what a
    /// publish would be refused for. It used to print a clean plan for an
    /// entry set Roblox rejects, and the 400 arrived at the end of a deploy
    /// instead.
    #[tokio::test]
    async fn a_dry_run_refuses_one_key_over_the_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let over = MAX_KEYS_PER_REPOSITORY + 1;
        let mut file = String::new();
        for index in 0..over {
            file.push_str(&format!(
                "[dev.entries.\"key{index}\"]\nvalue = {index}\n\n"
            ));
        }
        std::fs::write(dir.path().join("rbxconfig.toml"), &file).expect("write");

        let ctx = ConfigCtx {
            config: dir.path().join("rbxconfig.toml"),
            places: dir.path().join("rbxplace.toml"),
            api_key: Some("test-key".into()),
            env: Some("dev".into()),
            universe_id: Some(109876543210987),
            repository: None,
            // No server, and none needed: the refusal comes before the read.
            base_url: None,
        };

        let err = run(&ctx, None, true, &Strategy::Immediate, true, true)
            .await
            .expect_err("101 keys is over Roblox's limit");
        let message = err.to_string();
        assert!(message.contains(&over.to_string()), "{message}");
        assert!(
            message.contains(&MAX_KEYS_PER_REPOSITORY.to_string()),
            "{message}"
        );
    }
}
