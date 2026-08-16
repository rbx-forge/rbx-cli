//! What to do when two remote resources want the same config key.
//!
//! `init --from-remote` and `pull` key a newly discovered resource by its
//! display name, because that is the only human-meaningful handle Roblox
//! offers. Game-pass, badge and developer-product names are not unique, so two
//! distinct resources can want one key.
//!
//! This used to warn and drop the second. The warning named the id of the one
//! being *skipped* and not the one being kept, printed to stdout, and left the
//! exit code at zero — so a resource silently stopped being managed, and the
//! message did not carry enough to fix it.
//!
//! Now the key is a question, asked where there is somebody to ask. On a
//! terminal you name the key and the resource is kept. Off one — CI, a pipe, a
//! cron job — nothing prompts, because a command that hangs waiting for an
//! answer nobody will type is worse than one that skips loudly. There the
//! warning names both ids and the exact TOML that binds the resource for good.
//!
//! Not solved by auto-suffixing (`VIP-2`). That invents an identifier the
//! developer then lives with in `rbxshop.toml` for as long as the resource
//! exists, chosen by a tool that knows nothing about which "VIP" is the
//! premium one.

use colored::Colorize;
use dialoguer::theme::ColorfulTheme;
use dialoguer::Input;

use crate::config::ResourceKind;

/// True when there is a human on the other end to ask.
///
/// This used to be a private copy of the stdin/stderr terminal test. It is now
/// the shared one in `rbx_core::output`, which is also what
/// `OutputFormat::may_prompt` is built on — so "is anybody there" has one
/// answer in this tree rather than one per crate that asks. Without it, `pull`
/// in CI would stop on a prompt nobody answers.
fn interactive() -> bool {
    rbx_core::output::is_interactive()
}

/// The key to file a colliding resource under, or `None` to skip it.
///
/// `is_taken` is passed as a closure rather than the map itself because the two
/// callers hold different maps — `init` builds the config, `pull` builds the
/// lock — and both need the same answer.
pub(crate) fn resolve_duplicate(
    kind: ResourceKind,
    name: &str,
    id: u64,
    kept_id: Option<u64>,
    is_taken: &dyn Fn(&str) -> bool,
) -> Option<String> {
    if !interactive() {
        warn_and_skip(kind, name, id, kept_id);
        return None;
    }

    println!(
        "{} Two {}s are named '{}'{}.",
        "!".yellow(),
        kind,
        name,
        match kept_id {
            Some(kept) => format!(": id {kept} already has the key, and id {id} does not"),
            None => format!(", including id {id}"),
        }
    );
    println!(
        "{}",
        "  Give this one its own key, or leave it empty to skip it.".dimmed()
    );

    let suggested = suggest_key(name, is_taken);
    let answer: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("  Key for {kind} {id}"))
        .default(suggested)
        .allow_empty(true)
        .interact_text()
        .ok()?;

    let answer = answer.trim();
    if answer.is_empty() {
        println!("{}", format!("  skipped {kind} {id}").dimmed());
        return None;
    }
    if is_taken(answer) {
        // Not a retry loop: one clear refusal beats a prompt that will not let
        // go, and re-running the command is cheap.
        println!(
            "{} '{}' is taken too — skipping {} {}. Re-run and pick another.",
            "!".yellow(),
            answer,
            kind,
            id
        );
        return None;
    }
    Some(answer.to_string())
}

/// The message for when nobody can be asked.
///
/// Carries what the old one lacked: which resource kept the key, and the TOML
/// that binds this one so the collision cannot recur.
fn warn_and_skip(kind: ResourceKind, name: &str, id: u64, kept_id: Option<u64>) {
    let table = kind.section();
    println!(
        "{} Duplicate {} name '{}' — skipping id {}{}.",
        "!".yellow(),
        kind,
        name,
        id,
        match kept_id {
            Some(kept) => format!(" (id {kept} keeps the key)"),
            None => String::new(),
        }
    );
    println!(
        "{}",
        format!(
            "  To manage it, add an entry naming its id, then re-run:\n\
             \n\
             \x20     [{table}.<your_key>]\n\
             \x20     id = {id}\n"
        )
        .dimmed()
    );
}

/// A first suggestion for the prompt's default: the name with a numeric
/// suffix, skipping any that are already spoken for.
///
/// Only ever a default the developer can overwrite. The tool proposing
/// `VIP_2` is a convenience; the tool *deciding* on `VIP_2` is the thing this
/// module exists to avoid.
fn suggest_key(name: &str, is_taken: &dyn Fn(&str) -> bool) -> String {
    for n in 2..100 {
        let candidate = format!("{name}_{n}");
        if !is_taken(&candidate) {
            return candidate;
        }
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_suggestion_skips_keys_already_in_use() {
        let taken = |k: &str| k == "VIP_2" || k == "VIP_3";
        assert_eq!(suggest_key("VIP", &taken), "VIP_4");
    }

    #[test]
    fn the_suggestion_starts_at_two_because_the_first_one_kept_the_bare_name() {
        let taken = |_: &str| false;
        assert_eq!(suggest_key("VIP", &taken), "VIP_2");
    }

    /// The tests run without a terminal, which is the branch CI takes: it must
    /// skip rather than block on a prompt.
    #[test]
    fn without_a_terminal_nothing_prompts_and_the_resource_is_skipped() {
        let taken = |_: &str| false;
        assert_eq!(
            resolve_duplicate(ResourceKind::Pass, "VIP", 222, Some(111), &taken),
            None
        );
    }
}
