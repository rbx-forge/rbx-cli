use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use crate::api::universes::CreateUniverseResponse;
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

    finish(&client, &response, name, new_env.as_ref(), &global.places).await
}

/// Everything after the universe exists, in the order that keeps it findable.
///
/// The name comes last because it is the one step Roblox can refuse for what
/// it says: moderation turns down a name only after the universe is made. By
/// then the universe cannot be deleted, so the ids go on screen first, in case
/// the file write fails, and the file is written before the name is tried.
async fn finish(
    client: &RbxClient,
    response: &CreateUniverseResponse,
    name: Option<&str>,
    new_env: Option<&NewEnv>,
    places: &Path,
) -> Result<()> {
    println!(
        "{} universe (id {}) with root place (id {})",
        "Created".green().bold(),
        response.universe_id.to_string().cyan(),
        response.root_place_id.to_string().cyan(),
    );

    if let Some(target) = new_env {
        record::append_env(
            places,
            &target.env,
            response.universe_id,
            &target.place,
            response.root_place_id,
        )?;
        println!(
            "{} [{}] to {}",
            "Added".green().bold(),
            target.env.bold(),
            places.display()
        );
    }

    if let Some(new_name) = name {
        println!(
            "Renaming root place {} to {} ...",
            response.root_place_id.to_string().cyan(),
            new_name.bold()
        );
        client
            .rename_place(response.root_place_id, new_name)
            .await
            .map_err(|e| {
                record::name_not_applied(
                    e,
                    "universe",
                    new_env.is_some(),
                    &format!(
                        "rbx init rename-universe --universe-id {} --name \"<name>\"",
                        response.universe_id
                    ),
                )
            })?;
        println!("  name: {}", new_name.bold());
    }

    Ok(())
}

#[cfg(test)]
mod finish_tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const RESPONSE: CreateUniverseResponse = CreateUniverseResponse {
        universe_id: 66778899001,
        root_place_id: 109876543210987,
    };

    async fn server_answering_rename(status: u16) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path(format!("/v2/places/{}", RESPONSE.root_place_id)))
            .respond_with(ResponseTemplate::new(status).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;
        server
    }

    fn new_env() -> NewEnv {
        NewEnv {
            env: "test".into(),
            place: "main".into(),
        }
    }

    /// The failure this order exists for: moderation refuses the name, and the
    /// universe must not end up on the account with nothing pointing at it.
    #[tokio::test]
    async fn a_refused_name_still_leaves_the_universe_recorded() {
        let server = server_answering_rename(400).await;
        let client = RbxClient::new(Some("test-cookie".into())).with_base_url(server.uri());
        let dir = tempfile::tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(&places, "").unwrap();

        let err = finish(&client, &RESPONSE, Some("Name"), Some(&new_env()), &places)
            .await
            .unwrap_err();

        let recorded = std::fs::read_to_string(&places).unwrap();
        assert!(recorded.contains("[test]"), "{recorded}");
        assert!(recorded.contains("66778899001"), "{recorded}");
        let shown = format!("{err:#}");
        assert!(shown.contains("recorded in rbxplace.toml"), "{shown}");
        assert!(
            shown.contains("rename-universe --universe-id 66778899001"),
            "{shown}"
        );
    }

    /// `--no-record`: nothing is in the file, so the message must not say so.
    #[tokio::test]
    async fn a_refused_name_without_recording_does_not_claim_a_record() {
        let server = server_answering_rename(400).await;
        let client = RbxClient::new(Some("test-cookie".into())).with_base_url(server.uri());
        let dir = tempfile::tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");

        let err = finish(&client, &RESPONSE, Some("Name"), None, &places)
            .await
            .unwrap_err();

        assert!(!places.exists());
        let shown = format!("{err:#}");
        assert!(!shown.contains("recorded"), "{shown}");
        assert!(shown.contains("rename-universe"), "{shown}");
    }

    #[tokio::test]
    async fn an_accepted_name_records_and_succeeds() {
        let server = server_answering_rename(200).await;
        let client = RbxClient::new(Some("test-cookie".into())).with_base_url(server.uri());
        let dir = tempfile::tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(&places, "").unwrap();

        finish(&client, &RESPONSE, Some("Name"), Some(&new_env()), &places)
            .await
            .unwrap();

        assert!(std::fs::read_to_string(&places).unwrap().contains("[test]"));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
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
