//! Compare the local `rbxconfig.toml` entries against the live config.
//!
//! Drift leaves through `Err(Drift)` (exit code 2) rather than through the
//! screen alone: a CI step reads the status, not the log, and a check that
//! printed its findings and exited 0 reported a drifting repository as clean.

use anyhow::Result;
use colored::Colorize;

use rbx_core::generated::Drift;

use crate::config::ConfigsFile;
use crate::ctx::ConfigCtx;
use crate::diff::Diff;

use super::make_client;

pub async fn run(ctx: &ConfigCtx) -> Result<()> {
    let (env, universe_id, _confirm) = ctx.resolve_target()?;
    let client = make_client(ctx)?;
    let file = &ctx.config;

    let local = ConfigsFile::load(file)?;
    let local_entries = local.entries_as_json(&env)?;

    println!(
        "Check {} ↔ [{}] (universe {})",
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

    let diff = Diff::compute(&local_entries, &snapshot.entries);

    if diff.is_empty() {
        println!("{}", "✓ Local matches live.".green());
        return Ok(());
    }

    println!("{}", "Pending changes (would be applied by sync):".bold());
    diff.print();
    println!();

    Err(Drift::new(format!(
        "{} entr{} in {} differ{} from the live config of universe {universe_id} [{env}]. \
         Run `rbx config sync --env {env}` to publish.",
        diff.changes.len(),
        if diff.changes.len() == 1 { "y" } else { "ies" },
        file.display(),
        if diff.changes.len() == 1 { "s" } else { "" },
    ))
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNIVERSE: u64 = 100;
    const REPO_PATH: &str =
        "/creator-configs-public-api/v1/configs/universes/100/repositories/InExperienceConfig";

    const LOCAL: &str = r#"
[dev.entries."features.new_popup"]
value = true
"#;

    fn repo() -> TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("rbxconfig.toml"), LOCAL).expect("write");
        dir
    }

    /// Answer the one read `check` makes with the given live entries.
    async fn live(entries: serde_json::Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(REPO_PATH))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({ "metadata": { "configVersion": 3 }, "entries": entries }),
                ),
            )
            .mount(&server)
            .await;
        server
    }

    async fn check(dir: &TempDir, server: &MockServer) -> Result<()> {
        let ctx = ConfigCtx {
            config: dir.path().join("rbxconfig.toml"),
            places: dir.path().join("rbxplace.toml"),
            api_key: Some("test-key".into()),
            env: Some("dev".into()),
            universe_id: Some(UNIVERSE),
            base_url: Some(server.uri()),
        };
        run(&ctx).await
    }

    #[tokio::test]
    async fn a_local_file_matching_the_live_config_is_clean() {
        let dir = repo();
        let server = live(json!({ "features.new_popup": true })).await;

        check(&dir, &server).await.expect("nothing to publish");
    }

    /// The bug this command had: it printed the pending changes and returned
    /// `Ok(())`, so a CI step reading the exit code passed on a drifting repo.
    #[tokio::test]
    async fn an_entry_that_differs_from_live_exits_2() {
        let dir = repo();
        let server = live(json!({ "features.new_popup": false })).await;

        let err = check(&dir, &server)
            .await
            .expect_err("the local value differs from the live one");
        assert!(
            err.chain().any(|cause| cause.is::<Drift>()),
            "drift must exit 2, not 1: {err:#}"
        );
        assert!(
            format!("{err:#}").contains("rbx config sync --env dev"),
            "{err:#}"
        );
    }

    /// A universe with nothing published yet reads as an empty config, and a
    /// local file against an empty live config is drift like any other.
    #[tokio::test]
    async fn an_entry_missing_from_live_exits_2() {
        let dir = repo();
        let server = live(json!({})).await;

        let err = check(&dir, &server).await.expect_err("nothing published");
        assert!(
            err.chain().any(|cause| cause.is::<Drift>()),
            "drift must exit 2, not 1: {err:#}"
        );
    }
}
