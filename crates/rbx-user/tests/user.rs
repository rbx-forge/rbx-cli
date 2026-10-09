//! `rbx user`, against a mock of `users.roblox.com`.
//!
//! The lookups themselves live in `rbx_core::users` and are tested there; what
//! is worth protecting here is that `--id` and `--name` print bare values in
//! the order given, because that order is what a script pairs lines by.

use rbx_core::users::User;
use rbx_core::GlobalFlags;
use rbx_user::{render, run, Output, UserCli};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn user(id: u64, name: &str, display: &str) -> User {
    User {
        id,
        name: name.into(),
        display_name: display.into(),
        has_verified_badge: false,
    }
}

#[test]
fn bare_values_come_one_per_line_in_the_order_given() {
    let found = vec![
        user(156, "builderman", "builderman"),
        user(1, "Roblox", "Roblox"),
    ];
    assert_eq!(render(&found, Output::Ids).unwrap(), "156\n1");
    assert_eq!(render(&found, Output::Names).unwrap(), "builderman\nRoblox");
}

#[test]
fn the_human_line_shows_a_display_name_only_when_it_differs() {
    let found = vec![
        user(156, "builderman", "builderman"),
        user(1234567890, "somebody", "Some Body"),
    ];
    let text = render(&found, Output::Human).unwrap();
    assert!(text.contains("builderman (156)"), "{text}");
    assert!(text.contains("\"Some Body\""), "{text}");
}

#[test]
fn json_ids_are_strings() {
    let doc: serde_json::Value = serde_json::from_str(
        &render(&[user(156, "builderman", "builderman")], Output::Json).unwrap(),
    )
    .unwrap();
    assert_eq!(doc[0]["user_id"], "156");
    assert_eq!(doc[0]["username"], "builderman");
}

#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    user: UserCli,
}

fn flags() -> GlobalFlags {
    GlobalFlags {
        api_key: None,
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

/// No key is passed and none is asked for: the lookup is public.
#[tokio::test]
async fn a_name_and_an_id_resolve_without_an_api_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/usernames/users"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[{
                "requestedUsername":"builderman","id":156,
                "name":"builderman","displayName":"builderman","hasVerifiedBadge":true
            }]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/users/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id":1,"name":"Roblox","displayName":"Roblox","hasVerifiedBadge":true
        })))
        .expect(1)
        .mount(&server)
        .await;

    let cli = <Wrapper as clap::Parser>::parse_from(["user", "builderman", "1", "--id"])
        .user
        .with_users_url(server.uri());
    run(cli, &flags()).await.unwrap();
}

#[tokio::test]
async fn an_unknown_name_is_an_error_naming_it() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/usernames/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[]})))
        .mount(&server)
        .await;

    let cli = <Wrapper as clap::Parser>::parse_from(["user", "nobody_here_x"])
        .user
        .with_users_url(server.uri());
    let err = run(cli, &flags()).await.unwrap_err();
    assert!(format!("{err:#}").contains("nobody_here_x"), "{err:#}");
}
