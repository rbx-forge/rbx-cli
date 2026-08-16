use anyhow::{bail, Result};
use colored::Colorize;

use crate::ctx::ConfigCtx;

const TEMPLATE: &str = r#"# rbxconfig.toml — local source of truth for Roblox in-experience tunables.
#
# Structure: [<env>.entries."<key>"] with required `value` and optional `description`.
# Env names map to universes via rbxplace.toml (configurable with --places).
#
# Pull the live config:   `rbx config pull --env <name>`
# See pending changes:    `rbx config check --env <name>`
# Publish local changes:  `rbx config sync --env <name>`

[dev.entries."example.flag"]
value = true
description = "Example boolean flag. Replace or delete."

[dev.entries."example.number"]
value = 42

[dev.entries."example.object"]
value = { foo = 1, bar = 2 }
"#;

pub fn run(ctx: &ConfigCtx) -> Result<()> {
    let path = &ctx.config;
    if path.exists() {
        bail!("{} already exists", path.display());
    }

    std::fs::write(path, TEMPLATE)?;
    println!(
        "{} {}",
        "Created".green().bold(),
        path.display().to_string().cyan()
    );
    println!(
        "Edit it, then run `rbx config sync --env <name>` (or `rbx config pull --env <name>` \
         to populate from the live config)."
    );
    Ok(())
}
