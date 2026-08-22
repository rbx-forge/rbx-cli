use anyhow::Result;
use colored::Colorize;

use crate::api::RbxClient;
use crate::record::{self, NewEnv};
use rbx_core::confirm::confirm_always;
use rbx_core::owner::{Owner, OwnerType};
use rbx_core::places::PlacesFile;
use rbx_core::GlobalFlags;

/// Resolved owner for this create-universe invocation. Either a user account
/// (Roblox creates the universe under your account), or a group, or `None`
/// meaning "fall back to the authenticated user".
#[derive(Debug, Clone, Copy)]
enum ResolvedOwner {
    /// Roblox `groupId=` query param.
    Group(u64),
    /// Roblox-side default: the cookie's user.
    SelfUser,
    /// `[owner]` from rbxplace.toml told us "user X", so we explicitly target
    /// that user. Today Roblox only differentiates "no groupId" (== self) from
    /// "groupId=X". We accept the override but warn if it doesn't match self.
    User(u64),
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    global: &GlobalFlags,
    group: Option<u64>,
    user: Option<u64>,
    template_place_id: Option<u64>,
    name: Option<&str>,
    no_record: bool,
    yes: bool,
) -> Result<()> {
    let resolved = resolve_owner(global, group, user)?;

    // Everything this run needs is decided before the universe exists:
    // creating one is irreversible, so a Ctrl-C at any prompt below must cost
    // nothing more than a re-run. The name is asked first because the env
    // suggestion is derived from it.
    let prompted_name = match name {
        Some(_) => None,
        None => record::prompt_display_name("Universe name", yes)?,
    };
    let name = name.or(prompted_name.as_deref());

    let new_env = record::choose_new_env(
        &global.places,
        global.env.as_deref(),
        global.place.as_deref(),
        name,
        no_record,
        yes,
    )?;

    let cookie = global.resolve_cookie();
    let client = RbxClient::new(cookie);

    // #63: creating a universe is irreversible, and this run goes on to rename
    // it and record it in rbxplace.toml. The session is checked before any of
    // that, so a dead cookie costs a re-run rather than a stray universe.
    //
    // Ahead of the prompt, so the question can name the account it resolved:
    // auto-detection follows whichever account Studio is signed into, and an
    // owner id in the question does not tell you which session is about to
    // create under it.
    client.require_valid_session().await?;
    let account = client.known_account().await;

    confirm_always(
        &rbx_core::session::as_account(
            account.as_ref(),
            &build_prompt(resolved, name, new_env.as_ref()),
        ),
        yes,
    )?;

    match resolved {
        ResolvedOwner::Group(gid) => println!(
            "Creating universe under group {} ...",
            gid.to_string().bold()
        ),
        ResolvedOwner::SelfUser => println!("Creating universe under your user ..."),
        ResolvedOwner::User(uid) => println!(
            "Creating universe under user {} (your account) ...",
            uid.to_string().bold()
        ),
    }

    let group_arg = match resolved {
        ResolvedOwner::Group(gid) => Some(gid),
        ResolvedOwner::SelfUser | ResolvedOwner::User(_) => None,
    };
    let response = client.create_universe(template_place_id, group_arg).await?;

    if let Some(new_name) = name {
        println!(
            "Renaming root place {} to {} ...",
            response.root_place_id.to_string().cyan(),
            new_name.bold()
        );
        client
            .rename_place(response.root_place_id, new_name)
            .await?;
    }

    // Print the ids before touching rbxplace.toml: if the write fails, the
    // universe still exists and the user needs its ids to record it by hand.
    println!(
        "{} universe (id {}) with root place (id {})",
        "Created".green().bold(),
        response.universe_id.to_string().cyan(),
        response.root_place_id.to_string().cyan(),
    );
    if let Some(new_name) = name {
        println!("  name: {}", new_name.bold());
    }

    if let Some(target) = new_env {
        record::append_env(
            &global.places,
            &target.env,
            response.universe_id,
            &target.place,
            response.root_place_id,
        )?;
        println!(
            "{} [{}] to {}",
            "Added".green().bold(),
            target.env.bold(),
            global.places.display()
        );
    }

    Ok(())
}

/// CLI flags win; otherwise fall back to `[owner]` in rbxplace.toml; otherwise
/// no explicit target (Roblox uses the authenticated user).
///
/// `--group` and `--user` are mutually exclusive at the clap layer.
fn resolve_owner(
    global: &GlobalFlags,
    group: Option<u64>,
    user: Option<u64>,
) -> Result<ResolvedOwner> {
    if let Some(gid) = group {
        return Ok(ResolvedOwner::Group(gid));
    }
    if let Some(uid) = user {
        return Ok(ResolvedOwner::User(uid));
    }
    // Optional: pick up the default from rbxplace.toml if present.
    let places_path = &global.places;
    if places_path.exists() {
        let places = PlacesFile::load(places_path)?;
        if let Some(owner) = places.owner {
            return Ok(from_owner(owner));
        }
    }
    Ok(ResolvedOwner::SelfUser)
}

fn from_owner(owner: Owner) -> ResolvedOwner {
    match owner.kind {
        OwnerType::Group => ResolvedOwner::Group(owner.id),
        OwnerType::User => ResolvedOwner::User(owner.id),
    }
}

/// The confirmation is the last gate before the irreversible call, so it
/// summarizes every decision made above, including the rbxplace.toml entry.
fn build_prompt(resolved: ResolvedOwner, name: Option<&str>, new_env: Option<&NewEnv>) -> String {
    let subject = match name {
        Some(n) => format!("Create universe '{n}'"),
        None => "Create a new universe".to_string(),
    };
    let target = match resolved {
        ResolvedOwner::Group(gid) => format!(" under group {gid}"),
        ResolvedOwner::User(uid) => format!(" under user {uid}"),
        ResolvedOwner::SelfUser => " on your account".to_string(),
    };
    let recorded = match new_env {
        Some(e) => format!(" and record it as [{}]", e.env),
        None => String::new(),
    };
    format!("{subject}{target}{recorded}?")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_owner_maps_group() {
        let r = from_owner(Owner {
            kind: OwnerType::Group,
            id: 42,
        });
        assert!(matches!(r, ResolvedOwner::Group(42)));
    }

    #[test]
    fn from_owner_maps_user() {
        let r = from_owner(Owner {
            kind: OwnerType::User,
            id: 7,
        });
        assert!(matches!(r, ResolvedOwner::User(7)));
    }

    #[test]
    fn prompt_mentions_the_recorded_env() {
        let target = NewEnv {
            env: "test".to_string(),
            place: "main".to_string(),
        };
        let prompt = build_prompt(ResolvedOwner::Group(42), Some("My Game"), Some(&target));
        assert_eq!(
            prompt,
            "Create universe 'My Game' under group 42 and record it as [test]?"
        );
    }

    #[test]
    fn prompt_without_recording_keeps_the_original_wording() {
        let prompt = build_prompt(ResolvedOwner::Group(42), None, None);
        assert_eq!(prompt, "Create a new universe under group 42?");
    }

    #[test]
    fn prompt_covers_self_owned_universes() {
        let prompt = build_prompt(ResolvedOwner::SelfUser, Some("Solo"), None);
        assert_eq!(prompt, "Create universe 'Solo' on your account?");
    }
}
