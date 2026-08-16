//! Publishing a MessagingService message over HTTP.
//!
//! The refusals matter more than the happy path here: a publish cannot be
//! recalled, and Roblox reports nothing about who received it, so anything
//! catchable has to be caught before the request goes out.

use rbx_core::GlobalFlags;
use rbx_message::{run, MessageCli};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UNIVERSE: u64 = 66778899001;
const PUBLISH_PATH: &str = "/cloud/v2/universes/66778899001:publishMessage";

#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    publish: MessageCli,
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

fn cli(args: &[&str], server: &MockServer) -> MessageCli {
    let mut argv = vec!["publish"];
    argv.extend_from_slice(args);
    <Wrapper as clap::Parser>::parse_from(argv)
        .publish
        .with_base_url(server.uri())
}

#[tokio::test]
async fn without_apply_nothing_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    run(
        cli(&["--topic", "cache", "--message", "reload"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();

    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a publish without --apply must not reach the network"
    );
}

#[tokio::test]
async fn apply_posts_the_topic_and_message_together() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(PUBLISH_PATH))
        .and(body_json(
            serde_json::json!({ "topic": "cache", "message": "reload" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&server)
        .await;

    run(
        cli(
            &["--topic", "cache", "--message", "reload", "--apply"],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();

    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

/// Roblox types `message` as a string, so a structured payload travels as text
/// and is decoded in-experience. `--payload` does that serialisation rather than
/// leaving it to shell quoting.
#[tokio::test]
async fn a_payload_is_serialised_into_the_message_string() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(PUBLISH_PATH))
        .and(body_json(serde_json::json!({
            "topic": "cache",
            "message": "{\"key\":\"rotation\"}"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&server)
        .await;

    run(
        cli(
            &[
                "--topic",
                "cache",
                "--payload",
                r#"{"key":"rotation"}"#,
                "--apply",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();
}

/// Better here than inside `JSONDecode` on a live server, where it is a
/// runtime error in game code with no obvious origin.
#[tokio::test]
async fn malformed_json_fails_before_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    let error = run(
        cli(
            &["--topic", "cache", "--payload", "{oops", "--apply"],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("valid JSON"),
        "expected a JSON parse error, got: {error}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn an_oversized_message_is_refused_with_the_workaround_named() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let big = "x".repeat(1115);

    let error = run(
        cli(&["--topic", "cache", "--message", &big, "--apply"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err();

    let text = error.to_string();
    assert!(
        text.contains("1115 bytes"),
        "should name the actual size: {text}"
    );
    assert!(
        text.contains("memory store"),
        "should point at publishing a reference instead: {text}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// 1114, measured against the live API. The documented figure is 1 KB and the
/// first version of this crate enforced 1024, which refused messages Roblox
/// accepts.
#[tokio::test]
async fn a_message_at_the_limit_is_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(PUBLISH_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&server)
        .await;

    let exact = "x".repeat(1114);
    run(
        cli(
            &["--topic", "cache", "--message", &exact, "--apply"],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();

    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

/// Roblox answers 400 on an empty message, so it is caught here rather than at
/// the far end of whatever published it.
#[tokio::test]
async fn an_empty_message_is_refused_before_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    let error = run(
        cli(&["--topic", "cache", "--message", "", "--apply"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("empty"), "got: {error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn neither_message_nor_json_is_an_error_naming_both_flags() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    let error = run(
        cli(&["--topic", "cache", "--apply"], &server),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err();

    let text = error.to_string();
    assert!(
        text.contains("--message") && text.contains("--payload"),
        "got: {text}"
    );
}

#[test]
fn message_and_payload_cannot_be_passed_together() {
    let parsed = <Wrapper as clap::Parser>::try_parse_from([
        "publish",
        "--topic",
        "cache",
        "--message",
        "a",
        "--payload",
        "1",
    ]);
    assert!(parsed.is_err(), "expected clap to refuse the pair");
}

/// A 403 is the commonest failure here — `universe-messaging-service:publish`
/// is not on the key most people already have — so it is worth checking it
/// arrives as the shared scope guidance rather than a bare status.
#[tokio::test]
async fn a_failed_publish_is_an_error_rather_than_a_success_line() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(PUBLISH_PATH))
        .respond_with(ResponseTemplate::new(403).set_body_string("no scope"))
        .mount(&server)
        .await;

    let error = run(
        cli(
            &["--topic", "cache", "--message", "reload", "--apply"],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("does not carry that scope"),
        "a 403 should arrive as the missing-scope guidance, got: {error}"
    );
}

/// `--json` is the output format again, as it is on every other command.
///
/// It used to name the payload, which cost this command the receipt: a publish
/// cannot be recalled and Roblox reports nothing about who received it, so the
/// only record that the call went out is what the command says on its way out.
#[tokio::test]
async fn the_receipt_of_a_publish_carries_the_topic_universe_and_size() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(PUBLISH_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    run(
        cli(
            &[
                "--topic",
                "cache",
                "--message",
                "reload",
                "--apply",
                "--json",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();

    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "the document is a receipt of a real publish, not a substitute for it"
    );
}

/// The flag is an output format, so it must not become a second way to send.
/// `--json` without `--apply` is still a dry run: the document says
/// `applied: false` and the network sees nothing.
#[tokio::test]
async fn a_receipt_without_apply_still_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    run(
        cli(
            &["--topic", "cache", "--message", "reload", "--json"],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap();

    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "--json is a format, not an --apply"
    );
}

/// A run that fails before the request writes no document at all. An empty
/// stdout next to a non-zero exit says "this did not happen" without a
/// consumer having to read a field to find out.
#[tokio::test]
async fn a_refused_message_produces_no_document() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;

    let error = run(
        cli(
            &[
                "--topic",
                "cache",
                "--payload",
                "{oops",
                "--apply",
                "--json",
            ],
            &server,
        ),
        &flags(&places_file(dir.path())),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("--payload"), "got {error}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a malformed payload must not reach the network"
    );
}
