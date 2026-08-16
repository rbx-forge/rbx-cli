use anyhow::Result;
use colored::Colorize;

use crate::api::RbxClient;
use rbx_core::GlobalFlags;

pub async fn run(global: &GlobalFlags) -> Result<()> {
    let cookie = global.resolve_cookie();
    let client = RbxClient::new(cookie);

    let groups = client.list_authenticated_user_groups().await?;

    if groups.is_empty() {
        println!("{}", "(you are not a member of any group)".dimmed());
        return Ok(());
    }
    println!("{} group(s):", groups.len().to_string().bold());
    for entry in &groups {
        println!(
            "  {}  {}  {}",
            entry.group.id.to_string().cyan(),
            entry.group.name.bold(),
            format!("(role: {}, rank {})", entry.role.name, entry.role.rank).dimmed()
        );
    }
    Ok(())
}
