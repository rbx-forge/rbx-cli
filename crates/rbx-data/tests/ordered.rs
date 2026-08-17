#![allow(clippy::unwrap_used)]
//! `rbx data ordered` over HTTP.
//!
//! The properties worth pinning are the ones a reader cannot check by looking:
//! that the ordering and the filtering happen on Roblox's side rather than
//! after the fact, that paging stops, and that a write that was not confirmed
//! never leaves.
//!
//! Driven through `run` with a mock server, like `data.rs`, so the assertions
//! are about requests on the wire.

use rbx_core::GlobalFlags;
use rbx_data::{run, DataCli};
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const UNIVERSE: u64 = 66778899001;
const ENTRIES: &str =
    "/cloud/v2/universes/66778899001/ordered-data-stores/Highscores/scopes/global/entries";

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
    let mut argv = vec!["data", "--datastore", "Highscores", "ordered"];
    argv.extend_from_slice(args);
    <Wrapper as clap::Parser>::parse_from(argv)
        .data
        .with_base_url(server.uri())
}

/// Run a command against the mock server and return the requests it made.
async fn exec(args: &[&str], server: &MockServer) -> (anyhow::Result<()>, Vec<Request>) {
    let dir = tempfile::tempdir().unwrap();
    let places = places_file(dir.path());
    let result = run(cli(args, server), &flags(&places)).await;
    let requests = server.received_requests().await.unwrap();
    (result, requests)
}

fn entry(id: &str, value: f64) -> serde_json::Value {
    serde_json::json!({ "id": id, "value": value })
}

async fn mount_page(server: &MockServer, entries: serde_json::Value, next: &str) {
    Mock::given(method("GET"))
        .and(path(ENTRIES))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "orderedDataStoreEntries": entries,
            "nextPageToken": next,
        })))
        .mount(server)
        .await;
}

// ── list ──

/// Descending is the default and it is asked for by name, not applied locally.
/// Sorting a page after it arrives would give the top of page one rather than
/// the top of the store.
#[tokio::test]
async fn a_listing_asks_roblox_to_order_it_descending() {
    let server = MockServer::start().await;
    mount_page(&server, serde_json::json!([entry("a", 10.0)]), "").await;

    let (result, requests) = exec(&["list"], &server).await;

    result.unwrap();
    let query = requests[0].url.query().unwrap();
    assert!(query.contains("orderBy=value"), "{query}");
    assert!(query.contains("desc"), "{query}");
}

#[tokio::test]
async fn asc_drops_the_order_by() {
    let server = MockServer::start().await;
    mount_page(&server, serde_json::json!([entry("a", 10.0)]), "").await;

    let (result, requests) = exec(&["list", "--asc"], &server).await;

    result.unwrap();
    assert!(!requests[0].url.query().unwrap().contains("orderBy"));
}

/// Both bounds in one filter expression, with the `&&` the grammar wants.
#[tokio::test]
async fn min_and_max_become_one_server_side_filter() {
    let server = MockServer::start().await;
    mount_page(&server, serde_json::json!([]), "").await;

    let (result, requests) = exec(&["list", "--min", "100", "--max", "500"], &server).await;

    result.unwrap();
    let filter = requests[0]
        .url
        .query_pairs()
        .find(|(k, _)| k == "filter")
        .map(|(_, v)| v.into_owned())
        .expect("a filter");
    assert_eq!(filter, "entry >= 100 && entry <= 500");
}

/// A range nothing can satisfy is a mistake worth naming rather than an empty
/// listing to puzzle over.
#[tokio::test]
async fn an_inverted_range_is_refused_before_any_request() {
    let server = MockServer::start().await;

    let (result, requests) = exec(&["list", "--min", "500", "--max", "100"], &server).await;

    assert!(result.is_err());
    assert!(requests.is_empty(), "nothing should have been sent");
}

/// The page size asked for is the smaller of the cap and what is still needed,
/// so `--limit 3` does not fetch a hundred rows to print three.
#[tokio::test]
async fn the_page_size_never_exceeds_the_limit() {
    let server = MockServer::start().await;
    mount_page(&server, serde_json::json!([entry("a", 1.0)]), "").await;

    let (result, requests) = exec(&["list", "--limit", "3"], &server).await;

    result.unwrap();
    assert!(requests[0].url.query().unwrap().contains("maxPageSize=3"));
}

/// A token pointing at an empty page is how a paginating loop runs for ever.
/// Roblox has been seen to return one at the end of a filtered listing.
#[tokio::test]
async fn an_empty_page_with_a_token_stops_the_loop() {
    let server = MockServer::start().await;
    mount_page(&server, serde_json::json!([]), "more-please").await;

    let (result, requests) = exec(&["list", "--limit", "50"], &server).await;

    result.unwrap();
    assert_eq!(requests.len(), 1, "should not have asked for a second page");
}

// ── get ──

/// A key nobody has written is not an error, matching the standard-store
/// `get`. A cleanup script that checks before acting must not have to treat
/// "absent" as a failure.
#[tokio::test]
async fn a_missing_entry_is_not_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{ENTRIES}/nobody")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let (result, _) = exec(&["get", "nobody"], &server).await;

    result.unwrap();
}

// ── set ──

/// The write carries the value and asks Roblox to create the entry when it is
/// absent, which is what makes `set` usable on a fresh leaderboard.
#[tokio::test]
async fn set_sends_the_value_and_allows_creation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{ENTRIES}/Player_1")))
        .respond_with(ResponseTemplate::new(200).set_body_json(entry("Player_1", 5.0)))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("{ENTRIES}/Player_1")))
        .and(query_param("allowMissing", "true"))
        .and(body_json(serde_json::json!({ "value": 42 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(entry("Player_1", 42.0)))
        .expect(1)
        .mount(&server)
        .await;

    let (result, _) = exec(&["set", "Player_1", "42", "--yes"], &server).await;

    result.unwrap();
}

/// `--no-create` has to reach the request, not just the local check: the entry
/// could appear between the read and the write.
#[tokio::test]
async fn no_create_turns_off_allow_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{ENTRIES}/Player_1")))
        .respond_with(ResponseTemplate::new(200).set_body_json(entry("Player_1", 5.0)))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("{ENTRIES}/Player_1")))
        .and(query_param("allowMissing", "false"))
        .respond_with(ResponseTemplate::new(200).set_body_json(entry("Player_1", 42.0)))
        .expect(1)
        .mount(&server)
        .await;

    let (result, _) = exec(&["set", "Player_1", "42", "--no-create", "--yes"], &server).await;

    result.unwrap();
}

/// Asking to not create an entry that does not exist is answered locally,
/// before a request Roblox would reject anyway.
#[tokio::test]
async fn no_create_on_a_missing_entry_fails_without_writing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{ENTRIES}/ghost")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let (result, requests) = exec(&["set", "ghost", "1", "--no-create", "--yes"], &server).await;

    assert!(result.is_err());
    assert!(
        requests.iter().all(|r| r.method.as_str() == "GET"),
        "nothing should have been written"
    );
}

// ── increment ──

/// The atomic path. It sends an amount, not a value, and it does not read
/// first: reading would reintroduce the lost update this endpoint exists to
/// avoid.
#[tokio::test]
async fn increment_posts_an_amount_and_reads_nothing_first() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("{ENTRIES}/Player_1:increment")))
        .and(body_json(serde_json::json!({ "amount": -5 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(entry("Player_1", 95.0)))
        .expect(1)
        .mount(&server)
        .await;

    let (result, requests) = exec(&["increment", "Player_1", "-5", "--yes"], &server).await;

    result.unwrap();
    assert!(
        requests.iter().all(|r| r.method.as_str() == "POST"),
        "increment must not read before writing"
    );
}

// ── delete ──

#[tokio::test]
async fn deleting_a_missing_entry_is_a_no_op_not_a_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{ENTRIES}/ghost")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let (result, requests) = exec(&["delete", "ghost", "--yes"], &server).await;

    result.unwrap();
    assert!(requests.iter().all(|r| r.method.as_str() == "GET"));
}

#[tokio::test]
async fn deleting_an_existing_entry_sends_the_delete() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{ENTRIES}/Player_1")))
        .respond_with(ResponseTemplate::new(200).set_body_json(entry("Player_1", 7.0)))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{ENTRIES}/Player_1")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let (result, _) = exec(&["delete", "Player_1", "--yes"], &server).await;

    result.unwrap();
}

// ── authentication ──

/// Every request is authenticated. An unauthenticated one would 401 in
/// production and read here as a mock-shape problem.
#[tokio::test]
async fn every_request_carries_the_api_key() {
    let server = MockServer::start().await;
    mount_page(&server, serde_json::json!([entry("a", 1.0)]), "").await;

    let (result, requests) = exec(&["list"], &server).await;

    result.unwrap();
    assert!(!requests.is_empty());
    for request in &requests {
        assert_eq!(
            request
                .headers
                .get("x-api-key")
                .map(|v| v.to_str().unwrap()),
            Some("test-key")
        );
    }
}

/// The page size is decided once and repeated, not recomputed as the remaining
/// count falls.
///
/// The spec is explicit: "When paginating, all other parameters provided to the
/// subsequent call must match the call that provided the page token." A second
/// page asking for a different `maxPageSize` than the call that issued its
/// token is a request Roblox may reject, and the failure would only appear on
/// listings long enough to page — which is exactly the listing nobody tests by
/// hand.
#[tokio::test]
async fn every_page_asks_for_the_same_size() {
    let server = MockServer::start().await;
    let full_page: Vec<serde_json::Value> = (0..100)
        .map(|i| entry(&format!("p{i}"), i as f64))
        .collect();
    mount_page(&server, serde_json::json!(full_page), "next-page").await;

    let (result, requests) = exec(&["list", "--limit", "150"], &server).await;

    result.unwrap();
    assert!(
        requests.len() >= 2,
        "expected paging, got {}",
        requests.len()
    );

    let sizes: Vec<String> = requests
        .iter()
        .map(|r| {
            r.url
                .query_pairs()
                .find(|(k, _)| k == "maxPageSize")
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default()
        })
        .collect();
    assert!(
        sizes.windows(2).all(|w| w[0] == w[1]),
        "maxPageSize changed between pages: {sizes:?}"
    );
}
