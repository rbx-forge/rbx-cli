//! `rbx rtbf sync`: publish `rbxrtbf.toml` as the canonical set of templates.

use anyhow::Result;
use colored::Colorize;

use rbx_core::confirm::confirm_always;

use super::{print_env_header, RtbfCtx};
use crate::config;
use crate::model::Templates;

/// Roblox's two rollout strategies, and this one is not a choice.
///
/// A template set is not a feature flag: there is no value in a fifteen-minute
/// gradual rollout of "which keys hold a user's data", and a request arriving
/// mid-rollout would be served by whichever half of the fleet answered.
const STRATEGY: &str = "Immediate";

pub async fn run(
    ctx: &RtbfCtx<'_>,
    message: Option<&str>,
    no_message: bool,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    let loaded = config::load(&ctx.config)?;
    // The wipe guard first, and before the network: an empty declaration in a
    // file that also names a table this build cannot read is a typo, not a
    // decision, and this is the only command that would make it permanent.
    // Nothing below catches it, which is why it is checked here and not left
    // to look like somebody else's job: `validate` passes an empty set on
    // purpose (`model.rs:772`), the read-before-write loop below bails only on
    // live templates this build cannot parse, and `--yes` skips the prompt
    // whose count was the last signal.
    loaded.refuse_if_emptied_by_a_typo(&ctx.config)?;
    let declared = loaded.templates;
    // Then the file's own rules: an invalid file is the caller's to fix, and a
    // template that would match nothing must not reach a publish that makes it
    // authoritative.
    declared.validate()?;

    let targets = ctx.targets()?;
    let many = targets.len() > 1;

    super::show::print(&declared, 1234567890, &ctx.config.display().to_string());
    println!();

    if dry_run {
        println!(
            "{}",
            format!(
                "Nothing sent. `--apply` is not this command's flag: drop --dry-run to publish \
                 to {}.",
                targets
                    .iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .yellow()
        );
        return Ok(());
    }

    // The client before the question, not after: `require_api_key` performs no
    // request, and a publish prompt nothing can act on is a question asked
    // before it was refused. `rbx place upload` states the same rule.
    let client = ctx.client()?;

    // Read before writing. `overwrite_draft` replaces the whole entry, so a
    // template this build cannot parse (a newer release's, or one written in
    // the Creator Hub) would be published over and gone, and Roblox's only
    // undo is restoring a previous revision, which this crate has no command
    // for. `pull` already refuses to lose one from a text file; losing one
    // from the live compliance artefact is strictly worse.
    for target in &targets {
        let live = client.get_config(target.universe_id).await?;
        let (_, unrecognised) = Templates::from_entries(&live.entries);
        if unrecognised > 0 {
            anyhow::bail!(
                "[{}] publishes over {unrecognised} template(s) this release cannot read, and \
                 they would be gone: a publish replaces the whole set and Roblox's only undo \
                 is restoring a revision. Upgrade `rbx`, or remove them in the Creator Hub \
                 first.",
                target.name
            );
        }
    }

    // One confirmation for the whole run, for the reason `rbx shop sync`
    // records: a prompt inside the loop is reached after the earlier envs have
    // already been published, which is no longer a question anyone can answer
    // no to.
    let names: Vec<&str> = targets.iter().map(|t| t.name.as_str()).collect();
    confirm_always(&prompt(&names, declared.total()), yes)?;

    let entries = declared.to_entries();
    let effective_message = resolve_message(message, no_message, declared.total());

    for target in &targets {
        print_env_header(target, many);
        print!("  publishing ... ");
        let (published, replaced) = client
            .overwrite_and_publish(target.universe_id, &entries, &effective_message, STRATEGY)
            .await?;
        println!("{}", format!("v{}", published.config_version).green());

        // Said out loud because the draft is gone by the time anyone could ask.
        // Not a refusal: a pipeline that publishes on every merge legitimately
        // overwrites, and stopping it would be worse than telling it.
        if !replaced.is_empty() {
            println!(
                "{}",
                format!(
                    "  replaced a staged draft holding: {}",
                    replaced.keys.join(", ")
                )
                .yellow()
            );
        }
    }

    println!();
    println!("{}", "Published.".green().bold());
    println!(
        "{}",
        "Roblox matches these when a request arrives. `rbx rtbf verify` is what says they \
         match something real."
            .dimmed()
    );
    Ok(())
}

/// The question, naming every universe it covers.
fn prompt(envs: &[&str], total: usize) -> String {
    let what = if total == 1 {
        "1 template".to_string()
    } else {
        format!("{total} templates")
    };
    match envs {
        [one] => format!("Publish {what} to [{one}]? This replaces the published set."),
        many => format!(
            "Publish {what} to [{}]? This replaces the published set in each.",
            many.join(", ")
        ),
    }
}

/// The revision message.
///
/// A default rather than a prompt, unlike `rbx config sync`. That command's
/// values are a running conversation with a live game and the message is how a
/// revision list stays readable; a template set changes rarely and its own
/// content is the interesting part, so "what this publish contained" is a better
/// default than an empty string and better than stopping to ask.
fn resolve_message(message: Option<&str>, no_message: bool, total: usize) -> String {
    if no_message {
        return String::new();
    }
    match message {
        Some(text) => text.to_string(),
        None => format!("rbx rtbf sync: {total} template(s)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_names_every_env_it_will_publish_to() {
        assert_eq!(
            prompt(&["prod"], 1),
            "Publish 1 template to [prod]? This replaces the published set."
        );
        assert!(prompt(&["dev", "prod"], 3).contains("[dev, prod]"));
        assert!(prompt(&["dev", "prod"], 3).contains("3 templates"));
    }

    /// The default says what the publish contained, so a revision list is
    /// readable without anyone having remembered to pass `--message`.
    #[test]
    fn the_default_message_describes_the_publish() {
        assert_eq!(
            resolve_message(None, false, 4),
            "rbx rtbf sync: 4 template(s)"
        );
        assert_eq!(resolve_message(Some("onboarding"), false, 4), "onboarding");
        assert_eq!(resolve_message(None, true, 4), "");
        // `--no-message` wins over an explicit one, matching `rbx config`.
        assert_eq!(resolve_message(Some("ignored"), true, 4), "");
    }

    /// Not a choice, and worth a test so nobody makes it one: a gradual rollout
    /// of "which keys hold a user's data" would serve a request arriving
    /// mid-rollout from whichever half of the fleet answered.
    #[test]
    fn the_strategy_is_immediate() {
        assert_eq!(STRATEGY, "Immediate");
    }
}
