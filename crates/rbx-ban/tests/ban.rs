//! HTTP behaviour of `rbx-ops ban`, against a mock of both hosts it uses.
//!
//! This crate is the only one in `rbx-ops` that writes, so it is the one whose
//! request bodies matter most: a wrong `active`, a missing `updateMask` or a
//! duration in the wrong unit are all silent until a real player is affected.
//!
//! The commands are driven end to end through `run` rather than through an API
//! struct, because the parts worth protecting are the guards: that a dry run
//! sends nothing, that a bad duration never reaches the network, that the write
//! carries an idempotency key.

use rbx_ban::{run, BanCli};
use rbx_core::GlobalFlags;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 66778899001;

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

/// `--env` resolves against a real `rbxplace.toml`, so the tests write one.
fn places_file(dir: &std::path::Path) -> String {
    let file = dir.join("rbxplace.toml");
    std::fs::write(
        &file,
        format!("[ops]\nuniverse_id = {UNIVERSE}\n\n[ops.places]\nmain = 1\n"),
    )
    .unwrap();
    file.to_string_lossy().into_owned()
}

/// The name lookup every command starts with.
async fn mount_user(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/usernames/users"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[{
                "requestedUsername":"builderman","id":156,
                "name":"builderman","displayName":"builderman","hasVerifiedBadge":true
            }]})),
        )
        .mount(server)
        .await;
}

/// `BanCli` is an `Args` group, not a parser in its own right, so the tests
/// wrap it the same way the `rbx-ops` binary does. Building it from a real
/// argv rather than by hand means the tests also cover the flag definitions.
#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    ban: BanCli,
}

fn cli(args: &[&str], api: &MockServer, users: &MockServer) -> BanCli {
    let mut argv = vec!["ban"];
    argv.extend_from_slice(args);
    <Wrapper as clap::Parser>::parse_from(argv)
        .ban
        .with_hosts(api.uri(), users.uri())
}

#[tokio::test]
async fn add_without_apply_reaches_the_lookup_but_never_the_write() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;
    // No PATCH mock at all: any write would 404 and fail the run.

    run(
        cli(&["add", "builderman", "--reason", "testing"], &api, &users),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();

    assert!(
        api.received_requests().await.unwrap().is_empty(),
        "a dry run must not touch the Open Cloud host at all"
    );
}

#[tokio::test]
async fn add_with_apply_sends_the_expected_body_and_update_mask() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;

    Mock::given(method("PATCH"))
        .and(path(format!(
            "/cloud/v2/universes/{UNIVERSE}/user-restrictions/156"
        )))
        .and(header("x-api-key", "test-key"))
        .and(query_param("updateMask", "gameJoinRestriction"))
        .and(body_json(serde_json::json!({
            "gameJoinRestriction": {
                "active": true,
                "duration": "604800s",
                "privateReason": "fly hack",
                "displayReason": "Banned for a week"
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&api)
        .await;

    run(
        cli(
            &[
                "add",
                "builderman",
                "--reason",
                "fly hack",
                "--display-reason",
                "Banned for a week",
                "--duration",
                "7d",
                "--apply",
                "--yes",
            ],
            &api,
            &users,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_write_carries_an_idempotency_key_so_a_retry_is_not_a_second_ban() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;

    Mock::given(method("PATCH"))
        .and(query_param(
            "idempotencyKey.key",
            format!("rbx-ops-{UNIVERSE}-156-restrict"),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&api)
        .await;

    run(
        cli(
            &["add", "builderman", "--reason", "x", "--apply", "--yes"],
            &api,
            &users,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn omitting_the_duration_sends_no_duration_which_is_how_permanent_is_expressed() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;

    Mock::given(method("PATCH"))
        .and(body_json(serde_json::json!({
            "gameJoinRestriction": { "active": true, "privateReason": "x" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&api)
        .await;

    run(
        cli(
            &["add", "builderman", "--reason", "x", "--apply", "--yes"],
            &api,
            &users,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn remove_sends_active_false_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;

    Mock::given(method("PATCH"))
        .and(body_json(serde_json::json!({
            "gameJoinRestriction": { "active": false }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&api)
        .await;

    run(
        cli(&["remove", "builderman", "--apply", "--yes"], &api, &users),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_bad_duration_fails_before_anything_is_sent_anywhere() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;

    let error = run(
        cli(
            &[
                "add",
                "builderman",
                "--reason",
                "x",
                "--duration",
                "7y",
                "--apply",
                "--yes",
            ],
            &api,
            &users,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("unknown duration unit"), "got: {error}");
    assert!(api.received_requests().await.unwrap().is_empty());
    assert!(
        users.received_requests().await.unwrap().is_empty(),
        "validation should happen before the name lookup, not after"
    );
}

#[tokio::test]
async fn an_over_long_reason_is_rejected_locally_rather_than_as_an_opaque_400() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;

    let error = run(
        cli(
            &[
                "add",
                "builderman",
                "--reason",
                &"x".repeat(1001),
                "--apply",
                "--yes",
            ],
            &api,
            &users,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("1001"),
        "the error says how long it was: {error}"
    );
    assert!(api.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn env_all_is_refused_for_a_command_that_touches_players() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    let mut global = flags(&places_file(dir.path()));
    global.env = Some("all".into());

    let error = run(cli(&["status", "builderman"], &api, &users), &global)
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("--env all"), "got: {error}");
    assert!(api.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn status_reports_an_active_restriction_with_its_duration() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;

    Mock::given(method("GET"))
        .and(path(format!(
            "/cloud/v2/universes/{UNIVERSE}/user-restrictions/156"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "user": "users/156",
            "gameJoinRestriction": {
                "active": true,
                "duration": "604800s",
                "privateReason": "fly hack",
                "startTime": "2026-08-01T10:12:03Z"
            }
        })))
        .expect(1)
        .mount(&api)
        .await;

    run(
        cli(&["status", "builderman"], &api, &users),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn listing_stops_on_an_empty_page_token_instead_of_looping() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;

    // `cloud/v2` ends a listing with "" rather than by omitting the field.
    // Requesting again with "" would return this same page forever.
    Mock::given(method("GET"))
        .and(path(format!(
            "/cloud/v2/universes/{UNIVERSE}/user-restrictions"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "userRestrictions": [
                {"user":"users/156","gameJoinRestriction":{"active":true,"privateReason":"x"}}
            ],
            "nextPageToken": ""
        })))
        .expect(1)
        .mount(&api)
        .await;

    run(
        cli(&["list"], &api, &users),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn alts_are_restricted_by_default_and_the_field_is_left_out() {
    // Roblox's `excludeAltAccounts` defaults to false, meaning the restriction
    // *does* propagate to alts. Sending nothing keeps that, which is what you
    // want for an exploiter.
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;

    Mock::given(method("PATCH"))
        .and(body_json(serde_json::json!({
            "gameJoinRestriction": { "active": true, "privateReason": "x" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&api)
        .await;

    run(
        cli(
            &["add", "builderman", "--reason", "x", "--apply", "--yes"],
            &api,
            &users,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn allow_alts_sets_the_roblox_field_that_stops_propagation() {
    // The one that was backwards: Roblox's field is `excludeAltAccounts`, and
    // `true` means "do NOT propagate". A flag named after the field would read
    // as the opposite of what it does, so the flag is `--allow-alts` and this
    // test pins the mapping.
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;

    Mock::given(method("PATCH"))
        .and(body_json(serde_json::json!({
            "gameJoinRestriction": {
                "active": true,
                "privateReason": "x",
                "excludeAltAccounts": true
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&api)
        .await;

    run(
        cli(
            &[
                "add",
                "builderman",
                "--reason",
                "x",
                "--allow-alts",
                "--apply",
                "--yes",
            ],
            &api,
            &users,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}
