//! Analytics over HTTP, against a mock.
//!
//! The fixture tests next door prove the response parses. These prove the
//! request is built correctly and that a rejection is surfaced with the message
//! Roblox put in it, which is the part a parsing test cannot see.

use rbx_analytics::{run, AnalyticsCli};
use rbx_core::GlobalFlags;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 5544332211;

#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    analytics: AnalyticsCli,
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

fn cli(args: &[&str], server: &MockServer) -> AnalyticsCli {
    let mut argv = vec!["analytics"];
    argv.extend_from_slice(args);
    <Wrapper as clap::Parser>::parse_from(argv)
        .analytics
        .with_base_url(server.uri())
}

fn fixture(name: &str) -> serde_json::Value {
    let raw = std::fs::read_to_string(format!("tests/fixtures/{name}"))
        .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"));
    serde_json::from_str(&raw).unwrap()
}

#[tokio::test]
async fn a_query_posts_the_metric_and_granularity_with_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/analytics-query-api/v1/universes/{UNIVERSE}/metrics"
        )))
        .and(header("x-api-key", "test-key"))
        .and(body_partial_json(serde_json::json!({
            "metric": "DailyActiveUsers",
            "granularity": "OneDay"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("analytics_metrics.json")))
        .expect(1)
        .mount(&server)
        .await;

    run(
        cli(
            &["query", "--metric", "DailyActiveUsers", "--days", "7"],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_granularity_is_spelled_the_way_roblox_expects_not_the_way_it_is_typed() {
    // The flag is `--granularity half-hour`, kebab-case as clap wants it, and
    // Roblox accepts only `HalfHour`. A regression here is a 400 nobody can
    // read.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(
            serde_json::json!({ "granularity": "HalfHour" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("analytics_metrics.json")))
        .expect(1)
        .mount(&server)
        .await;

    run(
        cli(
            &[
                "query",
                "--metric",
                "Visits",
                "--granularity",
                "half-hour",
                "--days",
                "1",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_rejected_metric_surfaces_roblox_s_own_message() {
    // Roblox answers 400 with the operation envelope, and the message inside
    // names the bad metric. Reporting only "API error 400" would throw away
    // the one useful part.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(fixture("analytics_error.json")))
        .mount(&server)
        .await;

    let error = format!(
        "{:?}",
        run(
            cli(
                &["query", "--metric", "NoSuchMetric", "--days", "7"],
                &server
            ),
            &flags(&places_file(dir.path())),
        )
        .await
        .unwrap_err()
    );

    assert!(error.contains("NoSuchMetric"), "got: {error}");
    assert!(error.contains("was not found"), "got: {error}");
}

#[tokio::test]
async fn a_breakdown_is_sent_and_an_absent_one_is_omitted_entirely() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(
            serde_json::json!({ "breakdown": ["Platform"] }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("analytics_metrics.json")))
        .expect(1)
        .mount(&server)
        .await;

    run(
        cli(
            &[
                "query",
                "--metric",
                "Visits",
                "--days",
                "7",
                "--breakdown",
                "Platform",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_nonsense_day_range_fails_before_any_request() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    let error = run(
        cli(&["query", "--metric", "Visits", "--days", "0"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("--days"), "got: {error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn env_all_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let mut global = flags(&places_file(dir.path()));
    global.env = Some("all".into());

    let error = run(
        cli(&["query", "--metric", "Visits", "--days", "7"], &server),
        &global,
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("--env all"), "got: {error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_scope_says_to_check_which_key_is_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(403).set_body_string(
            r#"{"code":"PERMISSION_DENIED","message":"The required scope <universe.analytics:read> is missing."}"#,
        ))
        .mount(&server)
        .await;

    let error = format!(
        "{:?}",
        run(
            cli(&["query", "--metric", "Visits", "--days", "7"], &server),
            &flags(&places_file(dir.path())),
        )
        .await
        .unwrap_err()
    );

    assert!(error.contains("RBX_API_KEY"), "got: {error}");
}

// ---------------- filters, polling, dimension values ----------------

const METRICS_PATH: &str = "/analytics-query-api/v1/universes/5544332211/metrics";
const DIMENSIONS_PATH: &str = "/analytics-query-api/v1/universes/5544332211/dimension-values";

#[tokio::test]
async fn a_filter_is_sent_as_an_in_clause_on_the_named_dimension() {
    // The whole point of --filter: FunnelName is filter-only on Roblox's side,
    // so this is the only way to ask about one funnel rather than all of them.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(METRICS_PATH))
        .and(body_partial_json(serde_json::json!({
            "metric": "FunnelUserTotalCount",
            "filter": [{ "dimension": "FunnelName", "values": ["Tutorial"], "operation": "In" }]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "done": true,
            "response": { "values": [] }
        })))
        .expect(1)
        .mount(&server)
        .await;

    run(
        cli(
            &[
                "query",
                "--metric",
                "FunnelUserTotalCount",
                "--filter",
                "FunnelName=Tutorial",
                "--granularity",
                "none",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_malformed_filter_fails_before_any_request() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    // No mock: reaching the network would 404 and fail differently.

    let err = run(
        cli(
            &["query", "--metric", "Visits", "--filter", "FunnelName"],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("--filter"), "got: {err}");
    assert_eq!(server.received_requests().await.unwrap().len(), 0);
}

#[tokio::test]
async fn an_unfinished_query_is_polled_until_it_completes() {
    // Measured against the live API: a 365-day funnel query comes back
    // `done: false` with a path. The command used to bail and tell the user to
    // poll it themselves, with no command to do so.
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path(METRICS_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "done": false,
            "path": "v1/universes/5544332211/operations/metrics/abc123"
        })))
        .mount(&server)
        .await;

    // The path is relative to the service, not to the host.
    Mock::given(method("GET"))
        .and(path(
            "/analytics-query-api/v1/universes/5544332211/operations/metrics/abc123",
        ))
        .and(header("x-api-key", "test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "done": true,
            "response": {
                "values": [{
                    "dataPoints": [{ "time": "2026-08-01T00:00:00Z", "value": 42.0 }]
                }]
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    run(
        cli(
            &[
                "query",
                "--metric",
                "FunnelUserTotalCount",
                "--granularity",
                "none",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn dimension_values_posts_the_metric_that_gives_them_meaning() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(DIMENSIONS_PATH))
        .and(header("x-api-key", "test-key"))
        .and(body_partial_json(serde_json::json!({
            "metric": "FunnelUserTotalCount",
            "dimensions": ["FunnelName"]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "done": true,
            "response": {
                "values": [{
                    "dimension": "FunnelName",
                    "values": [
                        { "value": "Tutorial" },
                        { "value": "step-7f3a", "displayValue": "Shop opened" }
                    ]
                }]
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    run(
        cli(
            &[
                "dimensions",
                "--metric",
                "FunnelUserTotalCount",
                "--dimension",
                "FunnelName",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn asking_for_no_dimension_fails_before_any_request() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    let err = run(
        cli(&["dimensions", "--metric", "FunnelUserTotalCount"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("--dimension"), "got: {err}");
    assert_eq!(server.received_requests().await.unwrap().len(), 0);
}
