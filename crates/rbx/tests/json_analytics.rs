//! `analytics --json` against the real binary: stdout carries the document and
//! nothing else.
//!
//! The unit tests in `rbx_analytics::json` pin what the documents *say*. They
//! cannot pin what else reaches stdout, because a stray `println!` three layers
//! down is invisible to a test that renders a struct into a buffer, and a
//! stray `println!` is exactly the failure that breaks `jq` in somebody's
//! pipeline. `analytics query` has two of those to get right: the unknown-key
//! warning every command that reads `rbxplace.toml` can emit, and the "queued
//! by Roblox" note this command prints while it waits. So these run the binary
//! and parse its stdout.
//!
//! Deliberately a file of its own rather than more cases in `json_output.rs`:
//! the `--json` work lands as several parallel branches, and a shared test file
//! is where they would collide. `run_json` below is duplicated for the same
//! reason; it is ten lines.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 5544332211;

/// An `rbxplace.toml` with an unknown key in it, so `PlacesFile::load` has a
/// warning to emit on every command that reads the file. A warning that has
/// nowhere safe to go is how stdout gets polluted.
const PLACES_WITH_UNKNOWN_KEY: &str = r#"
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

/// The `--metric`/`--env`/`--api-key` boilerplate every query case repeats.
fn query_args<'a>(places: &'a str, base_url: &'a str) -> Vec<&'a str> {
    vec![
        "--places",
        places,
        "--api-key",
        "test-key",
        "--env",
        "prod",
        "analytics",
        "query",
        "--metric",
        "DailyActiveUsers",
        "--days",
        "7",
        "--base-url",
        base_url,
    ]
}

async fn mock_metrics(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path(format!(
            "/analytics-query-api/v1/universes/{UNIVERSE}/metrics"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// A completed query whose series has both kinds of hole in it: a bucket
/// Roblox returned with no number, and a day that never came back at all.
fn series_with_holes() -> serde_json::Value {
    serde_json::json!({
        "done": true,
        "response": { "values": [{
            "breakdowns": [],
            "dataPoints": [
                { "time": "2026-07-27T00:00:00+00:00", "value": 0 },
                { "time": "2026-07-28T00:00:00+00:00", "value": null },
                // 2026-07-29 is missing entirely.
                { "time": "2026-07-30T00:00:00+00:00", "value": 1288 }
            ]
        }]}
    })
}

/// The distinction the document exists to preserve, end to end: a measured
/// zero, a reported bucket with no value, and a bucket that never came back
/// are three different things and stay three different things.
#[tokio::test(flavor = "multi_thread")]
async fn query_keeps_a_measured_zero_apart_from_a_hole() {
    let server = MockServer::start().await;
    mock_metrics(&server, series_with_holes()).await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let (doc, stderr) = run_json(&{
        let mut args = query_args(places.to_str().unwrap(), &uri);
        args.push("--json");
        args
    });

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["metric"], "DailyActiveUsers");
    assert_eq!(doc["granularity"], "one-day");
    assert_eq!(doc["days"], 7);
    assert_eq!(doc["queued"], false);
    assert_eq!(doc["totals"]["series"], 1);
    assert_eq!(doc["totals"]["points"], 3);
    assert_eq!(doc["totals"]["missing"], 1);

    let points = &doc["series"][0]["points"];
    assert_eq!(doc["series"][0]["label"], "total");
    // Measured zero: a number, and zero.
    assert_eq!(points[0]["value"], 0.0);
    // Reported bucket, no number: the key is absent, not null.
    assert!(points[1].get("value").is_none(), "{points}");
    assert_eq!(points[1]["time"], "2026-07-28T00:00:00+00:00");
    // The missing day is missing, not invented.
    assert_eq!(points.as_array().map(Vec::len), Some(3));
    assert_eq!(points[2]["value"], 1288.0);

    // The unknown-key warning had to go somewhere. If it had gone to stdout
    // the parse above would have failed.
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

/// The range is echoed back because `--days` is relative to the moment the
/// command ran. Two stored documents cannot be lined up without it.
#[tokio::test(flavor = "multi_thread")]
async fn query_echoes_the_range_it_actually_asked_for() {
    let server = MockServer::start().await;
    mock_metrics(&server, series_with_holes()).await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let (doc, _) = run_json(&{
        let mut args = query_args(places.to_str().unwrap(), &uri);
        args.push("--json");
        args
    });

    let start = doc["start_time"].as_str().expect("a start time");
    let end = doc["end_time"].as_str().expect("an end time");
    assert!(start.ends_with('Z'), "{start}");
    assert!(end.ends_with('Z'), "{end}");
    assert!(start < end, "{start} .. {end}");
}

/// A breakdown makes several series, and their identity is a named object
/// rather than a column: `Phone` on its own does not say which dimension it
/// came from.
#[tokio::test(flavor = "multi_thread")]
async fn a_broken_down_query_keys_each_series_by_dimension_name() {
    let server = MockServer::start().await;
    mock_metrics(
        &server,
        serde_json::json!({
            "done": true,
            "response": { "values": [
                { "breakdowns": ["Phone"], "dataPoints": [
                    { "time": "2026-07-30T00:00:00+00:00", "value": 61000 }
                ]},
                { "breakdowns": ["Console"], "dataPoints": [] }
            ]}
        }),
    )
    .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let (doc, _) = run_json(&{
        let mut args = query_args(places.to_str().unwrap(), &uri);
        args.extend_from_slice(&["--breakdown", "Platform", "--json"]);
        args
    });

    assert_eq!(doc["breakdown"][0], "Platform");
    assert_eq!(doc["series"][0]["dimensions"]["Platform"], "Phone");
    assert_eq!(doc["series"][1]["dimensions"]["Platform"], "Console");
    // A series with nothing in it is an empty list, not an absent one.
    assert_eq!(doc["series"][1]["points"].as_array().map(Vec::len), Some(0));
    assert_eq!(doc["totals"]["series"], 2);
    assert_eq!(doc["totals"]["points"], 1);
}

/// The case this file exists for. Roblox queues the query, the command says so
/// while it waits, and under `--json` that note has to be on stderr, or the
/// document is unreadable on exactly the slow queries a pipeline runs.
#[tokio::test(flavor = "multi_thread")]
async fn a_queued_query_puts_its_waiting_note_on_stderr() {
    let server = MockServer::start().await;
    let poll_path = format!("v1/universes/{UNIVERSE}/operations/metrics/queued-1");
    mock_metrics(
        &server,
        serde_json::json!({ "done": false, "path": poll_path }),
    )
    .await;
    Mock::given(method("GET"))
        .and(path(format!("/analytics-query-api/{poll_path}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "done": true,
            "response": { "values": [{
                "breakdowns": [],
                "dataPoints": [{ "time": "2026-07-30T00:00:00+00:00", "value": 1288 }]
            }]}
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let (doc, stderr) = run_json(&{
        let mut args = query_args(places.to_str().unwrap(), &uri);
        args.push("--json");
        args
    });

    assert_eq!(doc["queued"], true);
    assert_eq!(doc["totals"]["points"], 1);
    // Said out loud, and said where it cannot corrupt the document.
    assert!(stderr.contains("queued"), "stderr was:\n{stderr}");
}

/// Nothing in the range is not an error and, under `--json`, not silence
/// either: an empty document a consumer can read a zero off.
#[tokio::test(flavor = "multi_thread")]
async fn an_empty_range_is_an_empty_document_rather_than_no_document() {
    let server = MockServer::start().await;
    mock_metrics(
        &server,
        serde_json::json!({
            "done": true,
            "response": { "values": [{ "breakdowns": [], "dataPoints": [] }] }
        }),
    )
    .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let (doc, stderr) = run_json(&{
        let mut args = query_args(places.to_str().unwrap(), &uri);
        args.push("--json");
        args
    });

    assert_eq!(doc["totals"]["points"], 0);
    assert_eq!(doc["series"].as_array().map(Vec::len), Some(1));
    assert!(stderr.contains("No data points"), "stderr was:\n{stderr}");
}

#[test]
fn metrics_emits_a_document_that_admits_it_is_not_a_whitelist() {
    let (doc, _) = run_json(&["analytics", "metrics", "--json"]);

    assert_eq!(doc["schema_version"], 1);
    // `--metric` forwards any string, so a consumer must not validate against
    // this list. The document says so rather than leaving it to the man page.
    assert_eq!(doc["exhaustive"], false);
    assert_eq!(doc["metrics"][0]["name"], "Visits");
    assert!(
        doc["metrics"].as_array().is_some_and(|m| m.len() >= 7),
        "{doc}"
    );
}

/// The JSON document already carries everything the CSV does, so asking for
/// both is a mistake rather than a question of which one wins. Same call the
/// `servers list` pilot made.
#[test]
fn query_refuses_csv_and_json_together() {
    Command::cargo_bin("rbx")
        .unwrap()
        .args([
            "analytics",
            "query",
            "--metric",
            "DailyActiveUsers",
            "--csv",
            "--json",
        ])
        .assert()
        .failure();
}

/// The human form is the default and is untouched by any of this, down to the
/// column widths and the `-` that marks a hole.
#[tokio::test(flavor = "multi_thread")]
async fn without_the_flag_the_output_is_still_the_table() {
    let server = MockServer::start().await;
    mock_metrics(&server, series_with_holes()).await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let assertion = Command::cargo_bin("rbx")
        .unwrap()
        .args(query_args(places.to_str().unwrap(), &uri))
        .assert()
        .success();

    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
    assert_eq!(
        stdout,
        "DailyActiveUsers (total)\n  \
         2026-07-27          0.00\n  \
         2026-07-28             -\n  \
         2026-07-30       1288.00\n\n"
    );
}
