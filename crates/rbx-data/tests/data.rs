//! Data store reads and overwrites over HTTP.
//!
//! This is the most destructive command in `rbx-ops`, so the tests are about
//! what it refuses to do as much as what it does: never write without
//! `--apply`, never drop the user association by accident, always leave a local
//! copy of what it replaced.

use rbx_core::GlobalFlags;
use rbx_data::{run, DataCli};
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 66778899001;
const ENTRY_PATH: &str =
    "/cloud/v2/universes/66778899001/data-stores/PlayerData/scopes/global/entries/Player_156";

#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    data: DataCli,
}

fn flags(places: &str) -> GlobalFlags {
    GlobalFlags {
        api_key: Some("test-key".into()),
        cookie: None,
        no_auto_cookie: true,
        auto_cookie: false,
        env: Some("ops".into()),
        place: None,
        places: places.into(),
        universe_id: None,
        place_id: Vec::new(),
    }
}

fn places_file(dir: &std::path::Path) -> String {
    let file = dir.join("rbxplace.toml");
    std::fs::write(
        &file,
        format!("[ops]\nuniverse_id = {UNIVERSE}\n\n[ops.places]\nmain = 1\n"),
    )
    .unwrap();
    file.to_string_lossy().into_owned()
}

fn cli(args: &[&str], server: &MockServer) -> DataCli {
    let mut argv = vec!["data", "--datastore", "PlayerData"];
    argv.extend_from_slice(args);
    <Wrapper as clap::Parser>::parse_from(argv)
        .data
        .with_base_url(server.uri())
}

async fn mount_existing(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(ENTRY_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn set_without_apply_reads_the_entry_and_never_writes() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    mount_existing(
        &server,
        serde_json::json!({ "value": { "coins": 500 }, "revisionId": "r1" }),
    )
    .await;
    // No PATCH mock: a write would 404 and fail the run.

    run(
        cli(&["set", "Player_156", "--value", r#"{"coins":0}"#], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();

    let writes = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method == wiremock::http::Method::PATCH)
        .count();
    assert_eq!(writes, 0, "a dry run must never touch player data");
}

#[tokio::test]
async fn an_overwrite_keeps_the_user_association_it_found() {
    // The quiet one. Sending only `value` would replace the entry with one that
    // has no `users`, severing the link Roblox uses to answer a player's data
    // request, and nothing in the response would say so.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    mount_existing(
        &server,
        serde_json::json!({
            "value": { "coins": 500 },
            "users": ["users/156"],
            "attributes": { "tier": "gold" }
        }),
    )
    .await;
    Mock::given(method("PATCH"))
        .and(path(ENTRY_PATH))
        .and(query_param("allowMissing", "true"))
        .and(body_json(serde_json::json!({
            "value": { "coins": 0 },
            "users": ["users/156"],
            "attributes": { "tier": "gold" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "revisionId": "r2"
        })))
        .expect(1)
        .mount(&server)
        .await;

    run(
        cli(
            &[
                "set",
                "Player_156",
                "--value",
                r#"{"coins":0}"#,
                "--backup",
                dir.path().join("b.json").to_str().unwrap(),
                "--apply",
                "--yes",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn drop_metadata_removes_it_deliberately() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    mount_existing(
        &server,
        serde_json::json!({ "value": 1, "users": ["users/156"] }),
    )
    .await;
    Mock::given(method("PATCH"))
        .and(body_json(serde_json::json!({ "value": { "coins": 0 } })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;

    run(
        cli(
            &[
                "set",
                "Player_156",
                "--value",
                r#"{"coins":0}"#,
                "--drop-metadata",
                "--backup",
                dir.path().join("b.json").to_str().unwrap(),
                "--apply",
                "--yes",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_replaced_value_is_written_to_disk_before_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("before.json");
    let server = MockServer::start().await;
    mount_existing(
        &server,
        serde_json::json!({ "value": { "coins": 500, "items": ["sword"] } }),
    )
    .await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    run(
        cli(
            &[
                "set",
                "Player_156",
                "--value",
                "0",
                "--backup",
                backup.to_str().unwrap(),
                "--apply",
                "--yes",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();

    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&backup).unwrap()).unwrap();
    assert_eq!(
        saved,
        serde_json::json!({ "coins": 500, "items": ["sword"] })
    );
}

/// Without `--backup`, the copy follows the project rather than the shell: it
/// lands beside `rbxplace.toml`, under the env that was written to.
#[tokio::test]
async fn the_default_backup_lands_under_the_env_beside_the_places_file() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    mount_existing(&server, serde_json::json!({ "value": { "coins": 500 } })).await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    run(
        cli(
            &["set", "Player_156", "--value", "0", "--apply", "--yes"],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();

    let backups = dir.path().join(".rbx").join("backups").join("ops");
    let written: Vec<_> = std::fs::read_dir(&backups)
        .unwrap_or_else(|err| panic!("no backup directory at {}: {err}", backups.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(written.len(), 1, "expected one backup, got {written:?}");

    let name = written[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        name.starts_with("Player_156-") && name.ends_with(".json"),
        "the name must carry the entry and a timestamp, got {name}"
    );
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&written[0]).unwrap()).unwrap();
    assert_eq!(saved, serde_json::json!({ "coins": 500 }));
}

/// The old default (`<entry>.backup.json`) let the second overwrite destroy the
/// copy of the value the first one destroyed, which is the copy you want.
#[tokio::test]
async fn a_second_overwrite_does_not_replace_the_first_backup() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    mount_existing(&server, serde_json::json!({ "value": { "coins": 500 } })).await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    let places = places_file(dir.path());

    for value in ["0", "1"] {
        run(
            cli(
                &["set", "Player_156", "--value", value, "--apply", "--yes"],
                &server,
            ),
            &flags(&places),
        )
        .await
        .unwrap();
    }

    let backups = dir.path().join(".rbx").join("backups").join("ops");
    assert_eq!(
        std::fs::read_dir(&backups).unwrap().count(),
        2,
        "each overwrite must leave its own copy behind"
    );
}

/// Retention is what keeps the directory from growing forever, and it counts
/// the file just written.
#[tokio::test]
async fn keep_prunes_the_oldest_backups_of_that_entry() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    mount_existing(&server, serde_json::json!({ "value": { "coins": 500 } })).await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    let places = places_file(dir.path());

    for value in ["0", "1", "2"] {
        run(
            cli(
                &[
                    "set",
                    "Player_156",
                    "--value",
                    value,
                    "--keep",
                    "2",
                    "--apply",
                    "--yes",
                ],
                &server,
            ),
            &flags(&places),
        )
        .await
        .unwrap();
    }

    let backups = dir.path().join(".rbx").join("backups").join("ops");
    assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 2);
}

/// `--keep` describes the default directory; with `--backup` there is no
/// directory to prune, and with `--no-backup` there is nothing to keep.
#[test]
fn keep_cannot_be_combined_with_the_flags_that_leave_it_nothing_to_do() {
    for other in [["--backup", "somewhere.json"], ["--no-backup", "--yes"]] {
        let parsed = <Wrapper as clap::Parser>::try_parse_from(
            [
                "data",
                "--datastore",
                "PlayerData",
                "set",
                "Player_156",
                "--value",
                "0",
                "--keep",
                "3",
            ]
            .iter()
            .copied()
            .chain(other),
        );
        assert!(
            parsed.is_err(),
            "expected clap to refuse --keep with {other:?}"
        );
    }
}

/// Zero backups is `--no-backup`, said plainly. Reading it as "keep none"
/// would delete the copy the write just made.
#[test]
fn keep_zero_is_refused_rather_than_read_as_no_backup() {
    let parsed = <Wrapper as clap::Parser>::try_parse_from([
        "data",
        "--datastore",
        "PlayerData",
        "set",
        "Player_156",
        "--value",
        "0",
        "--keep",
        "0",
    ]);
    assert!(parsed.is_err(), "expected clap to refuse --keep 0");
}

#[tokio::test]
async fn a_key_that_does_not_exist_yet_is_created_rather_than_refused() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ENTRY_PATH))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(query_param("allowMissing", "true"))
        .and(body_json(serde_json::json!({ "value": { "coins": 0 } })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;

    run(
        cli(
            &[
                "set",
                "Player_156",
                "--value",
                r#"{"coins":0}"#,
                "--apply",
                "--yes",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn malformed_json_fails_before_the_entry_is_even_read() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    let error = run(
        cli(&["set", "Player_156", "--value", "{not json"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("valid JSON"), "got: {error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn env_all_is_refused_for_a_command_that_rewrites_player_data() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let mut global = flags(&places_file(dir.path()));
    global.env = Some("all".into());

    let error = run(cli(&["get", "Player_156"], &server), &global)
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("--env all"), "got: {error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_datastore_name_is_caught_before_any_request() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let parsed = <Wrapper as clap::Parser>::parse_from(["data", "get", "Player_156"])
        .data
        .with_base_url(server.uri());

    let error = run(parsed, &flags(&places_file(dir.path())))
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("--datastore"), "got: {error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn getting_a_missing_entry_is_reported_rather_than_treated_as_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;

    run(
        cli(&["get", "Player_156"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_failure_whose_body_mentions_404_is_not_read_as_a_missing_entry() {
    // The regression this guards. "Is it missing?" used to be answered by
    // searching the rendered error for "404", and that string renders the
    // response body too, so any failure quoting those digits was reported as
    // "no such entry". Roblox echoes the entry id back in its 400s, which
    // makes `Player_404` enough to trigger it, and the user is told their save
    // does not exist when the request was simply malformed.
    //
    // 400 rather than 500 on purpose: a 5xx would be retried three times and
    // buy seven seconds of sleep for nothing.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ENTRY_PATH))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string(r#"{"message":"Invalid entry key: Player_404"}"#),
        )
        .mount(&server)
        .await;

    let error = run(
        cli(&["get", "Player_156"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .expect_err("a 400 is a failure, whatever digits its body happens to contain")
    .to_string();
    assert!(error.contains("400"), "got: {error}");
}

#[tokio::test]
async fn a_missing_entry_is_detected_by_status_even_when_the_body_says_nothing() {
    // The other half: detection must not depend on the body either. Roblox
    // has answered 404 with an empty body, and a text-matching check reads
    // that as a hard failure and stops a `set` that should have created the
    // key.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ENTRY_PATH))
        .respond_with(ResponseTemplate::new(404).set_body_string(""))
        .mount(&server)
        .await;

    run(
        cli(&["get", "Player_156"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .expect("an empty-bodied 404 is still a missing entry");
}

#[tokio::test]
async fn reset_writes_the_template_and_keeps_the_user_association() {
    let dir = tempfile::tempdir().unwrap();
    let template = dir.path().join("t.json");
    std::fs::write(&template, r#"{"coins":0,"level":1}"#).unwrap();

    let server = MockServer::start().await;
    mount_existing(
        &server,
        serde_json::json!({ "value": { "coins": 9000 }, "users": ["users/156"] }),
    )
    .await;
    Mock::given(method("PATCH"))
        .and(body_json(serde_json::json!({
            "value": { "coins": 0, "level": 1 },
            "users": ["users/156"]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;

    run(
        cli(
            &[
                "reset",
                "Player_156",
                "--template",
                template.to_str().unwrap(),
                "--backup",
                dir.path().join("b.json").to_str().unwrap(),
                "--apply",
                "--yes",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn reset_without_a_template_says_where_it_looked() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    let error = run(
        cli(
            &[
                "reset",
                "Player_156",
                "--template",
                dir.path().join("absent.json").to_str().unwrap(),
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("absent.json"), "got: {error}");
    assert!(error.contains("--template"), "got: {error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn reset_without_apply_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let template = dir.path().join("t.json");
    std::fs::write(&template, "{}").unwrap();
    let server = MockServer::start().await;
    mount_existing(&server, serde_json::json!({ "value": { "coins": 9000 } })).await;

    run(
        cli(
            &[
                "reset",
                "Player_156",
                "--template",
                template.to_str().unwrap(),
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();

    let writes = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method == wiremock::http::Method::PATCH)
        .count();
    assert_eq!(writes, 0);
}

// ---------------- snapshot ----------------

const SNAPSHOT_PATH: &str = "/cloud/v2/universes/66778899001/data-stores:snapshot";

const STORES_PATH: &str = "/cloud/v2/universes/66778899001/data-stores";

/// `stores` is the command you run *because* you have no store name, so like
/// `snapshot` it must parse and run with no `--datastore`.
fn stores_cli(args: &[&str], server: &MockServer) -> DataCli {
    let mut argv = vec!["data", "stores"];
    argv.extend_from_slice(args);
    <Wrapper as clap::Parser>::parse_from(argv)
        .data
        .with_base_url(server.uri())
}

#[tokio::test]
async fn stores_needs_no_datastore() {
    // The regression this guards: `data` bails without `--datastore`, which
    // would make the discovery command need the answer it exists to find.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(STORES_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "dataStores": [{ "id": "PlayerData-v1", "createTime": "2026-08-27T16:41:02Z" }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    run(stores_cli(&[], &server), &flags(&places_file(dir.path())))
        .await
        .unwrap();
}

#[tokio::test]
async fn stores_leaves_soft_deleted_ones_out_unless_asked() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(STORES_PATH))
        .and(query_param("showDeleted", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "dataStores": [{ "id": "Old", "state": "DELETED" }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    run(
        stores_cli(&["--show-deleted"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn stores_follows_the_page_token_until_the_listing_ends() {
    // The trap `EntryList::next_token` already documents: cloud/v2 ends a
    // listing with an empty string rather than by omitting the field, so
    // sending it back would ask for the same page forever.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(STORES_PATH))
        .and(query_param("pageToken", "second"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "dataStores": [{ "id": "Tickets" }],
            "nextPageToken": ""
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(STORES_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "dataStores": [{ "id": "PlayerData-v1" }],
            "nextPageToken": "second"
        })))
        .expect(1)
        .mount(&server)
        .await;

    run(stores_cli(&[], &server), &flags(&places_file(dir.path())))
        .await
        .unwrap();
}

#[tokio::test]
async fn stores_stops_asking_once_the_limit_is_reached() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(STORES_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "dataStores": [{ "id": "A" }, { "id": "B" }],
            "nextPageToken": "more"
        })))
        .expect(1)
        .mount(&server)
        .await;

    run(
        stores_cli(&["--limit", "2"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn an_experience_with_no_stores_is_reported_rather_than_treated_as_an_error() {
    // A store exists from its first write, so an empty listing is the honest
    // answer for an experience nothing has written to yet.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(STORES_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    run(stores_cli(&[], &server), &flags(&places_file(dir.path())))
        .await
        .expect("an empty experience is a success, not a failure");
}

/// `snapshot` is experience-wide, so unlike every other subcommand it must
/// parse and run with no `--datastore`.
fn snapshot_cli(args: &[&str], server: &MockServer) -> DataCli {
    let mut argv = vec!["data", "snapshot"];
    argv.extend_from_slice(args);
    <Wrapper as clap::Parser>::parse_from(argv)
        .data
        .with_base_url(server.uri())
}

#[tokio::test]
async fn snapshot_without_apply_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    // No mock at all: any request would 404 and fail the run.

    run(snapshot_cli(&[], &server), &flags(&places_file(dir.path())))
        .await
        .unwrap();

    assert_eq!(
        server.received_requests().await.unwrap().len(),
        0,
        "a dry run must not spend the day's snapshot"
    );
}

#[tokio::test]
async fn snapshot_needs_no_datastore() {
    // The regression this guards: `data` bails without `--datastore`, and
    // demanding one here would be asking for a value the call does not use.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SNAPSHOT_PATH))
        .and(body_json(serde_json::json!({})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "newSnapshotTaken": true,
            "latestSnapshotTime": "2026-08-04T09:00:00Z"
        })))
        .expect(1)
        .mount(&server)
        .await;

    run(
        snapshot_cli(&["--apply"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_second_snapshot_the_same_day_is_not_an_error() {
    // Roblox caps it at one per experience per UTC day and answers 200 with
    // newSnapshotTaken=false rather than failing. Treating that as an error
    // would break any script that snapshots before each migration.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SNAPSHOT_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "newSnapshotTaken": false,
            "latestSnapshotTime": "2026-08-04T02:11:00Z"
        })))
        .mount(&server)
        .await;

    run(
        snapshot_cli(&["--apply"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .expect("an already-snapshotted day is a success, not a failure");
}

/// The escape hatch for a read-only working directory, and for after a
/// snapshot has made Roblox keep the previous value anyway.
#[tokio::test]
async fn no_backup_writes_the_entry_and_leaves_no_file_behind() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    mount_existing(&server, serde_json::json!({ "value": { "coins": 500 } })).await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    // Built before the snapshot: it lands in the same directory, and counting
    // it as a new file would hide whether a backup was written.
    let places = places_file(dir.path());
    let before: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();

    run(
        cli(
            &[
                "set",
                "Player_156",
                "--value",
                "0",
                "--no-backup",
                "--apply",
                "--yes",
            ],
            &server,
        ),
        &flags(&places),
    )
    .await
    .unwrap();

    let after: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(
        before.len(),
        after.len(),
        "--no-backup must not create a file"
    );

    let writes = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method == wiremock::http::Method::PATCH)
        .count();
    assert_eq!(writes, 1, "the write itself must still happen");
}

/// The two flags contradict each other: one names where the copy goes, the
/// other says there is none. clap refuses the pair rather than silently
/// preferring one.
#[test]
fn no_backup_and_backup_cannot_be_passed_together() {
    let parsed = <Wrapper as clap::Parser>::try_parse_from([
        "data",
        "--datastore",
        "PlayerData",
        "set",
        "Player_156",
        "--value",
        "0",
        "--backup",
        "somewhere.json",
        "--no-backup",
    ]);
    assert!(parsed.is_err(), "expected clap to refuse the pair");
}
