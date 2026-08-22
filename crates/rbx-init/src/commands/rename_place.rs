use anyhow::Result;
use colored::Colorize;

use crate::api::RbxClient;
use rbx_core::confirm::confirm_always;
use rbx_core::GlobalFlags;

pub async fn run(global: &GlobalFlags, place: u64, name: &str, yes: bool) -> Result<()> {
    let client = RbxClient::new(global.resolve_cookie());

    // #63: a write with the cookie, so the session is checked first, and
    // before the prompt, so the question names the account. A place id says
    // nothing about which account is about to rename it.
    client.require_valid_session().await?;
    let account = client.known_account().await;

    confirm_always(
        &rbx_core::session::as_account(
            account.as_ref(),
            &format!("Rename place {} to '{}'?", place, name),
        ),
        yes,
    )?;

    println!(
        "Renaming place {} to {} ...",
        place.to_string().cyan(),
        name.bold()
    );
    client.rename_place(place, name).await?;

    println!(
        "{} place {} renamed to {}",
        "Done:".green().bold(),
        place.to_string().cyan(),
        name.bold()
    );
    Ok(())
}
