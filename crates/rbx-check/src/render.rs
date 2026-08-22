//! The human renderers. The only thing in this crate that writes to stdout.
//!
//! Two of them over one report, which is the reason `report` prints nothing:
//!
//! - [`summary`] is `rbx check`. Terse on purpose: it runs in CI, where it is
//!   read in a log next to a hundred other lines, and detail belongs to the
//!   per-tool commands it names.
//! - [`status`] is `rbx status`. Grouped by environment, because the question
//!   it answers is "where does this project stand" rather than "may this build
//!   continue". It is the one thing a single monolithic config file bought
//!   that the per-concern TOML layout gave up.

use colored::Colorize;

use crate::report::{Outcome, Report, ToolReport};

fn mark(outcome: Outcome) -> colored::ColoredString {
    match outcome {
        Outcome::Clean => "✓".green(),
        Outcome::Drift => "!".yellow(),
        Outcome::Error => "✗".red(),
        Outcome::Skipped => "-".dimmed(),
    }
}

/// One line per check, then one line for the verdict.
pub fn summary(report: &Report) {
    if report.tools.is_empty() {
        println!(
            "{} No rbx config files found here. Nothing to check.",
            "-".dimmed()
        );
        return;
    }

    println!("{}", "rbx check".bold());

    let width = report
        .tools
        .iter()
        .map(|t| t.label().chars().count())
        .max()
        .unwrap_or(0);

    for tool in &report.tools {
        println!(
            "  {} {:<width$}  {}",
            mark(tool.outcome),
            tool.label(),
            tool.summary,
            width = width
        );
        for detail in &tool.details {
            println!("      {}", detail.dimmed());
        }
    }

    println!();
    let clean = report.count(Outcome::Clean);
    let drift = report.count(Outcome::Drift);
    let errors = report.count(Outcome::Error);
    let skipped = report.count(Outcome::Skipped);

    match report.worst() {
        Outcome::Clean => println!(
            "{} {clean} check{} clean, {skipped} skipped.",
            "✓".green(),
            plural(clean)
        ),
        Outcome::Skipped => println!("{} Nothing to check ({skipped} skipped).", "-".dimmed()),
        Outcome::Drift => println!(
            "{} {drift} check{} found drift ({clean} clean, {skipped} skipped). Exit code 2.",
            "!".yellow(),
            plural(drift)
        ),
        Outcome::Error => println!(
            "{} {errors} check{} failed, {drift} found drift ({clean} clean, {skipped} skipped). Exit code 1.",
            "✗".red(),
            plural(errors)
        ),
    }
}

/// One block per environment, then a verdict that never becomes an exit code.
///
/// The rows are the same rows `rbx check` prints; what status adds is the
/// grouping. A repository's state is per-env (`prod` is in sync and `staging`
/// is three badges behind) and a flat list of `tool/check [env]` labels makes
/// the reader do that grouping in their head every time.
pub fn status(report: &Report) {
    if report.tools.is_empty() {
        println!(
            "{} No rbx config files found here. Nothing to report.",
            "-".dimmed()
        );
        return;
    }

    println!("{}", "rbx status".bold());

    let groups = group_by_env(report);
    let width = report
        .tools
        .iter()
        .map(|tool| row_label(tool).chars().count())
        .max()
        .unwrap_or(0);

    for (env, rows) in &groups {
        let worst = rows
            .iter()
            .map(|row| row.outcome)
            .max()
            .unwrap_or(Outcome::Skipped);
        println!();
        match env {
            Some(name) => println!("  {} {}", mark(worst), name.bold()),
            // Not an env: `env/gen-module` compares a generated file, and
            // `apikey/status` answers for the credential. Naming the group
            // rather than inventing an env for it keeps the two apart.
            None => println!("  {} {}", mark(worst), "repository".bold()),
        }
        for row in rows {
            println!(
                "      {} {:<width$}  {}",
                mark(row.outcome),
                row_label(row),
                row.summary,
                width = width
            );
            for detail in &row.details {
                println!("          {}", detail.dimmed());
            }
        }
    }

    println!();
    println!("{} {}", mark(report.worst()), verdict(report));
    // Said out loud, every time. A reader who sees `!` and a zero exit code
    // should not have to wonder which of the two is the bug.
    println!(
        "{}",
        format!(
            "rbx status always exits 0; rbx check here would exit {}.",
            report.worst().exit_code()
        )
        .dimmed()
    );
}

/// The verdict sentence, without the mark the caller prints in front of it.
///
/// The rule it exists to keep: the overview may not claim anything the rows do
/// not carry. Returned as a `String` rather than printed so a test can read
/// it, which is the only way that rule is enforceable.
fn verdict(report: &Report) -> String {
    let clean = report.count(Outcome::Clean);
    let skipped = report.count(Outcome::Skipped);
    let drifting = report.count(Outcome::Drift);
    let errors = report.count(Outcome::Error);
    let scope = env_scope(report);

    match report.worst() {
        // "Everything matches" is only true of a run where nothing sat out,
        // and skipped rows are the normal case rather than the corner one:
        // `apikey/status` is always skipped, and `--offline` skips
        // `config/live` by construction. So the moment a row did not run, the
        // verdict counts instead of concluding, in the words `summary` uses
        // for the same report.
        Outcome::Clean if skipped == 0 => {
            format!("Everything declared here matches what is recorded{scope}.")
        }
        Outcome::Clean => format!(
            "{clean} check{} clean, {skipped} skipped{scope}. Nothing that ran disagrees.",
            plural(clean)
        ),
        Outcome::Skipped => "Nothing to report; every check was skipped.".to_string(),
        Outcome::Drift => format!(
            "{drifting} check{} out of sync. Re-run the tool's own sync, or `rbx check` for the \
             CI verdict.",
            plural(drifting)
        ),
        Outcome::Error => format!(
            "{errors} check{} could not answer, {drifting} out of sync. Read the message above, \
             then `rbx check` for the CI verdict.",
            plural(errors)
        ),
    }
}

/// ` (2 envs)`, or nothing when the run targeted no env at all.
///
/// The envs are the ones the run was asked about, which the report carries;
/// counting the groups instead counts the envs that produced a row, and prints
/// "0 envs" for a repo that declares two of them and checks both repo-wide.
/// Nothing at all is the honest rendering of a run with no `--env`: that run
/// targets the standalone config blocks, and "0 envs" reads as "nothing is
/// configured here".
fn env_scope(report: &Report) -> String {
    match report.envs.len() {
        0 => String::new(),
        n => format!(" ({n} env{})", plural(n)),
    }
}

/// `shop/lockfile`, without the `[env]` suffix the group header already gives.
fn row_label(row: &ToolReport) -> String {
    format!("{}/{}", row.tool, row.check)
}

/// Rows grouped by env: the repo-wide ones first, then each env in the order
/// the engine reported it.
///
/// That order is alphabetical, not the order the operator declared their envs
/// in, and it cannot be the latter from here: `--env all` expands through
/// `PlacesFile::env_names`, which collects the sections of a map and sorts
/// them, so declaration order is gone long before a row exists. Grouping by
/// first appearance is still what happens here (the renderer does not
/// re-sort what the engine handed it) but it promises no more than the
/// engine can produce.
fn group_by_env(report: &Report) -> Vec<(Option<&str>, Vec<&ToolReport>)> {
    let mut groups: Vec<(Option<&str>, Vec<&ToolReport>)> = Vec::new();
    for row in &report.tools {
        let key = row.env.as_deref();
        match groups.iter_mut().find(|(env, _)| *env == key) {
            Some((_, rows)) => rows.push(row),
            None => groups.push((key, vec![row])),
        }
    }
    groups.sort_by_key(|(env, _)| env.is_some());
    groups
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::ToolReport;

    /// The renderer has no return value to assert on; what these pin is that
    /// it stays total: every outcome combination has a verdict line and none
    /// of the arithmetic panics on an empty or single-row report.
    #[test]
    fn every_shape_of_report_renders_without_panicking() {
        summary(&Report::default());

        for outcome in [
            Outcome::Clean,
            Outcome::Drift,
            Outcome::Error,
            Outcome::Skipped,
        ] {
            let mut report = Report::default();
            report.push(ToolReport::new("shop", "lockfile", outcome, "summary"));
            summary(&report);
        }

        let mut mixed = Report::default();
        mixed.push(ToolReport::new("env", "gen-module", Outcome::Clean, "ok"));
        mixed.push(
            ToolReport::new("shop", "lockfile", Outcome::Drift, "1 to create")
                .env("prod")
                .detail("orphan: VIP"),
        );
        mixed.push(ToolReport::new("config", "live", Outcome::Error, "no key"));
        mixed.push(ToolReport::new("apikey", "status", Outcome::Skipped, "n/a"));
        summary(&mixed);
    }

    fn row(tool: &'static str, check: &'static str, outcome: Outcome) -> ToolReport {
        ToolReport::new(tool, check, outcome, "summary")
    }

    /// An empty report for a run that was asked about `envs`, the way `gather`
    /// starts one.
    fn report_over(envs: &[&str]) -> Report {
        Report {
            envs: envs.iter().map(|env| env.to_string()).collect(),
            ..Report::default()
        }
    }

    #[test]
    fn every_shape_of_report_renders_a_status_without_panicking() {
        status(&Report::default());

        let mut mixed = Report::default();
        mixed.push(row("env", "gen-module", Outcome::Clean));
        mixed.push(row("shop", "lockfile", Outcome::Drift).env("prod"));
        mixed.push(row("config", "live", Outcome::Error).env("prod"));
        mixed.push(row("meta", "lockfile", Outcome::Skipped).env("dev"));
        status(&mixed);
    }

    /// The repo-wide rows are not an env and must not be rendered as one, and
    /// the envs keep the order the engine reported them in.
    ///
    /// The rows are the ones the pipeline can actually emit: it walks the
    /// tools in `Tool::ALL` order and, inside each, the envs `--env all`
    /// expanded, which `PlacesFile::env_names` sorts. A report built in an
    /// order the engine cannot produce would let this test pass on a claim the
    /// command never honours, which is how the declaration-order claim
    /// survived.
    #[test]
    fn grouping_puts_the_repo_wide_rows_first_and_keeps_the_engines_env_order() {
        let mut report = Report::default();
        report.push(row("env", "gen-module", Outcome::Clean));
        report.push(row("shop", "lockfile", Outcome::Clean).env("dev"));
        report.push(row("shop", "lockfile", Outcome::Clean).env("prod"));
        report.push(row("meta", "lockfile", Outcome::Clean).env("dev"));

        let groups = group_by_env(&report);

        assert_eq!(groups[0].0, None, "repo-wide rows come first");
        assert_eq!(groups[1].0, Some("dev"), "dev was reported first");
        assert_eq!(groups[2].0, Some("prod"));
        assert_eq!(
            groups[1].1.len(),
            2,
            "both dev rows belong to the same block"
        );
    }

    /// The verdict may not conclude more than the rows say. One clean row and
    /// one skipped row is not "everything matches": `apikey/status` is always
    /// skipped and `--offline` skips `config/live`, so the reading this
    /// protects against is "everything matches" over a run that compared
    /// nothing against Roblox.
    #[test]
    fn a_clean_verdict_does_not_swallow_the_skipped_rows() {
        let mut report = Report::default();
        report.push(row("env", "gen-module", Outcome::Clean));
        report.push(row("apikey", "status", Outcome::Skipped));

        let line = verdict(&report);

        assert!(
            !line.contains("Everything"),
            "a run with a skipped row did not check everything: {line}"
        );
        assert_eq!(
            line,
            "1 check clean, 1 skipped. Nothing that ran disagrees."
        );
    }

    /// The other half: with nothing skipped, the report does carry the claim.
    #[test]
    fn a_wholly_clean_run_still_says_everything_matches() {
        let mut report = Report::default();
        report.push(row("env", "gen-module", Outcome::Clean));

        assert_eq!(
            verdict(&report),
            "Everything declared here matches what is recorded."
        );
    }

    /// The count is of the envs the run was asked about. A repo with two envs
    /// whose only checks are repo-wide has two envs, and printing "0 envs"
    /// tells its operator nothing is configured.
    #[test]
    fn the_env_count_counts_envs_not_groups() {
        let mut report = report_over(&["dev", "prod"]);
        report.push(row("env", "gen-module", Outcome::Clean));

        assert_eq!(
            verdict(&report),
            "Everything declared here matches what is recorded (2 envs)."
        );
    }

    /// No `--env` targets the standalone config blocks, which is not zero
    /// envs; the verdict says nothing about envs rather than saying "0".
    #[test]
    fn a_run_with_no_env_flag_claims_no_env_count() {
        let mut report = Report::default();
        report.push(row("env", "gen-module", Outcome::Clean));

        assert!(!verdict(&report).contains("env"), "{}", verdict(&report));
    }

    /// The count follows the run, not the rows: one env asked about, one env
    /// reported, whichever tools happened to answer for it.
    #[test]
    fn one_env_is_counted_in_the_singular() {
        let mut report = report_over(&["prod"]);
        report.push(row("meta", "lockfile", Outcome::Clean).env("prod"));
        report.push(row("config", "live", Outcome::Skipped).env("prod"));

        assert_eq!(
            verdict(&report),
            "1 check clean, 1 skipped (1 env). Nothing that ran disagrees."
        );
    }

    /// The env is in the block header, so repeating it on every row would be
    /// noise in the one view whose job is to be readable.
    #[test]
    fn a_row_label_drops_the_env_the_header_already_carries() {
        assert_eq!(
            row_label(&row("shop", "lockfile", Outcome::Clean)),
            "shop/lockfile"
        );
        assert_eq!(
            row_label(&row("shop", "lockfile", Outcome::Clean).env("prod")),
            "shop/lockfile"
        );
    }

    #[test]
    fn plural_agrees_with_the_count() {
        assert_eq!(plural(1), "");
        assert_eq!(plural(0), "s");
        assert_eq!(plural(2), "s");
    }
}
