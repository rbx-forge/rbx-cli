use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use crate::api::RbxClient;
use crate::record::{self, NewPlace};
use rbx_core::confirm::confirm_always;
use rbx_core::GlobalFlags;

pub async fn run(
    global: &GlobalFlags,
    universe: u64,
    template_place_id: Option<u64>,
    name: Option<&str>,
    no_record: bool,
    yes: bool,
) -> Result<()> {
    // Same ordering rule as create-universe: every decision is made before the
    // place exists, so aborting at a prompt leaves nothing behind. The display
    // name comes first because the rbxplace.toml key is suggested from it.
    let prompted_name = match name {
        Some(_) => None,
        None => record::prompt_display_name("Place name", yes)?,
    };
    let name = name.or(prompted_name.as_deref());

    let new_place = record::choose_new_place(
        &global.places,
        global.env.as_deref(),
        global.place.as_deref(),
        universe,
        name,
        no_record,
        yes,
    )?;

    let cookie = global.resolve_cookie();
    let client = RbxClient::new(cookie);

    // #63: this run creates a place and may then rename it. A session that
    // dies between the two leaves a place named after nothing. Checked ahead
    // of the prompt so the question can name the account, too.
    client.require_valid_session().await?;
    let account = client.known_account().await;

    confirm_always(
        &rbx_core::session::as_account(
            account.as_ref(),
            &build_prompt(universe, name, new_place.as_ref()),
        ),
        yes,
    )?;

    println!(
        "Creating place inside universe {} ...",
        universe.to_string().bold()
    );
    let response = client.create_place(universe, template_place_id).await?;

    finish(
        &client,
        universe,
        response.place_id,
        name,
        new_place.as_ref(),
        &global.places,
    )
    .await
}

/// Everything after the place exists, in the same order as `create-universe`
/// and for the same reason: ids on screen, then the file, then the name, the
/// one step Roblox can refuse for what it says.
async fn finish(
    client: &RbxClient,
    universe: u64,
    place_id: u64,
    name: Option<&str>,
    new_place: Option<&NewPlace>,
    places: &Path,
) -> Result<()> {
    println!(
        "{} place (id {}) in universe {}",
        "Created".green().bold(),
        place_id.to_string().cyan(),
        universe.to_string().cyan(),
    );

    if let Some(target) = new_place {
        record::insert_place(places, &target.env, &target.place, place_id)?;
        println!(
            "{} places.{} to [{}] in {}",
            "Added".green().bold(),
            target.place.bold(),
            target.env,
            places.display()
        );
    }

    if let Some(new_name) = name {
        println!(
            "Renaming place {} to {} ...",
            place_id.to_string().cyan(),
            new_name.bold()
        );
        client.rename_place(place_id, new_name).await.map_err(|e| {
            record::name_not_applied(
                e,
                "place",
                new_place.is_some(),
                &format!("rbx init rename-place --place {place_id} --name \"<name>\""),
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

    const UNIVERSE: u64 = 66778899001;
    const PLACE: u64 = 109876543210987;

    /// A refused name must not leave a place that no env points at.
    #[tokio::test]
    async fn a_refused_name_still_leaves_the_place_recorded() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path(format!("/v2/places/{PLACE}")))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;
        let client = RbxClient::new(Some("test-cookie".into())).with_base_url(server.uri());
        let dir = tempfile::tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(
            &places,
            "[dev]\nuniverse_id = 66778899001\n\n[dev.places]\nmain = 1\n",
        )
        .unwrap();
        let target = NewPlace {
            env: "dev".into(),
            place: "lobby".into(),
        };

        let err = finish(
            &client,
            UNIVERSE,
            PLACE,
            Some("Name"),
            Some(&target),
            &places,
        )
        .await
        .unwrap_err();

        let recorded = std::fs::read_to_string(&places).unwrap();
        assert!(recorded.contains("lobby"), "{recorded}");
        assert!(recorded.contains(&PLACE.to_string()), "{recorded}");
        let shown = format!("{err:#}");
        assert!(shown.contains("recorded in rbxplace.toml"), "{shown}");
        assert!(
            shown.contains(&format!("rename-place --place {PLACE}")),
            "{shown}"
        );
    }
}

/// Last gate before the irreversible call, so it states the rbxplace.toml
/// entry too.
fn build_prompt(universe: u64, name: Option<&str>, new_place: Option<&NewPlace>) -> String {
    let subject = match name {
        Some(n) => format!("Create place '{n}'"),
        None => "Create a new place".to_string(),
    };
    let recorded = match new_place {
        Some(p) => format!(" and record it as [{}].places.{}", p.env, p.place),
        None => String::new(),
    };
    format!("{subject} under universe {universe}{recorded}?")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_mentions_the_recorded_place() {
        let target = NewPlace {
            env: "test".to_string(),
            place: "lobby".to_string(),
        };
        let prompt = build_prompt(42, Some("Lobby"), Some(&target));
        assert_eq!(
            prompt,
            "Create place 'Lobby' under universe 42 and record it as [test].places.lobby?"
        );
    }

    #[test]
    fn prompt_without_recording_keeps_the_original_wording() {
        assert_eq!(
            build_prompt(42, None, None),
            "Create a new place under universe 42?"
        );
    }
}
