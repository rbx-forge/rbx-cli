//! `--json` on the credential commands, against the real binary.
//!
//! `ban list`, `ban status`, `apikey list`, `apikey status`,
//! `apikey scopes show`.
//!
//! The unit tests in `rbx_ban::json` and `rbx_apikey::json` pin what the
//! documents *say*. They cannot pin what else reaches stdout, because a stray
//! `println!` three layers down is invisible to a test that renders a struct
//! into a buffer, and a stray `println!` is exactly the failure that breaks
//! `jq` in somebody's pipeline. So these run the binary and parse its stdout.
//!
//! These two commands are the most sensitive in the tree: one holds live Open
//! Cloud secrets, the other reports on real players who are locked out of a
//! real game. So one test here does something the others do not: it puts a
//! secret in the fixture and asserts that neither it nor any fragment of it
//! reaches stdout *or* stderr. A run that leaks a credential leaks it into a CI
//! log, where nobody notices until the key has to be rotated, and a CI log is
//! `2>&1`: the split that keeps stdout parseable is no barrier there.
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

/// The secret the lockfile fixture holds. Never a real one, and never anything
/// stdout is allowed to contain, in whole or in part.
const SECRET: &str = "RBX-DO-NOT-LEAK-9f3c1a7b2d4e";

/// The path the fixture stores a secret at. A pointer to a credential is not
/// the credential, and it is still not something a document publishes.
const SECRET_FILE: &str = ".secrets/deploy.env";

/// An `rbxplace.toml` with an unknown key in it, so `PlacesFile::load` has a
/// warning to emit on every command that reads the file.
const PLACES_WITH_UNKNOWN_KEY: &str = r#"
[owner]
type = "user"
id = 1234567890

[ops]
universe_id = 5544332211
notakey = "warn about me"
[ops.places]
main = 55443322110099
"#;

const APIKEY_CONFIG: &str = r#"
[settings]
default_enabled = true

[keys.deploy]
envs = ["ops"]
scopes = ["universe:read"]
secret_file = ".secrets/deploy.env"

[keys.newkey]
envs = ["ops"]
scopes = ["universe:read"]
"#;

fn apikey_lock() -> String {
    format!(
        r#"
version = 4

[envs.ops]
universe_id = {UNIVERSE}
owner_type = "user"
owner_id = 1234567890

[keys.deploy]
cloud_auth_id = "f58b4055-cafe-4e2f-9c2a-000000000001"
secret = "{SECRET}"
secret_file = "{SECRET_FILE}"
creator_id = 1234567890
is_enabled = true
created_at = "2026-01-01T00:00:00.000Z"
expires_at = "2027-08-01T10:00:00.000Z"

[keys.leftover]
cloud_auth_id = "f58b4055-cafe-4e2f-9c2a-000000000002"
secret = "{SECRET}"
creator_id = 1234567890
is_enabled = true
created_at = "2026-01-01T00:00:00.000Z"
"#
    )
}

fn places_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("rbxplace.toml");
    std::fs::write(&path, PLACES_WITH_UNKNOWN_KEY).unwrap();
    path
}

/// A directory `rbx apikey` can be run from: both of its files, plus the
/// `rbxplace.toml` that `status` resolves envs against. The secret file backend
/// points at a path that does not exist, which is the `SECRET_MISSING` case.
fn apikey_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    places_file(&dir);
    std::fs::write(dir.path().join("rbxapikey.toml"), APIKEY_CONFIG).unwrap();
    std::fs::write(dir.path().join("rbxapikey.lock.toml"), apikey_lock()).unwrap();
    dir
}

/// Run `rbx`, require success, and return `(parsed stdout, stderr)`.
fn run_json(args: &[&str]) -> (serde_json::Value, String) {
    run_json_in(None, args)
}

/// The same, from a working directory: `apikey` reads `rbxapikey.toml` and its
/// lockfile relative to the process's cwd and has no flag to point elsewhere.
fn run_json_in(dir: Option<&std::path::Path>, args: &[&str]) -> (serde_json::Value, String) {
    let output = run_ok_in(dir, args);
    let document = parse_stdout(&output);
    (
        document,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Run `rbx` from an optional working directory, require success, and hand back
/// both streams as the process wrote them.
///
/// The leak test uses this rather than `run_json_in`, because what it has to
/// inspect is bytes: a document parsed and re-serialised has been through
/// `serde_json`'s escaping and key ordering, and "no secret reached stdout" is a
/// claim about what the process emitted, not about what survived a round trip.
fn run_ok_in(dir: Option<&std::path::Path>, args: &[&str]) -> std::process::Output {
    let mut command = Command::cargo_bin("rbx").unwrap();
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    command
        .env_remove("RBX_API_KEY")
        .env_remove("RBX_COOKIE")
        .args(args)
        .assert()
        .success()
        .get_output()
        .clone()
}

/// Parsing is the assertion: anything printed alongside the document makes
/// `from_slice` fail, which is the whole contract under test.
fn parse_stdout(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout must be one JSON document and nothing else ({e}). It was:\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// The two hosts `ban` talks to, and the arguments that point it at them.
struct BanHosts {
    api: MockServer,
    users: MockServer,
}

impl BanHosts {
    async fn start() -> Self {
        Self {
            api: MockServer::start().await,
            users: MockServer::start().await,
        }
    }

    fn args<'a>(&'a self, places: &'a str, rest: &[&'a str]) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "--places".into(),
            places.into(),
            "--env".into(),
            "ops".into(),
            "--api-key".into(),
            "test-key".into(),
            "--no-auto-cookie".into(),
            "ban".into(),
            "--base-url".into(),
            self.api.uri(),
            "--users-url".into(),
            self.users.uri(),
        ];
        args.extend(rest.iter().map(|arg| (*arg).to_string()));
        args
    }
}

fn as_str(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

async fn mount_restrictions(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/cloud/v2/universes/{UNIVERSE}/user-restrictions"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mount_one_restriction(server: &MockServer, user_id: u64, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/cloud/v2/universes/{UNIVERSE}/user-restrictions/{user_id}"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mount_user(server: &MockServer, id: u64, name: &str, display: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/users/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": id, "name": name, "displayName": display, "hasVerifiedBadge": false
        })))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ban_list_emits_a_document_on_stdout_and_its_warning_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let hosts = BanHosts::start().await;
    mount_restrictions(
        &hosts.api,
        serde_json::json!({
            "userRestrictions": [
                {"user": "users/156", "gameJoinRestriction": {
                    "active": true, "duration": "604800s",
                    "startTime": "2026-08-01T10:12:03Z",
                    "privateReason": "exploit: fly hack",
                    "displayReason": "Banned 7 days for cheating"}},
                {"user": "users/881", "gameJoinRestriction": {"active": true}}
            ],
            "nextPageToken": ""
        }),
    )
    .await;

    let args = hosts.args(places.to_str().unwrap(), &["list", "--json"]);
    let (doc, stderr) = run_json(&as_str(&args));

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["env"], "ops");
    assert_eq!(doc["universe_id"], "5544332211");
    assert_eq!(doc["count"], 2);
    assert_eq!(doc["include_inactive"], false);
    assert_eq!(doc["restrictions"][0]["user_id"], "156");
    assert_eq!(doc["restrictions"][0]["permanent"], false);
    assert_eq!(doc["restrictions"][0]["duration"], "604800s");
    assert_eq!(
        doc["restrictions"][0]["private_reason"],
        "exploit: fly hack"
    );
    // Permanent is the harshest outcome and Roblox expresses it by omission, so
    // the document states it.
    assert_eq!(doc["restrictions"][1]["permanent"], true);

    // The listing prints neither of these, so it promises neither.
    let rendered = doc.to_string();
    assert!(!rendered.contains("Banned 7 days"), "{rendered}");
    assert!(!rendered.contains("start_time"), "{rendered}");

    // The unrecognised `rbxplace.toml` key was reported, and reported where it
    // cannot corrupt the document. If it had gone to stdout the parse above
    // would have failed.
    assert!(stderr.contains("notakey"), "stderr was:\n{stderr}");
}

/// Nobody restricted is an answer, not silence: a document with a zero in it.
#[tokio::test(flavor = "multi_thread")]
async fn ban_list_answers_an_empty_experience_with_a_document() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let hosts = BanHosts::start().await;
    mount_restrictions(
        &hosts.api,
        serde_json::json!({"userRestrictions": [], "nextPageToken": ""}),
    )
    .await;

    let args = hosts.args(places.to_str().unwrap(), &["list", "--json"]);
    let (doc, _) = run_json(&as_str(&args));

    assert_eq!(doc["count"], 0);
    assert_eq!(doc["restrictions"].as_array().map(Vec::len), Some(0));
}

#[tokio::test(flavor = "multi_thread")]
async fn ban_status_emits_one_document_for_every_player_asked_about() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let hosts = BanHosts::start().await;
    mount_user(&hosts.users, 156, "builderman", "Builder Man").await;
    mount_user(&hosts.users, 881, "someone", "someone").await;
    mount_one_restriction(
        &hosts.api,
        156,
        serde_json::json!({"user": "users/156", "gameJoinRestriction": {
            "active": true, "startTime": "2026-08-01T10:12:03Z",
            "privateReason": "exploit: fly hack",
            "displayReason": "Banned for cheating"}}),
    )
    .await;
    mount_one_restriction(
        &hosts.api,
        881,
        serde_json::json!({"user": "users/881", "gameJoinRestriction": {"active": false}}),
    )
    .await;

    let args = hosts.args(
        places.to_str().unwrap(),
        &["status", "156", "881", "--json"],
    );
    let (doc, _) = run_json(&as_str(&args));

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["count"], 2);
    assert_eq!(doc["players"][0]["user_id"], "156");
    assert_eq!(doc["players"][0]["username"], "builderman");
    assert_eq!(doc["players"][0]["display_name"], "Builder Man");
    assert_eq!(
        doc["players"][0]["profile_url"],
        "https://www.roblox.com/users/156/profile"
    );
    assert_eq!(doc["players"][0]["restricted"], true);
    assert_eq!(doc["players"][0]["permanent"], true);
    assert_eq!(doc["players"][0]["start_time"], "2026-08-01T10:12:03Z");
    // `status` is the command that prints what the player is shown, so it is
    // the command whose document carries it.
    assert_eq!(doc["players"][0]["display_reason"], "Banned for cheating");
    assert_eq!(doc["players"][1]["restricted"], false);
    assert!(doc["players"][1].get("permanent").is_none(), "{doc}");
}

/// The human form is the default and is untouched by any of this.
#[tokio::test(flavor = "multi_thread")]
async fn without_the_flag_ban_list_still_prints_its_table() {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(&dir);
    let hosts = BanHosts::start().await;
    mount_restrictions(
        &hosts.api,
        serde_json::json!({
            "userRestrictions": [
                {"user": "users/156", "gameJoinRestriction": {
                    "active": true, "duration": "604800s", "privateReason": "fly hack"}}
            ],
            "nextPageToken": ""
        }),
    )
    .await;

    let args = hosts.args(places.to_str().unwrap(), &["list"]);
    let assertion = Command::cargo_bin("rbx")
        .unwrap()
        .args(as_str(&args))
        .assert()
        .success();

    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("USER"), "stdout was:\n{stdout}");
    assert!(stdout.contains("REASON"), "stdout was:\n{stdout}");
    // The table renders the duration; the document keeps Roblox's spelling.
    assert!(stdout.contains("1w"), "stdout was:\n{stdout}");
    assert!(stdout.contains("1 restricted"), "stdout was:\n{stdout}");
    assert!(
        stdout.contains("Names are not returned"),
        "stdout was:\n{stdout}"
    );
    assert!(!stdout.contains("schema_version"), "stdout was:\n{stdout}");
}

#[test]
fn apikey_list_emits_a_document_carrying_what_the_human_form_prints() {
    let dir = apikey_dir();
    let (doc, _) = run_json_in(Some(dir.path()), &["apikey", "list", "--json"]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["sort"], "name");
    assert_eq!(doc["count"], 3);

    let keys: Vec<&serde_json::Value> = doc["keys"].as_array().unwrap().iter().collect();
    let deploy = keys.iter().find(|k| k["name"] == "deploy").unwrap();
    assert_eq!(deploy["declared"], true);
    assert_eq!(deploy["created"], true);
    assert_eq!(deploy["id"], "f58b4055-cafe-4e2f-9c2a-000000000001");
    assert_eq!(deploy["creator_id"], "1234567890");
    assert_eq!(deploy["universe_ids"][0], "5544332211");
    assert_eq!(deploy["expires_at"], "2027-08-01T10:00:00.000Z");
    // The fixture's secret file does not exist, so the secret is not there.
    assert_eq!(deploy["secret_backend"], "file");
    assert_eq!(deploy["secret_present"], false);

    // Declared and never created.
    let pending = keys.iter().find(|k| k["name"] == "newkey").unwrap();
    assert_eq!(pending["created"], false);
    assert!(pending.get("id").is_none(), "{pending}");

    // Created and no longer declared: the orphan.
    let orphan = keys.iter().find(|k| k["name"] == "leftover").unwrap();
    assert_eq!(orphan["declared"], false);
    assert_eq!(orphan["created"], true);
    assert_eq!(orphan["secret_backend"], "lockfile");
    assert_eq!(orphan["secret_present"], true);
}

/// The test this file exists for. The lockfile the documents are built from
/// holds a live-shaped secret and the secret's own storage path; neither stream
/// carries either, nor any fragment long enough to be worth grepping a CI log
/// for.
///
/// Both streams, because under `--json` the run is split across them: the
/// document goes to stdout and the command's prose to stderr. That split is
/// what makes stdout parseable, and it is not a boundary a credential stops at:
/// a CI log captures `2>&1`, so the two streams are one file by the time
/// anyone greps it. The per-key detail `apikey status` builds names the secret
/// file's path, and one refactor routing it through `OutputFormat::note` the
/// way the summary already went would put that path on stderr.
///
/// On the raw bytes of stdout rather than a re-serialised document: the other
/// tests here take the parsed value, and a value that has been through
/// `serde_json` twice is no longer evidence of what the process wrote.
#[test]
fn no_secret_and_no_fragment_of_one_reaches_stdout_or_stderr() {
    let dir = apikey_dir();

    for args in [
        vec!["apikey", "list", "--json"],
        vec!["apikey", "status", "--json"],
    ] {
        let output = run_ok_in(Some(dir.path()), &args);
        // Still one document and nothing else on stdout, so a leak cannot hide
        // behind stdout having stopped being a document.
        parse_stdout(&output);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        for (stream, text) in [("stdout", &stdout), ("stderr", &stderr)] {
            assert!(
                !text.contains(SECRET),
                "{args:?} leaked the secret on {stream}:\n{text}"
            );
            assert!(
                !text.contains(SECRET_FILE),
                "{args:?} leaked the secret's path on {stream}:\n{text}"
            );
            // Not just the whole string: a preview is a prefix, and a prefix is
            // still credential material. Every fragment of four characters or
            // more has to be absent too.
            for length in 4..=SECRET.len() {
                let fragment = &SECRET[..length];
                assert!(
                    !text.contains(fragment),
                    "{args:?} leaked `{fragment}` on {stream}:\n{text}"
                );
            }
        }

        // These three are about the document's shape (a field that carries a
        // secret, the Roblox wire name for one, a truncated one) so they are
        // asked of the document's stream only.
        for word in ["secret\":", "apikeySecret", "preview"] {
            assert!(!stdout.contains(word), "{args:?} carries `{word}`");
        }
    }
}

#[test]
fn apikey_status_emits_a_document_and_keeps_its_summary_off_stdout() {
    let dir = apikey_dir();
    let (doc, stderr) = run_json_in(Some(dir.path()), &["apikey", "status", "--json"]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["remote"], false);
    assert_eq!(doc["count"], 3);
    assert_eq!(doc["issues"], 3);

    let keys: Vec<&serde_json::Value> = doc["keys"].as_array().unwrap().iter().collect();
    let deploy = keys.iter().find(|k| k["name"] == "deploy").unwrap();
    // The fixture points `secret_file` at a path that does not exist.
    assert_eq!(deploy["status"], "SECRET_MISSING");
    assert_eq!(deploy["healthy"], false);
    let pending = keys.iter().find(|k| k["name"] == "newkey").unwrap();
    assert_eq!(pending["status"], "PENDING");
    let orphan = keys.iter().find(|k| k["name"] == "leftover").unwrap();
    assert_eq!(orphan["status"], "ORPHAN_LOCK");

    // The summary and the tip are notes about the run, not the result. Under
    // `--json` they are on stderr, where they cannot corrupt the document.
    assert!(
        stderr.contains("key(s) need attention"),
        "stderr was:\n{stderr}"
    );
    assert!(stderr.contains("status --remote"), "stderr was:\n{stderr}");
    // And the advice sentence the human form prints per key is nowhere at all.
    assert!(!doc.to_string().contains("regenerate"), "{doc}");
}

#[test]
fn without_the_flag_apikey_list_still_prints_its_lines() {
    let dir = apikey_dir();
    let assertion = Command::cargo_bin("rbx")
        .unwrap()
        .current_dir(dir.path())
        .args(["apikey", "list"])
        .assert()
        .success();

    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("id:"), "stdout was:\n{stdout}");
    assert!(stdout.contains("creator:"), "stdout was:\n{stdout}");
    assert!(stdout.contains("secret:"), "stdout was:\n{stdout}");
    assert!(stdout.contains("orphan"), "stdout was:\n{stdout}");
    assert!(stdout.contains("need attention"), "stdout was:\n{stdout}");
    assert!(!stdout.contains("schema_version"), "stdout was:\n{stdout}");
}

#[test]
fn apikey_scopes_show_emits_a_document_for_a_known_and_an_unknown_scope() {
    let (doc, _) = run_json(&["apikey", "scopes", "show", "universe", "--json"]);

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["scope_type"], "universe");
    assert_eq!(doc["known"], true);
    assert_eq!(doc["target_type"], "universe");
    assert!(doc["catalog_version"].is_string(), "{doc}");
    assert!(doc["operations"].as_array().is_some(), "{doc}");

    // The catalog is advisory: an unknown scope is a document saying so, exit
    // 0, and not an error.
    let (doc, _) = run_json(&["apikey", "scopes", "show", "not-a-scope", "--json"]);
    assert_eq!(doc["known"], false);
    assert!(doc.get("target_type").is_none(), "{doc}");
    assert!(doc.get("operations").is_none(), "{doc}");
}

/// A command that cannot answer must leave stdout empty. Half a document is
/// worse than none: the consumer parses it, gets a shape it recognises, and
/// acts on a state that was never read.
#[test]
fn a_failing_json_command_writes_no_partial_document() {
    let empty = tempfile::tempdir().unwrap();
    let places = places_file(&empty);

    let cases: Vec<(Option<&std::path::Path>, Vec<&str>)> = vec![
        // No `rbxapikey.toml` in this directory at all.
        (Some(empty.path()), vec!["apikey", "list", "--json"]),
        (Some(empty.path()), vec!["apikey", "status", "--json"]),
        // No API key, so `ban` fails before it has anything to report.
        (
            None,
            vec![
                "--places",
                places.to_str().unwrap(),
                "--env",
                "ops",
                "ban",
                "list",
                "--json",
            ],
        ),
    ];

    for (dir, args) in cases {
        let mut command = Command::cargo_bin("rbx").unwrap();
        if let Some(dir) = dir {
            command.current_dir(dir);
        }
        let output = command
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
