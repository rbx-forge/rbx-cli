//! `rbx apikey delete <key>|--all` — remove a key from Roblox and the local lock.

use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::Confirm;

use crate::{config, lock, secret_store};
use rbx_core::places::PlacesFile;
use rbx_core::GlobalFlags;

use super::{make_client, require_no_collision};

pub async fn run(
    global: &GlobalFlags,
    name: Option<&str>,
    all: bool,
    skip_confirm: bool,
    clean_files: bool,
) -> Result<()> {
    require_no_collision(all, name)?;
    if !all && name.is_none() {
        bail!("usage: rbx apikey delete <key>|--all [--yes] [--clean-files]");
    }

    let cfg = config::load()?;
    let mut lk = lock::load()?;
    let places = PlacesFile::load(&global.places)?;
    let client = make_client(global);

    crate::drift::check_all(&lk, &places)?;

    // Asked here so the confirmations below can name the account. It is the
    // same call the pre-delete gate makes, cached for the process, so this
    // costs one round trip rather than two — and a dead session now refuses
    // before a person is asked to approve anything.
    client.require_valid_session().await?;

    let names: Vec<String> = if all {
        let mut v: Vec<String> = lk.keys.keys().cloned().collect();
        v.sort();
        if v.is_empty() {
            bail!("no keys in {}", lock::FILE);
        }

        println!("About to DELETE {} keys: {}", v.len(), v.join(", "));
        println!("{}", "This is IRREVERSIBLE.".yellow());
        if !skip_confirm {
            if let Some(a) = client.known_account().await {
                println!("On account {}.", a.label());
            }
            if !Confirm::new()
                .with_prompt("First confirm: really delete ALL keys?")
                .default(false)
                .interact()?
            {
                println!("{}", "Aborted.".yellow());
                return Ok(());
            }
            if !Confirm::new()
                .with_prompt("Second confirm: are you really sure?")
                .default(false)
                .interact()?
            {
                println!("{}", "Aborted.".yellow());
                return Ok(());
            }
        }
        v
    } else {
        let name = name.unwrap();
        let entry = lock::get(&lk, name)
            .ok_or_else(|| anyhow::anyhow!("\"{}\" has no entry in {}", name, lock::FILE))?;
        if !skip_confirm {
            match client.known_account().await {
                Some(a) => println!(
                    "About to DELETE \"{}\" (id={}) on account {}.",
                    name,
                    entry.cloud_auth_id,
                    a.label()
                ),
                None => println!("About to DELETE \"{}\" (id={}).", name, entry.cloud_auth_id),
            }
            if !Confirm::new()
                .with_prompt("This is irreversible. Proceed?")
                .default(false)
                .interact()?
            {
                println!("{}", "Aborted.".yellow());
                return Ok(());
            }
        }
        vec![name.to_string()]
    };

    // Before the first delete. `delete_one` swallows its own failures so one
    // bad key does not abort the rest, which is the right call for a per-key
    // fault and the wrong one for a dead session: without this, `--all` on an
    // expired cookie prints a failure per key and deletes none, having asked
    // Roblox once per key to find out.
    //
    // Cached: the confirmations above already asked, which is what let them
    // name the account. Deleting is the verb where that matters most, because
    // the same key names recur across accounts and a delete cannot be undone.
    client.require_valid_session().await?;

    for n in &names {
        delete_one(&cfg, &mut lk, &client, n, skip_confirm, clean_files).await;
    }
    lock::save(&lk)?;
    Ok(())
}

/// Delete one key the project tracks: Roblox, then the lockfile entry, then
/// the stored secret. `prune` calls this for its tracked selections so the
/// cleanup cannot drift between the two commands.
pub(crate) async fn delete_one(
    cfg: &config::Config,
    lk: &mut lock::Lock,
    client: &crate::api::RbxApiKeyClient,
    name: &str,
    skip_confirm: bool,
    clean_files: bool,
) {
    let entry = match lock::get(lk, name).cloned() {
        Some(e) => e,
        None => {
            println!(
                "{}",
                format!("skipping \"{}\": not in {}", name, lock::FILE).yellow()
            );
            return;
        }
    };

    let key_cfg = config::get(cfg, name);
    let resolved = secret_store::backend_for(cfg, key_cfg, name);

    if let Err(e) = client.delete_api_key(&entry.cloud_auth_id).await {
        println!(
            "{}",
            format!(
                "\"{}\": Roblox delete failed (removing locally anyway): {}",
                name, e
            )
            .yellow()
        );
    }

    lock::remove(lk, name);

    let should_delete_file =
        if resolved.backend == secret_store::Backend::File && !clean_files && !skip_confirm {
            Confirm::new()
                .with_prompt(format!("Also delete secret file \"{}\"?", resolved.target))
                .default(false)
                .interact()
                .unwrap_or(false)
        } else {
            clean_files
        };

    let cleanup = secret_store::cleanup(&resolved, should_delete_file);
    for msg in &cleanup.auto_cleaned {
        println!("{}", format!("  ✓ {}", msg).green());
    }
    for msg in &cleanup.manual_action_needed {
        println!("{}", format!("  ⚠ {}", msg).yellow());
    }

    println!(
        "{}",
        format!("✓ \"{}\" deleted (id={})", name, entry.cloud_auth_id).green()
    );
}
