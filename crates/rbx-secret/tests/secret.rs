//! The secrets store over HTTP.
//!
//! The tests worth having here are the ones covering what would otherwise cost
//! a real request — or a real leaked credential — to learn:
//!
//! - **The ciphertext is a genuine sealed box.** Every write is decrypted with
//!   the mock universe's private key and compared against the plaintext. A
//!   test asserting "the body has a `secret` field" would pass just as happily
//!   on base64 of the value itself, which is the one bug in this crate that
//!   could not be walked back once it had run against a real universe.
//! - **Create and update are one command and two requests.** `POST`, and on
//!   `409` a `PATCH` whose body carries no `id`.
//! - **A dry run sends nothing at all.** Asserted by mounting no mock: any
//!   request would 404 and fail the run.

#![allow(clippy::unwrap_used)]

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use crypto_box::aead::OsRng;
use crypto_box::SecretKey;
use rbx_core::GlobalFlags;
use rbx_secret::{run, SecretCli};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const UNIVERSE: u64 = 66778899001;
const SECRETS_PATH: &str = "/cloud/v2/universes/66778899001/secrets";
const PUBLIC_KEY_PATH: &str = "/cloud/v2/universes/66778899001/secrets/public-key";
const DISCORD_PATH: &str = "/cloud/v2/universes/66778899001/secrets/discord";
const KEY_ID: &str = "key-2026-08";

#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    secret: SecretCli,
}

fn flags() -> GlobalFlags {
    GlobalFlags {
        api_key: Some("test-key".into()),
        cookie: None,
        no_auto_cookie: true,
        auto_cookie: false,
        env: None,
        place: None,
        places: "rbxplace.toml".into(),
        // Named directly, so these tests do not depend on a places file.
        universe_id: Some(UNIVERSE),
        place_id: Vec::new(),
    }
}

fn cli(args: &[&str], server: &MockServer) -> SecretCli {
    let mut argv = vec!["secret"];
    argv.extend_from_slice(args);
    <Wrapper as clap::Parser>::parse_from(argv)
        .secret
        .with_base_url(server.uri())
}

/// Stand in for Roblox: a real X25519 keypair whose private half stays in the
/// test, so a write can be opened and checked rather than merely shaped.
fn universe_keypair() -> SecretKey {
    SecretKey::generate(&mut OsRng)
}

async fn mount_public_key(server: &MockServer, private: &SecretKey) {
    Mock::given(method("GET"))
        .and(path(PUBLIC_KEY_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "public-key",
            "secret": STANDARD.encode(private.public_key().as_bytes()),
            "key_id": KEY_ID,
        })))
        .mount(server)
        .await;
}

/// The plaintext behind the `secret` field of a captured request.
fn opened(private: &SecretKey, request: &Request) -> Vec<u8> {
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    let sealed = body["secret"].as_str().expect("a write carries `secret`");
    let ciphertext = STANDARD.decode(sealed).expect("the content is base64");
    private
        .unseal(&ciphertext)
        .expect("the content is a sealed box this universe can open")
}

#[tokio::test]
async fn set_without_apply_sends_nothing() {
    let server = MockServer::start().await;
    // No mock at all, not even for the public key: a dry run that fetched one
    // would 404 here and fail the run.
    let cli = cli(
        &[
            "set",
            "discord",
            "--value",
            "hunter2",
            "--domain",
            "discord.com",
        ],
        &server,
    );

    run(cli, &flags()).await.unwrap();

    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_new_secret_is_posted_sealed_and_never_in_the_clear() {
    let server = MockServer::start().await;
    let private = universe_keypair();
    mount_public_key(&server, &private).await;
    Mock::given(method("POST"))
        .and(path(SECRETS_PATH))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "discord", "key_id": KEY_ID, "domain": "discord.com",
        })))
        .mount(&server)
        .await;

    let cli = cli(
        &[
            "set",
            "discord",
            "--value",
            "hunter2",
            "--domain",
            "discord.com",
            "--apply",
        ],
        &server,
    );
    run(cli, &flags()).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let post = requests
        .iter()
        .find(|r| r.method == wiremock::http::Method::POST)
        .expect("a create");

    // The whole point: what went out opens with the universe's private key.
    assert_eq!(opened(&private, post), b"hunter2");

    let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(body["id"], "discord");
    assert_eq!(body["key_id"], KEY_ID);
    assert_eq!(body["domain"], "discord.com");
    // And the plaintext is nowhere in the request, under any encoding.
    let raw = String::from_utf8_lossy(&post.body);
    assert!(!raw.contains("hunter2"), "{raw}");
    assert!(!raw.contains(&STANDARD.encode("hunter2")), "{raw}");
}

/// A `409` is the branch, not an error: it is how this finds out that the
/// secret already exists without spending a listing — and a read scope — on
/// every write.
#[tokio::test]
async fn an_existing_secret_falls_back_from_post_to_patch() {
    let server = MockServer::start().await;
    let private = universe_keypair();
    mount_public_key(&server, &private).await;
    Mock::given(method("POST"))
        .and(path(SECRETS_PATH))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "title": "Conflict", "detail": "A secret with this id already exists.",
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(DISCORD_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "discord", "key_id": KEY_ID, "domain": "*",
        })))
        .mount(&server)
        .await;

    let cli = cli(
        &[
            "set", "discord", "--value", "rotated", "--domain", "*", "--apply",
        ],
        &server,
    );
    run(cli, &flags()).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let patch = requests
        .iter()
        .find(|r| r.method == wiremock::http::Method::PATCH)
        .expect("an update");

    assert_eq!(opened(&private, patch), b"rotated");

    let body: serde_json::Value = serde_json::from_slice(&patch.body).unwrap();
    // The id is in the path and cannot be changed, so it is not in the body.
    assert!(body.get("id").is_none(), "{body}");
    assert_eq!(body["domain"], "*");
}

/// `--no-domain` has to reach Roblox as an empty string rather than as an
/// absent field: a `set` replaces the whole secret, and an omitted domain on a
/// `PATCH` is the ambiguous case the flag exists to avoid.
#[tokio::test]
async fn no_domain_travels_as_an_explicit_empty_domain() {
    let server = MockServer::start().await;
    let private = universe_keypair();
    mount_public_key(&server, &private).await;
    Mock::given(method("POST"))
        .and(path(SECRETS_PATH))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": "signing"})),
        )
        .mount(&server)
        .await;

    let cli = cli(
        &["set", "signing", "--value", "pem", "--no-domain", "--apply"],
        &server,
    );
    run(cli, &flags()).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let post = requests
        .iter()
        .find(|r| r.method == wiremock::http::Method::POST)
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(body["domain"], "");
}

#[tokio::test]
async fn a_domain_has_to_be_decided_before_anything_is_sent() {
    let server = MockServer::start().await;
    let cli = cli(&["set", "discord", "--value", "x", "--apply"], &server);

    let error = run(cli, &flags()).await.unwrap_err();

    assert!(error.to_string().contains("--no-domain"), "{error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// A name Roblox would refuse costs no request, and the message says which
/// rule was broken instead of relaying a `400` that does not.
#[tokio::test]
async fn an_invalid_name_is_refused_locally() {
    let server = MockServer::start().await;
    let cli = cli(
        &["set", "api-key", "--value", "x", "--domain", "*", "--apply"],
        &server,
    );

    let error = run(cli, &flags()).await.unwrap_err();

    assert!(error.to_string().contains("underscores"), "{error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_listing_follows_the_cursor_and_carries_no_values() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(SECRETS_PATH))
        .and(query_param("cursor", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "secrets": [{"id": "stripe", "domain": "api.stripe.com", "key_id": KEY_ID}],
            "nextPageCursor": null,
        })))
        .mount(&server)
        .await;
    // Mounted second so wiremock prefers the more specific cursor mock above.
    Mock::given(method("GET"))
        .and(path(SECRETS_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "secrets": [{"id": "discord", "domain": "discord.com", "key_id": KEY_ID,
                         "create_time": "2026-08-01T10:00:00Z"}],
            "nextPageCursor": "page2",
        })))
        .mount(&server)
        .await;

    run(cli(&["list"], &server), &flags()).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "one page each");
    // The page size is fixed for the whole walk: a cursor is issued against
    // the parameters of the call that produced it.
    for request in &requests {
        assert!(request.url.query().unwrap().contains("limit=500"));
    }
}

/// An empty page that still carries a cursor must end the walk rather than
/// spin on it.
#[tokio::test]
async fn an_empty_page_with_a_cursor_does_not_loop() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(SECRETS_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "secrets": [], "nextPageCursor": "forever",
        })))
        .mount(&server)
        .await;

    run(cli(&["list", "--json"], &server), &flags())
        .await
        .unwrap();

    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn delete_without_apply_sends_nothing() {
    let server = MockServer::start().await;

    run(cli(&["delete", "discord"], &server), &flags())
        .await
        .unwrap();

    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn delete_with_apply_removes_the_secret() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(DISCORD_PATH))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    run(cli(&["delete", "discord", "--apply"], &server), &flags())
        .await
        .unwrap();
}

#[tokio::test]
async fn deleting_a_secret_that_is_not_there_says_so_by_name() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(DISCORD_PATH))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "title": "Not Found",
        })))
        .mount(&server)
        .await;

    let error = run(cli(&["delete", "discord", "--apply"], &server), &flags())
        .await
        .unwrap_err();

    assert!(error.to_string().contains("discord"), "{error}");
}

#[tokio::test]
async fn the_public_key_is_reported_as_roblox_sent_it() {
    let server = MockServer::start().await;
    let private = universe_keypair();
    mount_public_key(&server, &private).await;

    run(cli(&["public-key", "--json"], &server), &flags())
        .await
        .unwrap();

    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

/// A public key that is not 32 bytes must fail before anything is sealed
/// against it, and the message must make clear it was the response and not the
/// user's input that was wrong.
#[tokio::test]
async fn a_malformed_public_key_stops_the_write() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(PUBLIC_KEY_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "public-key",
            "secret": STANDARD.encode([0u8; 20]),
            "key_id": KEY_ID,
        })))
        .mount(&server)
        .await;

    let error = run(
        cli(
            &["set", "discord", "--value", "x", "--domain", "*", "--apply"],
            &server,
        ),
        &flags(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("20 bytes"), "{error}");
    // Nothing was written: the only request was the key fetch.
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, wiremock::http::Method::GET);
}

/// A file is taken byte for byte — a PEM keeps its trailing newline — where a
/// pipe is not. `--file` is the escape hatch for a value whose exact bytes
/// matter.
#[tokio::test]
async fn a_file_value_is_sent_byte_for_byte() {
    let server = MockServer::start().await;
    let private = universe_keypair();
    mount_public_key(&server, &private).await;
    Mock::given(method("POST"))
        .and(path(SECRETS_PATH))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": "signing"})),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("key.pem");
    std::fs::write(&file, "-----BEGIN-----\nabc\n").unwrap();

    let cli = cli(
        &[
            "set",
            "signing",
            "--file",
            file.to_str().unwrap(),
            "--no-domain",
            "--apply",
        ],
        &server,
    );
    run(cli, &flags()).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let post = requests
        .iter()
        .find(|r| r.method == wiremock::http::Method::POST)
        .unwrap();
    assert_eq!(opened(&private, post), b"-----BEGIN-----\nabc\n");
}
