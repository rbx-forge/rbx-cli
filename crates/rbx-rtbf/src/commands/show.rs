//! `rbx rtbf show`: print the declared templates and what they would match.
//!
//! Local only. The samples are the point: a pattern is read for what it was
//! meant to say, and a sample is read for what it will actually match, which is
//! the form you can hold against your Luau.

use anyhow::Result;
use colored::Colorize;

use super::RtbfCtx;
use crate::config;
use crate::model::{Templates, MAX_TEMPLATES};

pub fn run(ctx: &RtbfCtx<'_>, user_id: u64) -> Result<()> {
    let templates = config::load(&ctx.config)?;
    templates.validate()?;
    print(&templates, user_id, &ctx.config.display().to_string());
    Ok(())
}

/// Split out of [`run`] so `check` and `sync` print the same block rather than
/// each growing their own rendering of the same list.
pub fn print(templates: &Templates, user_id: u64, source: &str) {
    println!("{}", source.dimmed());
    println!();

    if templates.is_empty() {
        println!(
            "{}",
            "No templates declared. An RTBF request would delete nothing.".yellow()
        );
        return;
    }

    if !templates.keys.is_empty() {
        println!("{}", "Keys".bold());
        for template in &templates.keys {
            let kind = if template.ordered {
                " (ordered)".dimmed()
            } else {
                "".dimmed()
            };
            println!("  {}{}", template.pattern.cyan(), kind);
            println!("    {} {}", "matches".dimmed(), template.sample(user_id));
        }
        println!();
    }

    if !templates.stores.is_empty() {
        println!("{}", "Stores".bold());
        for template in &templates.stores {
            println!("  {}", template.pattern.cyan());
            println!("    {} {}", "matches".dimmed(), template.sample(user_id));
        }
        println!();
    }

    println!(
        "{} template(s) of {}. Sample id {}.",
        templates.total(),
        MAX_TEMPLATES,
        user_id
    );
    println!(
        "{}",
        "A pattern that matches nothing is accepted by Roblox and deletes nothing: \
         `rbx rtbf verify` checks these against the stores that exist."
            .dimmed()
    );
}
