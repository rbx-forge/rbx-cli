//! What the engine produces, independent of how it is shown.
//!
//! The split matters more than it looks: `rbx status` (the human view in #9)
//! is the same gathering pass with a different renderer, and a check that
//! prints as it goes cannot be reused that way. So nothing in this module
//! writes to stdout: `render` does that, and only that.

use rbx_core::generated::Drift;
use rbx_core::output::SCHEMA_VERSION;
use serde::Serialize;

/// What one tool concluded.
///
/// Ordered by how loudly it should win an aggregation: an error outranks
/// drift, drift outranks clean, and a tool that did not run cannot outrank
/// anything. That ordering is the exit-code contract, so it is derived from
/// `Ord` rather than restated at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The tool did not run: no config file, or `--offline` and it needs the
    /// network. Never a failure: a repo without `rbxshop.toml` is not a repo
    /// with a broken shop.
    Skipped,
    /// Declared state matches recorded state.
    Clean,
    /// Declared state and recorded state disagree. Actionable by re-running
    /// the tool's own sync or codegen.
    Drift,
    /// The tool could not answer: unreadable config, failed request, missing
    /// credential. Actionable by reading the message.
    Error,
}

impl Outcome {
    /// The process exit code this outcome maps to on its own.
    pub fn exit_code(self) -> u8 {
        match self {
            Outcome::Skipped | Outcome::Clean => 0,
            Outcome::Drift => rbx_core::generated::DRIFT_EXIT_CODE,
            Outcome::Error => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Outcome::Skipped => "skipped",
            Outcome::Clean => "clean",
            Outcome::Drift => "drift",
            Outcome::Error => "error",
        }
    }
}

/// One tool's answer for one env.
///
/// Field names are the `--json` contract; see `docs/check.md`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolReport {
    /// Stable tool identifier: `env`, `shop`, `meta`, `config`, `rtbf`,
    /// `apikey`.
    pub tool: &'static str,
    /// Which check within the tool, when a tool contributes more than one
    /// (`shop` runs both a lockfile diff and a codegen comparison).
    pub check: &'static str,
    /// The env this answer is about. Omitted for checks that are not per-env.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    pub outcome: Outcome,
    /// One line, terse. This is what CI logs show.
    pub summary: String,
    /// Optional extra lines, shown indented under the summary.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
}

impl ToolReport {
    pub fn new(
        tool: &'static str,
        check: &'static str,
        outcome: Outcome,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            tool,
            check,
            env: None,
            outcome,
            summary: summary.into(),
            details: Vec::new(),
        }
    }

    pub fn env(mut self, env: impl Into<String>) -> Self {
        self.env = Some(env.into());
        self
    }

    pub fn detail(mut self, line: impl Into<String>) -> Self {
        self.details.push(line.into());
        self
    }

    pub fn details(mut self, lines: impl IntoIterator<Item = String>) -> Self {
        self.details.extend(lines);
        self
    }

    /// `shop/lockfile [prod]`: how a row is named in output and in an error.
    pub fn label(&self) -> String {
        let base = format!("{}/{}", self.tool, self.check);
        match &self.env {
            Some(env) => format!("{base} [{env}]"),
            None => base,
        }
    }
}

/// Every tool's answer for this invocation.
#[derive(Debug, Default, Clone)]
pub struct Report {
    pub tools: Vec<ToolReport>,
    /// The envs this run was asked about, in the order the engine expanded
    /// them. Empty when no `--env` was given, which targets the standalone
    /// config blocks rather than zero envs.
    ///
    /// Carried on the report rather than recomputed by a renderer, because a
    /// renderer counting the envs that produced rows counts something else:
    /// a repo declaring `dev` and `prod` whose only checks are repo-wide has
    /// two envs and no env-tagged row.
    pub envs: Vec<String>,
}

impl Report {
    pub fn push(&mut self, report: ToolReport) {
        self.tools.push(report);
    }

    /// The aggregate outcome: error beats drift beats clean.
    ///
    /// An empty report is [`Outcome::Skipped`]: nothing was found to check,
    /// which exits 0 but says so rather than claiming everything is clean.
    pub fn worst(&self) -> Outcome {
        self.tools
            .iter()
            .map(|t| t.outcome)
            .max()
            .unwrap_or(Outcome::Skipped)
    }

    pub fn count(&self, outcome: Outcome) -> usize {
        self.tools.iter().filter(|t| t.outcome == outcome).count()
    }

    /// Rows that did not come back clean, in report order.
    pub fn problems(&self) -> impl Iterator<Item = &ToolReport> {
        self.tools
            .iter()
            .filter(|t| matches!(t.outcome, Outcome::Drift | Outcome::Error))
    }

    /// Turn the aggregate into the process result.
    ///
    /// Drift becomes `Err(Drift)` because that is what the binary maps to exit
    /// code 2: the same channel `rbx env gen-module --check` already uses, so
    /// there is one definition of "exit 2" in the tree rather than two.
    pub fn into_result(&self) -> anyhow::Result<()> {
        match self.worst() {
            Outcome::Skipped | Outcome::Clean => Ok(()),
            Outcome::Drift => Err(Drift::new(self.failure_message("found drift")).into()),
            Outcome::Error => Err(anyhow::anyhow!(self.failure_message("did not pass"))),
        }
    }

    fn failure_message(&self, verb: &str) -> String {
        let names: Vec<String> = self.problems().map(|t| t.label()).collect();
        format!(
            "rbx check: {} of {} check{} {}, {}",
            names.len(),
            self.tools.len(),
            if self.tools.len() == 1 { "" } else { "s" },
            verb,
            names.join(", ")
        )
    }

    /// The `--json` document.
    pub fn as_json(&self) -> CheckDocument<'_> {
        CheckDocument {
            schema_version: SCHEMA_VERSION,
            outcome: self.worst(),
            exit_code: self.worst().exit_code(),
            totals: Totals {
                total: self.tools.len(),
                clean: self.count(Outcome::Clean),
                drift: self.count(Outcome::Drift),
                error: self.count(Outcome::Error),
                skipped: self.count(Outcome::Skipped),
            },
            checks: &self.tools,
        }
    }
}

/// What `rbx check --json` writes to stdout.
///
/// A named object rather than a positional array, all the way down: a consumer
/// survives a field being added and does not survive a column shifting. Field
/// names are documented in `docs/check.md` and are the compatibility surface:
/// `schema_version` is bumped if one changes meaning or goes away.
#[derive(Debug, Serialize)]
pub struct CheckDocument<'a> {
    pub schema_version: u32,
    /// The aggregate: `clean`, `drift`, `error`, or `skipped`.
    pub outcome: Outcome,
    /// The process exit code, so a consumer that already captured stdout does
    /// not have to plumb `$?` through as well.
    pub exit_code: u8,
    pub totals: Totals,
    pub checks: &'a [ToolReport],
}

#[derive(Debug, Serialize)]
pub struct Totals {
    pub total: usize,
    pub clean: usize,
    pub drift: usize,
    pub error: usize,
    pub skipped: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(outcome: Outcome) -> ToolReport {
        ToolReport::new("shop", "lockfile", outcome, "summary")
    }

    #[test]
    fn error_outranks_drift_outranks_clean_outranks_skipped() {
        assert!(Outcome::Error > Outcome::Drift);
        assert!(Outcome::Drift > Outcome::Clean);
        assert!(Outcome::Clean > Outcome::Skipped);
    }

    #[test]
    fn outcomes_map_to_the_documented_exit_codes() {
        assert_eq!(Outcome::Clean.exit_code(), 0);
        assert_eq!(Outcome::Skipped.exit_code(), 0);
        assert_eq!(Outcome::Drift.exit_code(), 2);
        assert_eq!(Outcome::Error.exit_code(), 1);
    }

    #[test]
    fn drift_wins_over_clean() {
        let mut report = Report::default();
        report.push(row(Outcome::Clean));
        report.push(row(Outcome::Drift));
        report.push(row(Outcome::Clean));
        assert_eq!(report.worst(), Outcome::Drift);
    }

    /// The ordering that matters most: a failing tool must not be masked by a
    /// drifting one, because the two ask for different things from whoever
    /// reads the CI log.
    #[test]
    fn error_wins_over_drift() {
        let mut report = Report::default();
        report.push(row(Outcome::Drift));
        report.push(row(Outcome::Error));
        report.push(row(Outcome::Drift));
        assert_eq!(report.worst(), Outcome::Error);
    }

    #[test]
    fn a_skipped_tool_never_raises_the_aggregate() {
        let mut report = Report::default();
        report.push(row(Outcome::Clean));
        report.push(row(Outcome::Skipped));
        assert_eq!(report.worst(), Outcome::Clean);
    }

    #[test]
    fn an_empty_report_is_skipped_not_clean() {
        assert_eq!(Report::default().worst(), Outcome::Skipped);
    }

    #[test]
    fn a_clean_report_is_ok() {
        let mut report = Report::default();
        report.push(row(Outcome::Clean));
        assert!(report.into_result().is_ok());
    }

    #[test]
    fn a_drifting_report_errors_with_a_drift_the_binary_maps_to_exit_2() {
        let mut report = Report::default();
        report.push(row(Outcome::Clean));
        report.push(row(Outcome::Drift).env("prod"));

        let err = report.into_result().expect_err("drift must not be Ok");

        assert!(
            err.chain().any(|cause| cause.is::<Drift>()),
            "the binary keys exit code 2 off a Drift in the chain: {err:#}"
        );
        assert!(
            format!("{err:#}").contains("shop/lockfile [prod]"),
            "{err:#}"
        );
    }

    /// An erroring report must *not* carry `Drift`, or the binary reports the
    /// failure as staleness and CI tells somebody to regenerate a file.
    #[test]
    fn an_erroring_report_is_not_reported_as_drift() {
        let mut report = Report::default();
        report.push(row(Outcome::Drift));
        report.push(row(Outcome::Error));

        let err = report.into_result().expect_err("error must not be Ok");

        assert!(
            !err.chain().any(|cause| cause.is::<Drift>()),
            "an error must exit 1, not 2: {err:#}"
        );
    }

    #[test]
    fn a_label_names_the_tool_the_check_and_the_env() {
        assert_eq!(
            row(Outcome::Clean).env("dev").label(),
            "shop/lockfile [dev]"
        );
        assert_eq!(row(Outcome::Clean).label(), "shop/lockfile");
    }
}
