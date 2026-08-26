//! `rbx config versions`: the publish history for one universe.
//!
//! `--json` emits the same revisions as one document, with the keys each one
//! changed rather than a count of them; see `crate::json`.

use anyhow::Result;
use colored::Colorize;

use rbx_core::output::{self, OutputFormat};

use crate::ctx::ConfigCtx;
use crate::json::VersionsDocument;

use super::make_client;

pub async fn run(ctx: &ConfigCtx, count: usize, json: bool) -> Result<()> {
    let universe_id = ctx.resolve_universe_only()?;
    let client = make_client(ctx, ctx.flag_repository())?;
    let format = OutputFormat::from_json_flag(json);

    // Decoration over a single request, and the document already carries the
    // env, the universe and the revisions, so under `--json` there is nothing
    // here to move to stderr, only something not to print.
    if !format.is_json() {
        print!(
            "Revisions for {} ({}) ... ",
            ctx.env_label().bold(),
            universe_id
        );
    }
    let revisions = client.list_revisions(universe_id, count).await?;

    if format.is_json() {
        return output::emit(&VersionsDocument::new(
            ctx.env.as_deref(),
            universe_id,
            count,
            &revisions,
        ));
    }

    println!("{}", format!("{} found", revisions.len()).green());
    println!();

    if revisions.is_empty() {
        println!("  (no revisions)");
        return Ok(());
    }

    for (i, rev) in revisions.iter().enumerate() {
        let message = rev.message.as_deref().unwrap_or("(no message)");

        let change_count = rev.changes.len();
        let change_summary = if change_count == 0 {
            String::new()
        } else {
            format!("  ({})", format!("{} key(s)", change_count).dimmed())
        };

        let tag = if i == 0 {
            format!("  {}", "[published]".cyan())
        } else {
            String::new()
        };

        let short_id: String = rev.revision_id.chars().take(8).collect();
        println!(
            "  {}  v{:<4}  {} UTC  {}{}{}",
            short_id.bright_cyan(),
            rev.version,
            rev.time.replace('T', " ").trim_end_matches('Z').dimmed(),
            message,
            change_summary,
            tag
        );
    }

    Ok(())
}
