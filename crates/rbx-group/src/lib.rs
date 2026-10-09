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

    match &cli.command {
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

            let users_host = cli
                .users_url
                .clone()
                .unwrap_or_else(|| "https://users.roblox.com".to_string());
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
