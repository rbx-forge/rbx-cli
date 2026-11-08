//! `--json` against the real binary: stdout carries the document and nothing
//! else.
//!
//! The unit tests in `rbx_env::json` pin what the documents *say*. They cannot
//! pin what else reaches stdout, because a stray `println!` three layers down
//! is invisible to a test that renders a struct into a buffer, and a stray
//! `println!` is exactly the failure that breaks `jq` in somebody's pipeline. So these run the binary and parse its stdout.
//!
//! Each case is arranged to have something to say on stderr as well: an
//! unknown key in `rbxplace.toml`. A warning that has nowhere safe to go is
//! how stdout gets polluted.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;

/// The prod universe in the file below.
const UNIVERSE: u64 = 5544332211;

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
