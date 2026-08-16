//! Restart over HTTP.
//!
//! The one that matters is the first: a `launch` without `--apply` must fetch
//! the forecast and never POST. Everything else in this crate is reversible or
//! read-only; closing live servers is not.

use rbx_core::GlobalFlags;
use rbx_restart::{run, RestartCli};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 5544332211;

#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    restart: RestartCli,
}

fn flags(places: &str) -> GlobalFlags {
    GlobalFlags {
        api_key: Some("test-key".into()),
        cookie: None,
        no_auto_cookie: true,
        auto_cookie: false,
        env: Some("prod".into()),
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
        format!("[prod]\nuniverse_id = {UNIVERSE}\n\n[prod.places]\nmain = 1\n"),
    )
    .unwrap();
    file.to_string_lossy().into_owned()
}

fn cli(args: &[&str], server: &MockServer) -> RestartCli {
    let mut argv = vec!["restart"];
    argv.extend_from_slice(args);
    <Wrapper as clap::Parser>::parse_from(argv)
        .restart
        .with_base_url(server.uri())
}

async fn mount_forecast(server: &MockServer, players: i32, instances: i32) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/restarts:forecast"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "placeForecasts": {
                "55443322110099": {
                    "playersImpacted": players,
                    "totalPlayers": players + 100,
                    "instancesImpacted": instances,
                    "totalInstances": instances + 20,
                    "latestPlaceVersion": "3991"
                }
            }
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn launch_without_apply_reads_the_forecast_and_never_posts() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    mount_forecast(&server, 42, 7).await;
    // No POST mock: a request to launch would 404 and fail the run.

    run(cli(&["launch"], &server), &flags(&places_file(dir.path())))
        .await
        .unwrap();

    let posts = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method == wiremock::http::Method::POST)
        .count();
    assert_eq!(posts, 0, "a dry run must never close a server");
}

#[tokio::test]
async fn launch_with_apply_sends_the_bleed_off() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    mount_forecast(&server, 42, 7).await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/restarts"
        )))
        .and(body_json(
            serde_json::json!({ "bleedOffDurationMinutes": 45 }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "abc", "playersImpacted": 42, "instancesImpacted": 7
        })))
        .expect(1)
        .mount(&server)
        .await;

    run(
        cli(
            &["launch", "--bleed-off", "45", "--apply", "--yes"],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn nothing_to_restart_stops_before_asking_to_confirm() {
    // Every server already on the newest version. Prompting to close nothing,
    // or worse posting a restart that closes nothing, would both be wrong.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    mount_forecast(&server, 0, 0).await;

    run(
        cli(&["launch", "--apply", "--yes"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();

    let posts = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method == wiremock::http::Method::POST)
        .count();
    assert_eq!(posts, 0);
}

#[tokio::test]
async fn a_bleed_off_outside_roblox_bounds_fails_before_any_request() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    for bad in ["0", "241"] {
        let error = run(
            cli(&["launch", "--bleed-off", bad, "--apply", "--yes"], &server),
            &flags(&places_file(dir.path())),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("--bleed-off"), "got: {error}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn env_all_is_refused_before_anything_is_fetched() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let mut global = flags(&places_file(dir.path()));
    global.env = Some("all".into());

    let error = run(cli(&["forecast"], &server), &global)
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("--env all"), "got: {error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn status_reads_the_restart_list() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/restarts"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "restartStatuses": {
                "r1": {
                    "scheduledTime": "2026-08-03T20:00:00Z",
                    "startTime": "2026-08-03T20:30:00Z",
                    "placeRestartStatuses": { "55443322110099": { "state": "DELAYING" } }
                }
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    run(cli(&["status"], &server), &flags(&places_file(dir.path())))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_missing_scope_says_to_check_which_key_is_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_string(
            r#"{"code":"PERMISSION_DENIED","message":"The required scope <universe:write> is missing."}"#,
        ))
        .mount(&server)
        .await;

    let error = format!(
        "{:?}",
        run(
            cli(&["forecast"], &server),
            &flags(&places_file(dir.path()))
        )
        .await
        .unwrap_err()
    );
    assert!(error.contains("RBX_API_KEY"), "got: {error}");
}
