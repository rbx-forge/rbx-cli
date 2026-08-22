//! `--json` against the real binary: stdout carries the document and nothing
//! else.
//!
//! The unit tests in `rbx_env::json` and `rbx_servers::json` pin what the
//! documents *say*. They cannot pin what else reaches stdout, because a stray
//! `println!` three layers down is invisible to a test that renders a struct
//! into a buffer, and a stray `println!` is exactly the failure that breaks
//! `jq` in somebody's pipeline. So these run the binary and parse its stdout.
//!
//! Each case is arranged to have something to say on stderr as well: an
//! unknown key in `rbxplace.toml` for `env`, a partial page for `servers`. A
//! warning that has nowhere safe to go is how stdout gets polluted.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 5544332211;
const PLACE: u64 = 55443322110099;

/// An `rbxplace.toml` with an unknown key in it, so `PlacesFile::load` has a
/// warning to emit on every command that reads the file.
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
env = "production"
confirm = true
owner = { type = "user", id = 42 }
[prod.places]
main = 55443322110099
"#;

fn places_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("rbxplace.toml");
    std::fs::write(&path, PLACES_WITH_UNKNOWN_KEY).unwrap();
    path
}

/// Run `rbx`, require success, and return `(parsed stdout, stderr)`.
///
/// Parsing is the assertion: anything printed alongside the document makes
/// `from_slice` fail, which is the whole contract under test.
fn run_json(args: &[&str]) -> (serde_json::Value, String) {
    let output = Command::cargo_bin("rbx")
        .unwrap()
        .args(args)
        .assert()
        .success()
        .get_output()
        .clone();

    let stdout = output.stdout.clone();
    let document: serde_json::Value = serde_json::from_slice(&stdout).unwrap_or_else(|e| {
        panic!(
            "stdout must be one JSON document and nothing else ({e}). It was:\n{}",
            String::from_utf8_lossy(&stdout)
        )
    });
    (
        document,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn env_list_emits_a_document_on_stdout_and_its_warning_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);

    let (doc, stderr) = run_json(&[
        "--places",
        places.to_str().unwrap(),
        "env",
        "list",
        "--json",
    ]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["owner"]["type"], "group");
    assert_eq!(doc["envs"][0]["name"], "dev");
    assert_eq!(doc["envs"][0]["places"]["main"], 1001);
    assert_eq!(doc["envs"][1]["name"], "prod");
    assert_eq!(doc["envs"][1]["universe_id"], UNIVERSE);
    // The unknown key was reported, and reported where it cannot corrupt the
    // document. If this moved to stdout the parse above would have failed.
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

#[test]
fn env_list_narrows_to_one_env_without_changing_the_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);

    let (doc, _) = run_json(&[
        "--places",
        places.to_str().unwrap(),
        "--env",
        "prod",
        "env",
        "list",
        "--json",
    ]);

    assert_eq!(doc["envs"].as_array().map(Vec::len), Some(1));
    assert_eq!(doc["envs"][0]["name"], "prod");
    assert_eq!(doc["envs"][0]["env"], "production");
    assert_eq!(doc["envs"][0]["confirm"], true);
    assert_eq!(doc["envs"][0]["owner"]["id"], 42);
}

#[test]
fn env_get_emits_a_document_on_stdout_and_its_warning_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);

    let (doc, stderr) = run_json(&[
        "--places",
        places.to_str().unwrap(),
        "--env",
        "dev",
        "env",
        "get",
        "universe-id",
        "--json",
    ]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["field"], "universe-id");
    assert_eq!(doc["value"], "100");
    assert_eq!(doc["results"][0]["env"], "dev");
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

#[test]
fn env_get_across_every_env_fills_results_instead_of_value() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);

    let (doc, _) = run_json(&[
        "--places",
        places.to_str().unwrap(),
        "--env",
        "all",
        "env",
        "get",
        "universe-id",
        "--json",
    ]);

    assert!(doc.get("value").is_none(), "{doc}");
    assert_eq!(doc["results"][0]["env"], "dev");
    assert_eq!(doc["results"][0]["value"], "100");
    assert_eq!(doc["results"][1]["env"], "prod");
}

/// The human forms are the default and are untouched by any of this.
#[test]
fn without_the_flag_the_output_is_still_the_bare_value() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);

    let assertion = Command::cargo_bin("rbx")
        .unwrap()
        .args([
            "--places",
            places.to_str().unwrap(),
            "--env",
            "dev",
            "env",
            "get",
            "universe-id",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
    assert_eq!(stdout.trim_end(), "100");
}

/// `--names` already answers "every env name"; the JSON document answers it
/// too, so asking for both is a mistake rather than a precedence question.
#[test]
fn env_list_refuses_names_and_json_together() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);

    Command::cargo_bin("rbx")
        .unwrap()
        .args([
            "--places",
            places.to_str().unwrap(),
            "env",
            "list",
            "--names",
            "--json",
        ])
        .assert()
        .failure();
}

/// A page Roblox admits is incomplete. The warning belongs on stderr and the
/// fact belongs in the document, and this is the case where getting that wrong
/// would silently corrupt a monitoring script's input.
#[tokio::test(flavor = "multi_thread")]
async fn servers_list_keeps_stdout_parsable_when_the_page_is_partial() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/versions/3991/game-servers"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "gameServers": [
                {
                    "jobId": "9f2a1c00-0000-4000-8000-000000000001",
                    "status": "crashed",
                    "placeId": PLACE.to_string(),
                    "placeVersion": "3991",
                    "uptime": "00:05:02.0020000",
                    "memoryUsageBytes": 1_048_576,
                    "frameRate": 0,
                    "occupancy": 3,
                    "maxOccupancy": 50,
                    "playerIds": [1, 2, 3]
                }
            ],
            "totalCount": 83_711,
            "shutdownServersFetchError": true,
            "nextPageToken": ""
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();

    let (doc, stderr) = run_json(&[
        "--places",
        places.to_str().unwrap(),
        "--api-key",
        "test-key",
        "--env",
        "prod",
        "servers",
        "list",
        "--version",
        "3991",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["place_version"], "3991");
    assert_eq!(doc["partial"], true);
    assert_eq!(doc["totals"]["returned"], 1);
    assert_eq!(doc["totals"]["failed"], 1);
    assert_eq!(doc["totals"]["available"], 83_711);
    assert_eq!(doc["servers"][0]["status"], "crashed");
    assert_eq!(doc["servers"][0]["failure"], true);
    assert_eq!(doc["servers"][0]["uptime_seconds"], 302);
    assert_eq!(doc["servers"][0]["player_count"], 3);
    // Reported, not silently dropped, and not on stdout.
    assert!(
        stderr.contains("this page is incomplete"),
        "stderr was:\n{stderr}"
    );
    // The unknown-key warning had to go somewhere too.
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

/// Nothing has run in thirty days. Not an error, and under `--json` not
/// silence either: an empty document a consumer can read a zero off.
#[tokio::test(flavor = "multi_thread")]
async fn servers_list_emits_an_empty_document_when_no_version_has_servers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/game-servers:filter-options"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "filters": {}
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();

    let (doc, _) = run_json(&[
        "--places",
        places.to_str().unwrap(),
        "--api-key",
        "test-key",
        "--env",
        "prod",
        "servers",
        "list",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["totals"]["returned"], 0);
    assert_eq!(doc["servers"].as_array().map(Vec::len), Some(0));
    assert!(doc.get("place_version").is_none(), "{doc}");
}

#[test]
fn servers_list_refuses_csv_and_json_together() {
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["servers", "list", "--csv", "--json"])
        .assert()
        .failure();
}
