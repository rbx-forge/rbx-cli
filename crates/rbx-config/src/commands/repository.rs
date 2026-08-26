//! Which repository each command addresses, for every command there is.
//!
//! Every test here runs the invocation that existed before `--repository` did:
//! no flag, and a `rbxconfig.toml` that names no repository. The mock server
//! answers the `InExperienceConfig` path and nothing else, so a command that
//! started sending another path segment would get wiremock's 404 rather than a
//! config, and fail here.
//!
//! The two commands with their own coverage elsewhere are `check` (in
//! `check.rs`, which is also where the precedence rules are pinned) and the
//! entry limits (`sync.rs`).

use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use rbx_core::api::Repository;

use crate::config::ConfigsFile;
use crate::ctx::ConfigCtx;
use crate::Strategy;

const UNIVERSE: u64 = 109876543210987;
const REPO_PATH: &str = "/creator-configs-public-api/v1/configs/universes/109876543210987/repositories/InExperienceConfig";
const REVISION: &str = "aaaaaaaa-1111-4000-8000-000000000001";

const LOCAL: &str = r#"
[dev.entries."features.new_popup"]
value = true
"#;

/// Two envs and no `repository` line: the shape `pull --repository` may not
/// stamp, because the field governs the whole file and `prod` was not fetched.
const TWO_ENVS: &str = r#"
[dev.entries."features.new_popup"]
value = true

[prod.entries."retention.player_data"]
value = "30d"
"#;

fn repo() -> TempDir {
    repo_with(LOCAL)
}

fn repo_with(local: &str) -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("rbxconfig.toml"), local).expect("write");
    dir
}

fn ctx(dir: &TempDir, server: &MockServer, repository: Option<Repository>) -> ConfigCtx {
    ConfigCtx {
        config: dir.path().join("rbxconfig.toml"),
        places: dir.path().join("rbxplace.toml"),
        api_key: Some("test-key".into()),
        env: Some("dev".into()),
        universe_id: Some(UNIVERSE),
        repository,
        base_url: Some(server.uri()),
    }
}

/// The live config read, on the default path only.
///
/// At least one read, not exactly one: `sync` reads the repository root twice,
/// once for the diff it prints and once inside `overwrite_and_publish`, which
/// has to know the published conditional rules before an overwrite that omits
/// them clears them (`rbx-core/src/api/configs.rs`). What this file pins is
/// the path, and an unmatched path still 404s the command's own read.
async fn published(entries: serde_json::Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(REPO_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "metadata": { "configVersion": 3 }, "entries": entries })),
        )
        .expect(1..)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn get_reads_the_default_repository() {
    let dir = repo();
    let server = published(json!({ "features.new_popup": true })).await;

    super::get::run(&ctx(&dir, &server, None), None, false)
        .await
        .expect("get");
}

#[tokio::test]
async fn get_of_one_key_reads_the_default_repository() {
    let dir = repo();
    let server = published(json!({ "features.new_popup": true })).await;

    super::get::run(&ctx(&dir, &server, None), Some("features.new_popup"), false)
        .await
        .expect("get one key");
}

#[tokio::test]
async fn list_reads_the_default_repository() {
    let dir = repo();
    let server = published(json!({ "features.new_popup": true })).await;

    super::list::run(&ctx(&dir, &server, None), false)
        .await
        .expect("list");
}

#[tokio::test]
async fn versions_reads_the_default_repository() {
    let dir = repo();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/revisions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "revisions": [{
                "revisionId": REVISION,
                "version": 3,
                "time": "2026-08-15T09:30:00Z",
                "message": "raise the cap",
                "changes": { "features.new_popup": {} },
            }],
        })))
        .expect(1)
        .mount(&server)
        .await;

    super::versions::run(&ctx(&dir, &server, None), 20, false)
        .await
        .expect("versions");
}

#[tokio::test]
async fn pull_reads_the_default_repository_and_writes_no_repository_line() {
    let dir = repo();
    let server = published(json!({ "features.new_popup": false })).await;

    super::pull::run(&ctx(&dir, &server, None), true)
        .await
        .expect("pull");

    let written = std::fs::read_to_string(dir.path().join("rbxconfig.toml")).expect("read");
    assert!(!written.contains("repository"), "{written}");
    assert!(written.contains("value = false"), "{written}");
}

/// The one thing `pull` records that it did not before: a file it wrote from
/// another repository has to say so, or the next `sync` reads a silent file,
/// takes the default, and publishes these entries into `InExperienceConfig`.
#[tokio::test]
async fn pull_records_a_repository_that_is_not_the_default() {
    let dir = repo();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/creator-configs-public-api/v1/configs/universes/109876543210987/repositories/DataStoresConfig",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "metadata": { "configVersion": 3 }, "entries": { "features.new_popup": false } }),
        ))
        .expect(1)
        .mount(&server)
        .await;

    super::pull::run(
        &ctx(&dir, &server, Some(Repository::DataStoresConfig)),
        true,
    )
    .await
    .expect("pull");

    let written = ConfigsFile::load(&dir.path().join("rbxconfig.toml")).expect("reload");
    assert_eq!(
        written.declared_repository().unwrap(),
        Some(Repository::DataStoresConfig)
    );
    assert_eq!(written.environments["dev"].entries.len(), 1);
}

/// The refusal that keeps the stamp above from reaching a file it does not
/// describe. `repository` governs the whole file, so recording it after a
/// `pull --env dev` would move `prod`'s entries to that repository too, and
/// the next `sync --env prod` overwrites the repository with prod's keys
/// alone (`pull.rs:76-100`). Against `DataStoresConfig` that erases the
/// universe's right-to-be-forgotten templates with nothing said, which is why
/// the file has to come out of a refused pull byte for byte unchanged.
#[tokio::test]
async fn pull_refuses_to_record_a_repository_over_an_env_it_did_not_fetch() {
    let dir = repo_with(TWO_ENVS);
    let config = dir.path().join("rbxconfig.toml");
    let before = std::fs::read(&config).expect("read");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/creator-configs-public-api/v1/configs/universes/109876543210987/repositories/DataStoresConfig",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "metadata": { "configVersion": 3 }, "entries": { "features.new_popup": false } }),
        ))
        .mount(&server)
        .await;

    let err = super::pull::run(
        &ctx(&dir, &server, Some(Repository::DataStoresConfig)),
        true,
    )
    .await
    .expect_err("a repository line would speak for prod as well");

    // Naming the env is the whole point of the message: the user has to know
    // which one blocks the stamp to decide between `--config` and editing the
    // field by hand.
    let message = format!("{err:#}");
    assert!(message.contains("prod"), "{message}");
    assert!(message.contains("DataStoresConfig"), "{message}");

    assert_eq!(
        std::fs::read(&config).expect("read"),
        before,
        "a refused pull writes nothing, not even the dev entries it fetched"
    );
    assert!(
        !dir.path().join(crate::lock::LOCKFILE_NAME).exists(),
        "a refused pull records no lock either"
    );
}

#[tokio::test]
async fn sync_writes_to_the_default_repository() {
    let dir = repo();
    let server = published(json!({})).await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/draft")))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("{REPO_PATH}/draft:overwrite")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "draftHash": "aaaaBBBBccccDDDD" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{REPO_PATH}/publish")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "configVersion": 4 })))
        .expect(1)
        .mount(&server)
        .await;

    super::sync::run(
        &ctx(&dir, &server, None),
        Some("ship it"),
        false,
        &Strategy::Immediate,
        false,
        true,
    )
    .await
    .expect("sync");
}

/// A staged draft is overwritten, not treated as a reason to stop: a pipeline
/// that publishes on every merge legitimately replaces whatever the Creator
/// Hub had staged. What it may not do is say nothing, and the keys are what it
/// says, which is why the draft read is asserted to have happened.
#[tokio::test]
async fn sync_over_a_staged_draft_publishes_and_does_not_refuse() {
    let dir = repo();
    let server = published(json!({})).await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/draft")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "draftHash": "aaaaBBBBccccDDDD",
            "entries": { "features.old_popup": { "value": true } },
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("{REPO_PATH}/draft:overwrite")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "draftHash": "eeeeFFFFgggg0000" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{REPO_PATH}/publish")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "configVersion": 4 })))
        .expect(1)
        .mount(&server)
        .await;

    super::sync::run(
        &ctx(&dir, &server, None),
        None,
        true,
        &Strategy::Immediate,
        false,
        true,
    )
    .await
    .expect("a staged draft is replaced, not a refusal");
}

/// `rollback` restores into the default repository, on the path the vendored
/// spec documents: `POST .../revisions/{revisionId}/restore`, a subresource
/// and not the `:restore` custom-method form, which that document uses only
/// for `/assets/v1/assets/{assetId}:restore`. The old spelling reached Roblox
/// as an unknown path and the rollback never happened.
///
/// It cannot get further than the restore in a test: the publish message is
/// composed interactively and there is nobody to ask, so the assertion is on
/// the request that was made and on which question stopped the run.
#[tokio::test]
async fn rollback_restores_from_the_default_repository() {
    let dir = repo();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("{REPO_PATH}/revisions/{REVISION}/restore")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "draftHash": "aaaaBBBBccccDDDD" })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let err = super::rollback::run(&ctx(&dir, &server, None), Some(REVISION.to_string()), 10)
        .await
        .expect_err("a publish message needs a terminal");
    assert!(format!("{err:#}").contains("--message"), "{err:#}");

    let requests = server.received_requests().await.expect("recorded");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url.path(),
        format!("{REPO_PATH}/revisions/{REVISION}/restore")
    );
}

/// `rbx config init` writes the file it always wrote. A `repository` line
/// naming the default would be a fact about the default rather than a
/// decision, and one more thing to keep in step with it.
#[test]
fn init_writes_no_repository_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("rbxconfig.toml");
    let ctx = ConfigCtx {
        config: out.clone(),
        places: dir.path().join("rbxplace.toml"),
        api_key: None,
        env: None,
        universe_id: None,
        repository: None,
        base_url: None,
    };

    super::init::run(&ctx).expect("init");

    let written = std::fs::read_to_string(&out).expect("read");
    assert!(!written.contains("repository"), "{written}");
    assert!(
        written.starts_with("# rbxconfig.toml: local source of truth"),
        "{written}"
    );
    let file = ConfigsFile::load(&out).expect("parse");
    assert_eq!(file.declared_repository().unwrap(), None);
    assert_eq!(file.environments["dev"].entries.len(), 3);
}

/// With a repository named, the line goes above the first table header, or it
/// would be read back as an entry of whichever environment came first.
#[test]
fn init_writes_the_repository_line_when_it_is_not_the_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("rbxconfig.toml");
    let ctx = ConfigCtx {
        config: out.clone(),
        places: dir.path().join("rbxplace.toml"),
        api_key: None,
        env: None,
        universe_id: None,
        repository: Some(Repository::DataStoresConfig),
        base_url: None,
    };

    super::init::run(&ctx).expect("init");

    let file = ConfigsFile::load(&out).expect("parse");
    assert_eq!(
        file.declared_repository().unwrap(),
        Some(Repository::DataStoresConfig)
    );
    assert_eq!(file.environments["dev"].entries.len(), 3);
}
