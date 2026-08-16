use std::path::Path;

use anyhow::{bail, Context, Result};
use colored::Colorize;

use crate::api::RbxClient;
use rbx_core::confirm::confirm_always;
use rbx_core::GlobalFlags;

pub async fn run(
    global: &GlobalFlags,
    name: &str,
    description: &str,
    public: bool,
    icon: &Path,
    yes: bool,
) -> Result<()> {
    if !icon.exists() {
        bail!("Icon file not found: {}", icon.display());
    }
    let icon_bytes =
        std::fs::read(icon).with_context(|| format!("Failed to read icon: {}", icon.display()))?;
    let icon_name = icon
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "icon.png".to_string());

    let cookie = global.resolve_cookie();
    let client = RbxClient::new(cookie);

    // #63: creating a group costs 100 Robux and cannot be undone. Finding out
    // from Roblox that the session died is a worse place to find out than
    // here, where nothing has been spent.
    //
    // Before the prompt rather than after it, which is also what lets the
    // question name the account: the check has just identified it, so reading
    // it back costs nothing, and asking somebody to approve a purchase against
    // an account they did not expect is the mistake a prompt about Robux
    // cannot catch on its own.
    client.require_valid_session().await?;
    let account = client.known_account().await;

    confirm_always(
        &rbx_core::session::as_account(
            account.as_ref(),
            "This will create a Roblox group (costs 100 Robux). Proceed?",
        ),
        yes,
    )?;

    println!(
        "Creating group {} ({}) ...",
        name.bold(),
        if public { "public" } else { "invite-only" }.dimmed()
    );
    let response = client
        .create_group(name, description, public, icon_bytes, icon_name)
        .await?;

    println!(
        "{} group {} (id {})",
        "Created".green().bold(),
        response.name.bold(),
        response.id.to_string().cyan()
    );
    if let Some(owner) = &response.owner {
        println!("  owner:   {} (id {})", owner.name, owner.id);
    }
    Ok(())
}
