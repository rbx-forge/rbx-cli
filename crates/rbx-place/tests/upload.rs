#![allow(clippy::unwrap_used)]
//! What `rbx place upload` sends when `--env` names more than one env, and
//! what the commands that cannot fan out do when it does.
//!
//! Driven end to end through `run` rather than through the API client, because
//! the parts worth protecting are the ordering decisions: that a single env is
//! still exactly one upload, that a fan-out walks its envs in target order,
//! that the confirmation is refused before the first byte goes out, and that
//! the walk stops at the env that failed instead of carrying on.
//!
//! stdout is the process's own, so the human lines and the `--json` document
//! are not assertable from here. The documents are pinned in `json.rs`, next to
//! the structs that build them; what these tests pin is the traffic.

use rbx_core::GlobalFlags;
use rbx_place::{run, PlaceCli};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const DEV_UNIVERSE: u64 = 109_876_543_210_987;
const DEV_PLACE: u64 = 109_876_543_210_001;
const PROD_UNIVERSE: u64 = 109_876_543_210_988;
const PROD_PLACE: u64 = 109_876_543_210_002;
const QA_UNIVERSE: u64 = 109_876_543_210_989;
const QA_PLACE: u64 = 109_876_543_210_003;

/// Three envs and a group, written for real because `--env` resolves against
/// the file and `all` means "whatever is in it".
///
/// `prod` is the only one that asks for confirmation, so a run over the lot
/// exercises the "any target env" gate rather than "every" or "the first".
/// The group lists its members in the reverse of sorted order, which is how a
/// test can tell declared order from `all`'s.
fn places_file(dir: &std::path::Path) -> std::path::PathBuf {
    let file = dir.join("rbxplace.toml");
    std::fs::write(
        &file,
        format!(
            "[groups]\n\
             both = [\"prod\", \"dev\"]\n\
             \n\
             [dev]\n\
             universe_id = {DEV_UNIVERSE}\n\
             [dev.places]\n\
             main = {DEV_PLACE}\n\
             \n\
             [prod]\n\
             universe_id = {PROD_UNIVERSE}\n\
             confirm = true\n\
             [prod.places]\n\
             main = {PROD_PLACE}\n\
             \n\
             [qa]\n\
             universe_id = {QA_UNIVERSE}\n\
             [qa.places]\n\
             main = {QA_PLACE}\n"
        ),
    )
    .unwrap();
    file
}

fn flags(places: &std::path::Path, env: &str) -> GlobalFlags {
    GlobalFlags {
        api_key: Some("test-key".into()),
        cookie: None,
        no_auto_cookie: true,
        auto_cookie: false,
        env: Some(env.into()),
        place: None,
        places: places.to_path_buf(),
        universe_id: None,
        place_id: Vec::new(),
    }
}

/// `PlaceCli` is an `Args` group, not a parser in its own right, so the tests
/// wrap it the way the `rbx` binary does. Building it from a real argv means
/// these tests also cover the flag definitions.
#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    place: PlaceCli,
}

fn cli(args: &[&str], server: &MockServer) -> PlaceCli {
    let mut argv: Vec<String> = vec!["place".to_string()];
    argv.extend(args.iter().map(|arg| (*arg).to_string()));
    argv.push("--base-url".to_string());
    argv.push(server.uri());
    <Wrapper as clap::Parser>::parse_from(argv).place
}

fn rbxl(dir: &std::path::Path) -> std::path::PathBuf {
    let file = dir.join("place.rbxl");
    std::fs::write(&file, b"not really a place file").unwrap();
    file
}

fn upload_path(universe: u64, place: u64) -> String {
    format!("/universes/v1/{universe}/places/{place}/versions")
}

/// Answer one env's upload with a version number.
async fn mount_upload(server: &MockServer, universe: u64, place: u64, version: u64) {
    Mock::given(method("POST"))
        .and(path(upload_path(universe, place)))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "versionNumber": version
        })))
        .mount(server)
        .await;
}

/// The upload paths the server was asked for, in the order they arrived.
async fn uploads(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|request: &Request| request.url.path().to_string())
        .collect()
}

/// The regression that matters most: naming one env is still one upload, and
/// an env with no `confirm` still asks nothing.
#[tokio::test]
async fn one_env_is_still_exactly_one_upload() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());
    let file = rbxl(dir.path());
    let server = MockServer::start().await;
    mount_upload(&server, DEV_UNIVERSE, DEV_PLACE, 3).await;

    run(
        cli(
            &["upload", "--env", "dev", "--file", &file.to_string_lossy()],
            &server,
        ),
        &flags(&places, "dev"),
    )
    .await
    .expect("a single env with no confirm gate uploads");

    assert_eq!(
        uploads(&server).await,
        vec![upload_path(DEV_UNIVERSE, DEV_PLACE)]
    );
}

#[tokio::test]
async fn env_all_uploads_to_every_env_in_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());
    let file = rbxl(dir.path());
    let server = MockServer::start().await;
    mount_upload(&server, DEV_UNIVERSE, DEV_PLACE, 1).await;
    mount_upload(&server, PROD_UNIVERSE, PROD_PLACE, 2).await;
    mount_upload(&server, QA_UNIVERSE, QA_PLACE, 3).await;

    run(
        cli(
            &[
                "upload",
                "--env",
                "all",
                "--file",
                &file.to_string_lossy(),
                "--yes",
            ],
            &server,
        ),
        &flags(&places, "all"),
    )
    .await
    .expect("every env is answered");

    // `all` is the file's envs sorted, which is what makes the order a
    // property of the file rather than of a HashMap's iteration.
    assert_eq!(
        uploads(&server).await,
        vec![
            upload_path(DEV_UNIVERSE, DEV_PLACE),
            upload_path(PROD_UNIVERSE, PROD_PLACE),
            upload_path(QA_UNIVERSE, QA_PLACE),
        ]
    );
}

/// The `--json` fan-out end to end. The document's shape is pinned in
/// `json.rs`; what this covers is that the run reaches the emit at all, with
/// one receipt per env behind it, rather than tripping over the single-env
/// shortcut on the way out.
#[tokio::test]
async fn env_all_under_json_still_writes_every_env() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());
    let file = rbxl(dir.path());
    let server = MockServer::start().await;
    mount_upload(&server, DEV_UNIVERSE, DEV_PLACE, 1).await;
    mount_upload(&server, PROD_UNIVERSE, PROD_PLACE, 2).await;
    mount_upload(&server, QA_UNIVERSE, QA_PLACE, 3).await;

    run(
        cli(
            &[
                "upload",
                "--env",
                "all",
                "--file",
                &file.to_string_lossy(),
                "--yes",
                "--json",
            ],
            &server,
        ),
        &flags(&places, "all"),
    )
    .await
    .expect("every env is answered");

    assert_eq!(uploads(&server).await.len(), 3);
}

/// A group reaches this command already expanded, and in the order it was
/// declared rather than sorted, so `both = ["prod", "dev"]` writes prod first.
#[tokio::test]
async fn a_group_uploads_to_its_members_in_declared_order() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());
    let file = rbxl(dir.path());
    let server = MockServer::start().await;
    mount_upload(&server, DEV_UNIVERSE, DEV_PLACE, 1).await;
    mount_upload(&server, PROD_UNIVERSE, PROD_PLACE, 2).await;

    run(
        cli(
            &[
                "upload",
                "--env",
                "both",
                "--file",
                &file.to_string_lossy(),
                "--yes",
            ],
            &server,
        ),
        &flags(&places, "both"),
    )
    .await
    .expect("a group's members are answered");

    assert_eq!(
        uploads(&server).await,
        vec![
            upload_path(PROD_UNIVERSE, PROD_PLACE),
            upload_path(DEV_UNIVERSE, DEV_PLACE),
        ]
    );
}

/// The walk stops at the env that failed. The env before it keeps the version
/// it was given, the env after it is never asked, and the run still fails.
#[tokio::test]
async fn a_failed_env_stops_the_walk_and_leaves_the_earlier_one_written() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());
    let file = rbxl(dir.path());
    let server = MockServer::start().await;
    mount_upload(&server, DEV_UNIVERSE, DEV_PLACE, 1).await;
    mount_upload(&server, QA_UNIVERSE, QA_PLACE, 3).await;
    // 403 rather than a 5xx: a server error is retried, and what is being
    // tested here is the walk, not the retry policy.
    Mock::given(method("POST"))
        .and(path(upload_path(PROD_UNIVERSE, PROD_PLACE)))
        .respond_with(ResponseTemplate::new(403).set_body_string("no permission"))
        .mount(&server)
        .await;

    let error = run(
        cli(
            &[
                "upload",
                "--env",
                "all",
                "--file",
                &file.to_string_lossy(),
                "--yes",
            ],
            &server,
        ),
        &flags(&places, "all"),
    )
    .await
    .expect_err("a refused env fails the run");

    assert!(format!("{error:#}").contains("403"), "got: {error:#}");
    assert_eq!(
        uploads(&server).await,
        vec![
            upload_path(DEV_UNIVERSE, DEV_PLACE),
            upload_path(PROD_UNIVERSE, PROD_PLACE),
        ],
        "qa comes after the failure and must never be asked"
    );
}

/// The confirmation gate covers every target env, so one env with
/// `confirm = true` gates the whole fan-out. `--json` cannot answer a prompt,
/// so the refusal has to land before the first write rather than after two.
#[tokio::test]
async fn a_confirm_gated_env_anywhere_in_the_list_refuses_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());
    let file = rbxl(dir.path());
    let server = MockServer::start().await;
    mount_upload(&server, DEV_UNIVERSE, DEV_PLACE, 1).await;
    mount_upload(&server, PROD_UNIVERSE, PROD_PLACE, 2).await;
    mount_upload(&server, QA_UNIVERSE, QA_PLACE, 3).await;

    let error = run(
        cli(
            &[
                "upload",
                "--env",
                "all",
                "--file",
                &file.to_string_lossy(),
                "--json",
            ],
            &server,
        ),
        &flags(&places, "all"),
    )
    .await
    .expect_err("prod asks for confirmation and --json cannot answer");

    assert!(format!("{error:#}").contains("--yes"), "got: {error:#}");
    assert!(
        uploads(&server).await.is_empty(),
        "dev sorts before prod and must not have been written"
    );
}

/// `download` writes one file to one `--out`, so N envs would resolve onto one
/// path and every download but the last would be overwritten.
#[tokio::test]
async fn download_refuses_a_plural_selector_and_names_the_flag() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());
    let server = MockServer::start().await;

    for selector in ["all", "both"] {
        let error = run(
            cli(&["download", "--env", selector], &server),
            &flags(&places, selector),
        )
        .await
        .expect_err("a plural selector has nowhere to put the second file");

        let error = format!("{error:#}");
        assert!(error.contains("--out"), "got: {error}");
        assert!(
            uploads(&server).await.is_empty(),
            "nothing should have been fetched"
        );
    }
}

/// `promote` names its two envs itself. A plural `--env` there selects nothing,
/// and accepting the flag and ignoring it is the failure this repo already has
/// a doc-comment about.
#[tokio::test]
async fn promote_refuses_a_plural_selector_and_names_from_and_to() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());
    let server = MockServer::start().await;

    for selector in ["all", "both"] {
        let error = run(
            cli(
                &["promote", "--from", "dev", "--to", "prod", "--yes"],
                &server,
            ),
            &flags(&places, selector),
        )
        .await
        .expect_err("a plural selector is not how promote names an env");

        let error = format!("{error:#}");
        assert!(error.contains("--from"), "got: {error}");
        assert!(error.contains("--to"), "got: {error}");
        assert!(
            uploads(&server).await.is_empty(),
            "nothing should have been written"
        );
    }
}
