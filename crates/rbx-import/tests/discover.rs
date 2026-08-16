//! Resolving a universe over HTTP.
//!
//! This is the half of `import` that talks to Roblox, and the half where a
//! wrong answer is silent: a misparsed owner writes the wrong `[owner]`, a
//! missed place leaves `--place` unable to name it, and neither shows up until
//! much later. So the requests and the parsing are asserted against a mock
//! server rather than trusted.

#![allow(clippy::unwrap_used)]

use rbx_import::discover::{fetch_places, fetch_universe, Hosts, Owner};
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 99887766554;
const ROOT_PLACE: u64 = 55501;

fn hosts(server: &MockServer) -> Hosts {
    Hosts::with_base_url(server.uri())
}

async fn mount_universe(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/universes/{UNIVERSE}")))
        .and(header("x-api-key", "test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mount_root_place(server: &MockServer, root: u64) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/universes/{UNIVERSE}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "rootPlaceId": root })))
        .mount(server)
        .await;
}

async fn mount_places(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/universes/{UNIVERSE}/places")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

// ── the universe ──

#[tokio::test]
async fn a_group_owned_universe_resolves_to_a_group_owner() {
    let server = MockServer::start().await;
    mount_universe(
        &server,
        json!({ "displayName": "Tower Defence", "group": "groups/456" }),
    )
    .await;

    let (name, owner) = fetch_universe(
        &rbx_core::api::build_client(),
        &hosts(&server).cloud,
        "test-key",
        UNIVERSE,
    )
    .await
    .unwrap();

    assert_eq!(name.as_deref(), Some("Tower Defence"));
    assert_eq!(
        owner,
        Some(Owner {
            kind: "group",
            id: 456
        })
    );
}

#[tokio::test]
async fn a_user_owned_universe_resolves_to_a_user_owner() {
    let server = MockServer::start().await;
    mount_universe(
        &server,
        json!({ "displayName": "Solo", "user": "users/123" }),
    )
    .await;

    let (_, owner) = fetch_universe(
        &rbx_core::api::build_client(),
        &hosts(&server).cloud,
        "test-key",
        UNIVERSE,
    )
    .await
    .unwrap();

    assert_eq!(
        owner,
        Some(Owner {
            kind: "user",
            id: 123
        })
    );
}

/// A universe that reports no owner still imports — `[owner]` is a
/// convenience, and only badge creation needs it later.
#[tokio::test]
async fn a_universe_without_an_owner_is_not_an_error() {
    let server = MockServer::start().await;
    mount_universe(&server, json!({ "displayName": "Anonymous" })).await;

    let (name, owner) = fetch_universe(
        &rbx_core::api::build_client(),
        &hosts(&server).cloud,
        "test-key",
        UNIVERSE,
    )
    .await
    .unwrap();

    assert_eq!(name.as_deref(), Some("Anonymous"));
    assert_eq!(owner, None);
}

/// A key without `universe.read` is the most likely first failure, and it has
/// to fail here — before anything is written to disk.
#[tokio::test]
async fn a_refused_universe_read_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/universes/{UNIVERSE}")))
        .respond_with(ResponseTemplate::new(403).set_body_string("insufficient scope"))
        .mount(&server)
        .await;

    let err = fetch_universe(
        &rbx_core::api::build_client(),
        &hosts(&server).cloud,
        "test-key",
        UNIVERSE,
    )
    .await
    .unwrap_err();

    assert!(
        err.to_string().contains(&UNIVERSE.to_string()),
        "the error must name the universe: {err:#}"
    );
}

// ── the places ──

#[tokio::test]
async fn the_root_place_comes_first_and_is_keyed_main() {
    let server = MockServer::start().await;
    mount_root_place(&server, ROOT_PLACE).await;
    mount_places(
        &server,
        json!({
            "data": [
                { "id": 77702, "name": "Lobby" },
                { "id": ROOT_PLACE, "name": "Tower Defence" },
                { "id": 77703, "name": "The Arena" }
            ],
            "nextPageCursor": ""
        }),
    )
    .await;

    let places = fetch_places(
        &rbx_core::api::build_client(),
        &hosts(&server).develop,
        None,
        UNIVERSE,
    )
    .await
    .unwrap();

    // `main` regardless of what Roblox calls it — the rest of the toolkit
    // resolves `main` as the default place.
    assert_eq!(places[0].key, "main");
    assert_eq!(places[0].id, ROOT_PLACE);

    let rest: Vec<(&str, u64)> = places[1..].iter().map(|p| (p.key.as_str(), p.id)).collect();
    assert_eq!(rest, [("lobby", 77702), ("the_arena", 77703)]);
}

/// The root place is not always in the listing. Seeding it rather than
/// searching for it is what stops the import from producing an env with no
/// `main`, which every place-scoped command then fails on.
#[tokio::test]
async fn a_root_place_missing_from_the_listing_is_still_written() {
    let server = MockServer::start().await;
    mount_root_place(&server, ROOT_PLACE).await;
    mount_places(
        &server,
        json!({ "data": [{ "id": 77702, "name": "Lobby" }], "nextPageCursor": "" }),
    )
    .await;

    let places = fetch_places(
        &rbx_core::api::build_client(),
        &hosts(&server).develop,
        None,
        UNIVERSE,
    )
    .await
    .unwrap();

    assert_eq!(places.len(), 2);
    assert_eq!((places[0].key.as_str(), places[0].id), ("main", ROOT_PLACE));
}

#[tokio::test]
async fn the_place_listing_follows_its_cursor() {
    let server = MockServer::start().await;
    mount_root_place(&server, ROOT_PLACE).await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/universes/{UNIVERSE}/places")))
        .and(query_param("cursor", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "id": 77703, "name": "Arena" }],
            "nextPageCursor": ""
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/universes/{UNIVERSE}/places")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "id": 77702, "name": "Lobby" }],
            "nextPageCursor": "page2"
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let places = fetch_places(
        &rbx_core::api::build_client(),
        &hosts(&server).develop,
        None,
        UNIVERSE,
    )
    .await
    .unwrap();

    let keys: Vec<&str> = places.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(
        keys,
        ["main", "lobby", "arena"],
        "a dropped page is a lost place"
    );
}

/// A private universe answers only with a session cookie, so it has to be sent
/// when one is available.
#[tokio::test]
async fn a_cookie_is_forwarded_to_the_legacy_host() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/universes/{UNIVERSE}")))
        .and(header("cookie", ".ROBLOSECURITY=secret"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "rootPlaceId": ROOT_PLACE })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/universes/{UNIVERSE}/places")))
        .and(header("cookie", ".ROBLOSECURITY=secret"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": [], "nextPageCursor": "" })),
        )
        .mount(&server)
        .await;

    let places = fetch_places(
        &rbx_core::api::build_client(),
        &hosts(&server).develop,
        Some(".ROBLOSECURITY=secret"),
        UNIVERSE,
    )
    .await
    .unwrap();

    assert_eq!(places.len(), 1, "only the root place, but resolved");
}

/// A universe id that resolves to no root place cannot produce a usable env,
/// so it fails with something actionable rather than writing a broken one.
///
/// The hint must point at the id and must **not** suggest `--cookie`. This
/// endpoint answers 200 on a private universe with no credential at all, so a
/// cookie is never the missing piece, and the old wording sent the reader to
/// fix the one thing that was never wrong.
#[tokio::test]
async fn a_universe_with_no_root_place_fails_with_a_hint() {
    let server = MockServer::start().await;
    mount_root_place(&server, 0).await;

    let err = fetch_places(
        &rbx_core::api::build_client(),
        &hosts(&server).develop,
        None,
        UNIVERSE,
    )
    .await
    .unwrap_err();

    let text = format!("{err:#}");
    assert!(
        text.contains("Check the id"),
        "the hint must point at the id: {text}"
    );
    assert!(
        !text.contains("--cookie"),
        "the hint must not send the reader after a credential this call never needed: {text}"
    );
}
