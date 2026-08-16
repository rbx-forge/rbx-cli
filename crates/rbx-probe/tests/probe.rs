//! HTTP-level tests for `rbx-ops probe`, against a `wiremock` server.
//!
//! These are the first tests in the suite that exercise a domain crate's
//! request over real HTTP rather than its argument parsing. They are only
//! possible because the host is injectable (`--base-url` / `ApiBase`); the
//! existing crates build their URLs from inline literals and cannot be
//! pointed anywhere.

use rbx_core::GlobalFlags;
use rbx_probe::{run, ProbeCli};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn flags(api_key: Option<&str>) -> GlobalFlags {
    GlobalFlags {
        api_key: api_key.map(str::to_string),
        cookie: None,
        no_auto_cookie: true,
        auto_cookie: false,
        env: None,
        place: None,
        places: "rbxplace.toml".into(),
        universe_id: None,
        place_id: Vec::new(),
    }
}

fn probe(server: &MockServer, path: &str) -> ProbeCli {
    ProbeCli {
        path: path.into(),
        method: "GET".into(),
        data: None,
        apply: false,
        base_url: Some(server.uri()),
    }
}

#[tokio::test]
async fn a_get_sends_the_api_key_as_x_api_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/cloud/v2/users/1"))
        .and(header("x-api-key", "secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": 1 })))
        .expect(1)
        .mount(&server)
        .await;

    run(probe(&server, "/cloud/v2/users/1"), &flags(Some("secret")))
        .await
        .unwrap();
    // `expect(1)` is asserted when the server drops at end of test.
}

#[tokio::test]
async fn a_write_without_apply_sends_nothing() {
    let server = MockServer::start().await;
    // No mock is mounted: any request at all would 404 and fail the run.
    let cli = ProbeCli {
        method: "POST".into(),
        data: Some(r#"{"a":1}"#.into()),
        ..probe(&server, "/cloud/v2/users/1")
    };

    run(cli, &flags(Some("secret"))).await.unwrap();
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a write without --apply must not reach the network"
    );
}

#[tokio::test]
async fn a_write_with_apply_sends_the_parsed_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/cloud/v2/users/1"))
        .and(body_json(serde_json::json!({ "a": 1 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })))
        .expect(1)
        .mount(&server)
        .await;

    let cli = ProbeCli {
        method: "POST".into(),
        data: Some(r#"{"a":1}"#.into()),
        apply: true,
        ..probe(&server, "/cloud/v2/users/1")
    };

    run(cli, &flags(Some("secret"))).await.unwrap();
}

#[tokio::test]
async fn a_malformed_body_fails_before_any_request_is_made() {
    let server = MockServer::start().await;
    let cli = ProbeCli {
        method: "POST".into(),
        data: Some("{not json".into()),
        apply: true,
        ..probe(&server, "/cloud/v2/users/1")
    };

    let err = run(cli, &flags(Some("secret"))).await.unwrap_err();
    assert!(err.to_string().contains("--data"), "got: {err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_api_key_fails_before_any_request_is_made() {
    let server = MockServer::start().await;
    let err = run(probe(&server, "/cloud/v2/users/1"), &flags(None))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("--api-key"), "got: {err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_429_is_retried_and_the_retry_succeeds() {
    let server = MockServer::start().await;
    // wiremock serves mounted mocks in order of registration, so the first
    // request gets the 429 and the second the 200.
    Mock::given(method("GET"))
        .and(path("/cloud/v2/users/1"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/cloud/v2/users/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": 1 })))
        .expect(1)
        .mount(&server)
        .await;

    run(probe(&server, "/cloud/v2/users/1"), &flags(Some("secret")))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_404_surfaces_the_status_and_the_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/cloud/v2/nope"))
        .respond_with(ResponseTemplate::new(404).set_body_string("no such endpoint"))
        .mount(&server)
        .await;

    let err = run(probe(&server, "/cloud/v2/nope"), &flags(Some("secret")))
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("404"), "got: {message}");
    assert!(message.contains("no such endpoint"), "got: {message}");
}
