//! `rbx config list` — the published config keys, with types and previews.
//!
//! `--json` emits the same snapshot as one document; see `crate::json` for why
//! it is the same document `get --json` emits.

use anyhow::Result;
use colored::Colorize;

use rbx_core::output::{self, OutputFormat};

use crate::ctx::ConfigCtx;
use crate::json::LiveDocument;
use crate::value::{compact, type_label};

use super::make_client;

pub async fn run(ctx: &ConfigCtx, json: bool) -> Result<()> {
    let universe_id = ctx.resolve_universe_only()?;
    let client = make_client(ctx)?;
    let format = OutputFormat::from_json_flag(json);

    let snapshot = client.get_config(universe_id).await?;

    if format.is_json() {
        return output::emit(&LiveDocument::snapshot(
            ctx.env.as_deref(),
            universe_id,
            &snapshot,
        ));
    }

    println!(
        "Live config keys — env: {} (configVersion {})",
        ctx.env_label().bold(),
        snapshot.metadata.config_version
    );

    if snapshot.entries.is_empty() {
        println!("  (none)");
        return Ok(());
    }

    let max_key_len = snapshot.entries.keys().map(|k| k.len()).max().unwrap_or(0);
    let max_type_len = snapshot
        .entries
        .values()
        .map(|v| type_label(v).len())
        .max()
        .unwrap_or(0);

    for (k, v) in &snapshot.entries {
        let t = type_label(v);
        println!(
            "  {}{} [{}]{}  {}",
            k.bold(),
            " ".repeat(max_key_len - k.len()),
            t.cyan(),
            " ".repeat(max_type_len - t.len()),
            compact(v).dimmed()
        );
    }

    Ok(())
}
