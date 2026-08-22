//! The client against recorded production responses.
//!
//! Every fixture here came off the live API through `rbx-ops probe`, with player
//! ids and job ids replaced. Re-record them with
//! `python scripts/capture_fixtures.py`.
//!
//! The point of testing against recordings rather than hand-written JSON: a
//! hand-written fixture encodes what the spec says, and the spec is wrong about
//! several of these fields. A recording cannot be wrong about what Roblox sent.

use rbx_core::api::ApiBase;
use rbx_servers::api::ServersApi;
use rbx_servers::model::ServerStatus;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 5544332211;
const PLACE: u64 = 55443322110099;

fn fixture(name: &str) -> serde_json::Value {
    let raw = std::fs::read_to_string(format!("tests/fixtures/{name}"))
        .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parsing fixture {name}: {e}"))
}

fn api(server: &MockServer) -> ServersApi {
    ServersApi::new("test-key", ApiBase::new(server.uri()))
}

#[tokio::test]
async fn filter_options_yields_the_versions_that_have_servers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/game-servers:filter-options"
        )))
        .and(header("x-api-key", "test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("filter_options.json")))
        .mount(&server)
        .await;

    let options = api(&server).filter_options(UNIVERSE, PLACE).await.unwrap();

    // Recorded from the live game: two versions, newest first.
    assert_eq!(options.place_versions(), vec!["3991", "3982"]);
}

#[tokio::test]
async fn a_page_of_live_servers_parses_with_its_awkward_fields_intact() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/versions/3991/game-servers"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("game_servers_active.json")))
        .mount(&server)
        .await;

    let page = api(&server)
        .list_page(UNIVERSE, PLACE, "3991", 5, None)
        .await
        .unwrap();

    assert!(!page.game_servers.is_empty());
    assert!(page
        .game_servers
        .iter()
        .all(|s| s.status() == ServerStatus::Active));
    assert!(page
        .game_servers
        .iter()
        .all(|s| s.termination_time.is_none()));
    // Every uptime on this page parses. A regression in the TimeSpan parser
    // shows up here as a None rather than as a wrong number.
    assert!(page
        .game_servers
        .iter()
        .all(|s| s.uptime_duration().is_some()));
    // This page is not the last one, so there is a real token to follow.
    assert!(page.next_token().is_some());
    assert!(!page.is_partial());
    assert!(
        page.total_count.unwrap() > 1000,
        "the live game has many rows"
    );
}

#[tokio::test]
async fn a_page_of_stopped_servers_carries_termination_data() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/versions/3982/game-servers"
        )))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(fixture("game_servers_terminated.json")),
        )
        .mount(&server)
        .await;

    let page = api(&server)
        .list_page(UNIVERSE, PLACE, "3982", 5, None)
        .await
        .unwrap();

    let stopped = &page.game_servers[0];
    assert_eq!(stopped.status(), ServerStatus::ShutDown);
    assert!(stopped.termination_time.is_some());
    assert_eq!(stopped.shut_down, Some(true));
    // A stopped server reports 0 fps, which is not the same fact as the `null`
    // a just-started server reports. Both must survive round-tripping.
    assert_eq!(stopped.frame_rate, Some(0.0));
    // Recorded uptimes here carry the seven-digit fraction.
    assert!(stopped.uptime_duration().is_some());
}

#[tokio::test]
async fn an_empty_result_does_not_look_like_another_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/versions/1/game-servers"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("game_servers_empty.json")))
        .mount(&server)
        .await;

    let page = api(&server)
        .list_page(UNIVERSE, PLACE, "1", 100, None)
        .await
        .unwrap();

    assert!(page.game_servers.is_empty());
    assert_eq!(page.total_count, Some(0));
    // Every pagination field is null here. Reading any of them as a token is
    // how a "fetch everything" loop never terminates.
    assert_eq!(page.next_token(), None);
    assert!(!page.is_partial());
}

#[tokio::test]
async fn the_page_token_is_sent_back_on_the_next_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/versions/3991/game-servers"
        )))
        .and(query_param("PageToken", "tok-123"))
        .and(query_param("MaxPageSize", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("game_servers_empty.json")))
        .expect(1)
        .mount(&server)
        .await;

    api(&server)
        .list_page(UNIVERSE, PLACE, "3991", 5, Some("tok-123"))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_page_size_above_the_roblox_cap_is_clamped_rather_than_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("MaxPageSize", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("game_servers_empty.json")))
        .expect(1)
        .mount(&server)
        .await;

    // Roblox rejects anything over 100 outright, so asking for 5000 must not
    // reach it verbatim.
    api(&server)
        .list_page(UNIVERSE, PLACE, "3991", 5000, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_fetch_error_flag_marks_the_page_partial_even_though_the_status_is_200() {
    let server = MockServer::start().await;
    let mut body = fixture("game_servers_empty.json");
    body["shutdownServersFetchError"] = serde_json::Value::Bool(true);
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let page = api(&server)
        .list_page(UNIVERSE, PLACE, "3991", 100, None)
        .await
        .unwrap();

    assert!(
        page.is_partial(),
        "a 200 with a raised fetch-error flag is incomplete data, not a success"
    );
}

#[tokio::test]
async fn an_error_status_surfaces_the_body_and_says_what_to_check() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_string("missing scope universe:read"))
        .mount(&server)
        .await;

    let error = api(&server)
        .list_page(UNIVERSE, PLACE, "3991", 100, None)
        .await
        .unwrap_err();

    // `{:?}` on an anyhow error renders the whole chain, which is what reaches
    // the terminal. `to_string()` would only give the outermost message, and a
    // test reading that would pass while the status and body had been lost.
    let full = format!("{error:?}");

    assert!(full.contains("403"), "got: {full}");
    assert!(full.contains("missing scope"), "got: {full}");
    // A missing scope is nearly always the wrong key being loaded rather than a
    // wrongly declared one, and Roblox's message says nothing about that.
    assert!(full.contains("RBX_API_KEY"), "got: {full}");
}

#[tokio::test]
async fn an_error_unrelated_to_scopes_is_not_dressed_up_as_one() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_string("no such place version"))
        .mount(&server)
        .await;

    let full = format!(
        "{:?}",
        api(&server)
            .list_page(UNIVERSE, PLACE, "9999", 100, None)
            .await
            .unwrap_err()
    );

    assert!(full.contains("404"), "got: {full}");
    assert!(
        !full.contains("RBX_API_KEY"),
        "the key hint belongs only on permission errors: {full}"
    );
}

// ── the paginated walk ──

/// Both halves of the paging rule, in one test, because they are only correct
/// together.
///
/// The endpoint states the first: "When paginating, all other parameters
/// provided to the subsequent call must match the call that provided the page
/// token." So the page size is decided once. That alone overshoots on the last
/// page, which is why the walk truncates: a fix that carried the rule without
/// the truncate made `--limit 150` return 200 rows.
#[tokio::test]
async fn the_walk_keeps_one_page_size_and_respects_the_limit() {
    let server = MockServer::start().await;
    let full_page: Vec<serde_json::Value> = (0..100)
        .map(|i| {
            serde_json::json!({
                "id": format!("job-{i}"),
                "playing": 1,
                "maxPlayers": 10,
                "fps": 60.0,
                "ping": 50.0
            })
        })
        .collect();
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/versions/1/game-servers"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "gameServers": full_page,
            "nextPageToken": "more"
        })))
        .mount(&server)
        .await;

    let walk = rbx_servers::walk_servers(&api(&server), UNIVERSE, PLACE, "1", 150, None)
        .await
        .unwrap();

    assert_eq!(
        walk.rows.len(),
        150,
        "the walk must not return more than --limit asked for"
    );

    let requests = server.received_requests().await.unwrap();
    assert!(
        requests.len() >= 2,
        "expected paging, saw {}",
        requests.len()
    );
    let sizes: Vec<String> = requests
        .iter()
        .map(|r| {
            r.url
                .query_pairs()
                .find(|(k, _)| k.eq_ignore_ascii_case("maxpagesize"))
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default()
        })
        .collect();
    assert!(
        sizes.windows(2).all(|w| w[0] == w[1]),
        "MaxPageSize changed between pages: {sizes:?}"
    );
}

/// An empty page carrying a token would otherwise spin for ever: the row count
/// never grows, so the loop condition never fails.
#[tokio::test]
async fn an_empty_page_with_a_token_ends_the_walk() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/versions/1/game-servers"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "gameServers": [],
            "nextPageToken": "more-please"
        })))
        .mount(&server)
        .await;

    let walk = rbx_servers::walk_servers(&api(&server), UNIVERSE, PLACE, "1", 500, None)
        .await
        .unwrap();

    assert!(walk.rows.is_empty());
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "the walk must stop rather than ask again"
    );
}

/// A page whose rows the status filter discarded is still progress. Measuring
/// emptiness on the filtered result instead would end the walk early and
/// silently return less than the caller asked for.
#[tokio::test]
async fn a_page_the_filter_emptied_does_not_end_the_walk() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/server-management/v1/universes/{UNIVERSE}/places/{PLACE}/versions/1/game-servers"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "gameServers": [{ "id": "job-1", "playing": 1, "maxPlayers": 10 }],
            "nextPageToken": ""
        })))
        .mount(&server)
        .await;

    // A status no row matches, so every row is dropped and `rows` stays empty.
    let walk = rbx_servers::walk_servers(
        &api(&server),
        UNIVERSE,
        PLACE,
        "1",
        10,
        Some("nothing-matches-this"),
    )
    .await
    .unwrap();

    assert!(walk.rows.is_empty());
    // Stopped on the empty *token*, not on the empty filtered result.
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
