//! `rbx rtbf check`: does `rbxrtbf.toml` match what Roblox is serving.
//!
//! Drift leaves through `Err(Drift)` (exit code 2) rather than through the
//! screen alone, the way every other check in this suite does: a CI step reads
//! the status, not the log.

use anyhow::Result;
use colored::Colorize;

use rbx_core::generated::Drift;

use super::{print_env_header, RtbfCtx};
use crate::config;
use crate::model::Templates;
use crate::DEADLINE_DAYS;

pub async fn run(ctx: &RtbfCtx<'_>) -> Result<()> {
    // `.templates` and no wipe guard, deliberately. `check` publishes nothing,
    // and it is the command somebody runs to find out why `sync` refused: a
    // file emptied by a typo reports here either as drift (exit 2, naming what
    // is published and not declared) or as the "declaring nothing" line below,
    // with `load`'s stderr warning naming the misspelled table above both.
    // Refusing would withhold the comparison the caller came for.
    let declared = config::load(&ctx.config)?.templates;
    // Ahead of the network: an invalid file is not drift, it is a file to fix,
    // and finding that out should not cost a request.
    declared.validate()?;

    let client = ctx.client()?;
    let targets = ctx.targets()?;
    let many = targets.len() > 1;
    let mut drifted: Vec<String> = Vec::new();

    for target in &targets {
        print_env_header(target, many);
        let live = client.get_config(target.universe_id).await?;
        let (published, unrecognised) = Templates::from_entries(&live.entries);

        if unrecognised > 0 {
            println!(
                "{}",
                format!(
                    "  {unrecognised} published template(s) are in a shape this release \
                     does not understand, so they are not compared below. `sync` refuses to \
                     publish over them, because a publish replaces the whole set: upgrade \
                     `rbx`, or remove them in the Creator Hub."
                )
                .yellow()
            );
        }

        // `render`, not `==`. Struct equality compares two `Vec`s and is
        // therefore order-sensitive, and declared order carries no meaning
        // here: deletion is a match, not a sequence. Comparing the structs
        // reported drift on a file somebody had merely tidied, and told them
        // to publish it, which would have been a no-op revision.
        if render(&published) == render(&declared) {
            println!("  {} {} template(s) match", "✓".green(), declared.total());
            continue;
        }

        drifted.push(target.name.clone());
        report_difference(&declared, &published);
    }

    if drifted.is_empty() {
        if declared.is_empty() {
            println!();
            println!(
                "{}",
                "In sync, and declaring nothing: an RTBF request would delete nothing.".yellow()
            );
        }
        return Ok(());
    }

    println!();
    Err(Drift::new(format!(
        "{} no longer matches the published templates for env {}. Run `rbx rtbf sync` to \
         publish it. A request you have not honoured within {DEADLINE_DAYS} days is the \
         deadline this drift is measured against.",
        ctx.config.display(),
        drifted.join(", ")
    ))
    .into())
}

/// What differs, as two lists rather than a diff.
///
/// A template is small and there are at most a hundred, so naming what is only
/// local and what is only published reads better than a line-oriented diff, and
/// it says which direction to move in.
fn report_difference(declared: &Templates, published: &Templates) {
    let local_keys = render(declared);
    let live_keys = render(published);

    for line in &local_keys {
        if !live_keys.contains(line) {
            println!("  {} {}", "+".green(), line);
        }
    }
    for line in &live_keys {
        if !local_keys.contains(line) {
            println!("  {} {}", "-".red(), line);
        }
    }
    println!(
        "  {} declared, {} published",
        declared.total(),
        published.total()
    );
}

/// One stable line per template, so set comparison is order-independent.
///
/// Declared order carries no meaning (deletion is a match, not a sequence), so
/// a file whose templates were reordered is not drift and must not report as
/// such.
fn render(templates: &Templates) -> Vec<String> {
    let mut lines: Vec<String> = templates
        .keys
        .iter()
        .map(|t| {
            format!(
                "key {} {} scope={}{}",
                t.store,
                t.pattern,
                t.effective_scope(),
                if t.ordered { " ordered" } else { "" }
            )
        })
        .chain(
            templates
                .stores
                .iter()
                .map(|t| format!("store {}", t.pattern)),
        )
        .collect();
    lines.sort();
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KeyTemplate, StoreTemplate};

    fn key(store: &str, pattern: &str) -> KeyTemplate {
        KeyTemplate {
            store: store.into(),
            pattern: pattern.into(),
            scope: None,
            ordered: false,
        }
    }

    /// Order is not meaning here, so a reordered file is not drift. Getting
    /// this wrong would make `check` red on a file somebody tidied.
    #[test]
    fn reordering_the_templates_is_not_a_difference() {
        let a = Templates {
            keys: vec![key("A", "U_{UserId}"), key("B", "U_{UserId}")],
            stores: vec![],
        };
        let b = Templates {
            keys: vec![key("B", "U_{UserId}"), key("A", "U_{UserId}")],
            stores: vec![],
        };
        assert_eq!(render(&a), render(&b));
    }

    #[test]
    fn the_default_scope_and_an_explicit_global_render_the_same() {
        let implicit = Templates {
            keys: vec![key("A", "U_{UserId}")],
            stores: vec![],
        };
        let mut explicit_key = key("A", "U_{UserId}");
        explicit_key.scope = Some("global".into());
        let explicit = Templates {
            keys: vec![explicit_key],
            stores: vec![],
        };
        assert_eq!(render(&implicit), render(&explicit));
    }

    #[test]
    fn ordered_and_standard_are_different_templates() {
        let standard = Templates {
            keys: vec![key("A", "U_{UserId}")],
            stores: vec![],
        };
        let mut ordered_key = key("A", "U_{UserId}");
        ordered_key.ordered = true;
        let ordered = Templates {
            keys: vec![ordered_key],
            stores: vec![],
        };
        assert_ne!(render(&standard), render(&ordered));
    }

    /// A key template and a store template that happen to share a pattern are
    /// not the same template, and the rendering has to keep them apart.
    #[test]
    fn a_key_and_a_store_sharing_a_pattern_stay_distinct() {
        let keys = Templates {
            keys: vec![key("Save_{UserId}", "Save_{UserId}")],
            stores: vec![],
        };
        let stores = Templates {
            keys: vec![],
            stores: vec![StoreTemplate {
                pattern: "Save_{UserId}".into(),
            }],
        };
        assert_ne!(render(&keys), render(&stores));
    }
}
