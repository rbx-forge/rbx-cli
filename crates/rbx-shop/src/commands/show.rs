//! `rbx shop show` — pretty-print the local `rbxshop.toml` (read-only).
//!
//! Loads the config (so serde defaults like `for_sale=true` are filled in) and
//! prints passes/badges/products as aligned tables. Targets the global
//! `--config` path (default `rbxshop.toml`) and sorts by `--sort`.
//!
//! `--json` writes the same resolved state as one document on stdout instead.
//! It is the declared side of the shop and says nothing about whether Roblox
//! agrees; `rbx check --json` is the command that answers that. See
//! `crate::json`.

use anyhow::Result;
use colored::Colorize;

use rbx_core::output::{self, OutputFormat};

use crate::config::{
    resolve_name, BadgeConfig, Config, PassConfig, ProductConfig, ResolvedResources, ResourceKind,
};
use crate::ctx::ShopCtx;
use crate::json::ShowDocument;
use crate::ShowSort;

pub fn run(ctx: &ShopCtx<'_>, sort: ShowSort, flat: bool, json: bool) -> Result<()> {
    let config = Config::load_merged(&ctx.config)?;
    let format = OutputFormat::from_json_flag(json);

    // Resolve resources for the targeted env so per-env overlays (and
    // env-exclusive resources) are included. `--env all` has no single
    // overlay to resolve, so fall back to the base view.
    let env = match ctx.env() {
        Some("all") => None,
        other => other,
    };
    let resources = config.resolve_env(env)?;

    if format.is_json() {
        // The overlay hint the human view prints under its tables. It is
        // advice about how to run the command, not part of the declared state,
        // so under `--json` it goes to stderr — stdout carries the document.
        // Plainer than the human line for the same reason: it is a diagnostic
        // now, not a laid-out block.
        if env.is_none() && !config.envs.is_empty() {
            let names: Vec<&str> = config.envs.keys().map(|s| s.as_str()).collect();
            format.note(format!(
                "per-env overlays defined: {} — pass --env <name> to include env-specific resources",
                names.join(", ")
            ));
        }
        return output::emit(&ShowDocument::new(
            &ctx.config,
            env,
            config.experience.as_ref().map(|exp| exp.universe_id),
            &resources,
        ));
    }

    println!(
        "{} {}",
        "Showing".cyan().bold(),
        ctx.config.display().to_string().dimmed()
    );
    if let Some(exp) = &config.experience {
        println!("  universe_id: {}", exp.universe_id);
    }
    match env {
        Some(name) => println!("  env: {}", name.cyan()),
        None => println!("  env: {} (base values)", "—".dimmed()),
    }

    let empty =
        resources.passes.is_empty() && resources.badges.is_empty() && resources.products.is_empty();

    if flat {
        show_flat(&resources, sort);
    } else {
        show_passes(&resources, sort);
        show_badges(&resources, sort);
        show_products(&resources, sort);
    }

    if empty {
        println!("\n{}", "(no passes, badges, or products defined)".dimmed());
    }
    // When showing the base view, point at overlays the user might want to see.
    if env.is_none() && !config.envs.is_empty() {
        let names: Vec<&str> = config.envs.keys().map(|s| s.as_str()).collect();
        println!(
            "\nper-env overlays defined: {} — pass {} to include env-specific resources",
            names.join(", "),
            "--env <name>".cyan()
        );
    }

    Ok(())
}

/// One row in the flattened (cross-type) view.
struct FlatRow {
    kind: ResourceKind,
    name: String,
    /// Sort key: price when on sale, else None (badges and off-sale items sort last).
    price_sort: Option<u64>,
    price_str: String,
    flags: String,
}

/// All three resource types merged into one list, sorted globally by `sort`.
fn show_flat(res: &ResolvedResources, sort: ShowSort) {
    let mut rows: Vec<FlatRow> = Vec::new();

    for (key, p) in &res.passes {
        rows.push(FlatRow {
            kind: ResourceKind::Pass,
            name: resolve_name(p.name.as_deref(), key).to_string(),
            price_sort: p.for_sale.then_some(p.price).flatten(),
            price_str: price_cell(p.price, p.for_sale),
            flags: flags_cell(&[("regional", p.regional_pricing)]),
        });
    }
    for (key, p) in &res.products {
        rows.push(FlatRow {
            kind: ResourceKind::Product,
            name: resolve_name(p.name.as_deref(), key).to_string(),
            price_sort: p.for_sale.then_some(p.price),
            price_str: price_cell(Some(p.price), p.for_sale),
            flags: flags_cell(&[
                ("regional", p.regional_pricing),
                ("store_page", p.store_page),
            ]),
        });
    }
    for (key, b) in &res.badges {
        rows.push(FlatRow {
            kind: ResourceKind::Badge,
            name: resolve_name(b.name.as_deref(), key).to_string(),
            price_sort: None,
            price_str: "—".dimmed().to_string(),
            flags: if b.enabled {
                "enabled".green().to_string()
            } else {
                "disabled".dimmed().to_string()
            },
        });
    }

    if rows.is_empty() {
        return;
    }

    match sort {
        // No stable key in the flat view; fall back to name for Key/Name.
        ShowSort::Key | ShowSort::Name => rows.sort_by_key(|a| a.name.to_lowercase()),
        ShowSort::Price => rows.sort_by(|a, b| {
            match (a.price_sort, b.price_sort) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        }),
    }

    let w = rows
        .iter()
        .map(|r| r.name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 44);

    header("All resources", rows.len());
    for r in &rows {
        let line = format!(
            "  {:<8}  {:<w$}  {:>10}  {}",
            r.kind.to_string().dimmed(),
            r.name.cyan(),
            r.price_str,
            r.flags,
            w = w
        );
        println!("{}", line.trim_end());
    }
}

/// Price column: respects for-sale state, distinguishes free from priced.
fn price_cell(price: Option<u64>, for_sale: bool) -> String {
    match (price, for_sale) {
        (_, false) => "off sale".dimmed().to_string(),
        (Some(p), true) => format!("{} R$", p),
        (None, true) => "free".to_string(),
    }
}

fn flags_cell(parts: &[(&str, bool)]) -> String {
    let on: Vec<&str> = parts
        .iter()
        .filter(|(_, v)| *v)
        .map(|(label, _)| *label)
        .collect();
    if on.is_empty() {
        String::new()
    } else {
        on.join(" ").dimmed().to_string()
    }
}

fn header(title: &str, count: usize) {
    println!("\n{} ({})", title.bold().underline(), count);
}

/// Sort a list of (key, name, price) by the chosen field. `price` is the raw
/// sort key (None last). Returns indices into the original slice.
fn sorted_indices(rows: &[(String, String, Option<u64>)], sort: ShowSort) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..rows.len()).collect();
    match sort {
        ShowSort::Key => idx.sort_by(|&a, &b| rows[a].0.cmp(&rows[b].0)),
        ShowSort::Name => {
            idx.sort_by(|&a, &b| rows[a].1.to_lowercase().cmp(&rows[b].1.to_lowercase()))
        }
        ShowSort::Price => idx.sort_by(|&a, &b| {
            // None (no price / off sale) sorts last; ties broken by name.
            let pa = rows[a].2;
            let pb = rows[b].2;
            match (pa, pb) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| rows[a].1.to_lowercase().cmp(&rows[b].1.to_lowercase()))
        }),
    }
    idx
}

fn name_width(rows: &[(String, String, Option<u64>)]) -> usize {
    rows.iter()
        .map(|(_, name, _)| name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 44)
}

fn show_passes(res: &ResolvedResources, sort: ShowSort) {
    if res.passes.is_empty() {
        return;
    }
    header("Game passes", res.passes.len());
    let rows: Vec<(String, String, Option<u64>)> = res
        .passes
        .iter()
        .map(|(key, p): (&String, &PassConfig)| {
            (
                key.clone(),
                resolve_name(p.name.as_deref(), key).to_string(),
                p.price,
            )
        })
        .collect();
    let w = name_width(&rows);
    for i in sorted_indices(&rows, sort) {
        let (key, name, price) = &rows[i];
        let p = &res.passes[key];
        let line = format!(
            "  {:<w$}  {:>10}  {}",
            name.cyan(),
            price_cell(*price, p.for_sale),
            flags_cell(&[("regional", p.regional_pricing)]),
            w = w
        );
        println!("{}", line.trim_end());
    }
}

fn show_badges(res: &ResolvedResources, sort: ShowSort) {
    if res.badges.is_empty() {
        return;
    }
    header("Badges", res.badges.len());
    let rows: Vec<(String, String, Option<u64>)> = res
        .badges
        .iter()
        .map(|(key, b): (&String, &BadgeConfig)| {
            (
                key.clone(),
                resolve_name(b.name.as_deref(), key).to_string(),
                None,
            )
        })
        .collect();
    let w = name_width(&rows);
    for i in sorted_indices(&rows, sort) {
        let (key, name, _) = &rows[i];
        let b = &res.badges[key];
        let state = if b.enabled {
            "enabled".green()
        } else {
            "disabled".dimmed()
        };
        println!("  {:<w$}  {}", name.cyan(), state, w = w);
    }
}

fn show_products(res: &ResolvedResources, sort: ShowSort) {
    if res.products.is_empty() {
        return;
    }
    header("Developer products", res.products.len());
    let rows: Vec<(String, String, Option<u64>)> = res
        .products
        .iter()
        .map(|(key, p): (&String, &ProductConfig)| {
            (
                key.clone(),
                resolve_name(p.name.as_deref(), key).to_string(),
                Some(p.price),
            )
        })
        .collect();
    let w = name_width(&rows);
    for i in sorted_indices(&rows, sort) {
        let (key, name, price) = &rows[i];
        let p = &res.products[key];
        let line = format!(
            "  {:<w$}  {:>10}  {}",
            name.cyan(),
            price_cell(*price, p.for_sale),
            flags_cell(&[
                ("regional", p.regional_pricing),
                ("store_page", p.store_page)
            ]),
            w = w
        );
        println!("{}", line.trim_end());
    }
}
