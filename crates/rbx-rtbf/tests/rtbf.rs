//! RTBF over HTTP.
//!
//! The unit tests in `src/` cover the pure halves: the validation rules, the
//! wire conversion, the pattern matching. What they cannot see is the thing
//! that matters most here, which is that `sync` sends the payload Roblox's
//! guide documents to the `DataStoresConfig` repository and nowhere else.
//!
//! These templates are a compliance artefact. A `sync` that published them
//! under the wrong repository, or dropped the `{UserId}` token on the way to
//! the wire, would be a legal obligation quietly unmet, and no local test would
//! have noticed.

#![allow(clippy::unwrap_used)]

use rbx_core::GlobalFlags;
use rbx_rtbf::{run, RtbfCli};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 109876543210987;

/// The path segment that must appear, and the one that must not.
///
/// Spelled out rather than built from the enum: a test that derives the
/// expected path from the same constant the code uses would pass if both were
/// wrong together.
const REPO_PATH: &str =
    "/creator-configs-public-api/v1/configs/universes/109876543210987/repositories/DataStoresConfig";

#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    global: GlobalFlags,
    #[command(subcommand)]
    command: Sub,
}

#[derive(clap::Subcommand)]
enum Sub {
    Rtbf(RtbfCli),
}

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// A places file so `--env dev` resolves without a real project.
fn places(at: &std::path::Path) -> String {
    let file = at.join("rbxplace.toml");
    std::fs::write(&file, format!("[dev]\nuniverse_id = {UNIVERSE}\n")).unwrap();
    file.to_string_lossy().to_string()
}

fn write_config(at: &std::path::Path, body: &str) -> std::path::PathBuf {
    let file = at.join("rbxrtbf.toml");
    std::fs::write(&file, body).unwrap();
    file
}

/// Build the invocation the binary would build, through clap, so the flags
/// under test are the real ones rather than a struct literal that cannot
/// disagree with them.
fn cli(args: &[&str], server: &MockServer, at: &std::path::Path) -> (RtbfCli, GlobalFlags) {
    let places = places(at);
    let config = at.join("rbxrtbf.toml").to_string_lossy().to_string();
    let base = server.uri();
    let mut argv = vec![
        "rbx",
        "--api-key",
        "test-key",
        "--no-auto-cookie",
        "--places",
        &places,
        "rtbf",
        "--config",
        &config,
        "--base-url",
        &base,
    ];
    argv.extend_from_slice(args);
    let parsed = <Wrapper as clap::Parser>::parse_from(argv);
    let Sub::Rtbf(rtbf) = parsed.command;
    (rtbf, parsed.global)
}

const WORKED_EXAMPLE: &str = r#"
[[key]]
store = "PlayerInventory"
pattern = "User_{UserId}"
scope = "Scope_{UserId}"

[[key]]
store = "PlayerLeaderboard"
pattern = "User_{UserId}"
ordered = true

[[store]]
pattern = "Player_{UserId}_Save"
"#;

/// The payload from Roblox's own RTBF guide, which is what `WORKED_EXAMPLE`
/// declares. Written out here rather than generated, so a change to
/// `to_entries` has to be defended against the documentation.
fn documented_payload() -> serde_json::Value {
    serde_json::json!({
        "entries": {
            "user_data_templates": [
                {"key_template": {
                    "data_store_type": "STANDARD",
                    "data_store_name": "PlayerInventory",
                    "key_pattern": "User_{UserId}",
                    "scope_pattern": "Scope_{UserId}"
                }},
                {"key_template": {
                    "data_store_type": "ORDERED",
                    "data_store_name": "PlayerLeaderboard",
                    "key_pattern": "User_{UserId}",
                    "scope_pattern": "global"
                }},
                {"data_store_template": {
                    "data_store_type": "STANDARD",
                    "data_store_pattern": "Player_{UserId}_Save"
                }}
            ]
        }
    })
}

async fn mount_no_draft(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/draft")))
        .respond_with(ResponseTemplate::new(404))
        .mount(server)
        .await;
}

async fn mount_published(server: &MockServer, templates: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(REPO_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "metadata": { "configVersion": 3 },
            "entries": { "user_data_templates": templates }
        })))
        .mount(server)
        .await;
}

/// The one that matters: the documented payload, at the documented path.
#[tokio::test]
async fn sync_sends_the_documented_payload_to_the_datastores_repository() {
    let d = dir();
    write_config(d.path(), WORKED_EXAMPLE);
    let server = MockServer::start().await;
    mount_no_draft(&server).await;

    Mock::given(method("PUT"))
        .and(path(format!("{REPO_PATH}/draft:overwrite")))
        .and(header("x-api-key", "test-key"))
        .and(body_json(documented_payload()))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"draftHash": "h1"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path(format!("{REPO_PATH}/publish")))
        .and(body_json(serde_json::json!({
            "message": "rbx rtbf sync: 3 template(s)",
            "deploymentStrategy": "Immediate",
            "draftHash": "h1"
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"configVersion": 4})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let (rtbf, global) = cli(&["sync", "--env", "dev", "--yes"], &server, d.path());
    run(rtbf, &global).await.unwrap();
}

/// `draftHash` is Roblox's concurrency check, and the reason a sync
/// reads before it writes. Without it a draft staged in the Creator Hub is
/// discarded in silence.
#[tokio::test]
async fn sync_hands_back_the_hash_of_the_draft_it_is_replacing() {
    let d = dir();
    write_config(
        d.path(),
        "[[key]]\nstore = \"A\"\npattern = \"User_{UserId}\"\n",
    );
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(format!("{REPO_PATH}/draft")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "draftHash": "staged-by-somebody-else",
            "entries": { "user_data_templates": {} }
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("PUT"))
        .and(path(format!("{REPO_PATH}/draft:overwrite")))
        .and(body_json(serde_json::json!({
            "entries": { "user_data_templates": [
                {"key_template": {
                    "data_store_type": "STANDARD",
                    "data_store_name": "A",
                    "key_pattern": "User_{UserId}",
                    "scope_pattern": "global"
                }}
            ]},
            "draftHash": "staged-by-somebody-else"
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"draftHash": "h2"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path(format!("{REPO_PATH}/publish")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"configVersion": 9})),
        )
        .mount(&server)
        .await;

    let (rtbf, global) = cli(&["sync", "--env", "dev", "--yes"], &server, d.path());
    run(rtbf, &global).await.unwrap();
}

/// A file the tool refuses must cost no request. The `{userId}` here is the
/// mistake Roblox accepts and never matches, so publishing it would be worse
/// than failing.
#[tokio::test]
async fn a_miscased_token_never_reaches_the_network() {
    let d = dir();
    write_config(
        d.path(),
        "[[key]]\nstore = \"A\"\npattern = \"User_{userId}\"\n",
    );
    let server = MockServer::start().await;

    let (rtbf, global) = cli(&["sync", "--env", "dev", "--yes"], &server, d.path());
    let err = run(rtbf, &global).await.unwrap_err().to_string();
    assert!(err.contains("case-sensitive"), "got: {err}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a template the tool refuses must not be published"
    );
}

/// The one character typo, at the level where it would have cost data.
///
/// `[[keys]]` (the plural, which is what the Rust field is called) parses to an
/// empty declaration, and every layer below reads that as legitimate: `validate`
/// passes it on purpose, the read-before-write loop finds nothing it cannot
/// parse, and `--yes` skips the prompt whose count was the last signal. What
/// used to happen next was `user_data_templates: []` published over every
/// deletion template the universe had, exit 0.
#[tokio::test]
async fn a_declaration_emptied_by_a_typo_never_reaches_the_network() {
    let d = dir();
    write_config(
        d.path(),
        "[[keys]]\nstore = \"PlayerInventory\"\npattern = \"User_{UserId}\"\n",
    );
    let server = MockServer::start().await;
    // Mounted so the assertion below is about the refusal rather than about a
    // missing mock: this is what the wipe would have replaced.
    mount_published(
        &server,
        documented_payload()["entries"]["user_data_templates"].clone(),
    )
    .await;

    let (rtbf, global) = cli(&["sync", "--env", "dev", "--yes"], &server, d.path());
    let err = format!("{:#}", run(rtbf, &global).await.unwrap_err());
    assert!(err.contains("keys"), "the misspelling must be named: {err}");
    assert!(
        err.contains("key, store"),
        "and what it should have been: {err}"
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "an empty declaration must not be published over a live template set"
    );
}

/// `verify` publishes nothing, but it is the command that signs a declaration
/// off: an empty set has no template naming a store that is gone, so a file a
/// typo emptied would report `ok: true` and exit 0.
#[tokio::test]
async fn verify_refuses_a_declaration_emptied_by_a_typo_rather_than_passing_it() {
    let d = dir();
    write_config(
        d.path(),
        "[[keys]]\nstore = \"PlayerInventory\"\npattern = \"User_{UserId}\"\n",
    );
    let server = MockServer::start().await;
    mount_stores(&server, &["PlayerInventory"]).await;

    let (rtbf, global) = cli(&["verify", "--env", "dev"], &server, d.path());
    let err = format!("{:#}", run(rtbf, &global).await.unwrap_err());
    assert!(err.contains("keys"), "{err}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a file it refuses must cost no request"
    );
}

#[tokio::test]
async fn check_is_clean_when_the_file_and_the_published_set_agree() {
    let d = dir();
    write_config(d.path(), WORKED_EXAMPLE);
    let server = MockServer::start().await;
    mount_published(
        &server,
        documented_payload()["entries"]["user_data_templates"].clone(),
    )
    .await;

    let (rtbf, global) = cli(&["check", "--env", "dev"], &server, d.path());
    run(rtbf, &global).await.unwrap();
}

/// Order carries no meaning (deletion is a match, not a sequence), so a file
/// somebody tidied must not report as drift.
#[tokio::test]
async fn check_is_not_fooled_by_a_reordered_published_set() {
    let d = dir();
    write_config(
        d.path(),
        "[[key]]\nstore = \"A\"\npattern = \"U_{UserId}\"\n\n\
         [[key]]\nstore = \"B\"\npattern = \"U_{UserId}\"\n",
    );
    let server = MockServer::start().await;
    mount_published(
        &server,
        serde_json::json!([
            {"key_template": {"data_store_type": "STANDARD", "data_store_name": "B",
                              "key_pattern": "U_{UserId}", "scope_pattern": "global"}},
            {"key_template": {"data_store_type": "STANDARD", "data_store_name": "A",
                              "key_pattern": "U_{UserId}", "scope_pattern": "global"}}
        ]),
    )
    .await;

    let (rtbf, global) = cli(&["check", "--env", "dev"], &server, d.path());
    run(rtbf, &global).await.unwrap();
}

/// Drift leaves through `Drift` so the exit code is 2, which is what a CI step
/// reads. Reporting on screen and exiting 0 would call a drifting repo clean.
#[tokio::test]
async fn check_exits_two_when_the_published_set_differs() {
    let d = dir();
    write_config(
        d.path(),
        "[[key]]\nstore = \"A\"\npattern = \"U_{UserId}\"\n",
    );
    let server = MockServer::start().await;
    mount_published(&server, serde_json::json!([])).await;

    let (rtbf, global) = cli(&["check", "--env", "dev"], &server, d.path());
    let err = run(rtbf, &global).await.unwrap_err();
    assert!(
        err.chain()
            .any(|cause| cause.is::<rbx_core::generated::Drift>()),
        "drift must exit 2, not 1: {err:#}"
    );
    assert!(format!("{err:#}").contains("rbx rtbf sync"), "{err:#}");
}

#[tokio::test]
async fn pull_writes_the_published_templates_into_the_file() {
    let d = dir();
    let server = MockServer::start().await;
    mount_published(
        &server,
        documented_payload()["entries"]["user_data_templates"].clone(),
    )
    .await;

    let config = d.path().join("rbxrtbf.toml");
    let (rtbf, global) = cli(&["pull", "--env", "dev", "--yes"], &server, d.path());
    run(rtbf, &global).await.unwrap();

    let written = rbx_rtbf::config::load(&config)
        .expect("the pulled file parses")
        .templates;
    assert_eq!(written.keys.len(), 2);
    assert_eq!(written.stores.len(), 1);
    assert_eq!(written.keys[0].store, "PlayerInventory");
    assert_eq!(written.keys[0].scope.as_deref(), Some("Scope_{UserId}"));
    // `global` came back as an absence, so a pull of a synced file is a no-op
    // rather than a line saying what its own absence already said.
    assert_eq!(written.keys[1].scope, None);
    assert!(written.keys[1].ordered);
}

/// A universe configured by a newer release must not be silently truncated:
/// writing the file would drop what this build cannot model, and the file is
/// what the next `sync` publishes.
#[tokio::test]
async fn pull_refuses_rather_than_dropping_a_template_it_does_not_understand() {
    let d = dir();
    let server = MockServer::start().await;
    mount_published(
        &server,
        serde_json::json!([{"future_template": {"whatever": true}}]),
    )
    .await;

    let config = d.path().join("rbxrtbf.toml");
    let (rtbf, global) = cli(&["pull", "--env", "dev", "--yes"], &server, d.path());
    let err = run(rtbf, &global).await.unwrap_err().to_string();
    assert!(err.contains("would lose 1 template"), "got: {err}");
    assert!(!config.exists(), "nothing may be written on that path");
}

async fn mount_stores(server: &MockServer, names: &[&str]) {
    let stores: Vec<serde_json::Value> = names
        .iter()
        .map(|name| {
            serde_json::json!({ "path": format!("universes/{UNIVERSE}/data-stores/{name}") })
        })
        .collect();
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/universes/{UNIVERSE}/data-stores")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"dataStores": stores})),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn verify_passes_when_every_template_names_a_store_that_exists() {
    let d = dir();
    write_config(
        d.path(),
        "[[key]]\nstore = \"PlayerInventory\"\npattern = \"User_{UserId}\"\n",
    );
    let server = MockServer::start().await;
    mount_stores(&server, &["PlayerInventory", "GlobalSettings"]).await;

    let (rtbf, global) = cli(&["verify", "--env", "dev"], &server, d.path());
    run(rtbf, &global).await.unwrap();
}

/// The failure this command exists for, and the one nothing else catches: a
/// store renamed out from under a template Roblox keeps storing happily.
#[tokio::test]
async fn verify_exits_two_when_a_template_names_a_store_that_is_gone() {
    let d = dir();
    write_config(
        d.path(),
        "[[key]]\nstore = \"PlayerInventoryV1\"\npattern = \"User_{UserId}\"\n",
    );
    let server = MockServer::start().await;
    mount_stores(&server, &["PlayerInventoryV2"]).await;

    let (rtbf, global) = cli(&["verify", "--env", "dev"], &server, d.path());
    let err = run(rtbf, &global).await.unwrap_err();
    assert!(
        err.chain()
            .any(|cause| cause.is::<rbx_core::generated::Drift>()),
        "an inert template must exit 2: {err:#}"
    );
    assert!(format!("{err:#}").contains("deletes nothing"), "{err:#}");
}

/// Open Cloud lists standard stores only, so an ordered store's absence says
/// nothing. Calling it missing would be a false alarm, and a command that cries
/// wolf gets ignored, which costs more than the check is worth.
#[tokio::test]
async fn verify_does_not_fail_an_ordered_store_it_cannot_see() {
    let d = dir();
    write_config(
        d.path(),
        "[[key]]\nstore = \"Leaderboard\"\npattern = \"User_{UserId}\"\nordered = true\n",
    );
    let server = MockServer::start().await;
    mount_stores(&server, &["SomethingElse"]).await;

    let (rtbf, global) = cli(&["verify", "--env", "dev"], &server, d.path());
    run(rtbf, &global)
        .await
        .expect("an unlistable store is unchecked, not missing");
}

/// `sync` fans out; `pull` and `verify` act on one universe and say so rather
/// than picking the first.
#[tokio::test]
async fn a_plural_selector_is_refused_where_the_command_reads_one_universe() {
    let d = dir();
    write_config(
        d.path(),
        "[[key]]\nstore = \"A\"\npattern = \"U_{UserId}\"\n",
    );
    let server = MockServer::start().await;

    // `--yes` belongs to `pull`, which prompts. `verify` is read-only.
    for (verb, args) in [
        ("pull", vec!["pull", "--env", "all", "--yes"]),
        ("verify", vec!["verify", "--env", "all"]),
    ] {
        let (rtbf, global) = cli(&args, &server, d.path());
        let err = run(rtbf, &global).await.unwrap_err().to_string();
        assert!(err.contains("acts on one"), "{verb}: {err}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}
