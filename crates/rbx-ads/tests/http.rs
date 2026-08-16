//! Ads over HTTP, against a mock.
//!
//! The unit tests next door prove the money conversion and the idempotency key.
//! These prove the parts only a request can show: that `launch` sends nothing
//! without `--apply`, that it sends one campaign per creative with the same
//! budget in each, and that the required idempotency header is on every create.

use rbx_ads::{run, AdsCli};
use rbx_core::GlobalFlags;
use wiremock::matchers::{body_partial_json, header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 5544332211;

#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    ads: AdsCli,
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

fn cli(args: &[&str], server: &MockServer) -> AdsCli {
    let mut argv = vec!["ads"];
    argv.extend_from_slice(args);
    <Wrapper as clap::Parser>::parse_from(argv)
        .ads
        .with_base_url(server.uri())
}

fn created(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": "whatever",
        "status": "ACTIVE",
        "deliveryStatus": "IN_REVIEW",
        "creativeAssetIds": ["1"],
        "targetUniverseId": UNIVERSE.to_string(),
    })
}

#[tokio::test]
async fn launch_without_apply_sends_nothing() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    // No mock is mounted. Any request at all fails the test.
    let cli = cli(
        &[
            "launch",
            "--creative",
            "111",
            "--creative",
            "222",
            "--name",
            "icon test",
            "--budget",
            "25",
        ],
        &server,
    );

    run(cli, &flags(&places)).await.unwrap();
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a dry run reached the network"
    );
}

#[tokio::test]
async fn launch_creates_one_campaign_per_creative() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    Mock::given(method("POST"))
        .and(path("/ads-management/v1/campaigns"))
        .and(header("x-api-key", "test-key"))
        .and(header_exists("x-idempotency-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(created("c1")))
        .expect(3)
        .mount(&server)
        .await;

    let cli = cli(
        &[
            "launch",
            "--creative",
            "111",
            "--creative",
            "222",
            "--creative",
            "333",
            "--name",
            "icon test",
            "--budget",
            "25.50",
            "--days",
            "14",
            "--country",
            "US",
            "--apply",
            "--yes",
        ],
        &server,
    );

    run(cli, &flags(&places)).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);

    let bodies: Vec<serde_json::Value> = requests
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();

    // One creative each, and the asset id is in the name: that name is the
    // only thread back to the creative when the numbers are read in Ads
    // Manager.
    let mut assets: Vec<String> = bodies
        .iter()
        .map(|b| b["creativeAssetIds"][0].as_str().unwrap().to_owned())
        .collect();
    assets.sort();
    assert_eq!(assets, ["111", "222", "333"]);
    for body in &bodies {
        let asset = body["creativeAssetIds"][0].as_str().unwrap();
        assert_eq!(body["creativeAssetIds"].as_array().unwrap().len(), 1);
        assert!(body["name"].as_str().unwrap().contains(asset));

        // Everything else has to be identical, or the comparison is between
        // two campaigns rather than two images.
        assert_eq!(body["budget"]["amountMicros"], "25500000");
        assert_eq!(body["budget"]["type"], "DAILY");
        assert_eq!(body["schedule"]["durationInDays"], 14);
        assert_eq!(body["targeting"]["countries"][0], "US");
        assert_eq!(body["objective"], "ENGAGEMENT");
        assert_eq!(body["bid"]["strategy"], "AUTOMATED");
        assert_eq!(body["targetUniverseId"], UNIVERSE.to_string());
    }

    // A distinct key per campaign, or Roblox would fold the three into one.
    let mut keys: Vec<String> = requests
        .iter()
        .map(|r| {
            r.headers
                .get("x-idempotency-key")
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), 3);
}

#[tokio::test]
async fn a_creative_listed_twice_is_refused_before_any_spend() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    let cli = cli(
        &[
            "launch",
            "--creative",
            "111",
            "--creative",
            "111",
            "--name",
            "icon test",
            "--budget",
            "25",
            "--apply",
            "--yes",
        ],
        &server,
    );

    let error = run(cli, &flags(&places)).await.unwrap_err();
    assert!(error.to_string().contains("listed twice"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_budget_that_is_not_money_is_refused_before_any_spend() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    let cli = cli(
        &[
            "launch",
            "--creative",
            "111",
            "--name",
            "icon test",
            "--budget",
            "twenty",
            "--apply",
            "--yes",
        ],
        &server,
    );

    assert!(run(cli, &flags(&places)).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn pause_without_apply_reads_the_campaign_but_changes_nothing() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    // The read is the point: a confirmation that only echoed the id would give
    // nobody a way to notice they are pausing the wrong campaign.
    Mock::given(method("GET"))
        .and(path("/ads-management/v1/campaigns/c1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(created("c1")))
        .mount(&server)
        .await;

    let cli = cli(&["pause", "c1"], &server);
    run(cli, &flags(&places)).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert!(
        requests.iter().all(|r| r.method.as_str() == "GET"),
        "a dry run wrote something"
    );
}

#[tokio::test]
async fn pause_with_apply_patches_the_status() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    Mock::given(method("GET"))
        .and(path("/ads-management/v1/campaigns/c1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(created("c1")))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/ads-management/v1/campaigns/c1"))
        .and(body_partial_json(serde_json::json!({ "status": "PAUSED" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(created("c1")))
        .expect(1)
        .mount(&server)
        .await;

    let cli = cli(&["pause", "c1", "--apply", "--yes"], &server);
    run(cli, &flags(&places)).await.unwrap();
}

#[tokio::test]
async fn a_name_pauses_the_whole_launch_group_and_leaves_the_rest() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    Mock::given(method("GET"))
        .and(path("/ads-management/v1/campaigns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "campaigns": [
                { "id": "c1", "name": "icon test [111]", "status": "ACTIVE" },
                { "id": "c2", "name": "icon test [222]", "status": "ACTIVE" },
                { "id": "c3", "name": "summer push", "status": "ACTIVE" },
                { "id": "c4", "name": "icon test [333]", "status": "CANCELLED" },
            ],
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(200).set_body_json(created("c1")))
        .expect(2)
        .mount(&server)
        .await;

    let cli = cli(
        &["pause", "--name", "icon test", "--apply", "--yes"],
        &server,
    );
    run(cli, &flags(&places)).await.unwrap();

    let patched: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "PATCH")
        .map(|r| r.url.path().rsplit('/').next().unwrap().to_owned())
        .collect();

    // The unrelated campaign is untouched, and the already-cancelled one is
    // not asked to change state again.
    assert_eq!(patched, ["c1", "c2"]);
}

#[tokio::test]
async fn list_follows_every_page() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    Mock::given(method("GET"))
        .and(path("/ads-management/v1/campaigns"))
        .and(wiremock::matchers::query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "campaigns": [{ "id": "c1", "name": "one", "status": "ACTIVE" }],
            "nextPageToken": "page-2",
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ads-management/v1/campaigns"))
        .and(wiremock::matchers::query_param("pageToken", "page-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "campaigns": [{ "id": "c2", "name": "two", "status": "ACTIVE" }],
        })))
        .expect(1)
        .mount(&server)
        .await;

    let cli = cli(&["list"], &server);
    run(cli, &flags(&places)).await.unwrap();
}

#[tokio::test]
async fn without_a_terminal_a_missing_id_is_an_error_not_a_prompt() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    Mock::given(method("GET"))
        .and(path("/ads-management/v1/campaigns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "campaigns": [{ "id": "c1", "name": "one", "status": "ACTIVE" }],
        })))
        .mount(&server)
        .await;

    // Tests do not run on a terminal, which is exactly the case being checked:
    // a script that omits the id has to fail rather than hang on a prompt.
    let cli = cli(&["pause", "--apply", "--yes"], &server);
    let error = run(cli, &flags(&places)).await.unwrap_err();
    assert!(error.to_string().contains("no terminal"), "{error}");
}

#[tokio::test]
async fn without_a_terminal_a_missing_creative_is_an_error_not_a_prompt() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    let cli = cli(
        &["launch", "--name", "icon test", "--budget", "25"],
        &server,
    );
    let error = run(cli, &flags(&places)).await.unwrap_err();
    assert!(error.to_string().contains("--creative"), "{error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn rename_changes_the_name_and_nothing_else() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    Mock::given(method("GET"))
        .and(path("/ads-management/v1/campaigns/c1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(created("c1")))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/ads-management/v1/campaigns/c1"))
        .and(body_partial_json(
            serde_json::json!({ "name": "icon test v2 [111]" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(created("c1")))
        .expect(1)
        .mount(&server)
        .await;

    let cli = cli(
        &[
            "rename",
            "c1",
            "--name",
            "icon test v2 [111]",
            "--apply",
            "--yes",
        ],
        &server,
    );
    run(cli, &flags(&places)).await.unwrap();

    let body: serde_json::Value = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .find(|r| r.method.as_str() == "PATCH")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .unwrap();
    assert!(body.get("status").is_none(), "rename touched delivery");
    assert!(body.get("budget").is_none(), "rename touched the budget");
}

#[tokio::test]
async fn a_rejection_is_surfaced_rather_than_swallowed() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    Mock::given(method("POST"))
        .and(path("/ads-management/v1/campaigns"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "title": "Forbidden",
            "detail": "the key is missing the ad.campaign:write scope",
        })))
        .mount(&server)
        .await;

    let cli = cli(
        &[
            "launch",
            "--creative",
            "111",
            "--name",
            "icon test",
            "--budget",
            "25",
            "--apply",
            "--yes",
        ],
        &server,
    );

    let error = run(cli, &flags(&places)).await.unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("scope"), "unhelpful error: {text}");
}

#[tokio::test]
async fn status_asks_about_every_id_in_one_call() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());

    Mock::given(method("POST"))
        .and(path("/ads-management/v1/campaigns:batchGetStatus"))
        .and(body_partial_json(
            serde_json::json!({ "campaignIds": ["c1", "c2"] }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "statuses": [
                { "id": "c1", "status": "ACTIVE", "deliveryStatus": "SERVING" },
                { "id": "c2", "status": "ACTIVE", "deliveryStatus": "REJECTED",
                  "deliveryStatusReasons": ["creative rejected"] }
            ],
            "failures": [],
        })))
        .expect(1)
        .mount(&server)
        .await;

    let cli = cli(&["status", "c1", "c2"], &server);
    run(cli, &flags(&places)).await.unwrap();
}
