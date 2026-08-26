//! `rbx place --json` against the real binary: stdout carries the document and
//! nothing else, including when the run fails partway through.
//!
//! The unit tests in `rbx_place::json` pin what the documents *say*. They
//! cannot pin what else reaches stdout, because a stray `println!` three layers
//! down is invisible to a test that renders a struct into a buffer, and a
//! stray `println!` is exactly the failure that breaks `jq` in somebody's
//! pipeline. So these run the binary and parse its stdout.
//!
//! Every case is arranged to have something to say on stderr: an unknown key in
//! `rbxplace.toml`, a `--log` notice, a Team Create lock, a refused prompt. A
//! diagnostic that has nowhere safe to go is how stdout gets polluted.
//!
//! `run_json` is a copy of the one in `json_output.rs` rather than a shared
//! helper: the two files are written in parallel by different changes, and ten
//! duplicated lines are cheaper than a merge conflict in the fixture both would
//! have to agree on.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DEV_UNIVERSE: u64 = 100;
const DEV_MAIN: u64 = 1001;
const DEV_LOBBY: u64 = 1002;
const PROD_UNIVERSE: u64 = 5544332211;
const PROD_MAIN: u64 = 55443322110099;

/// Two envs, an unknown key so `PlacesConfig::load` has a warning to emit on
/// every command that reads the file, and a `confirm = true` env so the
/// refusal-to-prompt path has somewhere to happen.
const PLACES_WITH_UNKNOWN_KEY: &str = r#"
[owner]
type = "group"
id = 1234567

[dev]
universe_id = 100
notakey = "warn about me"
[dev.places]
main = 1001
lobby = 1002

[prod]
universe_id = 5544332211
confirm = true
[prod.places]
main = 55443322110099
"#;

fn places_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("rbxplace.toml");
    std::fs::write(&path, PLACES_WITH_UNKNOWN_KEY).unwrap();
    path
}

/// A stand-in for a built place file. The contents never matter: every upload
/// endpoint is mocked, and the bytes only have to survive the round trip.
fn rbxl_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("build.rbxl");
    std::fs::write(&path, b"not really a place file").unwrap();
    path
}

/// Run `rbx`, require success, and return `(parsed stdout, stderr)`.
///
/// Parsing is the assertion: anything printed alongside the document makes
/// `from_slice` fail, which is the whole contract under test.
fn run_json(args: &[&str]) -> (serde_json::Value, String) {
    let (stdout, stderr) = run(args, true);
    (parse(&stdout), stderr)
}

/// The same, for a run that must fail. Returns raw stdout, because half these
/// cases are about stdout being empty.
fn run_failing(args: &[&str]) -> (Vec<u8>, String) {
    run(args, false)
}

fn run(args: &[&str], success: bool) -> (Vec<u8>, String) {
    let assertion = Command::cargo_bin("rbx").unwrap().args(args).assert();
    let assertion = if success {
        assertion.success()
    } else {
        assertion.failure()
    };
    let output = assertion.get_output().clone();
    (
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn parse(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_slice(stdout).unwrap_or_else(|e| {
        panic!(
            "stdout must be one JSON document and nothing else ({e}). It was:\n{}",
            String::from_utf8_lossy(stdout)
        )
    })
}

/// Two versions of the dev main place, newest first, one live and one draft.
async fn versions_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/assets/v1/assets/{DEV_MAIN}/versions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "assetVersions": [
                {
                    "path": format!("assets/{DEV_MAIN}/versions/5"),
                    "createTime": "2024-01-05T00:00:00Z",
                    "published": true
                },
                {
                    "path": format!("assets/{DEV_MAIN}/versions/4"),
                    "createTime": "2024-01-04T00:00:00Z",
                    "published": false
                }
            ],
            "nextPageToken": ""
        })))
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread")]
async fn versions_emits_a_document_on_stdout_and_its_warning_on_stderr() {
    let server = versions_server().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();

    let (doc, stderr) = run_json(&[
        "--places",
        places.to_str().unwrap(),
        "--api-key",
        "test-key",
        "place",
        "versions",
        "--env",
        "dev",
        "--place",
        "main",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["env"], "dev");
    assert_eq!(doc["place"], "main");
    assert_eq!(doc["place_id"], "1001");
    assert_eq!(doc["filter"], "all");
    assert_eq!(doc["count_reached"], false);
    assert_eq!(doc["versions"][0]["version"], "5");
    assert_eq!(doc["versions"][0]["published"], true);
    // The API timestamp, not the listing's "2024-01-05 00:00 UTC" rendering.
    assert_eq!(doc["versions"][0]["create_time"], "2024-01-05T00:00:00Z");
    assert_eq!(doc["versions"][1]["published"], false);
    // The unknown key was reported, and reported where it cannot corrupt the
    // document. If this moved to stdout the parse above would have failed.
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

/// The human form is the default and is untouched by any of this: the header
/// line is byte for byte what it printed before `--json` existed.
#[tokio::test(flavor = "multi_thread")]
async fn without_the_flag_versions_still_prints_the_same_listing() {
    let server = versions_server().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();

    let (stdout, _) = run(
        &[
            "--places",
            places.to_str().unwrap(),
            "--api-key",
            "test-key",
            "place",
            "versions",
            "--env",
            "dev",
            "--place",
            "main",
            "--base-url",
            &uri,
        ],
        true,
    );

    let stdout = String::from_utf8(stdout).unwrap();
    assert!(
        stdout.starts_with("Versions for dev/main (1001) ... 2 found"),
        "stdout was:\n{stdout}"
    );
    assert!(stdout.contains("v5"), "stdout was:\n{stdout}");
    assert!(stdout.contains("[published]"), "stdout was:\n{stdout}");
    assert!(
        stdout.contains("2024-01-04 00:00:00 UTC"),
        "stdout was:\n{stdout}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn places_reports_which_places_the_toml_knows_about() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/universes/{DEV_UNIVERSE}/places")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                { "id": DEV_MAIN, "name": "Main" },
                { "id": 9_999, "name": "Never Added" }
            ]
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();

    let (doc, stderr) = run_json(&[
        "--places",
        places.to_str().unwrap(),
        "place",
        "places",
        "--env",
        "dev",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["env"], "dev");
    assert_eq!(doc["universe_id"], "100");
    assert_eq!(doc["places"][0]["place_id"], "1001");
    assert_eq!(doc["places"][0]["display_name"], "Main");
    assert_eq!(doc["places"][0]["place"], "main");
    assert_eq!(doc["places"][0]["configured"], true);
    // The case the human listing marks `NOT in toml`.
    assert_eq!(doc["places"][1]["configured"], false);
    assert!(doc["places"][1].get("place").is_none(), "{doc}");
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_reports_the_version_roblox_assigned() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/universes/v1/{DEV_UNIVERSE}/places/{DEV_MAIN}/versions"
        )))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "versionNumber": 42 })),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let file = rbxl_file(&dir);
    let uri = server.uri();

    let (doc, stderr) = run_json(&[
        "--places",
        places.to_str().unwrap(),
        "--api-key",
        "test-key",
        "place",
        "upload",
        "--env",
        "dev",
        "--place",
        "main",
        "--file",
        file.to_str().unwrap(),
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["command"], "upload");
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["env"], "dev");
    assert_eq!(doc["universe_id"], "100");
    assert_eq!(doc["published"], false);
    assert_eq!(doc["place_id"], "1001");
    assert_eq!(doc["version"], "42");
    assert_eq!(doc["results"].as_array().map(Vec::len), Some(1));
    assert_eq!(doc["results"][0]["place"], "main");
    assert!(doc.get("error").is_none(), "{doc}");
    // The progress lines the human form prints did not go to stdout; the
    // unknown-key warning still had somewhere to go.
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

/// The case the write envelope is shaped around. Two places, the second one
/// locked by Team Create: the first upload happened and cannot be taken back,
/// so the receipt goes out reporting it, while the process still fails.
#[tokio::test(flavor = "multi_thread")]
async fn a_half_finished_all_places_upload_still_reports_what_landed() {
    let server = MockServer::start().await;
    // Targets are uploaded in name order, so `lobby` succeeds before `main`
    // fails.
    Mock::given(method("POST"))
        .and(path(format!(
            "/universes/v1/{DEV_UNIVERSE}/places/{DEV_LOBBY}/versions"
        )))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "versionNumber": 7 })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/universes/v1/{DEV_UNIVERSE}/places/{DEV_MAIN}/versions"
        )))
        .respond_with(ResponseTemplate::new(409).set_body_string("conflict"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let file = rbxl_file(&dir);
    let uri = server.uri();

    let (stdout, stderr) = run_failing(&[
        "--places",
        places.to_str().unwrap(),
        "--api-key",
        "test-key",
        "place",
        "upload",
        "--env",
        "dev",
        "--all-places",
        "--published",
        "--file",
        file.to_str().unwrap(),
        "--base-url",
        &uri,
        "--json",
    ]);

    let doc = parse(&stdout);
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["published"], true);
    assert_eq!(doc["results"].as_array().map(Vec::len), Some(1));
    assert_eq!(doc["results"][0]["place"], "lobby");
    assert_eq!(doc["results"][0]["place_id"], "1002");
    assert_eq!(doc["results"][0]["version"], "7");
    // `--all-places` never fills the single-target shortcut, however many
    // places actually got written.
    assert!(doc.get("version").is_none(), "{doc}");
    assert!(
        doc["error"]
            .as_str()
            .is_some_and(|e| e.contains("Team Create")),
        "{doc}"
    );
    // The failure is on stderr too, where the process's error message belongs.
    assert!(stderr.contains("Team Create"), "stderr was:\n{stderr}");
}

/// Promote is the command the issue names: the version number the target
/// received is the thing a pipeline cannot compute for itself.
#[tokio::test(flavor = "multi_thread")]
async fn promote_carries_both_ends_of_the_move_and_notes_its_log_on_stderr() {
    let server = MockServer::start().await;
    let uri = server.uri();
    Mock::given(method("GET"))
        .and(path(format!(
            "/asset-delivery-api/v1/assetId/{DEV_MAIN}/version/5"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "location": format!("{uri}/cdn/place.rbxl")
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/cdn/place.rbxl"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"place bytes".to_vec()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/universes/v1/{PROD_UNIVERSE}/places/{PROD_MAIN}/versions"
        )))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "versionNumber": 27 })),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let log = dir.path().join("deploy.json");

    let (doc, stderr) = run_json(&[
        "--places",
        places.to_str().unwrap(),
        "--api-key",
        "test-key",
        "place",
        "promote",
        "--from",
        "dev",
        "--to",
        "prod",
        "--place",
        "main",
        "--version",
        "5",
        "--published",
        "--log",
        log.to_str().unwrap(),
        "--yes",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["command"], "promote");
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["from_env"], "dev");
    assert_eq!(doc["env"], "prod");
    assert_eq!(doc["source_place"], "main");
    assert_eq!(doc["source_place_id"], "1001");
    assert_eq!(doc["source_version"], "5");
    assert_eq!(doc["universe_id"], "5544332211");
    assert_eq!(doc["place_id"], "55443322110099");
    assert_eq!(doc["version"], "27");
    assert_eq!(doc["published"], true);
    // The `--log` notice is a human line, so under `--json` it goes to stderr.
    // On stdout it would have broken the parse above.
    assert!(stderr.contains("Log written"), "stderr was:\n{stderr}");
    assert!(log.exists(), "the log itself is still written");
    // Ids stay strings so a 64-bit place id is never handed to a consumer that
    // would round it.
    assert!(doc["place_id"].is_string(), "{doc}");
    assert!(doc["version"].is_string(), "{doc}");
}

/// `--json` cannot prompt, and a rollback without `--version` is a prompt. The
/// refusal happens before anything is written, so stdout stays empty rather
/// than carrying a receipt for a run that never started.
#[test]
fn rollback_refuses_to_pick_a_version_instead_of_prompting() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);

    let (stdout, stderr) = run_failing(&[
        "--places",
        places.to_str().unwrap(),
        "--api-key",
        "test-key",
        "place",
        "rollback",
        "--env",
        "dev",
        "--place",
        "main",
        "--json",
    ]);

    assert!(
        stdout.is_empty(),
        "stdout was:\n{}",
        String::from_utf8_lossy(&stdout)
    );
    assert!(stderr.contains("--version"), "stderr was:\n{stderr}");
}

/// Same rule for the confirmation gate: `prod` has `confirm = true`, so the
/// only way through under `--json` is `--yes`.
#[test]
fn upload_to_a_confirming_env_names_the_flag_that_answers_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let file = rbxl_file(&dir);

    let (stdout, stderr) = run_failing(&[
        "--places",
        places.to_str().unwrap(),
        "--api-key",
        "test-key",
        "place",
        "upload",
        "--env",
        "prod",
        "--file",
        file.to_str().unwrap(),
        "--json",
    ]);

    assert!(
        stdout.is_empty(),
        "stdout was:\n{}",
        String::from_utf8_lossy(&stdout)
    );
    assert!(stderr.contains("--yes"), "stderr was:\n{stderr}");
}

/// A plural `--env` puts a different document on stdout: one receipt per env,
/// wrapped. Worth running through the binary rather than only through the
/// struct, because the fan-out prints an `env:` header per env and that header
/// must not reach stdout under `--json`.
#[tokio::test(flavor = "multi_thread")]
async fn upload_to_every_env_emits_one_receipt_per_env() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/universes/v1/{DEV_UNIVERSE}/places/{DEV_MAIN}/versions"
        )))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "versionNumber": 42 })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/universes/v1/{PROD_UNIVERSE}/places/{PROD_MAIN}/versions"
        )))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "versionNumber": 8 })),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let file = rbxl_file(&dir);
    let uri = server.uri();

    // `--place main` because dev declares two places and `--env all` resolves
    // the name inside each env. `--yes` because prod has `confirm = true`, and
    // one gated env gates the whole run.
    let (doc, stderr) = run_json(&[
        "--places",
        places.to_str().unwrap(),
        "--api-key",
        "test-key",
        "place",
        "upload",
        "--env",
        "all",
        "--place",
        "main",
        "--file",
        file.to_str().unwrap(),
        "--base-url",
        &uri,
        "--yes",
        "--json",
    ]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["command"], "upload");
    assert_eq!(doc["ok"], true);
    // Alphabetical, which is the order `--env all` expands in.
    assert_eq!(doc["results"].as_array().map(Vec::len), Some(2));
    assert_eq!(doc["results"][0]["env"], "dev");
    assert_eq!(doc["results"][0]["universe_id"], "100");
    assert_eq!(doc["results"][0]["version"], "42");
    assert_eq!(doc["results"][1]["env"], "prod");
    assert_eq!(doc["results"][1]["universe_id"], "5544332211");
    assert_eq!(doc["results"][1]["version"], "8");
    // Each entry is a whole receipt, so an existing consumer reads any element
    // of the array without changes.
    assert_eq!(doc["results"][0]["command"], "upload");
    assert_eq!(doc["results"][0]["place_id"], "1001");
    assert_eq!(doc["results"][0]["results"][0]["place"], "main");
    // The envelope itself carries no `env`: there is no single one to name.
    assert!(doc.get("env").is_none(), "{doc}");
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

/// The fan-out case of the rule the write envelope is shaped around. `dev`
/// landed and `prod` is locked, so the run fails with `dev`'s version intact
/// rather than losing a write that happened.
#[tokio::test(flavor = "multi_thread")]
async fn a_fan_out_that_fails_on_the_second_env_still_reports_the_first() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/universes/v1/{DEV_UNIVERSE}/places/{DEV_MAIN}/versions"
        )))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "versionNumber": 42 })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/universes/v1/{PROD_UNIVERSE}/places/{PROD_MAIN}/versions"
        )))
        .respond_with(ResponseTemplate::new(409).set_body_string("conflict"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let file = rbxl_file(&dir);
    let uri = server.uri();

    let (stdout, stderr) = run_failing(&[
        "--places",
        places.to_str().unwrap(),
        "--api-key",
        "test-key",
        "place",
        "upload",
        "--env",
        "all",
        "--place",
        "main",
        "--file",
        file.to_str().unwrap(),
        "--base-url",
        &uri,
        "--yes",
        "--json",
    ]);

    let doc = parse(&stdout);
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["results"].as_array().map(Vec::len), Some(2));
    // The env that landed keeps its version and its own verdict.
    assert_eq!(doc["results"][0]["env"], "dev");
    assert_eq!(doc["results"][0]["ok"], true);
    assert_eq!(doc["results"][0]["version"], "42");
    // The env that failed says why, and has nothing to show.
    assert_eq!(doc["results"][1]["env"], "prod");
    assert_eq!(doc["results"][1]["ok"], false);
    assert!(
        doc["results"][1]["error"]
            .as_str()
            .is_some_and(|e| e.contains("Team Create")),
        "{doc}"
    );
    assert_eq!(
        doc["results"][1]["results"].as_array().map(Vec::len),
        Some(0)
    );
    assert!(stderr.contains("Team Create"), "stderr was:\n{stderr}");
}
