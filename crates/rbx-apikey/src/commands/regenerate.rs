//! `rbx apikey regenerate <key>|--all`: rotate the API key secret.

use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::Confirm;

use crate::{config, lock, secret_store, time_iso};
use rbx_core::confirm::confirm_always;
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
        bail!("usage: rbx apikey regenerate <key>|--all [--yes]");
    }

    let cfg = config::load()?;
    let mut lk = lock::load()?;
    let places = PlacesFile::load(&global.places)?;
    let client = make_client(global);

    crate::drift::check_all(&lk, &places)?;

    let names: Vec<String> = if all {
        let mut v: Vec<String> = lk.keys.keys().cloned().collect();
        v.sort();
        if v.is_empty() {
            bail!("no keys in {}", lock::FILE);
        }

        println!("About to rotate {} secrets: {}", v.len(), v.join(", "));
        println!(
            "{}",
            "This invalidates ALL current secrets at once.".yellow()
        );
        if !skip_confirm {
            let ok = Confirm::new()
                .with_prompt("Proceed?")
                .default(false)
                .interact()?;
            if !ok {
                println!("{}", "Aborted.".yellow());
                return Ok(());
            }
        }
        v
    } else {
        let single = name.unwrap().to_string();
        // Single-key rotation: confirm once unless --yes. The --all path above
        // has its own explicit "Proceed?" prompt because it lists every key.
        //
        // The session is checked first so the question can name the account.
        // These keys are minted on whichever account the cookie signs in as,
        // and a key name says nothing about which one that is.
        client.require_valid_session().await?;
        let account = client.known_account().await;
        confirm_always(
            &rbx_core::session::as_account(
                account.as_ref(),
                &format!(
                    "Rotate \"{}\"? Old secret will be invalidated immediately.",
                    single
                ),
            ),
            skip_confirm,
        )?;
        vec![single]
    };

    // Before the first rotation. Rotating invalidates the old secret the
    // moment it lands, so a run that gets through two keys and is then refused
    // has left two projects holding secrets nothing wrote down. Cached, so the
    // single-key path above has already paid for this.
    client.require_valid_session().await?;

    for n in &names {
        regenerate_one(&cfg, &mut lk, &client, n, clean_files).await;
        lock::save(&lk)?;
    }
    println!("Old secrets invalid as of {}.", time_iso::iso_now());
    Ok(())
}

async fn regenerate_one(
    cfg: &config::Config,
    lk: &mut lock::Lock,
    client: &crate::api::RbxApiKeyClient,
    name: &str,
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

    println!("Regenerating \"{}\" (id={})...", name, entry.cloud_auth_id);
    let resp = match client.regenerate_secret(&entry.cloud_auth_id).await {
        Ok(r) => r,
        Err(e) => {
            println!(
                "{}",
                format!("\"{}\": regenerate failed: {}", name, e).yellow()
            );
            return;
        }
    };

    let new_secret = resp.apikey_secret;
    let key_cfg = config::get(cfg, name);

    // Where the OLD (now-invalidated) secret was stored, vs. where the NEW one belongs.
    // After a successful rotate, the OLD location holds garbage; we offer to clean it up
    // when `secret_file` was changed in rbxapikey.toml between writes.
    let previous_resolved = secret_store::previous_backend_from_entry(&entry, name);
    let new_resolved = secret_store::backend_for(cfg, key_cfg, name);
    let backend_changed = secret_store::backend_differs(&previous_resolved, &new_resolved);

    let mut entry = entry;
    if let Err(e) = secret_store::write(&new_resolved, &new_secret, &mut entry) {
        println!(
            "{}",
            format!(
                "  ✗ Roblox rotated the secret but writing it locally failed: {}",
                e
            )
            .red()
        );
        println!(
            "{}",
            format!("  Secret (save this manually): {}", new_secret).yellow()
        );
        return;
    }

    // Keep the lock entry's secret_file pointer in sync with the new backend so future
    // `previous_backend_from_entry` calls see the right "previous" state.
    entry.secret_file = if new_resolved.backend == secret_store::Backend::File {
        Some(new_resolved.target.clone())
    } else {
        None
    };
    lock::set(lk, name, entry);

    if backend_changed {
        println!(
            "{}",
            format!(
                "  ↪ Secret backend changed: {}:{} → {}:{} (new secret written at new location)",
                previous_resolved.backend.as_str(),
                previous_resolved.target,
                new_resolved.backend.as_str(),
                new_resolved.target
            )
            .cyan()
        );

        if previous_resolved.backend == secret_store::Backend::File {
            let should_delete = if clean_files {
                true
            } else {
                Confirm::new()
                    .with_prompt(format!(
                        "Old secret file \"{}\" holds the now-invalid secret. Delete it?",
                        previous_resolved.target
                    ))
                    .default(false)
                    .interact()
                    .unwrap_or(false)
            };
            let cleanup = secret_store::cleanup(&previous_resolved, should_delete);
            for msg in &cleanup.auto_cleaned {
                println!("{}", format!("  ✓ {}", msg).green());
            }
            for msg in &cleanup.manual_action_needed {
                println!("{}", format!("  ⚠ {}", msg).yellow());
            }
        }
    }

    println!("{}", format!("✓ \"{}\": secret rotated", name).green());
}
