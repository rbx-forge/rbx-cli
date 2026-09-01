use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use chrono::Local;
use colored::Colorize;
use serde_json::{json, Value};

use crate::config::PlacesConfig;
use crate::json::{WriteCommand, WriteDocument};
use rbx_core::confirm::confirm_destructive;
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

use super::{cannot_ask, make_client, upload_and_classify};

// promote has 10 user-facing flags (source env, target env, place, all_places,
// pinned version, from_published, from_saved, published, log path, etc.).
// Bundling them into a struct hides the CLI shape without making the call site
// any clearer, since each flag maps 1:1 onto a clap arg in lib.rs.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    global: &GlobalFlags,
    base_url: Option<&str>,
    from: &str,
    to: &str,
    place: Option<&str>,
    all_places: bool,
    version: Option<u64>,
    from_published: bool,
    from_saved: bool,
    published: bool,
    log: Option<&Path>,
    yes: bool,
    json: bool,
) -> Result<()> {
    if from == to {
        anyhow::bail!("Source and target environments are the same: '{}'", from);
    }

    // `--env` is not how promote names an env: `--from` and `--to` are, one
    // each, and a selector standing for several has no way to split itself
    // across the two. Refusing beats accepting the flag and ignoring it, which
    // is the failure `--universe-id` records in lib.rs from the last time it
    // happened. Asked through `EnvSelector` so a group is turned away wherever
    // `all` is.
    if global
        .env_selector()?
        .is_some_and(|selector| selector.is_plural())
    {
        anyhow::bail!(
            "`rbx place promote` names its two envs itself, with --from and --to, so a plural \
             --env selects nothing here. Drop it, or run promote once per pair of envs."
        );
    }

    let format = OutputFormat::from_json_flag(json);
    let config = PlacesConfig::load(&global.places)?;
    let from_env = config.get_env(from)?;
    let to_env = config.get_env(to)?;
    let client = make_client(global, base_url)?;

    let (src_name, src_id) = from_env.resolve_place(place)?;

    // Resolve the exact source version upfront so the log is always accurate.
    let src_version: u64 = if let Some(v) = version {
        v
    } else if from_published {
        client
            .find_version(src_id, true)
            .await?
            .ok_or_else(|| anyhow::anyhow!("No published version found for place {}", src_id))?
            .version_number
    } else if from_saved {
        client
            .find_version(src_id, false)
            .await?
            .ok_or_else(|| anyhow::anyhow!("No saved version found for place {}", src_id))?
            .version_number
    } else {
        // Latest: resolve the actual version number so we can log it.
        client
            .list_versions(src_id, 1)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No versions found for place {}", src_id))?
            .version_number
    };

    let targets: Vec<(String, u64)> = if all_places {
        to_env.all_places_sorted()
    } else {
        vec![to_env.resolve_place(Some(&src_name))?]
    };

    let version_type = if published { "published" } else { "saved" };
    let target_names: Vec<&str> = targets.iter().map(|(n, _)| n.as_str()).collect();

    if !format.is_json() {
        println!(
            "Promoting {}/{} (v{}) → {} [{}]",
            from.bold(),
            src_name.bold(),
            src_version,
            to.bold(),
            target_names.join(", ")
        );
        println!("  From universe: {}", from_env.universe_id);
        println!("  To universe:   {}", to_env.universe_id);
        println!("  Version type:  {}", version_type);
        println!();
    }

    // Refused before the download, let alone the writes: a run that cannot be
    // confirmed must not spend a place file's worth of transfer finding out.
    if to_env.confirm && !yes && !format.may_prompt() {
        return Err(cannot_ask(format, "for confirmation", "--yes"));
    }
    // `--all-places` is not a plural of the default, it is a different
    // operation: every target receives the *same* bytes, downloaded once from
    // the single source place. Listing the targets without saying so reads as
    // "lobby gets lobby", which is what the no-flag path does and this one
    // does not. Naming it in the question is the only place a person can still
    // stop it: three places get a new version each and the only way back is
    // three rollbacks.
    let fan_out = if all_places && targets.len() > 1 {
        format!(
            " Every one of them is overwritten with {}/{}, not with its own counterpart.",
            from, src_name
        )
    } else {
        String::new()
    };
    confirm_destructive(
        &format!(
            "Promote {}/{} v{} → {} ({})? This will {} {}.{}",
            from,
            src_name,
            src_version,
            to,
            target_names.join(", "),
            if published { "publish" } else { "save as" },
            if published { "live" } else { "draft" },
            fan_out
        ),
        to_env.confirm,
        yes,
    )?;

    if !format.is_json() {
        print!(
            "  Downloading {}/{} v{} ... ",
            from.bold(),
            src_name.bold(),
            src_version
        );
    }
    let url = client.get_download_url(src_id, Some(src_version)).await?;
    let data = client.download_from_url(&url).await?;
    let size_kb = data.len() as f64 / 1024.0;
    if !format.is_json() {
        println!("{}", format!("{:.1} KB", size_kb).green());
        println!();
    }

    // (target_name, target_id, new_version)
    let mut results: Vec<(String, u64, u64)> = Vec::new();
    let mut receipt = WriteDocument::new(
        WriteCommand::Promote,
        to,
        to_env.universe_id,
        published,
        !all_places,
    )
    .promoted_from(from, &src_name, src_id, src_version);

    for (target_name, target_id) in &targets {
        if !format.is_json() {
            print!("  {} ({}) ... ", target_name.bold(), target_id);
        }
        match upload_and_classify(
            &client,
            to_env.universe_id,
            *target_id,
            data.clone(),
            published,
        )
        .await
        {
            Ok((new_version, landing)) => {
                if !format.is_json() {
                    println!(
                        "{}{}",
                        format!("v{}", new_version).green(),
                        landing.note().unwrap_or_default()
                    );
                }
                results.push((target_name.clone(), *target_id, new_version));
                receipt.landed(target_name, *target_id, new_version, landing.created());
            }
            Err(e) => {
                // Whatever already landed stays landed, so the receipt reports
                // it. `--log` is not written: a partial promote does not write
                // one in the human form either, and the two must not disagree
                // about what a deploy record means.
                if format.is_json() {
                    output::emit(&receipt.failed(&e))?;
                } else {
                    println!("{}", "failed".red());
                }
                return Err(e);
            }
        }
    }

    if !format.is_json() {
        println!();
        println!("{}", "Promote complete.".green());
    }

    if let Some(log_path) = log {
        if let Err(e) = write_log(
            log_path,
            from,
            from_env.universe_id,
            src_id,
            src_version,
            to,
            to_env.universe_id,
            &results,
        ) {
            // An unwritable log does not un-upload the places. The receipt
            // still goes out saying what landed, and the run still fails.
            if format.is_json() {
                output::emit(&receipt.failed(&e))?;
            }
            return Err(e);
        }
        // Human: the same line on stdout as before. `--json`: stderr, because
        // stdout carries the document.
        format.note(format!("  Log written → {}", log_path.display()));
    }

    if format.is_json() {
        return output::emit(&receipt);
    }

    Ok(())
}

// write_log captures the full promote payload (source env, source universe,
// source place, source version, target env, target universe, version type, and
// the per-place result list). Reasonable to keep flat: it's an internal helper
// that's called once.
#[allow(clippy::too_many_arguments)]
fn write_log(
    path: &Path,
    from_env: &str,
    from_universe_id: u64,
    src_place_id: u64,
    src_version: u64,
    to_env: &str,
    to_universe_id: u64,
    results: &[(String, u64, u64)],
) -> Result<()> {
    let mut log: HashMap<String, Value> = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        HashMap::new()
    };

    let deployed_at = Local::now().to_rfc3339();

    for (place_name, to_place_id, to_version) in results {
        let entry = json!({
            "deployedAt": deployed_at,
            from_env: {
                "universeId": from_universe_id,
                "placeId": src_place_id,
                "version": src_version,
            },
            to_env: {
                "universeId": to_universe_id,
                "placeId": to_place_id,
                "version": to_version,
            },
        });
        log.insert(place_name.clone(), entry);
    }

    let content = serde_json::to_string_pretty(&log)?;
    std::fs::write(path, content)?;
    Ok(())
}
