use std::path::Path;

use anyhow::{Context, Result};
use bytes::Bytes;
use colored::Colorize;

use crate::config::PlacesConfig;
use crate::json::{WriteCommand, WriteDocument};
use rbx_core::confirm::confirm_destructive;
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

use super::{cannot_ask, make_client};

// One more argument than clippy's threshold, and the same reasoning the other
// commands here carry: every one of these maps 1:1 onto a clap arg in lib.rs,
// so a struct would hide the CLI shape without making the call site clearer.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    global: &GlobalFlags,
    base_url: Option<&str>,
    env: &str,
    place: Option<&str>,
    all_places: bool,
    file: &Path,
    published: bool,
    yes: bool,
    json: bool,
) -> Result<()> {
    let format = OutputFormat::from_json_flag(json);
    let config = PlacesConfig::load(&global.places)?;
    let env_config = config.get_env(env)?;
    let client = make_client(global, base_url)?;

    // `Bytes`, not `Vec<u8>`: the file is read once and every place in
    // `targets` (and every retry inside each upload) shares that one buffer.
    let data = Bytes::from(
        std::fs::read(file).with_context(|| format!("Failed to read file: {}", file.display()))?,
    );
    let size_kb = data.len() as f64 / 1024.0;

    let targets: Vec<(String, u64)> = if all_places {
        env_config.all_places_sorted()
    } else {
        vec![env_config.resolve_place(place)?]
    };

    let version_type = if published { "published" } else { "saved" };
    let target_names: Vec<&str> = targets.iter().map(|(n, _)| n.as_str()).collect();

    if !format.is_json() {
        println!(
            "Uploading {} ({:.1} KB) → {} [{}]",
            file.display(),
            size_kb,
            target_names.join(", "),
            env.bold()
        );
        println!("  Universe: {}", env_config.universe_id);
        println!("  Version type: {}", version_type);
        println!();
    }

    // The confirmation is a question, so it has to be refused before it is
    // asked when nothing can answer it. Nothing has been written at this point,
    // so this failure leaves stdout empty rather than emitting a receipt for a
    // run that never happened.
    if env_config.confirm && !yes && !format.may_prompt() {
        return Err(cannot_ask(format, "for confirmation", "--yes"));
    }
    confirm_destructive(
        &format!(
            "Upload to {} ({})? This will {} {}.",
            env,
            target_names.join(", "),
            if published { "publish" } else { "save as" },
            if published { "live" } else { "draft" }
        ),
        env_config.confirm,
        yes,
    )?;

    let mut receipt = WriteDocument::new(
        WriteCommand::Upload,
        env,
        env_config.universe_id,
        published,
        !all_places,
    );

    for (place_name, place_id) in &targets {
        if !format.is_json() {
            print!("  {} ({}) ... ", place_name.bold(), place_id);
        }
        match client
            .upload_place(env_config.universe_id, *place_id, data.clone(), published)
            .await
        {
            Ok(version) => {
                if !format.is_json() {
                    println!("{}", format!("v{}", version).green());
                }
                receipt.landed(place_name, *place_id, version);
            }
            Err(e) => {
                // The places already uploaded to have new versions whatever
                // happens to this one, so the receipt goes out reporting them
                // before the error propagates. The process still exits
                // non-zero, and the document says `"ok": false`.
                if format.is_json() {
                    output::emit(&receipt.failed(&e))?;
                } else {
                    println!("{}", "failed".red());
                }
                return Err(e);
            }
        }
    }

    if format.is_json() {
        return output::emit(&receipt);
    }

    println!();
    println!("{}", "Upload complete.".green());
    Ok(())
}
