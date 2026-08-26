//! `rbx rtbf verify`: do the templates match data stores that actually exist.
//!
//! The question `check` cannot answer, and the one that matters. `check` says
//! the file and the published set agree; both can agree perfectly on a template
//! that names a store you renamed last year. Roblox accepts it, stores it,
//! matches nothing, and reports nothing. Its own guidance is to compare the
//! patterns against your live usage by hand in the Creator Hub, which is an
//! admission that nothing verifies it for you.

use anyhow::Result;
use colored::Colorize;

use rbx_core::api::{build_client, require_api_key, ApiBase};
use rbx_core::generated::Drift;
use rbx_core::output::{self, OutputFormat};

use super::RtbfCtx;
use crate::config;
use crate::json::{Finding, VerifyDocument};
use crate::model::{Templates, USER_ID_TOKEN};
use crate::stores;
use crate::DEADLINE_DAYS;

pub async fn run(ctx: &RtbfCtx<'_>, uncovered: bool, json: bool) -> Result<()> {
    let format = OutputFormat::from_json_flag(json);
    let declared = config::load(&ctx.config)?;
    declared.validate()?;

    let universe_id = ctx.single()?;
    let api_key = require_api_key(ctx.global.api_key.as_deref())?;
    let base = match &ctx.base_url {
        Some(url) => ApiBase::new(url.clone()),
        None => ApiBase::default(),
    };
    let live = stores::list_standard(&build_client(), &base, api_key, universe_id).await?;
    let report = examine(&declared, &live);
    let failed = report.missing.len() + report.unmatched_stores.len();

    if format.is_json() {
        // The document goes out before the error, so a run that ends in exit 2
        // still writes it: a consumer branching on `.ok` should not have to
        // choose between the status and the findings.
        output::emit(&VerifyDocument {
            schema_version: output::SCHEMA_VERSION,
            config_file: ctx.config.display().to_string(),
            env: ctx.global.env.clone(),
            universe_id: universe_id.to_string(),
            ok: failed == 0,
            standard_store_count: live.len(),
            findings: findings(&report),
            uncovered: uncovered.then(|| report.uncovered.iter().map(|s| s.to_string()).collect()),
        })?;
    } else {
        println!("{}", ctx.config.display().to_string().dimmed());
        println!(
            "  {} standard data store(s) in universe {}",
            live.len(),
            universe_id
        );
        println!();
        print_report(&report, uncovered);
    }

    if failed == 0 {
        return Ok(());
    }
    Err(Drift::new(format!(
        "{failed} template(s) name nothing that exists. Roblox accepts them and deletes \
         nothing, and a request you have not honoured within {DEADLINE_DAYS} days is what \
         that costs."
    ))
    .into())
}

/// The report, flattened into the document's one list.
///
/// The verdicts are what keep `unverifiable` out of the failure count: it is a
/// limit of Open Cloud rather than a broken template, and a consumer folding it
/// into `ok` would fail a build over a store nothing can list.
fn findings(report: &Report<'_>) -> Vec<Finding> {
    report
        .missing
        .iter()
        .map(|store| Finding {
            kind: "key",
            target: (*store).to_string(),
            verdict: "missing",
            detail: "no such standard data store in this universe".into(),
        })
        .chain(report.unmatched_stores.iter().map(|pattern| Finding {
            kind: "store",
            target: (*pattern).to_string(),
            verdict: "unmatched",
            detail: "matches none of the live store names".into(),
        }))
        .chain(report.unverifiable.iter().map(|store| Finding {
            kind: "key",
            target: (*store).to_string(),
            verdict: "unverifiable",
            detail:
                "ordered store: Open Cloud does not list these, so this one is unchecked".into(),
        }))
        .collect()
}

/// What a verify found.
struct Report<'a> {
    /// Key templates naming a standard store the universe does not have.
    missing: Vec<&'a str>,
    /// Store patterns that match none of the live store names.
    unmatched_stores: Vec<&'a str>,
    /// Key templates on ordered stores, which this endpoint cannot see.
    unverifiable: Vec<&'a str>,
    /// Live stores no template covers.
    uncovered: Vec<&'a str>,
}

fn examine<'a>(declared: &'a Templates, live: &'a [String]) -> Report<'a> {
    let mut report = Report {
        missing: Vec::new(),
        unmatched_stores: Vec::new(),
        unverifiable: Vec::new(),
        uncovered: Vec::new(),
    };

    for template in &declared.keys {
        if template.ordered {
            // Not a failure. `Cloud_ListDataStores` covers standard stores
            // only, so an ordered store's absence from the listing says
            // nothing, and reporting it as missing would be a false alarm that
            // teaches people to ignore this command.
            report.unverifiable.push(&template.store);
        } else if !live.iter().any(|name| name == &template.store) {
            report.missing.push(&template.store);
        }
    }

    for template in &declared.stores {
        if !live
            .iter()
            .any(|name| matches_pattern(&template.pattern, name))
        {
            report.unmatched_stores.push(&template.pattern);
        }
    }

    for name in live {
        let covered = declared.keys.iter().any(|t| &t.store == name)
            || declared
                .stores
                .iter()
                .any(|t| matches_pattern(&t.pattern, name));
        if !covered {
            report.uncovered.push(name);
        }
    }

    report
}

/// Whether a live store name could have come from this pattern.
///
/// The token stands for a user id, so it matches a run of digits and nothing
/// else. A looser wildcard would call `Player_{UserId}_Save` a match for
/// `Player_Settings_Save` and report a template as verified when it is not, and
/// a verify that says yes too easily is worse than no verify at all.
fn matches_pattern(pattern: &str, name: &str) -> bool {
    let Some((prefix, suffix)) = pattern.split_once(USER_ID_TOKEN) else {
        // No token: `validate` refuses this, so reaching here means an exact
        // name is the only sensible reading.
        return pattern == name;
    };
    // A pattern with two tokens is legal and rare. The tail after the first
    // token is matched literally, which is exact for the common single-token
    // case and conservative for the rest: a second token makes this say no
    // rather than guessing which digits belong to which.
    let Some(rest) = name.strip_prefix(prefix) else {
        return false;
    };
    let Some(digits) = rest.strip_suffix(suffix) else {
        return false;
    };
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

fn print_report(report: &Report<'_>, show_uncovered: bool) {
    for store in &report.missing {
        println!(
            "  {} {} {}",
            "✗".red(),
            store.cyan(),
            "no such standard data store in this universe".red()
        );
    }
    for pattern in &report.unmatched_stores {
        println!(
            "  {} {} {}",
            "✗".red(),
            pattern.cyan(),
            "matches none of the live store names".red()
        );
    }
    for store in &report.unverifiable {
        println!(
            "  {} {} {}",
            "-".dimmed(),
            store.cyan(),
            "ordered store: Open Cloud does not list these, so this one is unchecked".dimmed()
        );
    }

    if report.missing.is_empty() && report.unmatched_stores.is_empty() {
        println!(
            "  {} every template names something that exists",
            "✓".green()
        );
    }

    if show_uncovered {
        println!();
        if report.uncovered.is_empty() {
            println!("{}", "Every live store is covered by a template.".green());
        } else {
            println!("{}", "Live stores no template covers".bold());
            for name in &report.uncovered {
                println!("  {}", name.cyan());
            }
            println!(
                "{}",
                "Not necessarily wrong: a store holding no user data needs no template. \
                 Worth reading once."
                    .dimmed()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KeyTemplate, StoreTemplate};

    fn key(store: &str, ordered: bool) -> KeyTemplate {
        KeyTemplate {
            store: store.into(),
            pattern: "User_{UserId}".into(),
            scope: None,
            ordered,
        }
    }

    fn live(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_key_template_naming_a_store_that_exists_passes() {
        let declared = Templates {
            keys: vec![key("PlayerInventory", false)],
            stores: vec![],
        };
        let stores = live(&["PlayerInventory", "Other"]);
        let report = examine(&declared, &stores);
        assert!(report.missing.is_empty());
    }

    /// The failure this command exists to catch: a store renamed out from under
    /// a template Roblox is perfectly happy to keep storing.
    #[test]
    fn a_key_template_naming_a_store_that_does_not_exist_is_reported() {
        let declared = Templates {
            keys: vec![key("PlayerInventoryV1", false)],
            stores: vec![],
        };
        let stores = live(&["PlayerInventoryV2"]);
        let report = examine(&declared, &stores);
        assert_eq!(report.missing, ["PlayerInventoryV1"]);
    }

    /// Open Cloud lists standard stores only, so an ordered store's absence
    /// says nothing. Calling it missing would be a false alarm, and a command
    /// that cries wolf gets ignored.
    #[test]
    fn an_ordered_store_is_reported_as_unchecked_rather_than_missing() {
        let declared = Templates {
            keys: vec![key("Leaderboard", true)],
            stores: vec![],
        };
        let stores = live(&["Something"]);
        let report = examine(&declared, &stores);
        assert!(report.missing.is_empty());
        assert_eq!(report.unverifiable, ["Leaderboard"]);
    }

    #[test]
    fn a_store_pattern_matches_a_live_name_with_digits_where_the_token_is() {
        assert!(matches_pattern(
            "Player_{UserId}_Save",
            "Player_1234567890_Save"
        ));
        assert!(matches_pattern("{UserId}", "1234567890"));
        assert!(matches_pattern("save_{UserId}", "save_1"));
    }

    /// The looseness that would make this command useless: a wildcard would
    /// call this a match and report a broken template as verified.
    #[test]
    fn a_store_pattern_does_not_match_arbitrary_text_where_the_token_is() {
        assert!(!matches_pattern(
            "Player_{UserId}_Save",
            "Player_Settings_Save"
        ));
        assert!(!matches_pattern("Player_{UserId}_Save", "Player__Save"));
        assert!(!matches_pattern("Player_{UserId}_Save", "Player_12_Backup"));
        assert!(!matches_pattern("Player_{UserId}_Save", "Other_12_Save"));
        assert!(!matches_pattern("Player_{UserId}_Save", "Player_12_Savex"));
    }

    #[test]
    fn an_unmatched_store_pattern_is_reported() {
        let declared = Templates {
            keys: vec![],
            stores: vec![StoreTemplate {
                pattern: "Player_{UserId}_Save".into(),
            }],
        };
        let stores = live(&["Player_Settings_Save"]);
        let report = examine(&declared, &stores);
        assert_eq!(report.unmatched_stores, ["Player_{UserId}_Save"]);
    }

    /// A live store nothing covers is worth a look and is not an error: plenty
    /// of stores hold no user data.
    #[test]
    fn an_uncovered_live_store_is_listed_without_failing_the_run() {
        let declared = Templates {
            keys: vec![key("PlayerInventory", false)],
            stores: vec![],
        };
        let stores = live(&["PlayerInventory", "GlobalSettings"]);
        let report = examine(&declared, &stores);
        assert_eq!(report.uncovered, ["GlobalSettings"]);
        assert!(report.missing.is_empty());
    }

    #[test]
    fn a_store_covered_by_a_pattern_is_not_reported_as_uncovered() {
        let declared = Templates {
            keys: vec![],
            stores: vec![StoreTemplate {
                pattern: "Player_{UserId}_Save".into(),
            }],
        };
        let stores = live(&["Player_1234567890_Save"]);
        let report = examine(&declared, &stores);
        assert!(report.uncovered.is_empty());
        assert!(report.unmatched_stores.is_empty());
    }
}
