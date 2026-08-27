//! `rbx config get`: the published config, or one key of it.
//!
//! `--json` wraps the answer in a document rather than printing it bare; see
//! `crate::json`.

use anyhow::Result;
use colored::Colorize;

use rbx_core::output::{self, OutputFormat};

use crate::ctx::ConfigCtx;
use crate::json::LiveDocument;
use crate::value::{compact, type_label};

use super::make_client;

pub async fn run(ctx: &ConfigCtx, key: Option<&str>, json: bool) -> Result<()> {
    let universe_id = ctx.resolve_universe_only()?;
    let client = make_client(ctx, ctx.flag_repository())?;
    let format = OutputFormat::from_json_flag(json);

    let snapshot = client.get_config(universe_id).await?;

    if let Some(k) = key {
        let value = snapshot.entries.get(k).ok_or_else(|| {
            anyhow::anyhow!("Key \"{}\" not found in [{}] config", k, ctx.env_label())
        })?;
        // An unknown key stays an error under `--json` too: a document
        // reporting a null value would let a pipeline treat "not published"
        // as "published as nothing".
        if format.is_json() {
            return output::emit(&LiveDocument::single(
                ctx.env.as_deref(),
                universe_id,
                &snapshot,
                k,
                value,
            ));
        }
        let s = serde_json::to_string_pretty(value)?;
        println!("{}", s);
    } else {
        if format.is_json() {
            return output::emit(&LiveDocument::snapshot(
                ctx.env.as_deref(),
                universe_id,
                &snapshot,
            ));
        }
        println!(
            "Live config: env: {} (universe {})",
            ctx.env_label().bold(),
            universe_id
        );
        println!("  configVersion: {}", snapshot.metadata.config_version);

        if snapshot.entries.is_empty() {
            println!("  (no entries published yet)");
            return Ok(());
        }

        println!("  entries ({}):", snapshot.entries.len());
        for (k, v) in &snapshot.entries {
            println!(
                "    {} [{}] = {}",
                k.bold(),
                type_label(v).dimmed(),
                compact(v)
            );
        }
    }

    Ok(())
}
