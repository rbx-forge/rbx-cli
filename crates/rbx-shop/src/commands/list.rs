//! `rbx shop list`: what Roblox currently has, for one resource kind.
//!
//! The remote side. `shop show` is the declared side, and the `shop/live`
//! row of `rbx check` says whether Roblox moved since the last sync. See
//! `crate::json` for how the two documents are kept apart.

use anyhow::Result;
use colored::Colorize;

use rbx_core::output::{self, OutputFormat};

use crate::api::RbxClient;
use crate::config::{Config, ResourceKind};
use crate::ctx::ShopCtx;
use crate::json::{ListDocument, Resource};

pub async fn run(ctx: &ShopCtx<'_>, resource: ResourceKind, json: bool) -> Result<()> {
    // A read of what Roblox has: `rbxshop.toml` only names the universe,
    // through `[experience]`, so with `--universe-id` or `--env` the file is
    // not needed and not read.
    let universe_id = if ctx.global.universe_id.is_some() || ctx.env().is_some() {
        ctx.global.single_universe()?
    } else {
        ctx.resolve_single_env(&Config::load(&ctx.config)?)?
            .universe_id
    };
    // Bleed applies to uploads only, and a listing uploads nothing.
    let client = RbxClient::new(ctx.api_key(), universe_id, false);
    let format = OutputFormat::from_json_flag(json);

    // The env as the command line named it, not a resolved name: with no
    // `--env` that field carries the internal `default` placeholder, which is
    // not an env anybody can pass back in. Omitted rather than invented.
    let env = ctx.env();

    // One place builds the document for all three kinds, so a field can only
    // be added to one of them by choosing to.
    let emit =
        |rows: Vec<Resource>| output::emit(&ListDocument::new(env, universe_id, resource, rows));

    match resource {
        ResourceKind::Pass => {
            let passes = client.list_all_game_passes().await?;
            if format.is_json() {
                return emit(passes.iter().map(Resource::from).collect());
            }
            println!("{}", "Game Passes".bold());
            println!("{:<12} {:<30} {:<10} Description", "ID", "Name", "Price");
            println!("{}", "-".repeat(70));
            for pass in &passes {
                let id = pass
                    .id
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let name = pass.name.as_deref().unwrap_or("-");
                let price = pass
                    .price()
                    .map(|p| format!("R${}", p))
                    .unwrap_or_else(|| "Free".to_string());
                let desc = pass.description.as_deref().unwrap_or("");
                println!("{:<12} {:<30} {:<10} {}", id, name, price, desc);
            }
            println!("\nTotal: {}", passes.len());
        }
        ResourceKind::Badge => {
            let badges = client.list_all_badges(universe_id).await?;
            if format.is_json() {
                return emit(badges.iter().map(Resource::from).collect());
            }
            println!("{}", "Badges".bold());
            println!("{:<12} {:<30} {:<10} Description", "ID", "Name", "Enabled");
            println!("{}", "-".repeat(70));
            for badge in &badges {
                let id = badge
                    .id
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let name = badge.name.as_deref().unwrap_or("-");
                let enabled = badge
                    .enabled
                    .map(|e| if e { "Yes" } else { "No" })
                    .unwrap_or("-");
                let desc = badge.description.as_deref().unwrap_or("");
                println!("{:<12} {:<30} {:<10} {}", id, name, enabled, desc);
            }
            println!("\nTotal: {}", badges.len());
        }
        ResourceKind::Product => {
            let products = client.list_all_developer_products().await?;
            if format.is_json() {
                return emit(products.iter().map(Resource::from).collect());
            }
            println!("{}", "Developer Products".bold());
            println!("{:<12} {:<30} {:<10} Description", "ID", "Name", "Price");
            println!("{}", "-".repeat(70));
            for product in &products {
                let id = product
                    .id
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let name = product.name.as_deref().unwrap_or("-");
                let price = product
                    .price()
                    .map(|p| format!("R${}", p))
                    .unwrap_or_else(|| "-".to_string());
                let desc = product.description.as_deref().unwrap_or("");
                println!("{:<12} {:<30} {:<10} {}", id, name, price, desc);
            }
            println!("\nTotal: {}", products.len());
        }
    }

    Ok(())
}
