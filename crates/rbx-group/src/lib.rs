//! `rbx group`: read a group's roles, and move a member between them.
//!
//! # What Open Cloud lets this command do, and what it does not
//!
//! Roles are **read-only** here, and that is Roblox's limit rather than a
//! choice: the document describes two operations on `/roles` and both are
//! `GET`. There is no way to create, rename, re-rank or delete a role through
//! an API, so a declarative `rbxgroup.toml` would have nothing to reconcile
//! toward. Roles are made in the Creator Hub; this command reads them and
//! moves people between them.
//!
//! # Why a member is not a user
//!
//! `:assignRole` is addressed to a *membership*
//! (`groups/{group}/memberships/{id}`), not to a user, and its body carries a
//! role *resource path* rather than an id. Nobody thinks in membership ids, so
//! this command takes a user the way `rbx ban` does (an id, a username,
//! `@name`, `name:`, or a pasted profile link) and resolves the membership
//! itself. That resolution is the one round trip this command cannot avoid.
//!
//! # Every one of these endpoints is BETA
//!
//! `x-roblox-stability` says `BETA` on all thirteen group operations in the
//! vendored document. Response shapes here are forgiving on purpose; see
//! [`model`].

pub mod model;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use colored::Colorize;
use reqwest::{Client, StatusCode};
use serde::Serialize;

use rbx_core::api::{
    build_client, encode_query_value, execute_json, execute_with_retry, explain_missing_scope,
    is_api_status, require_api_key, ApiBase,
};
use rbx_core::confirm::confirm_always;
use rbx_core::owner::OwnerType;
use rbx_core::places::{PlacesFile, PLACES_FILE};
use rbx_core::users::{self, UserRef};
use rbx_core::GlobalFlags;

use crate::model::{
    pick_role, GroupMembership, GroupRole, ListGroupMembershipsResponse, ListGroupRolesResponse,
    RoleAssignment, RoleRef,
};

/// Roblox caps a page at 100 and defaults it to 10. The default is the trap:
/// a group with eleven roles would list ten of them and look complete, so
/// every listing here asks for the maximum and follows `nextPageToken`.
const MAX_PAGE_SIZE: u32 = 100;

/// A ceiling on how many pages a membership walk will fetch.
///
/// Only reached when the `filter` below does not work, and a large group
/// paged 100 at a time is thousands of requests. Stopping with "say which
/// membership" beats hammering Roblox until a rate limit says no.
const MAX_MEMBERSHIP_PAGES: usize = 50;

/// How many pages one `rbx group members` run reads before handing back a
/// cursor, whatever `--limit` says.
///
/// The bound that makes a million-member group workable. Looking for a rare
/// role means reading members that do not hold it, and nothing documented
/// lets Roblox do that filtering, so without a ceiling one invocation could be
/// ten thousand requests. Twenty pages is two thousand members read per run,
/// a few seconds, and the cursor carries on from exactly where it stopped.
const MAX_PAGES_PER_RUN: usize = 20;

/// `--limit`'s default: one screen of members.
const DEFAULT_LIMIT: u64 = 100;

#[derive(Args, Debug)]
pub struct GroupCli {
    #[command(subcommand)]
    command: Command,

    /// The group id.
    ///
    /// Optional when `rbxplace.toml` names a group as its `[owner]`, which is
    /// what `rbx init create-group --record` writes, and what a per-env
    /// `[envs.<name>.owner]` overrides. Required otherwise, so the command
    /// works with no file on disk at all.
    #[arg(long, global = true)]
    group: Option<u64>,

    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,

    /// Override the API host. For testing against a mock server.
    #[arg(long, hide = true, global = true)]
    base_url: Option<String>,

    /// Override the users host that resolves a username. For testing against
    /// a mock server, the same seam `rbx ban` has.
    #[arg(long, hide = true, global = true)]
    users_url: Option<String>,
}

impl GroupCli {
    /// Tests only.
    #[doc(hidden)]
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = Some(url);
        self
    }

    /// Tests only.
    #[doc(hidden)]
    pub fn with_users_url(mut self, url: String) -> Self {
        self.users_url = Some(url);
        self
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List the group's roles, lowest rank first
    ///
    /// Read-only, and the one subcommand a deploy key can run: it needs
    /// `group:read`. Roles the caller cannot see are simply absent, and the
    /// two owner-only timestamps are absent for everybody else, so a listing
    /// taken with a group key and one taken by the owner are not the same
    /// document.
    Roles,

    /// Give a member a role
    ///
    /// Adds the role rather than replacing what the member has, which is what
    /// makes a multi-role group possible. Roblox takes no action when the
    /// member already holds it.
    Rank {
        /// The member: an id, a username, `@name`, `name:<name>`, or a pasted
        /// profile link.
        user: String,

        /// The role: an id, its name, or `name:<name>` when the name is all
        /// digits.
        role: String,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Show the roles one member holds
    ///
    /// Every role, not only the highest: a member of a multi-role group is
    /// listed with all of them, highest rank first.
    Member {
        /// The member: an id, a username, `@name`, `name:<name>`, or a pasted
        /// profile link.
        user: String,
    },

    /// List members, optionally only those holding a role
    ///
    /// Paged, never exhaustive. One run reads at most 20 pages of 100 members
    /// and prints a `--cursor` to carry on, so a group of a million costs the
    /// same per run as a group of a hundred. A role is matched against every
    /// role a member holds, so somebody holding it beside a higher one is
    /// listed too.
    Members {
        /// Only members holding this role: an id, its name, or `name:<name>`.
        role: Option<String>,

        /// Stop once this many members have been found.
        ///
        /// Checked between pages, never inside one, so a run may return up to
        /// a page more than asked: stopping mid-page would leave members that
        /// no cursor could reach.
        #[arg(long, default_value_t = DEFAULT_LIMIT, value_parser = clap::value_parser!(u64).range(1..))]
        limit: u64,

        /// Carry on from where an earlier run stopped. Printed at the end of
        /// every run that did not reach the end of the group.
        #[arg(long)]
        cursor: Option<String>,
    },

    /// Take a role away from a member
    Unrank {
        /// The member: an id, a username, `@name`, `name:<name>`, or a pasted
        /// profile link.
        user: String,

        /// The role: an id, its name, or `name:<name>` when the name is all
        /// digits.
        role: String,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

struct Api {
    client: Client,
    base: ApiBase,
    api_key: String,
    group_id: u64,
}

impl Api {
    /// Every role, following `nextPageToken` to the end.
    async fn roles(&self) -> Result<Vec<GroupRole>> {
        let mut all: Vec<GroupRole> = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut url = self.base.join(&format!(
                "/cloud/v2/groups/{}/roles?maxPageSize={MAX_PAGE_SIZE}",
                self.group_id
            ));
            if let Some(token) = &token {
                url.push_str("&pageToken=");
                url.push_str(&encode_query_value(token));
            }
            let page: ListGroupRolesResponse = execute_json(|| {
                let request = self.client.get(&url).header("x-api-key", &self.api_key);
                async move { request.send().await.map_err(Into::into) }
            })
            .await
            .map_err(explain_missing_scope)?;

            all.extend(page.group_roles);
            match page.next_page_token {
                Some(next) if !next.is_empty() => token = Some(next),
                _ => break,
            }
        }
        all.sort_by_key(|role| role.rank);
        Ok(all)
    }

    /// One page of memberships, optionally filtered.
    async fn memberships_page(
        &self,
        filter: Option<&str>,
        token: Option<&str>,
    ) -> Result<ListGroupMembershipsResponse> {
        let mut url = self.base.join(&format!(
            "/cloud/v2/groups/{}/memberships?maxPageSize={MAX_PAGE_SIZE}",
            self.group_id
        ));
        if let Some(filter) = filter {
            url.push_str("&filter=");
            url.push_str(&encode_query_value(filter));
        }
        if let Some(token) = token {
            url.push_str("&pageToken=");
            url.push_str(&encode_query_value(token));
        }
        execute_json(|| {
            let request = self.client.get(&url).header("x-api-key", &self.api_key);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)
    }

    /// The membership of one user.
    ///
    /// Tries the server-side filter first. The document defers the syntax for
    /// this endpoint to a page it does not contain, but the sibling
    /// `/join-requests` in the same family documents it exactly: *"Only the
    /// `user` field and `==` operator are supported. Example:
    /// `"user == 'users/156'"`"*, and `GroupMembership.user` is that same
    /// field. So the filter is sent on the strength of the sibling, and a
    /// refusal falls back to walking the pages rather than failing: an
    /// inference about a BETA endpoint should not be the reason a rank change
    /// does not work.
    async fn membership_of(&self, user_id: u64) -> Result<GroupMembership> {
        let filter = format!("user == 'users/{user_id}'");
        match self.memberships_page(Some(&filter), None).await {
            Ok(page) => match page.group_memberships.iter().find(|m| m.is_user(user_id)) {
                Some(found) => Ok(found.clone()),
                // Not found on the filtered page settles nothing. An empty page
                // may mean "not a member", or a filter quietly ignored; a page
                // of other members means it was ignored for certain. The walk
                // is the only answer that holds in both cases.
                None => self.membership_by_walking(user_id).await,
            },
            // 400 is what an unsupported filter looks like.
            Err(e) if is_api_status(&e, StatusCode::BAD_REQUEST) => {
                self.membership_by_walking(user_id).await
            }
            Err(e) => Err(e),
        }
    }

    async fn membership_by_walking(&self, user_id: u64) -> Result<GroupMembership> {
        let mut token: Option<String> = None;
        for _ in 0..MAX_MEMBERSHIP_PAGES {
            let page = self.memberships_page(None, token.as_deref()).await?;
            if let Some(found) = page
                .group_memberships
                .iter()
                .find(|m| m.is_user(user_id))
                .cloned()
            {
                return Ok(found);
            }
            match page.next_page_token {
                Some(next) if !next.is_empty() => token = Some(next),
                _ => bail!(
                    "user {user_id} is not a member of group {}. A role can only be given to \
                     somebody who has already joined.",
                    self.group_id
                ),
            }
        }
        bail!(
            "gave up looking for user {user_id} after {MAX_MEMBERSHIP_PAGES} pages of group {}'s \
             members. Roblox ignored the `user ==` filter on this endpoint and the group is too \
             large to walk.",
            self.group_id
        )
    }

    /// One bounded stretch of the member list.
    ///
    /// Reads from `cursor` until `limit` members have matched, the group ends,
    /// or [`MAX_PAGES_PER_RUN`] pages have been read, whichever comes first,
    /// and always stops on a page boundary so that `next_cursor` resumes with
    /// the first member this run did not see.
    ///
    /// The role is matched here rather than by Roblox. The document gives no
    /// filter syntax for a role on this endpoint, and the one plausible field,
    /// `role`, holds only a member's highest role: a server-side `role ==`
    /// would quietly drop everybody who holds the role beside a higher one.
    async fn members_run(
        &self,
        role_path: Option<&str>,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<MembersRun> {
        let mut run = MembersRun {
            found: Vec::new(),
            scanned: 0,
            next_cursor: cursor,
        };
        for _ in 0..MAX_PAGES_PER_RUN {
            let page = self
                .memberships_page(None, run.next_cursor.as_deref())
                .await?;
            run.scanned += page.group_memberships.len();
            run.found.extend(
                page.group_memberships
                    .into_iter()
                    .filter(|m| role_path.is_none_or(|path| m.holds(path))),
            );
            run.next_cursor = page.next_page_token.filter(|token| !token.is_empty());
            if run.next_cursor.is_none() || run.found.len() >= limit {
                break;
            }
        }
        Ok(run)
    }

    async fn assign(&self, membership_id: &str, role_path: &str, unassign: bool) -> Result<()> {
        // Two literal paths rather than one with `:{verb}` interpolated: the
        // drift check reads these strings, and `{membership_id}:{verb}` would
        // normalise to `*:*`, which matches neither documented action.
        let url = if unassign {
            self.base.join(&format!(
                "/cloud/v2/groups/{}/memberships/{membership_id}:unassignRole",
                self.group_id
            ))
        } else {
            self.base.join(&format!(
                "/cloud/v2/groups/{}/memberships/{membership_id}:assignRole",
                self.group_id
            ))
        };
        let body = RoleAssignment {
            role: role_path.to_string(),
        };
        execute_with_retry(|| {
            let request = self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .json(&body);
            async move { request.send().await.map_err(Into::into) }
        })
        .await
        .map_err(explain_missing_scope)?;
        Ok(())
    }
}

/// The group to act on: `--group` if given, otherwise `rbxplace.toml`'s owner.
///
/// Standalone use is the point of the flag. With no file and no flag the error
/// says both ways out, because "no group id" is useless to somebody who did not
/// know a file could supply one.
fn resolve_group(flag: Option<u64>, global: &GlobalFlags) -> Result<u64> {
    if let Some(id) = flag {
        return Ok(id);
    }

    // `--places` moves the file, so the flag is read rather than the default
    // name: a project that keeps it elsewhere must not look ownerless.
    let path = std::path::Path::new(&global.places);
    if !path.exists() {
        bail!(
            "no group id. Pass `--group <id>`, or run this where a {PLACES_FILE} names a group \
             as its [owner]."
        );
    }

    let places = PlacesFile::load(path)?;
    // An env's own [owner] wins over the top-level one, which is how a project
    // keeps one env under a group and another under a user.
    let env = global.env.as_deref().unwrap_or("");
    let owner = places.resolve_owner(env).ok_or_else(|| {
        anyhow::anyhow!(
            "{PLACES_FILE} declares no [owner], so there is no group to act on. \
             Pass `--group <id>`."
        )
    })?;

    match owner.kind {
        OwnerType::Group => Ok(owner.id),
        OwnerType::User => bail!(
            "{PLACES_FILE}'s owner is user {}, not a group, so `rbx group` has nothing to act \
             on. Pass `--group <id>` for a group you can manage.",
            owner.id
        ),
    }
}

pub async fn run(cli: GroupCli, global: &GlobalFlags) -> Result<()> {
    let group_id = resolve_group(cli.group, global)?;

    let api = Api {
        client: build_client(),
        base: match &cli.base_url {
            Some(url) => ApiBase::new(url.clone()),
            None => ApiBase::default(),
        },
        api_key: require_api_key(global.api_key.as_deref())?.to_string(),
        group_id,
    };
    let users_host = cli
        .users_url
        .clone()
        .unwrap_or_else(|| "https://users.roblox.com".to_string());

    match &cli.command {
        Command::Member { user } => {
            let user_ref = UserRef::parse(user)?;
            let resolved =
                users::resolve_with_host(&api.client, std::slice::from_ref(&user_ref), &users_host)
                    .await?;
            let target = resolved.first().with_context(|| {
                format!("Roblox returned no user for {user:?}, which it should not do")
            })?;
            let roles = api.roles().await?;
            let membership = api.membership_of(target.id).await?;
            let held = role_views(&membership, &roles);

            if cli.json {
                let doc = serde_json::json!({
                    "group_id": group_id.to_string(),
                    "user_id": target.id.to_string(),
                    "username": target.name,
                    "display_name": target.display_name,
                    "roles": held,
                });
                println!("{}", serde_json::to_string_pretty(&doc)?);
                return Ok(());
            }

            println!("{} in group {group_id}", target.label().bold());
            println!("{:>5}  {}", "RANK".bold(), "ROLE".bold());
            for role in &held {
                println!("{:>5}  {}", role.rank_text(), role.name_text());
            }
            Ok(())
        }

        Command::Members {
            role,
            limit,
            cursor,
        } => {
            let roles = api.roles().await?;
            let wanted = match role {
                Some(text) => Some(pick_role(&roles, &RoleRef::parse(text)?)?),
                None => None,
            };
            let wanted_count = usize::try_from(*limit).unwrap_or(usize::MAX);
            let run = api
                .members_run(
                    wanted.map(|r| r.path.as_str()),
                    wanted_count,
                    cursor.clone(),
                )
                .await?;

            let ids: Vec<u64> = run
                .found
                .iter()
                .filter_map(GroupMembership::user_id)
                .collect();
            let names = users::names_for_ids_with_host(&api.client, &ids, &users_host).await?;

            let members: Vec<MemberView> = run
                .found
                .iter()
                .map(|m| {
                    let user = m.user_id().and_then(|id| names.get(&id));
                    MemberView {
                        user_id: m.user_id().map(|id| id.to_string()).unwrap_or_default(),
                        username: user.map(|u| u.name.clone()),
                        display_name: user.map(|u| u.display_name.clone()),
                        roles: role_views(m, &roles),
                    }
                })
                .collect();

            if cli.json {
                let doc = serde_json::json!({
                    "group_id": group_id.to_string(),
                    "role": wanted.map(RoleView::of),
                    "members": members,
                    "scanned": run.scanned,
                    "next_cursor": run.next_cursor,
                });
                println!("{}", serde_json::to_string_pretty(&doc)?);
                return Ok(());
            }

            print_members(&members, &run, wanted, group_id);
            if let Some(next) = &run.next_cursor {
                let mut again = String::from("rbx group members");
                if let Some(text) = role {
                    again.push(' ');
                    again.push_str(&quote_arg(text));
                }
                if let Some(id) = cli.group {
                    again.push_str(&format!(" --group {id}"));
                }
                if *limit != DEFAULT_LIMIT {
                    again.push_str(&format!(" --limit {limit}"));
                }
                again.push_str(&format!(" --cursor {}", quote_arg(next)));
                println!("{} {again}", "More:".bold());
            } else {
                println!("{}", "End of the group.".dimmed());
            }
            Ok(())
        }

        Command::Roles => {
            let roles = api.roles().await?;
            if cli.json {
                let doc = serde_json::json!({
                    "group_id": group_id.to_string(),
                    "roles": roles,
                });
                println!("{}", serde_json::to_string_pretty(&doc)?);
                return Ok(());
            }
            print_roles(&roles, group_id);
            Ok(())
        }

        Command::Rank { user, role, yes } | Command::Unrank { user, role, yes } => {
            let unassign = matches!(cli.command, Command::Unrank { .. });

            // Both lookups happen before anything is written, so a typo in
            // either argument costs nothing. The roles listing is needed
            // regardless: a role name has to become an id, and an id has to be
            // checked against the group rather than sent blind.
            let user_ref = UserRef::parse(user)?;
            let role_ref = RoleRef::parse(role)?;

            let resolved =
                users::resolve_with_host(&api.client, std::slice::from_ref(&user_ref), &users_host)
                    .await?;
            let target = resolved.first().with_context(|| {
                format!("Roblox returned no user for {user:?}, which it should not do")
            })?;

            let roles = api.roles().await?;
            let role = pick_role(&roles, &role_ref)?;
            let membership = api.membership_of(target.id).await?;
            let membership_id = membership.id()?;

            let verb = if unassign { "Remove" } else { "Give" };
            let preposition = if unassign { "from" } else { "to" };
            confirm_always(
                &format!(
                    "{verb} role {} (rank {}) {preposition} {} in group {group_id}?",
                    role.display_name,
                    role.rank,
                    target.label(),
                ),
                *yes,
            )?;

            api.assign(membership_id, &role.path, unassign).await?;

            let done = if unassign { "removed" } else { "given" };
            println!(
                "{} {} {done}: {} (rank {})",
                "done".green().bold(),
                target.label(),
                role.display_name,
                role.rank
            );
            if !unassign && membership.roles.len() > 1 {
                println!(
                    "{}",
                    format!(
                        "{} already held {} other role(s). This adds one rather than \
                         replacing them.",
                        target.name,
                        membership.roles.len()
                    )
                    .dimmed()
                );
            }
            Ok(())
        }
    }
}

/// What one bounded run of `members` found.
struct MembersRun {
    found: Vec<GroupMembership>,
    /// Members read, matching or not, so a run that found nobody can still say
    /// how much of the group it covered.
    scanned: usize,
    /// `None` once the group is exhausted.
    next_cursor: Option<String>,
}

/// A role as `member` and `members` report it.
///
/// `name` and `rank` are optional because a member can hold a role the
/// listing did not return to this key. The id is still known, off the path,
/// and printing it beats dropping the role.
#[derive(Debug, Serialize)]
struct RoleView {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rank: Option<u32>,
}

impl RoleView {
    fn of(role: &GroupRole) -> Self {
        Self {
            id: role.id.clone(),
            name: Some(role.display_name.clone()),
            rank: Some(role.rank),
        }
    }

    fn rank_text(&self) -> String {
        self.rank
            .map_or_else(|| "?".to_string(), |rank| rank.to_string())
    }

    fn name_text(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => format!("(role {}, not visible to this key)", self.id),
        }
    }
}

#[derive(Debug, Serialize)]
struct MemberView {
    user_id: String,
    /// Absent when Roblox no longer returns the account, which a listing
    /// reports rather than fails on.
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    roles: Vec<RoleView>,
}

/// Every role a member holds, highest rank first, named from the listing.
fn role_views(membership: &GroupMembership, roles: &[GroupRole]) -> Vec<RoleView> {
    let mut views: Vec<RoleView> = membership
        .role_paths()
        .into_iter()
        .map(|path| match roles.iter().find(|role| role.path == path) {
            Some(role) => RoleView::of(role),
            None => RoleView {
                id: path.rsplit_once('/').map_or(path, |(_, id)| id).to_string(),
                name: None,
                rank: None,
            },
        })
        .collect();
    // Unknown ranks sort last: they are the roles this key cannot see.
    views.sort_by_key(|view| std::cmp::Reverse(view.rank));
    views
}

/// An argument as it can be pasted back into a shell, for the `More:` line.
///
/// Single quotes mean the same thing in bash and in PowerShell, which double
/// quotes do not (both expand `$` inside them). A value holding a single quote
/// falls back to double quotes, the one case where the two shells disagree.
fn quote_arg(text: &str) -> String {
    let plain = text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./:=+".contains(c));
    if plain {
        text.to_string()
    } else if text.contains('\'') {
        format!("\"{text}\"")
    } else {
        format!("'{text}'")
    }
}

fn print_members(
    members: &[MemberView],
    run: &MembersRun,
    wanted: Option<&GroupRole>,
    group_id: u64,
) {
    match wanted {
        Some(role) => println!(
            "{}",
            format!(
                "Members of group {group_id} holding {} (rank {})",
                role.display_name, role.rank
            )
            .bold()
        ),
        None => println!("{}", format!("Members of group {group_id}").bold()),
    }

    if !members.is_empty() {
        println!("{:<36}  {}", "USER".bold(), "ROLES".bold());
        for member in members {
            let user = match &member.username {
                Some(name) if member.display_name.as_deref() != Some(name.as_str()) => format!(
                    "{name} \"{}\" ({})",
                    member.display_name.as_deref().unwrap_or_default(),
                    member.user_id
                ),
                Some(name) => format!("{name} ({})", member.user_id),
                None => format!("({})", member.user_id),
            };
            let roles = member
                .roles
                .iter()
                .map(RoleView::name_text)
                .collect::<Vec<_>>()
                .join(", ");
            println!("{user:<36}  {roles}");
        }
        println!();
    }

    let summary = format!(
        "{} found, {} member(s) read this run.",
        members.len(),
        run.scanned
    );
    println!("{}", summary.dimmed());
}

fn print_roles(roles: &[GroupRole], group_id: u64) {
    if roles.is_empty() {
        println!(
            "{}",
            format!("Group {group_id} reports no roles your key can see.").dimmed()
        );
        return;
    }

    println!(
        "{:>5}  {:<28}  {:>9}  {}",
        "RANK".bold(),
        "NAME".bold(),
        "MEMBERS".bold(),
        "ID".bold()
    );
    for role in roles {
        // Absent rather than zero for a guest role, so a dash is the honest
        // rendering: "Roblox did not say" is not "nobody".
        let members = role
            .member_count
            .map_or_else(|| "-".to_string(), |count| count.to_string());
        println!(
            "{:>5}  {:<28}  {members:>9}  {}",
            role.rank, role.display_name, role.id
        );
    }
    println!();
    println!(
        "{}",
        "Roles are read-only: Open Cloud has no endpoint to create, rename or delete one. \
         Use the Creator Hub."
            .dimmed()
    );
}
