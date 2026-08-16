//! `--json` on the declared-state commands, against the real binary.
//!
//! The unit tests in `rbx_shop::json` and `rbx_config::json` pin what the
//! documents *say*. They cannot pin what else reaches stdout, because a stray
//! `println!` three layers down is invisible to a test that renders a struct
//! into a buffer — and a stray `println!` is exactly the failure that breaks
//! `jq` in somebody's pipeline. So these run the binary and parse its stdout.
//!
//! The fixture is arranged to have two things to say on stderr: an unrecognised
//! top-level key in `rbxshop.toml`, which `Config::load` warns about on every
//! command that reads the file, and the per-env overlay hint `shop show` prints
//! under its tables. A warning that has nowhere safe to go is how stdout gets
//! polluted.
//!
//! `shop list` and the three `config` commands talk to Roblox and the binary
//! exposes no base-url override for them, so what is asserted here is the half
//! that does not need a mock: a failure under `--json` leaves stdout empty
//! rather than half a document. Their shapes are covered by the unit tests.
//!
//! Deliberately a separate file from `json_output.rs` rather than an addition
//! to it: the two are written in parallel and share nothing but the helper
//! below, which is small enough to copy and awkward enough to fight over.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;

/// An `rbxshop.toml` with an unknown top-level key in it, so `Config::load`
/// has a warning to emit on every command that reads the file.
const SHOP_WITH_UNKNOWN_KEY: &str = r#"
notakey = "warn about me"

[experience]
universe_id = 5544332211

[passes.vip]
name = "VIP Pass"
price = 199
description = "the good one"

[passes.starter]
for_sale = false

[badges.first_win]
name = "First Win"

[badges.retired]
enabled = false

[products.coins_100]
price = 50
store_page = true

[envs.prod.passes.vip]
price = 299

[envs.prod.passes.prod_only]
price = 10
"#;

fn shop_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("rbxshop.toml");
    std::fs::write(&path, SHOP_WITH_UNKNOWN_KEY).unwrap();
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
fn shop_show_emits_a_document_on_stdout_and_its_warning_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let shop = shop_file(&dir);

    let (doc, stderr) = run_json(&["shop", "--config", shop.to_str().unwrap(), "show", "--json"]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["config_file"], shop.display().to_string());
    // A string: every id in every document this tool writes is one. `price`
    // below stays a number, which is the line the convention draws — ids
    // identify, prices count.
    assert_eq!(doc["experience"]["universe_id"], "5544332211");
    assert_eq!(doc["passes"]["vip"]["name"], "VIP Pass");
    assert_eq!(doc["passes"]["vip"]["price"], 199);
    assert_eq!(doc["passes"]["starter"]["for_sale"], false);
    assert_eq!(doc["badges"]["retired"]["enabled"], false);
    assert_eq!(doc["products"]["coins_100"]["store_page"], true);

    // The unrecognised key was reported, and reported where it cannot corrupt
    // the document. If this moved to stdout the parse above would have failed.
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
    // So was the overlay hint the human view prints under its tables: it is
    // advice about how to run the command, not part of the declared state.
    assert!(
        stderr.contains("per-env overlays defined"),
        "stderr was:\n{stderr}"
    );
}

/// The base view has no overlay to apply, so it omits `env` rather than
/// inventing a name for "none" — and the env-exclusive resource stays out.
#[test]
fn shop_show_omits_the_env_for_the_base_view() {
    let dir = tempfile::tempdir().unwrap();
    let shop = shop_file(&dir);

    let (doc, _) = run_json(&["shop", "--config", shop.to_str().unwrap(), "show", "--json"]);

    assert!(doc.get("env").is_none(), "{doc}");
    assert!(doc["passes"].get("prod_only").is_none(), "{doc}");
    assert_eq!(doc["passes"]["vip"]["price"], 199);
}

#[test]
fn shop_show_applies_the_env_overlay_and_names_the_env() {
    let dir = tempfile::tempdir().unwrap();
    let shop = shop_file(&dir);

    let (doc, stderr) = run_json(&[
        "--env",
        "prod",
        "shop",
        "--config",
        shop.to_str().unwrap(),
        "show",
        "--json",
    ]);

    assert_eq!(doc["env"], "prod");
    assert_eq!(doc["passes"]["vip"]["price"], 299);
    assert_eq!(doc["passes"]["prod_only"]["price"], 10);
    // Targeting an env is what the hint asks for, so it is not printed.
    assert!(
        !stderr.contains("per-env overlays defined"),
        "stderr was:\n{stderr}"
    );
}

/// The document describes declared state. `rbx check --json` describes drift,
/// and owns the vocabulary for it. A filter that reaches for a verdict here
/// must come back with nothing rather than with something plausible.
#[test]
fn shop_show_says_nothing_about_drift() {
    let dir = tempfile::tempdir().unwrap();
    let shop = shop_file(&dir);

    let (doc, _) = run_json(&["shop", "--config", shop.to_str().unwrap(), "show", "--json"]);

    for word in ["outcome", "checks", "check", "tool", "summary", "details"] {
        assert!(doc.get(word).is_none(), "{word} must not appear: {doc}");
    }
    assert!(doc.get("totals").is_none(), "{doc}");
}

/// The human form is the default and is untouched by any of this.
#[test]
fn without_the_flag_shop_show_still_prints_its_tables() {
    let dir = tempfile::tempdir().unwrap();
    let shop = shop_file(&dir);

    let assertion = Command::cargo_bin("rbx")
        .unwrap()
        .args(["shop", "--config", shop.to_str().unwrap(), "show"])
        .assert()
        .success();

    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Game passes (2)"), "stdout was:\n{stdout}");
    assert!(stdout.contains("Badges (2)"), "stdout was:\n{stdout}");
    assert!(
        stdout.contains("Developer products (1)"),
        "stdout was:\n{stdout}"
    );
    assert!(stdout.contains("199 R$"), "stdout was:\n{stdout}");
    // The hint stays on stdout without `--json`, exactly where it was.
    assert!(
        stdout.contains("per-env overlays defined"),
        "stdout was:\n{stdout}"
    );
    // And nothing about the layout was turned into JSON by accident.
    assert!(!stdout.contains("schema_version"), "stdout was:\n{stdout}");
}

/// `--sort` and `--flat` are layouts over a listing. The document is an object
/// keyed by TOML key, which has neither an order to pick nor a flat variant, so
/// asking for both is a mistake worth reporting rather than a precedence
/// question.
#[test]
fn shop_show_refuses_a_layout_flag_alongside_json() {
    let dir = tempfile::tempdir().unwrap();
    let shop = shop_file(&dir);
    let config = shop.to_str().unwrap().to_string();

    for extra in [vec!["--flat"], vec!["--sort", "price"]] {
        let mut args = vec!["shop", "--config", config.as_str(), "show", "--json"];
        args.extend(extra.iter().copied());
        Command::cargo_bin("rbx")
            .unwrap()
            .args(&args)
            .assert()
            .failure();
    }
}

/// The defaulted `--sort` must not count as "asked for", or every
/// `shop show --json` would be rejected by the conflict above.
#[test]
fn shop_show_accepts_json_when_sort_is_only_its_default() {
    let dir = tempfile::tempdir().unwrap();
    let shop = shop_file(&dir);

    let (doc, _) = run_json(&["shop", "--config", shop.to_str().unwrap(), "show", "--json"]);

    assert_eq!(doc["schema_version"], 1);
}

/// A command that cannot answer must leave stdout empty. Half a document is
/// worse than none: the consumer parses it, gets a shape it recognises, and
/// acts on a config that was never read.
#[test]
fn a_failing_json_command_writes_no_partial_document() {
    let dir = tempfile::tempdir().unwrap();
    let shop = shop_file(&dir);
    let places = dir.path().join("rbxplace.toml");
    std::fs::write(&places, "[dev]\nuniverse_id = 100\n").unwrap();

    // No API key, so each of these fails before it has anything to report.
    let cases: Vec<Vec<&str>> = vec![
        vec![
            "--places",
            places.to_str().unwrap(),
            "--env",
            "dev",
            "shop",
            "--config",
            shop.to_str().unwrap(),
            "list",
            "passes",
            "--json",
        ],
        vec![
            "--places",
            places.to_str().unwrap(),
            "--env",
            "dev",
            "config",
            "list",
            "--json",
        ],
        vec![
            "--places",
            places.to_str().unwrap(),
            "--env",
            "dev",
            "config",
            "get",
            "some.key",
            "--json",
        ],
        vec![
            "--places",
            places.to_str().unwrap(),
            "--env",
            "dev",
            "config",
            "versions",
            "--json",
        ],
    ];

    for args in cases {
        let output = Command::cargo_bin("rbx")
            .unwrap()
            .env_remove("RBX_API_KEY")
            .args(&args)
            .assert()
            .failure()
            .get_output()
            .clone();

        assert!(
            output.stdout.is_empty(),
            "{args:?} wrote to stdout under --json:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            !output.stderr.is_empty(),
            "{args:?} failed without saying why"
        );
    }
}
