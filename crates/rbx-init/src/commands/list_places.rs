use anyhow::Result;
use colored::Colorize;

use crate::api::RbxClient;
use rbx_core::GlobalFlags;

pub async fn run(global: &GlobalFlags, universe: u64) -> Result<()> {
    let client = RbxClient::new(global.resolve_cookie());

    let places = client.list_universe_places(universe).await?;

    if places.is_empty() {
        println!("{}", "(no places)".dimmed());
        return Ok(());
    }
    println!(
        "{} place(s) in universe {}:",
        places.len().to_string().bold(),
        universe.to_string().cyan()
    );
    for p in &places {
        println!("  {}  {}", p.id.to_string().cyan(), p.name.bold());
    }
    Ok(())
}
