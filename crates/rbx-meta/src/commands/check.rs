//! Compare `rbxmeta.toml` against `rbxmeta.lock`, offline.
//!
//! Drift leaves through `Err(Drift)` — exit code 2 — rather than through the
//! screen alone: a CI step reads the status, not the log, and a check that
//! printed its findings and exited 0 reported a drifting repository as clean.

use anyhow::Result;
use colored::Colorize;

use rbx_core::generated::Drift;

use crate::config::Config;
use crate::ctx::MetaCtx;
use crate::diff::{build_plan, IconPlan};
use crate::lockfile::{Lockfile, LOCKFILE_NAME};

pub async fn run(ctx: &MetaCtx<'_>) -> Result<()> {
    let config = Config::load(&ctx.config)?;
    let config_dir = ctx
        .config
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    let lockfile_path = config_dir.join(LOCKFILE_NAME);
    let lockfile = Lockfile::load(&lockfile_path)?;

    let (env, universe_id, place_id) = ctx.resolve_target(&config)?;
    let (game, media) = config.resolve_env(Some(&env));
    Config::validate_invariants(&game)?;
    Config::validate_media_paths(&media, &config_dir)?;

    let env_lock = lockfile.env_view(&env);
    let plan = build_plan(&game, &media, &env_lock.game, &env_lock.media, &config_dir)?;

    println!("Config:   {}", ctx.config.display().to_string().cyan());
    println!("Lockfile: {}", lockfile_path.display().to_string().cyan());
    println!("Env:      {}", env.cyan());
    println!("Universe: {}", universe_id);
    println!("Place:    {}", place_id);

    if plan.is_empty() {
        println!("\n{}", "✓ Everything is in sync.".green());
        return Ok(());
    }

    println!("\n{}", "Pending changes:".bold());

    if let Some(patch) = &plan.universe_patch {
        println!("\n  {} universe:", "▸".cyan());
        for d in &patch.descriptions {
            println!("    • {}", d);
        }
    }

    if let Some(patch) = &plan.place_patch {
        println!("\n  {} place:", "▸".cyan());
        for d in &patch.descriptions {
            println!("    • {}", d);
        }
    }

    if let Some(patch) = &plan.place_legacy_patch {
        println!("\n  {} place (legacy, cookie):", "▸".cyan());
        for d in &patch.descriptions {
            println!("    • {}", d);
        }
    }

    if let Some(patch) = &plan.universe_legacy_patch {
        println!("\n  {} universe config (legacy, cookie):", "▸".cyan());
        for d in &patch.descriptions {
            println!("    • {}", d);
        }
    }

    if let Some(v) = plan.visibility_change {
        println!("\n  {} visibility (cookie): → {:?}", "▸".cyan(), v);
    }

    if let Some(b) = plan.beta_mode_change {
        println!("\n  {} beta_mode (cookie): → {}", "▸".cyan(), b);
    }

    if let IconPlan::Upload { path, .. } = &plan.icon {
        println!(
            "\n  {} icon: upload {}",
            "▸".cyan(),
            path.display().to_string().yellow()
        );
    }

    if !plan.thumbnails.is_empty() {
        println!("\n  {} thumbnails:", "▸".cyan());
        for id in &plan.thumbnails.deletes {
            println!("    • delete image {}", id);
        }
        for upload in &plan.thumbnails.uploads {
            println!(
                "    • upload {}",
                upload.path.display().to_string().yellow()
            );
        }
    }

    Err(Drift::new(format!(
        "{} no longer matches {} for env {}. Run `rbx meta sync` to apply the changes listed above.",
        ctx.config.display(),
        lockfile_path.display(),
        env,
    ))
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{EnvLock, GameLock, LOCKFILE_VERSION};
    use rbx_core::env::DEFAULT_ENV;
    use rbx_core::GlobalFlags;
    use tempfile::TempDir;

    const DECLARED: &str = r#"
[experience]
universe_id = 100
place_id = 200

[game]
name = "Declared"
"#;

    /// A repo whose declared metadata is already recorded in the lockfile.
    fn synced_repo() -> TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("rbxmeta.toml"), DECLARED).expect("write");

        let mut lockfile = Lockfile {
            version: LOCKFILE_VERSION,
            ..Default::default()
        };
        lockfile.envs.insert(
            DEFAULT_ENV.to_string(),
            EnvLock {
                universe_id: 100,
                place_id: 200,
                game: GameLock {
                    name: Some("Declared".to_string()),
                    ..Default::default()
                },
                media: Default::default(),
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
        let ctx = MetaCtx {
            config: dir.path().join("rbxmeta.toml"),
            global: &global,
        };
        run(&ctx).await
    }

    #[tokio::test]
    async fn a_repo_that_matches_its_lockfile_is_clean() {
        let dir = synced_repo();
        check(&dir).await.expect("nothing to sync");
    }

    /// The bug this command had: it printed the pending changes and returned
    /// `Ok(())`, so a CI step reading the exit code passed on a drifting repo.
    #[tokio::test]
    async fn a_changed_field_exits_2() {
        let dir = synced_repo();
        std::fs::write(
            dir.path().join("rbxmeta.toml"),
            DECLARED.replace("Declared", "Renamed"),
        )
        .expect("write");

        let err = check(&dir).await.expect_err("the name changed");
        assert!(
            err.chain().any(|cause| cause.is::<Drift>()),
            "drift must exit 2, not 1: {err:#}"
        );
        assert!(format!("{err:#}").contains("rbx meta sync"), "{err:#}");
    }

    /// Declared but never synced is the same drift, reached the other way.
    #[tokio::test]
    async fn a_repo_with_no_lockfile_at_all_exits_2() {
        let dir = synced_repo();
        std::fs::remove_file(dir.path().join(LOCKFILE_NAME)).expect("remove");

        let err = check(&dir).await.expect_err("nothing is synced yet");
        assert!(
            err.chain().any(|cause| cause.is::<Drift>()),
            "drift must exit 2, not 1: {err:#}"
        );
    }
}
