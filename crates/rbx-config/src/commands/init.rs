use anyhow::{bail, Result};
use colored::Colorize;

use rbx_core::api::Repository;

use crate::ctx::ConfigCtx;

/// The comment block. Split from the entries so the `repository` line can go
/// between the two: a bare key after a table header belongs to that table, so
/// it cannot simply be appended.
const HEADER: &str = r#"# rbxconfig.toml: local source of truth for Roblox in-experience tunables.
#
# Structure: [<env>.entries."<key>"] with required `value` and optional `description`.
# Env names map to universes via rbxplace.toml (configurable with --places).
#
# Pull the live config:   `rbx config pull --env <name>`
# See pending changes:    `rbx config check --env <name>`
# Publish local changes:  `rbx config sync --env <name>`
"#;

const ENTRIES: &str = r#"[dev.entries."example.flag"]
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

    // The line appears only for a repository that is not the default, so
    // `rbx config init` writes the template it always wrote: a `repository`
    // line naming `InExperienceConfig` would be a fact about the default
    // rather than a decision, and one more thing to keep in step with it.
    let repository = ctx.flag_repository();
    let content = if repository == Repository::default() {
        format!("{HEADER}\n{ENTRIES}")
    } else {
        format!("{HEADER}\nrepository = \"{repository}\"\n\n{ENTRIES}")
    };

    std::fs::write(path, content)?;
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
