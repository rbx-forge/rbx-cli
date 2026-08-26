//! Compare `rbxmeta.toml` against `rbxmeta.lock`, offline.
//!
//! Drift leaves through `Err(Drift)` (exit code 2) rather than through the
//! screen alone: a CI step reads the status, not the log, and a check that
//! printed its findings and exited 0 reported a drifting repository as clean.
//!
//! `--env all` and `--env <group>` check every env they name. The status is
//! then the aggregate: any env drifting fails the command, because a CI step
//! asking "is this repository in sync" is asking about all of it.

use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use rbx_core::generated::Drift;

use crate::config::Config;
use crate::ctx::MetaCtx;
use crate::diff::{build_plan, IconPlan};
use crate::lockfile::{Lockfile, LOCKFILE_NAME};

pub async fn run(ctx: &MetaCtx<'_>) -> Result<()> {
    let config = Config::load(&ctx.config)?;
    let config_dir = ctx.config.parent().unwrap_or(Path::new(".")).to_path_buf();
    let lockfile_path = config_dir.join(LOCKFILE_NAME);
    let lockfile = Lockfile::load(&lockfile_path)?;

    let targets = ctx.resolve_targets(&config)?;
    // The header appears only when there is more than one env to separate, so
    // a single-env run prints exactly what it always has.
    let many = targets.len() > 1;

    let mut drifted: Vec<&str> = Vec::new();
    for target in &targets {
        if many {
            println!("\n{} {}", "env:".bold(), target.0.bold());
        }
        if report_env(ctx, &config, &config_dir, &lockfile, &lockfile_path, target)? {
            drifted.push(&target.0);
        }
    }

    if drifted.is_empty() {
        return Ok(());
    }

    Err(Drift::new(format!(
        "{} no longer matches {} for env {}. Run `rbx meta sync` to apply the changes listed above.",
        ctx.config.display(),
        lockfile_path.display(),
        drifted.join(", "),
    ))
    .into())
}

/// Print one env's status, returning whether it has drifted.
///
/// Reports rather than raising `Drift` itself: a run over several envs has to
/// visit all of them before it can say whether the repository is in sync, and
/// an early `Err` would name the first drifting env and hide the rest.
///
/// **Drift is what aggregates; a config that does not validate is not drift.**
/// The two `validate_*` calls below still abort the walk, on purpose: a repo
/// whose config is invalid for one env cannot be judged in sync for any of
/// them, that has to be fixed before the answer means anything, and it exits 1
/// rather than 2 so a CI step can tell "misconfigured" from "drifted". The
/// aggregation exists so a second *drifting* env is not hidden by the first,
/// which is the case a reader of the log actually acts on.
fn report_env(
    ctx: &MetaCtx<'_>,
    config: &Config,
    config_dir: &Path,
    lockfile: &Lockfile,
    lockfile_path: &Path,
    target: &(String, u64, u64),
) -> Result<bool> {
    let (env, universe_id, place_id) = target;
    let (game, media) = config.resolve_env(Some(env));
    Config::validate_invariants(&game)?;
    Config::validate_media_paths(&media, config_dir)?;

    let env_lock = lockfile.env_view(env);
    let plan = build_plan(&game, &media, &env_lock.game, &env_lock.media, config_dir)?;

    println!("Config:   {}", ctx.config.display().to_string().cyan());
    println!("Lockfile: {}", lockfile_path.display().to_string().cyan());
    println!("Env:      {}", env.cyan());
    println!("Universe: {}", universe_id);
    println!("Place:    {}", place_id);

    if plan.is_empty() {
        println!("\n{}", "✓ Everything is in sync.".green());
        return Ok(false);
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

    Ok(true)
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

    // -----------------------------------------------------------------------
    // Several envs at once
    // -----------------------------------------------------------------------

    const PLACES: &str = r#"
[groups]
nonprod = ["dev", "staging"]

[dev]
universe_id = 100
[dev.places]
main = 200

[staging]
universe_id = 150
[staging.places]
main = 250
"#;

    /// Two envs sharing one base `[game]`, both recorded in the lockfile.
    /// `overlay` is written under `[envs.staging]`, so a test can drift one env
    /// and leave the other in sync.
    fn two_env_repo(overlay: &str) -> TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("rbxplace.toml"), PLACES).expect("write places");
        std::fs::write(
            dir.path().join("rbxmeta.toml"),
            format!("[game]\nname = \"Declared\"\n\n{overlay}"),
        )
        .expect("write config");

        let mut lockfile = Lockfile {
            version: LOCKFILE_VERSION,
            ..Default::default()
        };
        for (env, universe_id, place_id) in [("dev", 100, 200), ("staging", 150, 250)] {
            lockfile.envs.insert(
                env.to_string(),
                EnvLock {
                    universe_id,
                    place_id,
                    game: GameLock {
                        name: Some("Declared".to_string()),
                        ..Default::default()
                    },
                    media: Default::default(),
                },
            );
        }
        lockfile
            .save(&dir.path().join(LOCKFILE_NAME))
            .expect("save the lockfile");
        dir
    }

    async fn check_env(dir: &TempDir, env: &str) -> Result<()> {
        let global = GlobalFlags {
            api_key: None,
            cookie: None,
            no_auto_cookie: true,
            auto_cookie: false,
            env: Some(env.to_string()),
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
    async fn every_env_is_clean_or_the_run_is_not() {
        let dir = two_env_repo("");
        check_env(&dir, "all").await.expect("both envs are synced");
        check_env(&dir, "nonprod")
            .await
            .expect("the group is the same two envs");
    }

    /// The aggregate is what a CI step reads, so drift anywhere fails the run.
    /// Checking only the last env is the failure this pins: `dev` is clean and
    /// sorts first, so a loop that kept only the final verdict would pass here
    /// while `staging` drifts, and a loop that returned on the first drift
    /// would never reach `staging` at all.
    #[tokio::test]
    async fn one_drifting_env_fails_the_whole_run() {
        let dir = two_env_repo("[envs.staging]\nname = \"Renamed\"\n");

        let err = check_env(&dir, "all")
            .await
            .expect_err("staging no longer matches");
        assert!(
            err.chain().any(|cause| cause.is::<Drift>()),
            "drift must exit 2, not 1: {err:#}"
        );
        let text = format!("{err:#}");
        assert!(text.contains("for env staging"), "names the env: {text}");
        assert!(
            !text.contains("dev"),
            "and only the env that drifted: {text}"
        );
    }

    /// A single-env run says exactly what it always said. The multi-env wording
    /// is the same sentence with the list in the same slot, so this is what
    /// stops the fan-out from rewording the one-env case.
    #[tokio::test]
    async fn a_single_env_run_reports_only_its_own_env() {
        let dir = two_env_repo("[envs.staging]\nname = \"Renamed\"\n");

        check_env(&dir, "dev").await.expect("dev is in sync");

        let err = check_env(&dir, "staging")
            .await
            .expect_err("staging is not");
        assert_eq!(
            format!("{err}"),
            format!(
                "{} no longer matches {} for env staging. \
                 Run `rbx meta sync` to apply the changes listed above.",
                dir.path().join("rbxmeta.toml").display(),
                dir.path().join(LOCKFILE_NAME).display(),
            )
        );
    }
}
