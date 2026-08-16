//! Compare `rbxshop.toml` against `rbxshop.lock`, offline.
//!
//! Drift leaves through `Err(Drift)` — exit code 2 — rather than through the
//! screen alone: a CI step reads the status, not the log, and a check that
//! printed its findings and exited 0 reported a drifting repository as clean.

use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use rbx_core::generated::Drift;

use crate::config::Config;
use crate::ctx::ShopCtx;
use crate::diff::{build_sync_plan, Action};
use crate::lockfile::{EnvLock, Lockfile};

pub async fn run(ctx: &ShopCtx<'_>) -> Result<()> {
    let config = Config::load_merged(&ctx.config)?;
    println!("{} Config is valid ({})", "✓".green(), ctx.config.display());

    let config_dir = ctx.config.parent().unwrap_or(Path::new("."));
    let lockfile_path = config_dir.join(crate::lockfile::LOCKFILE_NAME);

    let lockfile_exists = lockfile_path.exists();
    let lockfile = if lockfile_exists {
        let lf = Lockfile::load(&lockfile_path)?;
        println!(
            "{} Lockfile is valid ({})",
            "✓".green(),
            lockfile_path.display()
        );
        lf
    } else {
        println!(
            "{} No lockfile found. Run `rbx shop sync` to create one.",
            "!".yellow()
        );
        Lockfile::default()
    };

    let envs = ctx.resolve_envs(&config)?;
    let mut drifted: Vec<String> = Vec::new();

    for env_target in &envs {
        println!("\n{} {}", "env:".bold(), env_target.name.bold());
        let resources = config.resolve_env(Some(&env_target.name))?;
        Config::validate_icon_paths(&resources, config_dir)?;

        let default_lock = EnvLock {
            universe_id: env_target.universe_id,
            ..Default::default()
        };
        let env_lock = lockfile.env(&env_target.name).unwrap_or(&default_lock);

        if lockfile_exists
            && env_lock.universe_id != 0
            && env_lock.universe_id != env_target.universe_id
        {
            println!(
                "{} Universe ID mismatch: target={}, lockfile={}",
                "✗".red(),
                env_target.universe_id,
                env_lock.universe_id
            );
        }

        let plan = build_sync_plan(&resources, env_lock, config_dir)?;

        for warning in &plan.warnings {
            println!("{} {}", "!".yellow(), warning);
        }

        let mut creates = 0;
        let mut updates = 0;
        for action in plan.passes.iter().chain(&plan.badges).chain(&plan.products) {
            match &action.action {
                Action::Create => creates += 1,
                Action::Update { .. } => updates += 1,
                Action::Skip => {}
            }
        }

        if creates == 0 && updates == 0 {
            println!("{} Everything is in sync.", "✓".green());
        } else {
            println!(
                "{} Out of sync: {} to create, {} to update. Run `rbx shop sync` for details.",
                "!".yellow(),
                creates,
                updates
            );
            drifted.push(format!(
                "{} ({creates} to create, {updates} to update)",
                env_target.name
            ));
        }
    }

    if drifted.is_empty() {
        return Ok(());
    }

    Err(Drift::new(format!(
        "{} env{} out of sync with {}: {}. Run `rbx shop sync` to apply.",
        drifted.len(),
        if drifted.len() == 1 { " is" } else { "s are" },
        lockfile_path.display(),
        drifted.join(", "),
    ))
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_core::GlobalFlags;
    use tempfile::TempDir;

    const IN_SYNC: &str = r#"
[experience]
universe_id = 100

[passes.starter]
name = "Starter Pack"
price = 99
"#;

    /// A repo whose one declared pass is already recorded in the lockfile.
    fn synced_repo() -> TempDir {
        use crate::lockfile::{PassLock, LOCKFILE_NAME, LOCKFILE_VERSION};

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("rbxshop.toml"), IN_SYNC).expect("write");

        let mut lockfile = Lockfile {
            version: LOCKFILE_VERSION,
            ..Default::default()
        };
        lockfile
            .env_mut(crate::lockfile::DEFAULT_ENV, 100)
            .passes
            .insert(
                "starter".to_string(),
                PassLock {
                    id: 555,
                    name: "Starter Pack".to_string(),
                    price: Some(99),
                    description: None,
                    icon_asset_id: None,
                    icon_hash: None,
                    for_sale: true,
                    regional_pricing: false,
                },
            );
        lockfile
            .save(&dir.path().join(LOCKFILE_NAME))
            .expect("save the lockfile");
        dir
    }

    async fn check(dir: &TempDir) -> Result<()> {
        let global = GlobalFlags {
            api_key: None,
            cookie: None,
            no_auto_cookie: true,
            auto_cookie: false,
            env: None,
            place: None,
            places: dir.path().join("rbxplace.toml"),
            universe_id: None,
            place_id: Vec::new(),
        };
        let ctx = ShopCtx {
            config: dir.path().join("rbxshop.toml"),
            global: &global,
            base_url: None,
        };
        run(&ctx).await
    }

    #[tokio::test]
    async fn a_repo_that_matches_its_lockfile_is_clean() {
        let dir = synced_repo();
        check(&dir).await.expect("nothing to sync");
    }

    /// The bug this command had: it printed the drift and returned `Ok(())`,
    /// so a CI step reading the exit code passed on a drifting repo.
    #[tokio::test]
    async fn a_declared_resource_missing_from_the_lockfile_exits_2() {
        let dir = synced_repo();
        std::fs::write(
            dir.path().join("rbxshop.toml"),
            format!("{IN_SYNC}\n[badges.first_win]\nname = \"First Win\"\n"),
        )
        .expect("write");

        let err = check(&dir).await.expect_err("a new badge is drift");
        assert!(
            err.chain().any(|cause| cause.is::<Drift>()),
            "drift must exit 2, not 1: {err:#}"
        );
        assert!(format!("{err:#}").contains("rbx shop sync"), "{err:#}");
    }

    /// A repo that never synced is the same drift, reached the other way.
    #[tokio::test]
    async fn a_repo_with_no_lockfile_at_all_exits_2() {
        let dir = synced_repo();
        std::fs::remove_file(dir.path().join(crate::lockfile::LOCKFILE_NAME)).expect("remove");

        let err = check(&dir).await.expect_err("nothing is synced yet");
        assert!(
            err.chain().any(|cause| cause.is::<Drift>()),
            "drift must exit 2, not 1: {err:#}"
        );
    }
}
