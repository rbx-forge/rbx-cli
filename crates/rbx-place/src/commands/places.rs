use std::collections::HashMap;

use anyhow::Result;
use colored::Colorize;

use crate::api::RbxClient;
use crate::config::PlacesConfig;
use crate::json::PlacesDocument;
use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

use super::with_base;

pub async fn run(
    global: &GlobalFlags,
    base_url: Option<&str>,
    env: Option<&str>,
    universe_id_override: Option<u64>,
    json: bool,
) -> Result<()> {
    let format = OutputFormat::from_json_flag(json);
    let client = with_base(RbxClient::new(String::new()), base_url);

    let (universe_id, configured_places): (u64, Option<HashMap<String, u64>>) =
        if let Some(env_name) = env {
            let config = PlacesConfig::load(&global.places)?;
            let env_config = config.get_env(env_name)?;
            let uid = universe_id_override.unwrap_or(env_config.universe_id);
            (uid, Some(env_config.places.clone()))
        } else if let Some(uid) = universe_id_override {
            (uid, None)
        } else {
            anyhow::bail!("Either --env or --universe-id must be provided")
        };

    if !format.is_json() {
        print!("Places in universe {} ... ", universe_id.to_string().bold());
    }
    let places = client.list_universe_places(universe_id).await?;

    if format.is_json() {
        return output::emit(&PlacesDocument::new(
            env,
            universe_id,
            &places,
            configured_places.as_ref(),
        ));
    }

    println!("{}", format!("{} found", places.len()).green());
    println!();

    if places.is_empty() {
        println!("  (no places)");
        return Ok(());
    }

    for p in &places {
        let id = p
            .place_id()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "?".to_string());

        let status = if let Some(place_id) = p.place_id() {
            if let Some(ref configured) = configured_places {
                if let Some((key, _)) = configured.iter().find(|(_, pid)| **pid == place_id) {
                    format!(" → {}", key.bold().green())
                } else {
                    format!(" {} {}", "NOT in toml".dimmed(), "[missing]".red())
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        println!(
            "  {}  {}{}{}",
            id.cyan(),
            p.display_name,
            if p.max_player_count > 0 {
                format!("  (max {})", p.max_player_count)
                    .dimmed()
                    .to_string()
            } else {
                String::new()
            },
            status
        );
    }

    Ok(())
}
