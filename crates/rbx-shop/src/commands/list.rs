//! `rbx shop list`: what Roblox currently has, for one resource kind.
//!
//! The remote side. `shop show` is the declared side, and `rbx check --json`
//! is the only one of the three that says whether they agree. See
//! `crate::json` for how the two documents are kept apart.

use anyhow::Result;
use colored::Colorize;

use rbx_core::output::{self, OutputFormat};

use crate::api::RbxClient;
use crate::config::{Config, ResourceKind};
use crate::ctx::ShopCtx;
use crate::json::{ListDocument, Resource};

pub async fn run(ctx: &ShopCtx<'_>, resource: ResourceKind, json: bool) -> Result<()> {
    let config = Config::load(&ctx.config)?;
    let env_target = ctx.resolve_single_env(&config)?;
    let client = RbxClient::new(ctx.api_key(), env_target.universe_id, config.icons.bleed);
    let format = OutputFormat::from_json_flag(json);

    // The env as the command line named it, not `env_target.name`: with no
    // `--env` that field carries the internal `default` placeholder, which is
    // not an env anybody can pass back in. Omitted rather than invented.
    let env = ctx.env();

    // One place builds the document for all three kinds, so a field can only
    // be added to one of them by choosing to.
    let emit = |rows: Vec<Resource>| {
        output::emit(&ListDocument::new(
            env,
            env_target.universe_id,
            resource,
            rows,
        ))
    };

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
            let badges = client.list_all_badges(env_target.universe_id).await?;
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
