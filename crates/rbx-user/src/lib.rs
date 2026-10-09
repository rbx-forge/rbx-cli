//! `rbx user`: a username to an id, or an id to a username.
//!
//! The lookup every other command already does before acting on somebody
//! (`rbx ban`, `rbx group`), offered on its own so it composes with what does
//! not do it: `rbx data get --datastore PlayerData "User_$(rbx user builderman --id)"`.
//! That is the whole reason it exists. A data store key is the game's own
//! string, so `rbx data` cannot know which keys hold a user id, and a command
//! that prints one is what lets the caller say so without a file to declare it.
//!
//! No API key: `users.roblox.com` answers anonymously, which is also why this
//! works before anything else is configured.

use anyhow::Result;
use clap::Args;

use rbx_core::api::build_client;
use rbx_core::users::{self, User, UserRef};
use rbx_core::GlobalFlags;

#[derive(Args, Debug)]
pub struct UserCli {
    /// One or more users: an id, a username, `name:<name>` (or `@<name>`) to
    /// force the username reading, or a pasted profile link.
    #[arg(required = true, value_name = "USER")]
    users: Vec<String>,

    /// Print only the ids, one per line, in the order given.
    #[arg(long, conflicts_with_all = ["name", "json"])]
    id: bool,

    /// Print only the usernames, one per line, in the order given.
    #[arg(long, conflicts_with = "json")]
    name: bool,

    /// Machine-readable output.
    #[arg(long)]
    json: bool,

    /// Override the users host. For testing against a mock server.
    #[arg(long, hide = true)]
    users_url: Option<String>,
}

impl UserCli {
    /// Tests only.
    #[doc(hidden)]
    pub fn with_users_url(mut self, url: String) -> Self {
        self.users_url = Some(url);
        self
    }
}

/// What to print for the users found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Ids,
    Names,
    Json,
    Human,
}

/// The text printed for `users`, in the order they were asked for.
///
/// Pure, so the formats are tested without capturing stdout.
pub fn render(found: &[User], output: Output) -> Result<String> {
    let lines: Vec<String> = match output {
        Output::Ids => found.iter().map(|user| user.id.to_string()).collect(),
        Output::Names => found.iter().map(|user| user.name.clone()).collect(),
        Output::Json => {
            // Ids as strings, the convention of every `--json` in this tool:
            // a JavaScript consumer reads a large integer wrong otherwise.
            let doc: Vec<serde_json::Value> = found
                .iter()
                .map(|user| {
                    serde_json::json!({
                        "user_id": user.id.to_string(),
                        "username": user.name,
                        "display_name": user.display_name,
                        "has_verified_badge": user.has_verified_badge,
                        "profile_url": user.profile_url(),
                    })
                })
                .collect();
            return Ok(serde_json::to_string_pretty(&doc)?);
        }
        Output::Human => found.iter().map(User::label).collect(),
    };
    Ok(lines.join("\n"))
}

pub async fn run(cli: UserCli, _global: &GlobalFlags) -> Result<()> {
    // Parsed before any request, so a malformed argument costs nothing.
    let refs = cli
        .users
        .iter()
        .map(|input| UserRef::parse(input))
        .collect::<Result<Vec<_>>>()?;

    let host = cli
        .users_url
        .as_deref()
        .unwrap_or("https://users.roblox.com");
    // An unknown username or id is an error naming it, never a silent gap:
    // a script reading `--id` line by line would otherwise pair the wrong
    // lines together.
    let found = users::resolve_with_host(&build_client(), &refs, host).await?;

    let output = if cli.id {
        Output::Ids
    } else if cli.name {
        Output::Names
    } else if cli.json {
        Output::Json
    } else {
        Output::Human
    };
    println!("{}", render(&found, output)?);
    Ok(())
}
