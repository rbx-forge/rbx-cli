use anyhow::Result;
use colored::Colorize;

use crate::config::ConfigsFile;
use crate::ctx::ConfigCtx;
use crate::lock::{LockFile, LOCKFILE_NAME};

use super::make_client;
use rbx_core::api::Repository;
use rbx_core::confirm::confirm_always;

pub async fn run(ctx: &ConfigCtx, yes: bool) -> Result<()> {
    let (env, universe_id, _confirm) = ctx.resolve_target()?;
    let out = &ctx.config;

    // Read before the fetch, because the file names the repository the entries
    // are pulled from. A pull into a directory with no file is still a pull:
    // an absent file names nothing, and takes the flag or the default.
    let file_existed = out.exists();
    let mut file = if file_existed {
        ConfigsFile::load(out)?
    } else {
        ConfigsFile::default()
    };
    let repository = ctx.resolve_repository(file.declared_repository()?)?;
    let client = make_client(ctx, repository)?;

    crate::lock::check_drift_beside(out, &env, universe_id)?;
    let lock_path = out
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(LOCKFILE_NAME);

    println!(
        "Pull live config from [{}] (universe {}) → {}",
        env.bold(),
        universe_id,
        out.display().to_string().bold()
    );

    print!("  Fetching ... ");
    let snapshot = client.get_config(universe_id).await?;
    println!(
        "{} ({} entries, configVersion {})",
        "ok".green(),
        snapshot.entries.len(),
        snapshot.metadata.config_version
    );

    if file_existed {
        let other_envs: Vec<&String> = file
            .environments
            .keys()
            .filter(|e| e.as_str() != env)
            .collect();
        let prompt = if other_envs.is_empty() {
            format!("Update [{}] in {}?", env, out.display())
        } else {
            format!(
                "Update [{}] in {} (leaves {} other env(s) untouched)?",
                env,
                out.display(),
                other_envs.len()
            )
        };
        confirm_always(&prompt, yes)?;
    }

    let stats = file.replace_env_from_json(&env, snapshot.entries);
    // The file has to say which repository it mirrors, or the next `sync`
    // reads a silent file, falls back to the default, and publishes these
    // entries into `InExperienceConfig`. Written only when it is not the
    // default and the file does not already name one, so a plain `pull`
    // produces the bytes it always did.
    if file.repository.is_none() && repository != Repository::default() {
        file.repository = Some(repository.to_string());
    }
    file.save(out)?;

    println!(
        "{} Wrote {} entries to {}",
        "✓".green(),
        stats.total,
        out.display()
    );
    if stats.preserved_descriptions > 0 {
        println!(
            "  preserved {} local description(s)",
            stats.preserved_descriptions
        );
    }
    if !stats.added.is_empty() {
        println!(
            "  {} added: {}",
            stats.added.len(),
            stats.added.join(", ").dimmed()
        );
    }
    if !stats.removed.is_empty() {
        println!(
            "  {} removed: {}",
            stats.removed.len(),
            stats.removed.join(", ").dimmed()
        );
    }

    let env_config = file.get_env(&env).unwrap();
    let mut lock = LockFile::load(&lock_path).unwrap_or_default();
    lock.update_env(
        &env,
        universe_id,
        format!("v{}", snapshot.metadata.config_version),
        env_config
            .entries
            .iter()
            .map(|(k, entry)| (k.clone(), entry.value.clone()))
            .collect(),
    );
    lock.save(&lock_path)?;

    Ok(())
}
