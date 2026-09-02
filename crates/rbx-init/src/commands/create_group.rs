use std::path::Path;

use anyhow::{bail, Context, Result};
use colored::Colorize;

use crate::api::RbxClient;
use crate::record;
use rbx_core::confirm::confirm_always;
use rbx_core::owner::{Owner, OwnerType};
use rbx_core::places::PlacesFile;
use rbx_core::GlobalFlags;

pub async fn run(
    global: &GlobalFlags,
    name: &str,
    description: &str,
    public: bool,
    icon: &Path,
    record_owner: bool,
    yes: bool,
) -> Result<()> {
    if !icon.exists() {
        bail!("Icon file not found: {}", icon.display());
    }
    // Before anything is spent. Creating a group costs 100 Robux and cannot be
    // undone, so a file that already names an owner has to stop the run here
    // rather than after the purchase, when the only thing left to do is print
    // the id and hope somebody copies it. Same rule as `choose_new_env`:
    // decide, then create.
    if record_owner {
        ensure_owner_absent(&global.places)?;
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
            &build_prompt(record_owner, &global.places),
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

    // After the id is on screen: a failed write must still leave the reader
    // with everything needed to record the group by hand.
    if record_owner {
        record::append_owner(
            &global.places,
            Owner {
                kind: OwnerType::Group,
                id: response.id,
            },
        )?;
        println!(
            "{} [owner] to {}",
            "Added".green().bold(),
            global.places.display()
        );
    }
    Ok(())
}

/// Refuse to record over an owner the file already declares.
///
/// A missing file is fine: this is the one writer that may create
/// `rbxplace.toml`, because a group is created before any env exists. A file
/// that *is* there but does not parse is not fine, and the load error is
/// propagated rather than swallowed: appending to a broken file would spend
/// 100 Robux to produce a second problem.
fn ensure_owner_absent(places: &Path) -> Result<()> {
    if !places.exists() {
        return Ok(());
    }
    let file = PlacesFile::load(places)?;
    if let Some(owner) = file.owner {
        bail!(
            "{} already declares [owner] ({}). Remove that block to record a new group, \
             or drop --record to create the group without recording it.",
            places.display(),
            owner
        );
    }
    Ok(())
}

/// The last gate before a purchase, so it names what the run will do to disk
/// as well as what it will do on Roblox.
fn build_prompt(record_owner: bool, places: &Path) -> String {
    if record_owner {
        format!(
            "This will create a Roblox group (costs 100 Robux) and record it as [owner] in {}. \
             Proceed?",
            places.display()
        )
    } else {
        "This will create a Roblox group (costs 100 Robux). Proceed?".to_string()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn write(dir: &Path, contents: &str) -> std::path::PathBuf {
        let path = dir.join("rbxplace.toml");
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// The bare-repo case `--record` exists for: nothing on disk yet, and the
    /// group is the first thing the project creates.
    #[test]
    fn a_missing_file_is_not_an_obstacle() {
        let dir = tempfile::tempdir().unwrap();
        ensure_owner_absent(&dir.path().join("rbxplace.toml")).unwrap();
    }

    #[test]
    fn a_file_without_an_owner_block_accepts_the_recording() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "[prod]\nuniverse_id = 1\n");
        ensure_owner_absent(&path).unwrap();
    }

    /// The guard that makes the command re-runnable: a second run costs
    /// nothing instead of spending another 100 Robux on a duplicate group.
    #[test]
    fn an_existing_owner_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "[owner]\ntype = \"group\"\nid = 1234567890\n");
        let err = ensure_owner_absent(&path).unwrap_err().to_string();
        assert!(err.contains("group 1234567890"), "got: {err}");
        assert!(err.contains("--record"), "got: {err}");
    }

    /// A file that does not parse must stop the run before the purchase, not
    /// be silently treated as "no owner" and appended to.
    #[test]
    fn an_unparseable_file_stops_the_run_rather_than_being_appended_to() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "[owner]\ntype = \"neither\"\nid = 1\n");
        assert!(ensure_owner_absent(&path).is_err());
    }

    #[test]
    fn the_prompt_says_when_it_will_write_the_file() {
        let plain = build_prompt(false, Path::new("rbxplace.toml"));
        assert!(plain.contains("100 Robux"));
        assert!(!plain.contains("[owner]"));

        let recording = build_prompt(true, Path::new("rbxplace.toml"));
        assert!(recording.contains("[owner]"), "got: {recording}");
        assert!(recording.contains("rbxplace.toml"), "got: {recording}");
    }
}
