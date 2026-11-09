//! The line a `sync` draws at a value changed on Roblox since the lockfile
//! was written.
//!
//! A `sync` that reads Roblox before it plans can tell a change made in the
//! repository from one made on Roblox: the lockfile records what the last
//! `sync` or `pull` saw, so a value Roblox holds differently was changed
//! there, on the Creator Hub or from another checkout. That value exists
//! nowhere in the repository. Replacing it is a decision and not a side
//! effect, so the run stops, names both ways out, and writes nothing.
//!
//! Here rather than in each tool so every `sync` says it the same way.

use std::path::Path;

use anyhow::{bail, Result};
use colored::Colorize;

/// One env whose plan would write over values changed on Roblox.
#[derive(Debug, Clone, Copy)]
pub struct Newer<'a> {
    pub env: &'a str,
    /// One line per value, naming it and both sides.
    pub values: &'a [String],
}

/// Stop a run that would write over what was changed on Roblox, or say what
/// it replaces when that was asked for.
///
/// Called after the plans are shown, so whoever is refused has already seen
/// what would have gone, and before a dry run returns, so a preview cannot
/// report a plan the real run refuses.
///
/// `pull` is the command that brings those values in, spelled for the run's
/// target and already quoted for the message.
pub fn report(
    changed: &[Newer<'_>],
    overwrite_newer: bool,
    config: &Path,
    pull: &str,
) -> Result<()> {
    if changed.is_empty() {
        return Ok(());
    }
    let listed = listed(changed);

    if overwrite_newer {
        println!(
            "\n{} replacing what was changed on Roblox since the last sync or pull:{listed}",
            "--overwrite-newer:".yellow()
        );
        return Ok(());
    }

    bail!(
        "Roblox was changed since this repository last synced or pulled it:{listed}\n\n\
         Syncing would replace those values with the ones in {}, as the plan above shows.\n  \
         Bring them in with {pull}, or pass --overwrite-newer to replace them.",
        config.display(),
    )
}

/// Every value, under the env it belongs to.
fn listed(changed: &[Newer<'_>]) -> String {
    let mut out = String::new();
    for newer in changed {
        out.push_str(&format!("\n  [{}]", newer.env));
        for value in newer.values {
            out.push_str(&format!("\n    {value}"));
        }
    }
    out
}

/// The `pull` that brings newer values in, for the envs that have them.
///
/// `tool` is the subcommand (`meta`, `shop`), `standalone` the flags that name
/// a target outside `rbxplace.toml` when the run had one, and `default_env`
/// the name a tool gives the env of such a run.
pub fn pull_hint(tool: &str, envs: &[&str], default_env: &str, standalone: Option<&str>) -> String {
    match envs {
        [env] if *env != default_env => format!("`rbx {tool} pull --env {env}`"),
        [_] => match standalone {
            Some(flags) => format!("`rbx {tool} pull {flags}`"),
            None => format!("`rbx {tool} pull`"),
        },
        _ => format!("`rbx {tool} pull --env <name>` for each env listed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> Vec<String> {
        vec![r#"name: lockfile "a", Roblox "b""#.to_string()]
    }

    #[test]
    fn nothing_newer_is_not_a_refusal() {
        report(&[], false, Path::new("rbxmeta.toml"), "`rbx meta pull`").expect("nothing changed");
    }

    /// The refusal names the value, the env, the file that would win, and both
    /// ways out.
    #[test]
    fn a_newer_value_stops_the_run_and_names_both_ways_out() {
        let values = values();
        let err = report(
            &[Newer {
                env: "prod",
                values: &values,
            }],
            false,
            Path::new("rbxmeta.toml"),
            "`rbx meta pull --env prod`",
        )
        .expect_err("Roblox was changed");

        let text = err.to_string();
        assert!(text.contains("[prod]"), "{text}");
        assert!(text.contains(r#"name: lockfile "a", Roblox "b""#), "{text}");
        assert!(text.contains("rbxmeta.toml"), "{text}");
        assert!(text.contains("`rbx meta pull --env prod`"), "{text}");
        assert!(text.contains("--overwrite-newer"), "{text}");
    }

    #[test]
    fn asked_for_the_replacement_goes_ahead() {
        let values = values();
        report(
            &[Newer {
                env: "prod",
                values: &values,
            }],
            true,
            Path::new("rbxmeta.toml"),
            "`rbx meta pull --env prod`",
        )
        .expect("the replacement was asked for");
    }

    /// The hint is a command that can be pasted, for the target the run had.
    #[test]
    fn the_pull_hint_is_spelled_for_the_target() {
        assert_eq!(
            pull_hint("meta", &["prod"], "default", None),
            "`rbx meta pull --env prod`"
        );
        assert_eq!(
            pull_hint("shop", &["dev", "prod"], "default", None),
            "`rbx shop pull --env <name>` for each env listed"
        );
        assert_eq!(
            pull_hint("meta", &["default"], "default", None),
            "`rbx meta pull`"
        );
        assert_eq!(
            pull_hint(
                "shop",
                &["default"],
                "default",
                Some("--universe-id 1234567890")
            ),
            "`rbx shop pull --universe-id 1234567890`"
        );
    }
}
