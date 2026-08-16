use anyhow::Result;
use colored::Colorize;

use crate::api::RbxClient;
use rbx_core::GlobalFlags;

pub async fn run(global: &GlobalFlags, group: u64) -> Result<()> {
    let client = RbxClient::new(global.resolve_cookie());

    let universes = client.list_group_universes(group).await?;

    if universes.is_empty() {
        println!("{}", "(no public universes in this group)".dimmed());
        return Ok(());
    }
    println!(
        "{} universe(s) in group {}:",
        universes.len().to_string().bold(),
        group.to_string().cyan()
    );
    for u in &universes {
        println!(
            "  {}  {}  {}",
            u.id.to_string().cyan(),
            u.name.bold(),
            format!("(root place {})", u.root_place.id).dimmed()
        );
    }
    Ok(())
}
