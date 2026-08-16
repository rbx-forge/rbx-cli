use anyhow::Result;
use colored::Colorize;

use crate::api::RbxClient;
use rbx_core::confirm::confirm_always;
use rbx_core::GlobalFlags;

pub async fn run(global: &GlobalFlags, universe: u64, name: &str, yes: bool) -> Result<()> {
    let client = RbxClient::new(global.resolve_cookie());

    // #63: the read below (root place) answers for any universe with no
    // session at all, so it is no proof the rename that follows will be
    // accepted. The session is checked before either — and before the prompt,
    // so the question names the account doing the renaming.
    client.require_valid_session().await?;
    let account = client.known_account().await;

    confirm_always(
        &rbx_core::session::as_account(
            account.as_ref(),
            &format!("Rename universe {} to '{}'?", universe, name),
        ),
        yes,
    )?;

    println!(
        "Resolving root place for universe {} ...",
        universe.to_string().cyan()
    );
    let root_place_id = client.get_universe_root_place(universe).await?;

    println!(
        "Renaming root place {} to {} ...",
        root_place_id.to_string().cyan(),
        name.bold()
    );
    client.rename_place(root_place_id, name).await?;

    println!(
        "{} universe {} renamed to {} (root place {})",
        "Done:".green().bold(),
        universe.to_string().cyan(),
        name.bold(),
        root_place_id.to_string().dimmed()
    );
    Ok(())
}
