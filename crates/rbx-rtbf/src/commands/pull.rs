//! `rbx rtbf pull`: write the published templates into `rbxrtbf.toml`.

use anyhow::Result;
use colored::Colorize;

use rbx_core::confirm::confirm_always;

use super::RtbfCtx;
use crate::config;
use crate::model::Templates;

pub async fn run(ctx: &RtbfCtx<'_>, yes: bool) -> Result<()> {
    // One universe, not a fan-out. Several universes' templates cannot be
    // written into one file without deciding which wins, and the answer to that
    // is a question about their contents rather than about the command.
    let universe_id = ctx.single()?;
    let client = ctx.client()?;

    let live = client.get_config(universe_id).await?;
    let (published, unrecognised) = Templates::from_entries(&live.entries);

    if unrecognised > 0 {
        println!(
            "{}",
            format!(
                "{unrecognised} published template(s) are in a shape this release does not \
                 understand, and writing the file would drop them. Upgrade `rbx`, or edit in \
                 the Creator Hub."
            )
            .yellow()
        );
        anyhow::bail!("refusing to write a file that would lose {unrecognised} template(s)");
    }

    super::show::print(&published, 1234567890, "published");
    println!();

    // Asked whatever the file currently says. `pull` is authoritative, so it
    // overwrites hand-written declarations that were never published, and that
    // is precisely the work somebody would not want silently discarded.
    let existing = ctx.config.exists();
    let what = if existing {
        format!("Overwrite {} with the above?", ctx.config.display())
    } else {
        format!("Write {} with the above?", ctx.config.display())
    };
    confirm_always(&what, yes)?;

    config::save(&ctx.config, &published)?;
    println!(
        "{} {}",
        "Updated".green().bold(),
        ctx.config.display().to_string().cyan()
    );
    Ok(())
}
