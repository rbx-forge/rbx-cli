//! `--json` on the data store and memory store reads, against the real binary.
//!
//! The unit tests in `rbx_data::json` and `rbx_memorystore::json` pin what the
//! documents *say*. They cannot pin what else reaches stdout, and these
//! commands print a lot of it: a revision id, a count, an expiry, a "no entry"
//! line. A stray `println!` three layers down is invisible to a test that
//! renders a struct into a buffer, and is exactly what breaks `jq` in
//! somebody's pipeline. So these run the binary and parse its stdout.
//!
//! Deliberately its own file rather than an addition to `json_output.rs`: the
//! `--json` work is split across parallel branches, and one shared test file is
//! one shared conflict. `run_json` below is a copy of that file's, on purpose.
//!
//! Every case reads an `rbxplace.toml` with an unknown key in it, so
//! `PlacesFile::load` has a warning to emit on every run. A warning with
//! nowhere safe to go is how stdout gets polluted.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 5544332211;

/// An `rbxplace.toml` with an unknown key in it, so every command that reads
/// the file has something to say on stderr.
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

fn entries_path() -> String {
    format!("/cloud/v2/universes/{UNIVERSE}/data-stores/PlayerData/scopes/global/entries")
}

fn items_path() -> String {
    format!("/cloud/v2/universes/{UNIVERSE}/memory-store/sorted-maps/Cache/items")
}

/// The flags every `data` case here shares.
fn data_args<'a>(places: &'a str, uri: &'a str) -> Vec<&'a str> {
    vec![
        "--places",
        places,
        "--api-key",
        "test-key",
        "--env",
        "prod",
        "data",
        "--datastore",
        "PlayerData",
        "--base-url",
        uri,
    ]
}

fn memorystore_args<'a>(places: &'a str, uri: &'a str) -> Vec<&'a str> {
    vec![
        "--places",
        places,
        "--api-key",
        "test-key",
        "--env",
        "prod",
        "memorystore",
        "--map",
        "Cache",
        "--base-url",
        uri,
    ]
}

/// The arbitration the document is built on: the stored profile is nested as
/// JSON under `value`, so `jq` reaches into it, and the entry's `users` (the
/// association Roblox answers a player's data request from) is nowhere in the
/// output, exactly as in the human form.
#[tokio::test(flavor = "multi_thread")]
async fn data_get_nests_the_value_and_leaves_the_user_association_out() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{}/Player_156", entries_path())))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "Player_156",
            "revisionId": "08DEF1A1.0000000002.01",
            "state": "ACTIVE",
            "etag": "e1",
            "value": { "coins": 500, "items": ["hat"], "level": 3 },
            "users": ["users/156"],
            "attributes": { "tier": "gold" }
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let mut args = data_args(places.to_str().unwrap(), &uri);
    args.extend_from_slice(&["get", "Player_156", "--json"]);

    let (doc, stderr) = run_json(&args);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["datastore"], "PlayerData");
    assert_eq!(doc["scope"], "global");
    assert_eq!(doc["entry"], "Player_156");
    assert_eq!(doc["found"], true);
    assert_eq!(doc["deleted"], false);
    assert_eq!(doc["revision_id"], "08DEF1A1.0000000002.01");
    // Nested, not escaped: a number is still a number one level down.
    assert_eq!(doc["value"]["coins"], 500);
    assert_eq!(doc["value"]["items"][0], "hat");
    assert!(doc["value"]["level"].is_number(), "{doc}");

    let rendered = doc.to_string();
    for absent in ["users/156", "attributes", "etag"] {
        assert!(!rendered.contains(absent), "{absent} leaked:\n{rendered}");
    }
    // The revision line and the unknown-key warning both had to go somewhere,
    // and it was not stdout, or the parse above would have failed.
    assert!(stderr.contains("revision"), "stderr was:\n{stderr}");
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

/// A key that was never written is a non-event in both formats. Under `--json`
/// it is a document saying so rather than silence, so a script can tell "no
/// such key" from "the command printed nothing".
#[tokio::test(flavor = "multi_thread")]
async fn data_get_answers_a_missing_key_with_a_document_and_exit_zero() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{}/Player_999", entries_path())))
        .respond_with(ResponseTemplate::new(404).set_body_string("Entry not found"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let mut args = data_args(places.to_str().unwrap(), &uri);
    args.extend_from_slice(&["get", "Player_999", "--json"]);

    let (doc, stderr) = run_json(&args);

    assert_eq!(doc["found"], false);
    assert!(doc.get("value").is_none(), "{doc}");
    assert!(doc.get("revision_id").is_none(), "{doc}");
    // The "No entry" line is stdout in the human form and stderr here.
    assert!(stderr.contains("No entry"), "stderr was:\n{stderr}");
}

/// A soft-deleted entry still answers for thirty days. The document says which
/// of the two it is rather than letting "readable" imply "alive".
#[tokio::test(flavor = "multi_thread")]
async fn data_get_flags_a_soft_deleted_entry_that_still_reads() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{}/Player_156", entries_path())))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "state": "DELETED",
            "value": { "coins": 0 }
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let mut args = data_args(places.to_str().unwrap(), &uri);
    args.extend_from_slice(&["get", "Player_156", "--json"]);

    let (doc, stderr) = run_json(&args);

    assert_eq!(doc["found"], true);
    assert_eq!(doc["deleted"], true);
    assert!(stderr.contains("marked deleted"), "stderr was:\n{stderr}");
}

#[tokio::test(flavor = "multi_thread")]
async fn data_list_carries_the_keys_and_the_filter_that_produced_them() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(entries_path()))
        .and(query_param("maxPageSize", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "dataStoreEntries": [
                { "id": "Player_1", "path": "universes/1/x" },
                { "id": "Player_2", "path": "universes/1/y" }
            ],
            "nextPageToken": ""
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let mut args = data_args(places.to_str().unwrap(), &uri);
    args.extend_from_slice(&["list", "--prefix", "Player_", "--json"]);

    let (doc, stderr) = run_json(&args);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["prefix"], "Player_");
    assert_eq!(doc["show_deleted"], false);
    assert_eq!(doc["limit"], 100);
    assert_eq!(doc["limit_reached"], false);
    assert_eq!(doc["count"], 2);
    assert_eq!(doc["entries"][0]["id"], "Player_1");
    assert_eq!(doc["entries"][1]["id"], "Player_2");
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

/// A prefix matching nothing is an empty array and exit 0, not silence: the
/// human form's "No entries." moves to stderr and `.count` answers either way.
#[tokio::test(flavor = "multi_thread")]
async fn data_list_emits_an_empty_document_when_nothing_matches() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(entries_path()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "dataStoreEntries": [],
            "nextPageToken": ""
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let mut args = data_args(places.to_str().unwrap(), &uri);
    args.extend_from_slice(&["list", "--json"]);

    let (doc, stderr) = run_json(&args);

    assert_eq!(doc["count"], 0);
    assert_eq!(doc["entries"].as_array().map(Vec::len), Some(0));
    assert!(doc.get("prefix").is_none(), "{doc}");
    assert!(stderr.contains("No entries"), "stderr was:\n{stderr}");
}

/// Two documents from one subcommand, and `--revision` is what picks between
/// them: the listing without it, that revision's value with it.
#[tokio::test(flavor = "multi_thread")]
async fn data_revisions_lists_them_and_reads_one_by_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"/entries/Player_156:listRevisions$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "dataStoreEntries": [
                {
                    "id": "Player_156@r2",
                    "revisionId": "r2",
                    "state": "DELETED",
                    "revisionCreateTime": "2026-08-15T09:15:00.1234567Z"
                },
                {
                    "id": "Player_156@r1",
                    "revisionId": "r1",
                    "state": "ACTIVE",
                    "revisionCreateTime": "2026-08-14T11:02:33.0000000Z"
                }
            ],
            "nextPageToken": ""
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/entries/Player_156%40r1$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "revisionId": "r1",
            "value": { "coins": 12 }
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();

    let mut listing = data_args(places.to_str().unwrap(), &uri);
    listing.extend_from_slice(&["revisions", "Player_156", "--json"]);
    let (doc, _) = run_json(&listing);

    assert_eq!(doc["entry"], "Player_156");
    assert_eq!(doc["count"], 2);
    assert_eq!(doc["revisions"][0]["revision_id"], "r2");
    assert_eq!(doc["revisions"][0]["state"], "DELETED");
    assert_eq!(doc["revisions"][0]["deleted"], true);
    // Full precision, where the human table shortens to the second.
    assert_eq!(
        doc["revisions"][0]["create_time"],
        "2026-08-15T09:15:00.1234567Z"
    );
    assert_eq!(doc["revisions"][1]["deleted"], false);

    let mut one = data_args(places.to_str().unwrap(), &uri);
    one.extend_from_slice(&["revisions", "Player_156", "--revision", "r1", "--json"]);
    let (doc, _) = run_json(&one);

    assert_eq!(doc["revision_id"], "r1");
    assert_eq!(doc["value"]["coins"], 12);
    assert!(doc.get("revisions").is_none(), "{doc}");
}

/// `diff` reports where the two files went and nothing about what is in them.
/// The values are on disk, which is where the human form leaves them too.
#[tokio::test(flavor = "multi_thread")]
async fn data_diff_reports_two_paths_and_neither_value() {
    let server = MockServer::start().await;
    for (revision, coins) in [("r1", 1), ("r2", 2)] {
        Mock::given(method("GET"))
            .and(path_regex(format!(r"/entries/Player_156%40{revision}$")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "revisionId": revision,
                "value": { "coins": coins }
            })))
            .mount(&server)
            .await;
    }

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let mut args = data_args(places.to_str().unwrap(), &uri);
    args.extend_from_slice(&["diff", "Player_156", "--revisions", "r1,r2", "--json"]);

    let (doc, _) = run_json(&args);

    assert_eq!(doc["entry"], "Player_156");
    assert_eq!(doc["left"]["revision"], "r1");
    assert_eq!(doc["right"]["revision"], "r2");
    assert!(doc["left"].get("env").is_none(), "{doc}");
    assert!(!doc.to_string().contains("coins"), "{doc}");

    // The values did go somewhere: the paths the document names.
    let left = doc["left"]["path"].as_str().unwrap();
    let written = std::fs::read_to_string(left).unwrap();
    assert!(written.contains("\"coins\""), "{written}");
}

/// `--open` hands stdout to `git diff --no-index` and the terminal to
/// `code --diff`. Under `--json` that ruins the document exactly as a prompt
/// would, so the pair is refused rather than one quietly winning.
#[test]
fn data_diff_refuses_open_and_json_together() {
    Command::cargo_bin("rbx")
        .unwrap()
        .args([
            "data",
            "--datastore",
            "PlayerData",
            "diff",
            "Player_156",
            "--revisions",
            "a,b",
            "--open",
            "--json",
        ])
        .assert()
        .failure();
}

/// The writing subcommands never take `--json`, because every one of them
/// prompts and a format that owns stdout may not stop to ask.
#[test]
fn the_writing_subcommands_do_not_take_json_at_all() {
    for args in [
        vec!["set", "Player_156", "--value", "1"],
        vec!["reset", "Player_156"],
        vec!["increment", "Player_156", "--by", "1"],
        vec!["snapshot"],
    ] {
        let mut argv = vec!["data", "--datastore", "PlayerData"];
        argv.extend_from_slice(&args);
        argv.push("--json");
        Command::cargo_bin("rbx")
            .unwrap()
            .args(&argv)
            .assert()
            .failure();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn memorystore_get_nests_the_value_and_carries_the_expiry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{}/rotation", items_path())))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "rotation",
            "value": { "map": "desert", "weight": 3 },
            "expireTime": "2026-08-15T09:43:49Z",
            "etag": "e1"
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let mut args = memorystore_args(places.to_str().unwrap(), &uri);
    args.extend_from_slice(&["get", "rotation", "--json"]);

    let (doc, stderr) = run_json(&args);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["map"], "Cache");
    assert_eq!(doc["item"], "rotation");
    assert_eq!(doc["expire_time"], "2026-08-15T09:43:49Z");
    assert_eq!(doc["value"]["map"], "desert");
    assert_eq!(doc["value"]["weight"], 3);
    assert!(!doc.to_string().contains("etag"), "{doc}");
    // The expiry line the human form prints was already on stderr, and stayed.
    assert!(stderr.contains("expires"), "stderr was:\n{stderr}");
}

/// `--values` decides whether values are printed, in both formats. The shape
/// follows the invocation, so nothing starts reading a value it did not ask
/// for.
#[tokio::test(flavor = "multi_thread")]
async fn memorystore_list_carries_values_only_when_they_were_asked_for() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(items_path()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [
                {
                    "id": "rotation",
                    "numericSortKey": 3.5,
                    "expireTime": "2026-08-15T09:43:49Z",
                    "value": { "map": "desert" }
                },
                { "id": "banner", "stringSortKey": "zulu", "value": "hello" }
            ],
            "nextPageToken": ""
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();

    let mut bare = memorystore_args(places.to_str().unwrap(), &uri);
    bare.extend_from_slice(&["list", "--json"]);
    let (doc, _) = run_json(&bare);

    assert_eq!(doc["count"], 2);
    assert_eq!(doc["items"][0]["id"], "rotation");
    assert_eq!(doc["items"][0]["numeric_sort_key"], 3.5);
    assert_eq!(doc["items"][0]["expire_time"], "2026-08-15T09:43:49Z");
    assert_eq!(doc["items"][1]["string_sort_key"], "zulu");
    assert!(doc["items"][0].get("value").is_none(), "{doc}");

    let mut with_values = memorystore_args(places.to_str().unwrap(), &uri);
    with_values.extend_from_slice(&["list", "--values", "--json"]);
    let (doc, _) = run_json(&with_values);

    assert_eq!(doc["items"][0]["value"]["map"], "desert");
    assert_eq!(doc["items"][1]["value"], "hello");
}

/// A map that has never been written to answers exactly like one whose items
/// expired. Under `--json` that is an empty array, not silence.
#[tokio::test(flavor = "multi_thread")]
async fn memorystore_list_emits_an_empty_document_for_an_empty_map() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(items_path()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [],
            "nextPageToken": ""
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();
    let mut args = memorystore_args(places.to_str().unwrap(), &uri);
    args.extend_from_slice(&["list", "--json"]);

    let (doc, stderr) = run_json(&args);

    assert_eq!(doc["count"], 0);
    assert_eq!(doc["items"].as_array().map(Vec::len), Some(0));
    assert!(stderr.contains("is empty"), "stderr was:\n{stderr}");
}

/// The human forms are the default and are untouched by any of this: stdout is
/// the pretty-printed value and nothing else, exactly as before.
#[tokio::test(flavor = "multi_thread")]
async fn without_the_flag_both_reads_still_print_the_bare_value() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{}/Player_156", entries_path())))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "revisionId": "r1",
            "value": { "coins": 500 }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/rotation", items_path())))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "value": { "map": "desert" }
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let uri = server.uri();

    let mut entry = data_args(places.to_str().unwrap(), &uri);
    entry.extend_from_slice(&["get", "Player_156"]);
    let output = Command::cargo_bin("rbx")
        .unwrap()
        .args(&entry)
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout, "{\n  \"coins\": 500\n}\n");

    let mut item = memorystore_args(places.to_str().unwrap(), &uri);
    item.extend_from_slice(&["get", "rotation"]);
    let output = Command::cargo_bin("rbx")
        .unwrap()
        .args(&item)
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout, "{\n  \"map\": \"desert\"\n}\n");
}
