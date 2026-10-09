//! The shapes Roblox returns for a group, and the resource paths it wants back.
//!
//! Every one of these endpoints is marked `BETA` in the vendored document
//! (`x-roblox-stability`), which is why the types here are forgiving: a field
//! Roblox has not shipped yet, or stops sending, must not turn a listing into
//! `Failed to parse response`.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// One role of a group.
///
/// `permissions` is deliberately **not** modelled as thirty booleans. The
/// document describes thirty today and will describe more; a typed mirror would
/// need `#[serde(default)]` on each, which turns a field Roblox renames into a
/// silent `false`. For a permission that is the worst possible default: the
/// caller would read "cannot ban members" from a rename. Passing the object
/// through unread keeps whatever Roblox sent, which is also exactly what a
/// generator consuming `--json` wants.
///
/// `extra` catches anything else the document gains, so `--json` keeps emitting
/// it rather than quietly dropping it on the floor.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupRole {
    /// `groups/{group_id}/roles/{role_id}`.
    #[serde(default)]
    pub path: String,
    /// Unique across Roblox, unlike `rank`, which is only unique in the group.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// 0 to 255.
    #[serde(default)]
    pub rank: u32,
    /// Absent for guest roles, says the document. Not zero: absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_count: Option<u64>,
    /// Both timestamps are "visible only to owners of the group", so a key
    /// that is not the owner's sees neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListGroupRolesResponse {
    #[serde(default)]
    pub group_roles: Vec<GroupRole>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

/// One member's place in the group.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMembership {
    /// `groups/{group_id}/memberships/{membership_id}`. This is what
    /// `:assignRole` is addressed to, which is why a user id alone is never
    /// enough to move somebody.
    #[serde(default)]
    pub path: String,
    /// `users/{user_id}`.
    #[serde(default)]
    pub user: String,
    /// The member's **highest-ranked** role only. The document is explicit
    /// that this does not reflect the others when somebody holds several, so
    /// it is read only when `roles` is empty, as the one role Roblox did say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Every role assigned to the member. The document calls this "the
    /// recommended field for reading a member's roles", and it is the one that
    /// makes a multi-role group legible.
    #[serde(default)]
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListGroupMembershipsResponse {
    #[serde(default)]
    pub group_memberships: Vec<GroupMembership>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

/// The body of `:assignRole` and `:unassignRole`, which differ only in the URL.
#[derive(Debug, Clone, Serialize)]
pub struct RoleAssignment {
    /// `groups/{group_id}/roles/{role_id}`, not a bare id. Sending the id
    /// alone is a 400.
    pub role: String,
}

impl GroupMembership {
    /// The membership id, read off the resource path.
    ///
    /// Roblox gives a path and takes an id, so somebody has to split it. Done
    /// here rather than at the call site so that a path in an unexpected shape
    /// fails once, with the path in the message.
    pub fn id(&self) -> Result<&str> {
        match self.path.rsplit_once('/') {
            Some((_, id)) if !id.is_empty() => Ok(id),
            _ => bail!(
                "Roblox returned a group membership whose path is not \
                 `groups/<group>/memberships/<id>`: {:?}",
                self.path
            ),
        }
    }

    /// The member's user id, off `users/{id}`.
    ///
    /// Parses the whole last segment rather than asking whether the path
    /// *contains* the digits: `users/15` must not match `users/156`.
    pub fn user_id(&self) -> Option<u64> {
        self.user
            .rsplit_once('/')
            .and_then(|(_, id)| id.parse().ok())
    }

    /// Whether this membership belongs to `user_id`.
    pub fn is_user(&self, user_id: u64) -> bool {
        self.user_id() == Some(user_id)
    }

    /// Every role the member holds, as resource paths.
    ///
    /// `roles` when Roblox sent it, which is every role; otherwise the single
    /// highest one in `role`, rather than nothing.
    pub fn role_paths(&self) -> Vec<&str> {
        if self.roles.is_empty() {
            self.role.as_deref().into_iter().collect()
        } else {
            self.roles.iter().map(String::as_str).collect()
        }
    }

    /// Whether the member holds `role_path`, counting a role that is not their
    /// highest. Asking `role` alone would miss every member of a multi-role
    /// group who holds this one beside a higher one.
    pub fn holds(&self, role_path: &str) -> bool {
        self.role_paths().contains(&role_path)
    }
}

/// How a role was named on the command line.
///
/// The same shape as `rbx_core::users::UserRef`, and for the same reason: the
/// common case must need no punctuation, and the ambiguous case must have a way
/// to say which it meant. A role id is a string in the document, so bare digits
/// could in principle be either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleRef {
    Id(String),
    Name(String),
}

impl RoleRef {
    /// Parse one argument.
    ///
    /// ```text
    /// 2788109      a role id
    /// Moderator    a role name
    /// name:2788109 a role name, forced
    /// ```
    ///
    /// Bare digits are an id. Unlike a username, a role *name* is usually a
    /// word, so the forcing prefix is the rarer case here, but it exists
    /// because a group may well have a role called `2024`.
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            bail!("empty role reference");
        }
        if let Some(name) = input.strip_prefix("name:") {
            if name.is_empty() {
                bail!("`name:` needs a role name after it");
            }
            return Ok(Self::Name(name.to_string()));
        }
        if input.chars().all(|c| c.is_ascii_digit()) {
            return Ok(Self::Id(input.to_string()));
        }
        Ok(Self::Name(input.to_string()))
    }
}

/// Pick the role an argument names out of a group's roles.
///
/// A name matching several roles is an **error**, not a first-match win. The
/// document lets two roles share a `displayName`, groups really do have two
/// roles called `Owner`, and silently ranking somebody into whichever came
/// back first is the kind of mistake nobody notices until it matters.
pub fn pick_role<'a>(roles: &'a [GroupRole], wanted: &RoleRef) -> Result<&'a GroupRole> {
    match wanted {
        RoleRef::Id(id) => roles
            .iter()
            .find(|role| role.id == *id)
            .ok_or_else(|| anyhow::anyhow!("this group has no role with id {id}")),
        RoleRef::Name(name) => {
            let matches: Vec<&GroupRole> = roles
                .iter()
                .filter(|role| role.display_name.eq_ignore_ascii_case(name))
                .collect();
            match matches.as_slice() {
                [one] => Ok(one),
                [] => {
                    let mut known: Vec<&str> =
                        roles.iter().map(|r| r.display_name.as_str()).collect();
                    known.sort_unstable();
                    bail!(
                        "this group has no role called {name:?}. It has: {}",
                        known.join(", ")
                    )
                }
                several => {
                    let listed = several
                        .iter()
                        .map(|r| format!("  rank {:>3}  id {}", r.rank, r.id))
                        .collect::<Vec<_>>()
                        .join("\n");
                    bail!(
                        "{} roles of this group are called {name:?}, so the name does not \
                         say which one you mean. Pass one of these ids instead:\n{listed}",
                        several.len()
                    )
                }
            }
        }
    }
}
