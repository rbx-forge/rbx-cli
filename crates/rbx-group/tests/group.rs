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
                "--apply",
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
            "groupMemberships": [membership_with("m-156", 156, &["1", "7"])],
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
            &["--group", &group, "unrank", "156", "7", "--apply", "--yes"],
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
            &[
                "--group",
                &group,
                "rank",
                "builderman",
                "7",
                "--apply",
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
            &[
                "--group",
                &group,
                "rank",
                "builderman",
                "owner",
                "--apply",
                "--yes",
            ],
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

/// A member holding `roles` (every one of them, not only the highest).
fn membership_with(id: &str, user: u64, roles: &[&str]) -> serde_json::Value {
    let paths: Vec<String> = roles
        .iter()
        .map(|r| format!("groups/{GROUP}/roles/{r}"))
        .collect();
    serde_json::json!({
        "path": format!("groups/{GROUP}/memberships/{id}"),
        "user": format!("users/{user}"),
        "role": paths.first().cloned().unwrap_or_default(),
        "roles": paths,
    })
}

async fn mount_names(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id":156,"name":"builderman","displayName":"builderman","hasVerifiedBadge":true}],
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn members_of_a_role_count_a_role_held_beside_a_higher_one() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_roles(
        &api,
        vec![role("7", "Moderator", 50), role("9", "Admin", 200)],
    )
    .await;

    // 156 holds Moderator *under* Admin, so `role` (the highest) says Admin.
    // Matching on `role` alone would miss them, which is the whole reason the
    // match reads `roles`.
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .and(query_param_is_missing("filter"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupMemberships": [
                membership_with("m-15", 15, &["9"]),
                membership_with("m-156", 156, &["9", "7"]),
            ],
        })))
        .expect(1)
        .mount(&api)
        .await;

    // Names come in one batch call for the page, not one call per member.
    Mock::given(method("POST"))
        .and(path("/v1/users"))
        .and(body_json(serde_json::json!({
            "userIds": [156], "excludeBannedUsers": false,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id":156,"name":"builderman","displayName":"builderman","hasVerifiedBadge":true}],
        })))
        .expect(1)
        .mount(&users)
        .await;

    let group = GROUP.to_string();
    run(
        cli(&["--group", &group, "members", "Moderator"], &api, &users),
        &flags(&no_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_run_never_reads_more_than_twenty_pages() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_names(&users).await;
    mount_roles(&api, vec![role("7", "Moderator", 50)]).await;

    // A group that never ends and never holds the role: the shape of a rare
    // role in a million-member group. Without the ceiling this loops until
    // Roblox rate-limits it.
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupMemberships": [membership_with("m-15", 15, &["1"])],
            "nextPageToken": "more",
        })))
        .expect(20)
        .mount(&api)
        .await;

    let group = GROUP.to_string();
    run(
        cli(&["--group", &group, "members", "Moderator"], &api, &users),
        &flags(&no_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_limit_stops_on_a_page_boundary_and_the_cursor_resumes_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_names(&users).await;
    mount_roles(&api, vec![role("7", "Moderator", 50)]).await;

    // A run given a cursor starts there, and never re-reads the first page.
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .and(query_param("pageToken", "page-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupMemberships": [membership_with("m-156", 156, &["7"])],
            "nextPageToken": "page-3",
        })))
        .expect(1)
        .mount(&api)
        .await;
    // `--limit 1` is met on page 2, so page 3 is left to the next run.
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .and(query_param("pageToken", "page-3"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;

    let group = GROUP.to_string();
    run(
        cli(
            &[
                "--group", &group, "members", "--limit", "1", "--cursor", "page-2",
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
async fn member_reads_every_role_including_one_the_key_cannot_see() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user_by_id(&users).await;
    // Role 404 is held but not in the listing this key gets back.
    mount_roles(&api, vec![role("7", "Moderator", 50)]).await;

    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .and(query_param("filter", "user == 'users/156'"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupMemberships": [membership_with("m-156", 156, &["7", "404"])],
        })))
        .expect(1)
        .mount(&api)
        .await;

    let group = GROUP.to_string();
    run(
        cli(&["--group", &group, "member", "156"], &api, &users),
        &flags(&no_file(dir.path())),
    )
    .await
    .unwrap();
}

#[test]
fn holding_a_role_reads_every_role_and_falls_back_to_the_highest() {
    use rbx_group::model::GroupMembership;

    let multi: GroupMembership =
        serde_json::from_value(membership_with("m", 156, &["9", "7"])).unwrap();
    assert!(multi.holds(&format!("groups/{GROUP}/roles/7")));
    assert!(multi.holds(&format!("groups/{GROUP}/roles/9")));

    // No `roles` at all: the one role Roblox did name still counts.
    let single: GroupMembership = serde_json::from_value(serde_json::json!({
        "path": "groups/42/memberships/m",
        "user": "users/156",
        "role": "groups/42/roles/7",
    }))
    .unwrap();
    assert!(single.holds("groups/42/roles/7"));
    assert_eq!(single.user_id(), Some(156));
}

/// The names lookup is the one call that carries exactly the members a run
/// returns, so it is where a test can see which ones those were.
async fn expect_names_for(server: &MockServer, ids: &[u64]) {
    let data: Vec<serde_json::Value> = ids
        .iter()
        .map(|id| serde_json::json!({"id": id, "name": format!("user{id}"), "displayName": format!("user{id}")}))
        .collect();
    Mock::given(method("POST"))
        .and(path("/v1/users"))
        .and(body_json(serde_json::json!({
            "userIds": ids, "excludeBannedUsers": false,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "data": data })))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_two_owners_page(api: &MockServer) {
    mount_roles(api, vec![role("255", "Owner", 255)]).await;
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupMemberships": [
                membership_with("m-1", 1001, &["255"]),
                membership_with("m-2", 1002, &["1"]),
                membership_with("m-3", 1003, &["255"]),
            ],
            "nextPageToken": "page-2",
        })))
        .expect(1)
        .mount(api)
        .await;
}

#[tokio::test]
async fn limit_one_returns_exactly_one_even_when_the_page_holds_two() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_two_owners_page(&api).await;
    // Only the first Owner comes back; the second is held for the next run.
    expect_names_for(&users, &[1001]).await;

    let group = GROUP.to_string();
    run(
        cli(
            &["--group", &group, "members", "Owner", "--limit", "1"],
            &api,
            &users,
        ),
        &flags(&no_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_cursor_that_stopped_inside_a_page_rereads_it_and_skips_what_was_returned() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_two_owners_page(&api).await;
    // `1:` is "the first page, one match already returned": the second Owner.
    expect_names_for(&users, &[1003]).await;
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .and(query_param("pageToken", "page-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupMemberships": [],
        })))
        .mount(&api)
        .await;

    let group = GROUP.to_string();
    run(
        cli(
            &[
                "--group", &group, "members", "Owner", "--limit", "1", "--cursor", "1:",
            ],
            &api,
            &users,
        ),
        &flags(&no_file(dir.path())),
    )
    .await
    .unwrap();
}

#[test]
fn a_cursor_round_trips_and_a_bare_roblox_token_means_skip_nothing() {
    use rbx_group::model::Cursor;

    let bare = Cursor::parse("id_aaaaBBBBccccDDDD");
    assert_eq!(bare.page_token.as_deref(), Some("id_aaaaBBBBccccDDDD"));
    assert_eq!(bare.skip, 0);
    assert_eq!(bare.render(), "id_aaaaBBBBccccDDDD");

    let inside = Cursor::parse("3:id_abc");
    assert_eq!(inside.page_token.as_deref(), Some("id_abc"));
    assert_eq!(inside.skip, 3);
    assert_eq!(inside.render(), "3:id_abc");

    // The first page has no token of its own.
    let first = Cursor::parse("2:");
    assert_eq!(first.page_token, None);
    assert_eq!(first.skip, 2);
    assert_eq!(first.render(), "2:");
}

fn flags_with_session(places: &str) -> GlobalFlags {
    GlobalFlags {
        cookie: Some("test-cookie".into()),
        ..flags(places)
    }
}

fn cli_with_groups(
    args: &[&str],
    api: &MockServer,
    users: &MockServer,
    groups: &MockServer,
) -> GroupCli {
    cli(args, api, users).with_groups_url(groups.uri())
}

#[tokio::test]
async fn with_a_session_a_role_is_listed_directly_and_open_cloud_is_not_walked() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    let groups = MockServer::start().await;
    mount_roles(&api, vec![role("7", "Tester", 10)]).await;

    // The whole point of the route: nobody is read to be discarded.
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;
    // Names arrive with the listing, so there is no lookup either.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&users)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/v1/groups/{GROUP}/roles/7/users")))
        .and(query_param("limit", "100"))
        .and(query_param_is_missing("cursor"))
        .and(header("Cookie", ".ROBLOSECURITY=test-cookie"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "previousPageCursor": null,
            "nextPageCursor": null,
            "data": [
                {"userId": 1001, "username": "first", "displayName": "First", "hasVerifiedBadge": false},
                {"userId": 1002, "username": "second", "displayName": "second", "hasVerifiedBadge": false},
            ],
        })))
        .expect(1)
        .mount(&groups)
        .await;

    let group = GROUP.to_string();
    run(
        cli_with_groups(
            &["--group", &group, "members", "Tester"],
            &api,
            &users,
            &groups,
        ),
        &flags_with_session(&no_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_session_cursor_resumes_on_the_session_route_and_skips_what_was_returned() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    let groups = MockServer::start().await;
    mount_roles(&api, vec![role("7", "Tester", 10)]).await;

    Mock::given(method("GET"))
        .and(path(format!("/v1/groups/{GROUP}/roles/7/users")))
        .and(query_param("cursor", "page-b"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "nextPageCursor": null,
            "data": [
                {"userId": 1001, "username": "first", "displayName": "first"},
                {"userId": 1002, "username": "second", "displayName": "second"},
            ],
        })))
        .expect(1)
        .mount(&groups)
        .await;

    let group = GROUP.to_string();
    run(
        cli_with_groups(
            &[
                "--group",
                &group,
                "members",
                "Tester",
                "--cursor",
                "S1:page-b",
            ],
            &api,
            &users,
            &groups,
        ),
        &flags_with_session(&no_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_cursor_is_refused_on_the_route_it_did_not_come_from() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    let groups = MockServer::start().await;
    mount_roles(&api, vec![role("7", "Tester", 10)]).await;
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&groups)
        .await;

    let group = GROUP.to_string();
    // An Open Cloud token, replayed while a session is available.
    let err = run(
        cli_with_groups(
            &["--group", &group, "members", "Tester", "--cursor", "id_abc"],
            &api,
            &users,
            &groups,
        ),
        &flags_with_session(&no_file(dir.path())),
    )
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("Open Cloud walk"), "{err:#}");

    // A session token, replayed with no session.
    let err = run(
        cli_with_groups(
            &[
                "--group",
                &group,
                "members",
                "Tester",
                "--cursor",
                "S0:page-b",
            ],
            &api,
            &users,
            &groups,
        ),
        &flags(&no_file(dir.path())),
    )
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("session route"), "{err:#}");
}

#[test]
fn a_session_cursor_is_always_marked() {
    use rbx_group::model::Cursor;

    let resumed = Cursor::parse("S2:eyJrZXkiOiIwIn0=");
    assert!(resumed.session);
    assert_eq!(resumed.skip, 2);
    assert_eq!(resumed.page_token.as_deref(), Some("eyJrZXkiOiIwIn0="));
    assert_eq!(resumed.render(), "S2:eyJrZXkiOiIwIn0=");

    // Even with nothing to skip, so it can never pass for an Open Cloud one.
    let next = Cursor {
        page_token: Some("eyJr".into()),
        skip: 0,
        session: true,
    };
    assert_eq!(next.render(), "S0:eyJr");
}

#[tokio::test]
async fn rank_without_apply_resolves_everything_and_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;
    mount_roles(&api, vec![role("7", "Moderator", 50)]).await;
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupMemberships": [membership("m-156", 156)],
        })))
        .expect(1)
        .mount(&api)
        .await;
    // The live-operations contract: no `--apply`, no write.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;

    let group = GROUP.to_string();
    run(
        cli(
            &["--group", &group, "rank", "builderman", "Moderator"],
            &api,
            &users,
        ),
        &flags(&no_file(dir.path())),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_role_already_held_is_not_given_again_even_with_apply() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockServer::start().await;
    let users = MockServer::start().await;
    mount_user(&users).await;
    mount_roles(&api, vec![role("7", "Moderator", 50)]).await;
    Mock::given(method("GET"))
        .and(path(format!("/cloud/v2/groups/{GROUP}/memberships")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "groupMemberships": [membership_with("m-156", 156, &["1", "7"])],
        })))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
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
                "--apply",
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
