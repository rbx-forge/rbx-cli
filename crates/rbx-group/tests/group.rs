//! HTTP behaviour of `rbx group`, against a mock of both hosts it uses.
//!
//! The parts worth protecting are the ones that fail silently against a
//! permissive server: that a listing follows `nextPageToken` instead of
//! stopping at Roblox's default page of ten, that `:assignRole` carries a role
//! *resource path* rather than a bare id, that an ambiguous role name never
//! reaches the write, and that a refused `filter` falls back to walking the
//! members rather than failing the command.

use rbx_core::GlobalFlags;
use rbx_group::{run, GroupCli};
use wiremock::matchers::{body_json, header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

const GROUP: u64 = 42;

fn flags(places: &str) -> GlobalFlags {
    GlobalFlags {
        api_key: Some("test-key".into()),
        cookie: None,
        no_auto_cookie: true,
        auto_cookie: false,
        env: None,
        place: None,
        places: places.into(),
        universe_id: None,
        place_id: Vec::new(),
    }
}

/// A path that does not exist, so nothing is read from disk unless a test
/// writes the file on purpose.
fn no_file(dir: &std::path::Path) -> String {
    dir.join("rbxplace.toml").to_string_lossy().into_owned()
}

#[derive(clap::Parser)]
struct Wrapper {
    #[command(flatten)]
    group: GroupCli,
}

fn cli(args: &[&str], api: &MockServer, users: &MockServer) -> GroupCli {
    let mut argv = vec!["group"];
    argv.extend_from_slice(args);
    <Wrapper as clap::Parser>::parse_from(argv)
        .group
        .with_base_url(api.uri())
        .with_users_url(users.uri())
}

fn role(id: &str, name: &str, rank: u32) -> serde_json::Value {
    serde_json::json!({
        "path": format!("groups/{GROUP}/roles/{id}"),
        "id": id,
        "displayName": name,
        "rank": rank,
        "memberCount": 3,
    })
}

async fn mount_roles(server: &MockServer, roles: Vec<serde_json::Value>) {
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/roles")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupRoles": roles,
        })))
        .mount(server)
        .await;
}

async fn mount_user(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/usernames/users"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[{
                "requestedUsername":"builderman","id":156,
                "name":"builderman","displayName":"builderman","hasVerifiedBadge":true
            }]})),
        )
        .mount(server)
        .await;
}

/// An id is looked up too: it proves the account exists and gives the
/// confirmation prompt a name to show instead of a number.
async fn mount_user_by_id(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/users/156"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id":156,"name":"builderman","displayName":"builderman","hasVerifiedBadge":true
        })))
        .expect(1)
        .mount(server)
        .await;
}

fn membership(id: &str, user: u64) -> serde_json::Value {
    serde_json::json!({
        "path": format!("groups/{GROUP}/memberships/{id}"),
        "user": format!("users/{user}"),
        "role": format!("groups/{GROUP}/roles/1"),
        "roles": [format!("groups/{GROUP}/roles/1")],
    })
}

#[tokio::test]
async fn roles_follow_the_page_token_and_ask_for_full_pages() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;

    // Roblox defaults a page to ten. Asking for 100 and following the token
    // is what keeps an eleventh role from silently going missing.
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/roles")))
        .and(query_param("maxPageSize", "100"))
        .and(query_param_is_missing("pageToken"))
        .and(header("x-api-key", "test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupRoles": [role("1", "Guest", 0)],
            "nextPageToken": "page-2",
        })))
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/roles")))
        .and(query_param("pageToken", "page-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupRoles": [role("7", "Moderator", 50)],
        })))
        .expect(1)
        .mount(&api)
        .await;

    let group = GROUP.to_string();
    run(
        cli(&["--group", &group, "roles"], &api, &users),
        &flags(&no_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn rank_sends_the_role_as_a_resource_path_to_the_filtered_membership() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;
    mount_roles(
        &api,
        vec![role("1", "Guest", 0), role("7", "Moderator", 50)],
    )
    .await;

    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .and(query_param("filter", "user == 'users/156'"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupMemberships": [membership("m-156", 156)],
        })))
        .expect(1)
        .mount(&api)
        .await;

    // The body is the whole point: `"7"` alone is a 400 from Roblox.
    Mock::given(method("POST"))
        .and(path(format!(
            "/cloud/v2/groups/{GROUP}/memberships/m-156:assignRole"
        )))
        .and(body_json(serde_json::json!({
            "role": format!("groups/{GROUP}/roles/7"),
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&api)
        .await;

    let group = GROUP.to_string();
    run(
        cli(
            &[
                "--group",
                &group,
                "rank",
                "builderman",
                "Moderator",
                "--yes",
            ],
            &api,
            &users,
        ),
        &flags(&no_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn unrank_hits_unassign_role() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user_by_id(&users).await;
    mount_roles(&api, vec![role("7", "Moderator", 50)]).await;

    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupMemberships": [membership("m-156", 156)],
        })))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/cloud/v2/groups/{GROUP}/memberships/m-156:unassignRole"
        )))
        .and(body_json(serde_json::json!({
            "role": format!("groups/{GROUP}/roles/7"),
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&api)
        .await;

    let group = GROUP.to_string();
    run(
        cli(
            &["--group", &group, "unrank", "156", "7", "--yes"],
            &api,
            &users,
        ),
        &flags(&no_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_refused_filter_falls_back_to_walking_the_members() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;
    mount_roles(&api, vec![role("7", "Moderator", 50)]).await;

    // The filter syntax is inferred from the sibling `/join-requests`, not
    // documented for this endpoint. If Roblox refuses it, the command must
    // still work.
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .and(query_param("filter", "user == 'users/156'"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "code": "INVALID_ARGUMENT", "message": "unsupported filter",
        })))
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .and(query_param_is_missing("filter"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupMemberships": [membership("m-15", 15)],
            "nextPageToken": "page-2",
        })))
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .and(query_param("pageToken", "page-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupMemberships": [membership("m-156", 156)],
        })))
        .expect(1)
        .mount(&api)
        .await;
    // `users/15` on page one must not be mistaken for `users/156`.
    Mock::given(method("POST"))
        .and(path(format!(
            "/cloud/v2/groups/{GROUP}/memberships/m-156:assignRole"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&api)
        .await;

    let group = GROUP.to_string();
    run(
        cli(
            &["--group", &group, "rank", "builderman", "7", "--yes"],
            &api,
            &users,
        ),
        &flags(&no_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn an_ambiguous_role_name_never_reaches_the_write() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;
    // Two roles called Owner is a real configuration, not a contrived one.
    mount_roles(
        &api,
        vec![role("254", "Owner", 254), role("255", "Owner", 255)],
    )
    .await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;

    let group = GROUP.to_string();
    let err = run(
        cli(
            &["--group", &group, "rank", "builderman", "owner", "--yes"],
            &api,
            &users,
        ),
        &flags(&no_file(dir.path())),
    )
    .await
    .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("254") && message.contains("255"),
        "{message}"
    );
}

#[tokio::test]
async fn with_no_flag_and_no_file_it_says_both_ways_out() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;

    let err = run(cli(&["roles"], &api, &users), &flags(&no_file(dir.path())))
        .await
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(message.contains("--group"), "{message}");
    assert!(message.contains("[owner]"), "{message}");
}

#[tokio::test]
async fn the_group_comes_from_the_owner_when_no_flag_is_given() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;

    let file = dir.path().join("rbxplace.toml");
    std::fs::write(
        &file,
        format!(
            "[owner]\ntype = \"group\"\nid = {GROUP}\n\n\
             [ops]\nuniverse_id = 66778899001\n\n[ops.places]\nmain = 1\n"
        ),
    )
    .unwrap();

    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/roles")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupRoles": [],
        })))
        .expect(1)
        .mount(&api)
        .await;

    run(
        cli(&["roles"], &api, &users),
        &flags(&file.to_string_lossy()),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_user_owner_is_refused_rather_than_read_as_a_group() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;

    let file = dir.path().join("rbxplace.toml");
    std::fs::write(
        &file,
        "[owner]\ntype = \"user\"\nid = 1234567890\n\n\
         [ops]\nuniverse_id = 66778899001\n\n[ops.places]\nmain = 1\n",
    )
    .unwrap();

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;

    let err = run(
        cli(&["roles"], &api, &users),
        &flags(&file.to_string_lossy()),
    )
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("not a group"), "{err:#}");
}
