//! `--json` on the ops read commands, against the real binary.
//!
//! `servers versions`, `servers logs`, `ads list`, `ads get`, `ads status`.
//!
//! The unit tests in `rbx_servers::json` and `rbx_ads::json` pin what the
//! documents *say*. They cannot pin what else reaches stdout, because a stray
//! `println!` three layers down is invisible to a test that renders a struct
//! into a buffer, and a stray `println!` is exactly the failure that breaks
//! `jq` in somebody's pipeline. So these run the binary and parse its stdout.
//!
//! Every case is arranged to have something to say on stderr as well: the
//! `rbxplace.toml` these run against carries an unknown key, so `PlacesFile`
//! warns on every command that reads it. A warning with nowhere safe to go is
//! how stdout gets polluted.
//!
//! Deliberately its own file rather than an addition to `json_output.rs`: the
//! `--json` work is split across parallel branches, and one file per lot is
//! what keeps them from colliding in the same twenty lines. `run_json` is
//! duplicated for the same reason.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 5544332211;
const PLACE: u64 = 55443322110099;
const JOB: &str = "aba9aeae-bc55-49c8-bb0e-6363ee6ba820";

/// An `rbxplace.toml` with an unknown key in it, so `PlacesFile::load` has a
/// warning to emit on every command that reads the file.
const PLACES_WITH_UNKNOWN_KEY: &str = r#"
[owner]
type = "group"
id = 1234567

[prod]
universe_id = 5544332211
notakey = "warn about me"
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

/// The two versions `filter-options` reports for the mock experience.
fn versions_body() -> serde_json::Value {
    serde_json::json!({
        "filters": { "PlaceVersion": { "values": [3982, 3991] } }
    })
}

fn logs_body() -> serde_json::Value {
    serde_json::json!({
        "gameServerLogs": [
            {
                "messageTimestampMs": "2025-08-14T13:46:53.481Z",
                "severity": 3,
                "jobId": JOB,
                "placeVersion": "3982",
                "message": "attempt to index nil with 'Name'",
                "stackTrace": "Stack Begin\nScript 'X', Line 130\nStack End"
            },
            {
                "messageTimestampMs": "2025-08-14T13:47:01.002Z",
                "severity": 0,
                "jobId": JOB,
                "placeVersion": "3982",
                "message": "player joined"
            }
        ],
        "nextPageToken": ""
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn servers_versions_names_the_default_and_keeps_stdout_to_the_document() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/game-servers:filter-options"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(versions_body()))
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
        "versions",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["schema_version"], 1);
    // Newest first, and the default named rather than left to be inferred from
    // the order.
    assert_eq!(doc["place_versions"][0], "3991");
    assert_eq!(doc["place_versions"][1], "3982");
    assert_eq!(doc["default_place_version"], "3991");
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

/// Nothing has run in thirty days. Not an error, and under `--json` not silence
/// either: an empty document, with the explanation on stderr where it cannot
/// corrupt it.
#[tokio::test(flavor = "multi_thread")]
async fn servers_versions_emits_an_empty_document_when_nothing_has_run() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/game-servers:filter-options"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"filters": {}})))
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
        "versions",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["place_versions"].as_array().map(Vec::len), Some(0));
    assert!(doc.get("default_place_version").is_none(), "{doc}");
    assert!(
        stderr.contains("No place version has servers"),
        "stderr was:\n{stderr}"
    );
}

/// The human listing is the default and is unchanged: the `*` marker on the
/// newest version is what `--version` defaults to, and a table is not a
/// document.
#[tokio::test(flavor = "multi_thread")]
async fn servers_versions_without_the_flag_still_prints_the_marked_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/game-servers:filter-options"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(versions_body()))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();

    let assertion = Command::cargo_bin("rbx")
        .unwrap()
        .args([
            "--places",
            places.to_str().unwrap(),
            "--api-key",
            "test-key",
            "--env",
            "prod",
            "servers",
            "versions",
            "--base-url",
            &uri,
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("place versions with servers"), "{stdout}");
    assert!(stdout.contains("* 3991"), "{stdout}");
    assert!(!stdout.contains("schema_version"), "{stdout}");
}

/// One document, not one object per line. The envelope carries what the run
/// was, the lines carry what the server said, and the stack trace survives with
/// its newlines intact.
#[tokio::test(flavor = "multi_thread")]
async fn servers_logs_emit_one_document_carrying_the_run_and_the_lines() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/versions/3982\
             /game-servers/{JOB}/logs"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(logs_body()))
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
        "logs",
        JOB,
        "--version",
        "3982",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["job_id"], JOB);
    assert_eq!(doc["place_version"], "3982");
    assert_eq!(doc["limit"], 200);
    assert_eq!(doc["limit_reached"], false);
    assert!(doc.get("severity_filter").is_none(), "{doc}");
    assert_eq!(doc["totals"]["returned"], 2);
    assert_eq!(doc["totals"]["errors"], 1);
    assert_eq!(doc["lines"][0]["severity"], "error");
    assert_eq!(doc["lines"][0]["severity_code"], 3);
    assert_eq!(doc["lines"][0]["error"], true);
    assert_eq!(doc["lines"][0]["time"], "2025-08-14T13:46:53.481Z");
    assert_eq!(
        doc["lines"][0]["stack_trace"],
        "Stack Begin\nScript 'X', Line 130\nStack End"
    );
    assert_eq!(doc["lines"][1]["severity"], "output");
    assert_eq!(doc["lines"][1]["error"], false);
    assert!(doc["lines"][1].get("stack_trace").is_none(), "{doc}");
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

/// The filter is part of the run, so it is in the envelope, in the canonical
/// spelling rather than whatever case was typed.
#[tokio::test(flavor = "multi_thread")]
async fn servers_logs_report_the_severity_filter_they_applied() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/versions/3982\
             /game-servers/{JOB}/logs"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(logs_body()))
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
        "logs",
        JOB,
        "--version",
        "3982",
        "--severity",
        "ERROR",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["severity_filter"], "error");
    assert_eq!(doc["totals"]["returned"], 1);
    assert_eq!(doc["lines"].as_array().map(Vec::len), Some(1));
    assert_eq!(doc["lines"][0]["severity"], "error");
}

/// A job id from the wrong version returns nothing and no error. The advice
/// that says so is worth keeping under `--json`, so it goes to stderr and the
/// document still names what was asked about.
#[tokio::test(flavor = "multi_thread")]
async fn servers_logs_emit_an_empty_document_and_the_advice_on_stderr() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/versions/3991\
             /game-servers/{JOB}/logs"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "gameServerLogs": [],
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
        "logs",
        JOB,
        "--version",
        "3991",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["lines"].as_array().map(Vec::len), Some(0));
    assert_eq!(doc["totals"]["returned"], 0);
    assert_eq!(doc["job_id"], JOB);
    assert_eq!(doc["place_version"], "3991");
    assert!(
        stderr.contains("No logs for that server"),
        "stderr was:\n{stderr}"
    );
}

/// Two answers to the same question. Asking for both is a mistake rather than a
/// precedence question.
#[test]
fn servers_logs_refuse_csv_and_json_together() {
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["servers", "logs", JOB, "--csv", "--json"])
        .assert()
        .failure();
}

#[tokio::test(flavor = "multi_thread")]
async fn ads_list_emits_a_document_with_budgets_as_strings() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ads-management/v1/campaigns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "campaigns": [
                {
                    "id": "c1",
                    "name": "icon test [18234567890]",
                    "status": "ACTIVE",
                    "deliveryStatus": "SERVING",
                    "targetUniverseId": UNIVERSE.to_string(),
                    "creativeAssetIds": ["18234567890"],
                    "budget": { "amountMicros": "25500000", "type": "DAILY" }
                },
                {
                    "id": "c2",
                    "name": "icon test [18234567891]",
                    "status": "PAUSED",
                    "deliveryStatus": "REJECTED",
                    "deliveryStatusReasons": ["policy"],
                    "targetUniverseId": UNIVERSE.to_string(),
                    "creativeAssetIds": ["18234567891"]
                }
            ],
            "nextPageToken": ""
        })))
        .mount(&server)
        .await;

    let uri = server.uri();
    let (doc, _) = run_json(&[
        "--api-key",
        "test-key",
        "ads",
        "list",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["totals"]["returned"], 2);
    assert_eq!(doc["totals"]["active"], 1);
    assert_eq!(doc["campaigns"][0]["id"], "c1");
    assert_eq!(doc["campaigns"][0]["delivery_status"], "SERVING");
    // Money never arrives as a JSON number, in either form.
    assert!(
        doc["campaigns"][0]["budget"]["amount_micros"].is_string(),
        "{doc}"
    );
    assert_eq!(doc["campaigns"][0]["budget"]["amount_micros"], "25500000");
    assert_eq!(doc["campaigns"][0]["budget"]["amount_usd"], "25.50");
    assert_eq!(doc["campaigns"][0]["budget"]["type"], "DAILY");
    // No budget reported is an absent key, not a zero.
    assert!(doc["campaigns"][1].get("budget").is_none(), "{doc}");
    assert_eq!(doc["campaigns"][1]["delivery_status_reasons"][0], "policy");
}

/// An account with no campaigns is a document a consumer can read a zero off,
/// and the sentence saying so is on stderr.
#[tokio::test(flavor = "multi_thread")]
async fn ads_list_emits_an_empty_document_for_an_empty_account() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ads-management/v1/campaigns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "campaigns": [],
            "nextPageToken": ""
        })))
        .mount(&server)
        .await;

    let uri = server.uri();
    let (doc, stderr) = run_json(&[
        "--api-key",
        "test-key",
        "ads",
        "list",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["campaigns"].as_array().map(Vec::len), Some(0));
    assert_eq!(doc["totals"]["returned"], 0);
    assert!(
        stderr.contains("No campaigns on this account"),
        "stderr was:\n{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ads_get_wraps_the_campaign_under_a_named_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ads-management/v1/campaigns/c1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "c1",
            "name": "icon test [18234567890]",
            "status": "ACTIVE",
            "deliveryStatus": "IN_REVIEW",
            "deliveryStatusReasons": ["queued for ad-policy review"],
            "targetUniverseId": UNIVERSE.to_string(),
            "creativeAssetIds": ["18234567890"],
            "budget": { "amountMicros": "25000000", "type": "LIFETIME" }
        })))
        .mount(&server)
        .await;

    let uri = server.uri();
    let (doc, _) = run_json(&[
        "--api-key",
        "test-key",
        "ads",
        "get",
        "c1",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["campaign"]["id"], "c1");
    assert_eq!(doc["campaign"]["status"], "ACTIVE");
    assert_eq!(doc["campaign"]["delivery_status"], "IN_REVIEW");
    assert_eq!(doc["campaign"]["budget"]["amount_usd"], "25.00");
    assert_eq!(doc["campaign"]["creative_asset_ids"][0], "18234567890");
}

/// Roblox answers `200` with the ids it could read and the ids it could not.
/// Keeping them in separate arrays is what stops a script reading "never
/// answered" as "not serving".
#[tokio::test(flavor = "multi_thread")]
async fn ads_status_keeps_answered_ids_apart_from_refused_ones() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ads-management/v1/campaigns:batchGetStatus"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "statuses": [
                { "id": "c1", "status": "ACTIVE", "deliveryStatus": "SERVING" },
                {
                    "id": "c2",
                    "status": "ACTIVE",
                    "deliveryStatus": "REJECTED",
                    "deliveryStatusReasons": ["creative violates policy"]
                }
            ],
            "failures": [{ "id": "c3", "reason": "campaign not found" }]
        })))
        .mount(&server)
        .await;

    let uri = server.uri();
    let (doc, _) = run_json(&[
        "--api-key",
        "test-key",
        "ads",
        "status",
        "c1",
        "c2",
        "c3",
        "--base-url",
        &uri,
        "--json",
    ]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["totals"]["requested"], 3);
    assert_eq!(doc["totals"]["returned"], 2);
    assert_eq!(doc["totals"]["failed"], 1);
    assert_eq!(doc["statuses"][0]["id"], "c1");
    assert_eq!(doc["statuses"][1]["delivery_status"], "REJECTED");
    assert_eq!(
        doc["statuses"][1]["delivery_status_reasons"][0],
        "creative violates policy"
    );
    assert_eq!(doc["failures"][0]["id"], "c3");
    assert_eq!(doc["failures"][0]["reason"], "campaign not found");
}
