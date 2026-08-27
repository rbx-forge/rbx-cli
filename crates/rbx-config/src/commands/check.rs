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
    let file = &ctx.config;

    // The file is read before the client is built: it is what names the
    // repository the live config is read from.
    let local = ConfigsFile::load(file)?;
    let local_entries = local.entries_as_json(&env)?;
    let client = make_client(ctx, ctx.resolve_repository(local.declared_repository()?)?)?;

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
    use rbx_core::api::Repository;
    use serde_json::json;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNIVERSE: u64 = 100;
    const REPO_PATH: &str =
        "/creator-configs-public-api/v1/configs/universes/100/repositories/InExperienceConfig";
    const DATA_STORES_PATH: &str =
        "/creator-configs-public-api/v1/configs/universes/100/repositories/DataStoresConfig";

    const LOCAL: &str = r#"
[dev.entries."features.new_popup"]
value = true
"#;

    fn repo() -> TempDir {
        write_repo(LOCAL)
    }

    fn write_repo(content: &str) -> TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("rbxconfig.toml"), content).expect("write");
        dir
    }

    /// Answer the one read `check` makes with the given live entries.
    async fn live(entries: serde_json::Value) -> MockServer {
        live_at(REPO_PATH, entries).await
    }

    /// Same, on one named path and nothing else: a request aimed at another
    /// repository gets wiremock's 404, which `get_config` reads as an empty
    /// config, which is drift. That is what makes the path segment the thing
    /// under test rather than a detail of the mock.
    async fn live_at(repo_path: &str, entries: serde_json::Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(repo_path.to_string()))
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
        check_with(dir, server, None).await
    }

    async fn check_with(
        dir: &TempDir,
        server: &MockServer,
        repository: Option<Repository>,
    ) -> Result<()> {
        let ctx = ConfigCtx {
            config: dir.path().join("rbxconfig.toml"),
            places: dir.path().join("rbxplace.toml"),
            api_key: Some("test-key".into()),
            env: Some("dev".into()),
            universe_id: Some(UNIVERSE),
            repository,
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

    /// The whole of `--repository`: one path segment, nothing else. The mock
    /// answers only `DataStoresConfig`, so a request that still went to the
    /// default would read as an empty config and fail with drift.
    #[tokio::test]
    async fn a_named_repository_is_the_path_segment_the_request_carries() {
        let dir = repo();
        let server = live_at(DATA_STORES_PATH, json!({ "features.new_popup": true })).await;

        check_with(&dir, &server, Some(Repository::DataStoresConfig))
            .await
            .expect("the read must go to the repository the flag named");

        let requests = server.received_requests().await.expect("recorded");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url.path(), DATA_STORES_PATH);
    }

    /// The file is the source of truth: it names the repository its entries
    /// describe, and a command that reads the entries reads that too. Also
    /// says the field survives the flattened environments, since a
    /// `repository` read as an env name would leave `[dev]` intact and the
    /// repository unset, and this test would hit the default path.
    #[tokio::test]
    async fn the_file_names_the_repository_when_the_flag_does_not() {
        let dir = write_repo(
            r#"repository = "DataStoresConfig"

[dev.entries."features.new_popup"]
value = true
"#,
        );
        let server = live_at(DATA_STORES_PATH, json!({ "features.new_popup": true })).await;

        check(&dir, &server)
            .await
            .expect("the read must go to the repository the file named");
    }

    /// Neither wins. A `sync` that pushed this file's entries into the
    /// repository the other name points at could not be undone from here, so
    /// the refusal names both and the file holding one of them.
    #[tokio::test]
    async fn a_flag_contradicting_the_file_is_refused_naming_both() {
        let dir = write_repo(
            r#"repository = "InExperienceConfig"

[dev.entries."features.new_popup"]
value = true
"#,
        );
        let server = MockServer::start().await;

        let err = check_with(&dir, &server, Some(Repository::DataStoresConfig))
            .await
            .expect_err("the flag and the file name different repositories");
        let message = format!("{err:#}");
        assert!(message.contains("DataStoresConfig"), "{message}");
        assert!(message.contains("InExperienceConfig"), "{message}");
        assert!(message.contains("rbxconfig.toml"), "{message}");
        assert!(
            server
                .received_requests()
                .await
                .expect("recorded")
                .is_empty(),
            "a contradiction is refused before any request"
        );
    }

    /// Agreement is not a contradiction: the same name twice is one
    /// instruction, and `Option` on the flag is what tells "not passed" from
    /// "passed the default".
    #[tokio::test]
    async fn a_flag_repeating_what_the_file_says_is_accepted() {
        let dir = write_repo(
            r#"repository = "DataStoresConfig"

[dev.entries."features.new_popup"]
value = true
"#,
        );
        let server = live_at(DATA_STORES_PATH, json!({ "features.new_popup": true })).await;

        check_with(&dir, &server, Some(Repository::DataStoresConfig))
            .await
            .expect("the flag agrees with the file");
    }
}
