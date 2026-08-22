use anyhow::Result;
use colored::Colorize;

use crate::api::models::PlaceEntry;
use crate::api::RbxClient;
use crate::config::{Environment, PlacesConfig};
use rbx_core::GlobalFlags;

fn slugify(name: &str) -> String {
    let s = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>();
    let mut result = String::new();
    let mut prev_underscore = true;
    for c in s.chars() {
        if c == '_' {
            if !prev_underscore {
                result.push('_');
                prev_underscore = true;
            }
        } else {
            result.push(c);
            prev_underscore = false;
        }
    }
    result.trim_end_matches('_').to_string()
}

pub async fn run(
    global: &GlobalFlags,
    base_url: Option<&str>,
    env: &str,
    universe_id_override: Option<u64>,
    write: bool,
) -> Result<()> {
    let client = super::with_base(RbxClient::new(String::new()), base_url);

    let mut config = if global.places.exists() {
        PlacesConfig::load(&global.places)?
    } else {
        PlacesConfig::default()
    };

    let universe_id = universe_id_override
        .or_else(|| config.environments.get(env).map(|e| e.universe_id))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No universe_id found for env '{}'. Pass --universe-id <id> to specify one.",
                env
            )
        })?;

    print!(
        "Fetching places for universe {} (env: {}) ... ",
        universe_id.to_string().bold(),
        env.bold()
    );
    let remote: Vec<PlaceEntry> = client.list_universe_places(universe_id).await?;
    println!("{}", format!("{} found", remote.len()).green());
    println!();

    if remote.is_empty() {
        println!("  (no places found)");
        return Ok(());
    }

    let existing = config.environments.get(env);
    let mut new_places: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    for place in &remote {
        let Some(place_id) = place.place_id() else {
            continue;
        };

        let existing_key = existing
            .and_then(|e| e.places.iter().find(|(_, id)| **id == place_id))
            .map(|(k, _)| k.clone());

        let key = existing_key.unwrap_or_else(|| {
            let base = if place.display_name.is_empty() {
                format!("place_{}", place_id)
            } else {
                slugify(&place.display_name)
            };
            let mut candidate = base.clone();
            let mut n = 2;
            while new_places.contains_key(&candidate) {
                candidate = format!("{}_{}", base, n);
                n += 1;
            }
            candidate
        });

        println!(
            "  {}  {} → {}",
            place_id.to_string().cyan(),
            place.display_name.dimmed(),
            key.bold()
        );
        new_places.insert(key, place_id);
    }

    if write {
        let entry = config
            .environments
            .entry(env.to_string())
            .or_insert_with(|| Environment {
                universe_id,
                env: None,
                confirm: false,
                places: Default::default(),
                // A newly discovered env is visible to game code; opting out
                // is something you do on purpose.
                codegen: true,
            });
        entry.universe_id = universe_id;
        entry.places = new_places;
        config.save(&global.places)?;
        println!();
        println!(
            "{} Updated [{}] in {}",
            "✓".green(),
            env,
            global.places.display()
        );
    } else {
        println!();
        println!(
            "Dry run: pass {} to write to {}",
            "--write".bold(),
            global.places.display()
        );
    }

    Ok(())
}
