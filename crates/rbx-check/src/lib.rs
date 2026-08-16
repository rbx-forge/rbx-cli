//! `rbx check` and `rbx status` — every configured tool's check in one pass.
//!
//! A full CI integration used to mean knowing and chaining five commands
//! (`env gen-module --check`, `shop check`, `shop codegen --check`,
//! `meta check`, `config check`, `apikey status`) and getting the exit-code
//! handling right for each. This is that list, discovered from the repo rather
//! than configured, with one aggregated exit code.
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |---|---|
//! | 0 | every check that ran came back clean |
//! | 2 | at least one check found drift, none failed |
//! | 1 | at least one check failed |
//!
//! Error beats drift beats clean, because the three ask different things of
//! whoever reads the log: read the message, re-run a sync, or move on.
//!
//! # Two contracts, one engine
//!
//! One gathering pass per tool producing a `Report`, and two renderers over
//! it, which is why nothing outside the `render` module prints.
//!
//! | | `rbx check` | `rbx status` |
//! |---|---|---|
//! | answers | may this build continue | where does this project stand |
//! | shape | one line per check | one block per environment |
//! | exit code | 0 / 2 / 1, per the table above | **always 0** |
//!
//! Always 0 is not a detail of `status`: it is the reason it exists as a
//! separate command. A status that fails a script is a check with worse
//! output.

mod discovery;
mod render;
mod report;
mod tools;

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;

use rbx_core::output::{self, OutputFormat};
use rbx_core::GlobalFlags;

pub use report::{CheckDocument, Outcome, Report, ToolReport, Totals};

#[derive(Args, Debug)]
pub struct CheckCli {
    /// Skip checks that need network access and credentials.
    ///
    /// Everything else still runs: the generated-file comparisons and the
    /// config-against-lockfile diffs are all local. This is the mode for a
    /// pre-commit hook, where a network round trip is not acceptable and a
    /// credential is usually not present.
    #[arg(long)]
    pub offline: bool,

    /// Directory to look for config files in.
    ///
    /// Defaults to the working directory. `rbxplace.toml` is looked for here
    /// too, the same way `rbx import --dir` writes it here; an explicit
    /// `--places` still wins, for a shared env file kept elsewhere.
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,

    /// Write the report to stdout as one JSON document instead of a summary.
    ///
    /// stdout carries the document and nothing else; diagnostics stay on
    /// stderr, and the exit code is unchanged. Field names are documented in
    /// docs/check.md.
    #[arg(long)]
    pub json: bool,
}

/// `rbx status` — the same engine, the opposite contract.
///
/// The flags are `CheckCli`'s minus the ones that only make sense to a
/// machine, which is none of them: what differs is the renderer and the exit
/// code, so the two structs stay separate types over the same fields rather
/// than one struct with a mode flag. A shared struct would put "am I check or
/// status" inside every function that reads it.
#[derive(Args, Debug)]
pub struct StatusCli {
    /// Skip checks that need network access and credentials.
    ///
    /// The offline half still renders. That is the point of the flag here:
    /// `rbx status --offline` is the overview you can get on a plane, with the
    /// live rows marked skipped rather than missing.
    #[arg(long)]
    pub offline: bool,

    /// Directory to look for config files in, `rbxplace.toml` included.
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,

    /// Write the report to stdout as one JSON document instead of the
    /// overview.
    ///
    /// The same document `rbx check --json` emits, down to `exit_code` — which
    /// is what `rbx check` *would* exit with, since `rbx status` always exits
    /// 0. Documented in docs/check.md.
    #[arg(long)]
    pub json: bool,
}

/// Discover, check, render, and return the aggregated verdict.
pub async fn run(cli: CheckCli, global: &GlobalFlags) -> Result<()> {
    let report = gather(&cli.dir, cli.offline, global).await?;

    let format = OutputFormat::from_json_flag(cli.json);
    if format.is_json() {
        output::emit(&report.as_json())?;
    } else {
        render::summary(&report);
    }
    report.into_result()
}

/// Render the same report as a human overview, and always exit 0.
///
/// Always 0 is the whole contract. A status command that fails a script is a
/// check command with worse output, and somebody who runs it in a shell that
/// has `set -e` should not lose their session because staging drifted. That
/// includes the discovery step, which `status_report` explains.
pub async fn status(cli: StatusCli, global: &GlobalFlags) -> Result<()> {
    let report = status_report(&cli, global).await;

    let format = OutputFormat::from_json_flag(cli.json);
    if format.is_json() {
        output::emit(&report.as_json())?;
    } else {
        render::status(&report);
    }
    Ok(())
}

/// The report `rbx status` renders, for every repository including the ones
/// that cannot be read.
///
/// Discovery is the one step that runs before a report exists, so it is the
/// one place the always-0 contract could leak. It does not get to: a failure
/// becomes a row like any other outcome. Rendering it rather than writing it
/// to stderr keeps it in the channel the reader is already looking at, and
/// puts it in the `--json` document too, which a stderr line would deny a
/// consumer. Exiting 0 on silence would be the worse bug of the two.
async fn status_report(cli: &StatusCli, global: &GlobalFlags) -> Report {
    match gather(&cli.dir, cli.offline, global).await {
        Ok(report) => report,
        Err(err) => discovery_failure(&err),
    }
}

/// A discovery error as a one-row report: summary is the message, the causes
/// under it are the details, the way any other error row reads.
fn discovery_failure(err: &anyhow::Error) -> Report {
    let mut causes = err.chain().map(ToString::to_string);
    let summary = causes
        .next()
        .unwrap_or_else(|| "this repository could not be read".to_string());
    let details: Vec<String> = causes.collect();

    let mut report = Report::default();
    report.push(ToolReport::new("env", "discovery", Outcome::Error, summary).details(details));
    report
}

/// The one gathering pass, shared by both contracts.
///
/// The `Err` it can still return is not a check outcome: it is a repository
/// that cannot be read at all, such as an unreadable places file or `--env
/// all` against one with no envs in it. `check` fails on it; `status` renders
/// it as an error row, which is what keeps its exit code at 0 without turning
/// the failure into an empty screen.
async fn gather(dir: &Path, offline: bool, global: &GlobalFlags) -> Result<Report> {
    // `--dir` names the project, and `rbxplace.toml` is part of the project,
    // so the implicit lookup moves with it; an explicit `--places` still wins.
    // The rule is `rbx_core::places::resolve_places_path`, shared with `rbx
    // import` so the two commands cannot disagree about the same directory.
    //
    // Rebound rather than threaded as a second argument: `tools::config`
    // resolves the universe through `global.places` too, and a `--dir` that
    // moved discovery but not that lookup would be the same split one level
    // down.
    let global = &GlobalFlags {
        places: rbx_core::places::resolve_places_path(&global.places, dir),
        ..global.clone()
    };

    let found = discovery::discover(dir, &global.places);
    let envs = discovery::target_envs(global.env.as_deref(), &global.places)?;

    let mut report = Report {
        // What the run was asked about, not what happened to produce a row:
        // the overview reports on these, and a tool that contributes only
        // repo-wide checks does not make the envs disappear.
        envs: envs.iter().flatten().cloned().collect(),
        ..Report::default()
    };
    for entry in &found {
        let rows = match entry.tool {
            discovery::Tool::Env => tools::env(&entry.path),
            discovery::Tool::Shop => tools::shop(&entry.path, &envs),
            discovery::Tool::Meta => tools::meta(&entry.path, &envs),
            discovery::Tool::Config => tools::config(&entry.path, &envs, global, offline).await,
            discovery::Tool::Apikey => tools::apikey(),
        };
        for row in rows {
            report.push(row);
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn global(dir: &Path, env: Option<&str>) -> GlobalFlags {
        GlobalFlags {
            api_key: None,
            cookie: None,
            no_auto_cookie: true,
            auto_cookie: false,
            env: env.map(str::to_string),
            place: None,
            places: dir.join("rbxplace.toml"),
            universe_id: None,
            place_id: Vec::new(),
        }
    }

    fn cli(dir: &Path, offline: bool) -> CheckCli {
        CheckCli {
            offline,
            dir: dir.to_path_buf(),
            json: false,
        }
    }

    /// `--dir` names the project, and `rbxplace.toml` is part of the project.
    /// With no explicit `--places`, the implicit lookup has to move with it,
    /// or `rbx check --dir game` reports on no envs at all in the directory
    /// `rbx import --dir game` just wrote them into. Same rule for both
    /// commands: `rbx_core::places::resolve_places_path`.
    #[tokio::test]
    async fn dir_moves_the_implicit_places_lookup() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("rbxplace.toml"),
            "[codegen]\noutput = \"Envs.luau\"\n\n[prod]\nuniverse_id = 1\n",
        )
        .expect("write");

        // The default `--places`, so only `--dir` can reach the file above.
        let mut flags = global(dir.path(), None);
        flags.places = PathBuf::from("rbxplace.toml");

        let err = run(cli(dir.path(), true), &flags)
            .await
            .expect_err("the env module was never generated, which is drift");

        assert!(format!("{err:#}").contains("env/gen-module"), "{err:#}");
    }

    /// The other half of the rule: a shared env file named outright is not
    /// moved by `--dir`.
    #[tokio::test]
    async fn an_explicit_places_path_is_not_moved_by_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let shared = dir.path().join("shared");
        std::fs::create_dir(&shared).expect("mkdir");
        std::fs::write(
            shared.join("envs.toml"),
            "[codegen]\noutput = \"Envs.luau\"\n\n[prod]\nuniverse_id = 1\n",
        )
        .expect("write");

        let mut flags = global(dir.path(), None);
        flags.places = shared.join("envs.toml");

        let err = run(cli(dir.path(), true), &flags)
            .await
            .expect_err("the env module was never generated, which is drift");

        assert!(format!("{err:#}").contains("env/gen-module"), "{err:#}");
    }

    /// A directory with no rbx config files is not a failure. Running `rbx
    /// check` in the wrong place should say so, not exit 1.
    #[tokio::test]
    async fn an_unconfigured_directory_is_clean() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = run(cli(dir.path(), false), &global(dir.path(), None)).await;
        assert!(result.is_ok(), "{:#}", result.unwrap_err());
    }

    /// The offline contract: a repo whose only remote check is `config` must
    /// come back clean under `--offline` even with no credential anywhere.
    #[tokio::test]
    async fn offline_skips_the_remote_check_instead_of_failing_on_a_missing_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("rbxconfig.toml"), "[dev]\n[dev.entries]\n").expect("write");

        let offline = run(cli(dir.path(), true), &global(dir.path(), Some("dev"))).await;
        assert!(offline.is_ok(), "{:#}", offline.unwrap_err());

        // Without --offline the same repo asks for a key it does not have,
        // and that is an error rather than silent success.
        let online = run(cli(dir.path(), false), &global(dir.path(), Some("dev"))).await;
        let err = online.expect_err("a remote check with no key must not pass silently");
        assert!(format!("{err:#}").contains("config/live"), "{err:#}");
    }

    /// The same repo must not exit 1 just because no key was loaded when
    /// there was nothing to check: with no `--env`, `config/live` skips, so a
    /// keyless run is exit 0 exactly like a run that has a key.
    #[tokio::test]
    async fn a_keyless_run_with_nothing_to_check_exits_0_without_offline() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("rbxconfig.toml"), "[dev]\n[dev.entries]\n").expect("write");

        let result = run(cli(dir.path(), false), &global(dir.path(), None)).await;
        assert!(result.is_ok(), "{:#}", result.unwrap_err());
    }

    /// A meta config declaring state against no lockfile is drift, and drift
    /// must surface as exit code 2 — not as a failure and not as success.
    #[tokio::test]
    async fn a_meta_config_with_no_lockfile_reports_drift_as_exit_code_2() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("rbxmeta.toml"),
            "[experience]\nuniverse_id = 1\nplace_id = 2\n\n[game]\nname = \"Declared\"\n",
        )
        .expect("write");

        let err = run(cli(dir.path(), false), &global(dir.path(), None))
            .await
            .expect_err("declared but never synced is drift");

        assert!(
            err.chain()
                .any(|cause| cause.is::<rbx_core::generated::Drift>()),
            "drift must exit 2, not 1: {err:#}"
        );
        assert!(format!("{err:#}").contains("meta/lockfile"), "{err:#}");
    }

    /// An unreadable config is an error, and an error must outrank the drift
    /// reported by another tool in the same run.
    #[tokio::test]
    async fn a_broken_config_fails_the_run_even_alongside_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("rbxmeta.toml"),
            "[experience]\nuniverse_id = 1\nplace_id = 2\n\n[game]\nname = \"Declared\"\n",
        )
        .expect("write");
        std::fs::write(
            dir.path().join("rbxshop.toml"),
            "this is not = valid toml [[[",
        )
        .expect("write");

        let err = run(cli(dir.path(), false), &global(dir.path(), None))
            .await
            .expect_err("a broken config must fail the run");

        assert!(
            !err.chain()
                .any(|cause| cause.is::<rbx_core::generated::Drift>()),
            "an error must exit 1 even when another tool drifted: {err:#}"
        );
    }

    /// `--env all` fans every per-env check out over `rbxplace.toml`, so one
    /// drifting env fails the run for the whole repo.
    #[tokio::test]
    async fn env_all_checks_every_env_named_in_the_places_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("rbxplace.toml"),
            "[dev]\nuniverse_id = 1\n\n[prod]\nuniverse_id = 2\n",
        )
        .expect("write");
        // Base is empty; only `prod` declares anything, so only `prod` drifts.
        std::fs::write(
            dir.path().join("rbxmeta.toml"),
            "[envs.prod]\nname = \"Prod name\"\n",
        )
        .expect("write");

        let err = run(cli(dir.path(), false), &global(dir.path(), Some("all")))
            .await
            .expect_err("prod drifts");

        let text = format!("{err:#}");
        assert!(text.contains("meta/lockfile [prod]"), "{text}");
        assert!(
            !text.contains("meta/lockfile [dev]"),
            "dev declares nothing and must stay clean: {text}"
        );
    }

    // -----------------------------------------------------------------------
    // --json
    // -----------------------------------------------------------------------

    fn json_of(report: &Report) -> serde_json::Value {
        let mut buf = Vec::new();
        output::write_json(&mut buf, &report.as_json()).expect("write");
        serde_json::from_slice(&buf).expect("the document must be valid JSON")
    }

    fn sample_report() -> Report {
        let mut report = Report::default();
        report.push(ToolReport::new(
            "env",
            "gen-module",
            Outcome::Clean,
            "1 generated file up to date",
        ));
        report.push(
            ToolReport::new("meta", "lockfile", Outcome::Drift, "2 pending changes")
                .env("prod")
                .detail("name: (unset) → My Game"),
        );
        report.push(ToolReport::new(
            "apikey",
            "status",
            Outcome::Skipped,
            "not yet wired",
        ));
        report
    }

    /// The documented field names are the compatibility surface, so they are
    /// asserted by name rather than by shape.
    #[test]
    fn the_json_document_carries_the_documented_fields() {
        let doc = json_of(&sample_report());

        assert_eq!(doc["schema_version"], output::SCHEMA_VERSION);
        assert_eq!(doc["outcome"], "drift");
        assert_eq!(doc["exit_code"], 2);
        assert_eq!(doc["totals"]["total"], 3);
        assert_eq!(doc["totals"]["clean"], 1);
        assert_eq!(doc["totals"]["drift"], 1);
        assert_eq!(doc["totals"]["error"], 0);
        assert_eq!(doc["totals"]["skipped"], 1);

        let checks = doc["checks"].as_array().expect("checks is an array");
        assert_eq!(checks.len(), 3);
        assert_eq!(checks[1]["tool"], "meta");
        assert_eq!(checks[1]["check"], "lockfile");
        assert_eq!(checks[1]["env"], "prod");
        assert_eq!(checks[1]["outcome"], "drift");
        assert_eq!(checks[1]["summary"], "2 pending changes");
        assert_eq!(checks[1]["details"][0], "name: (unset) → My Game");
    }

    /// Rows are objects keyed by name, not positional arrays: a consumer must
    /// survive a field being added, which a column index does not.
    #[test]
    fn a_check_row_is_an_object_not_a_tuple() {
        let doc = json_of(&sample_report());
        assert!(doc["checks"][0].is_object(), "{doc}");
    }

    /// Absent optional fields are omitted rather than emitted as null, so
    /// `has("env")` is a usable test in jq.
    #[test]
    fn fields_that_do_not_apply_are_absent_rather_than_null() {
        let doc = json_of(&sample_report());

        assert!(doc["checks"][0].get("env").is_none(), "{doc}");
        assert!(doc["checks"][0].get("details").is_none(), "{doc}");
        assert!(doc["checks"][1].get("env").is_some(), "{doc}");
    }

    /// The exit code in the document must agree with the process's, or a
    /// consumer reading one and branching on the other silently disagrees
    /// with CI.
    #[test]
    fn the_documents_exit_code_agrees_with_the_process_result() {
        for (outcome, code) in [
            (Outcome::Clean, 0u8),
            (Outcome::Drift, 2),
            (Outcome::Error, 1),
        ] {
            let mut report = Report::default();
            report.push(ToolReport::new("shop", "lockfile", outcome, "s"));
            let doc = json_of(&report);

            assert_eq!(doc["exit_code"], code);
            match report.into_result() {
                Ok(()) => assert_eq!(code, 0),
                Err(err) => {
                    let is_drift = err.chain().any(|c| c.is::<rbx_core::generated::Drift>());
                    assert_eq!(if is_drift { 2 } else { 1 }, code, "{err:#}");
                }
            }
        }
    }

    #[test]
    fn an_empty_report_still_produces_a_document() {
        let doc = json_of(&Report::default());
        assert_eq!(doc["outcome"], "skipped");
        assert_eq!(doc["exit_code"], 0);
        assert_eq!(doc["checks"].as_array().expect("array").len(), 0);
    }

    // -----------------------------------------------------------------------
    // rbx status
    // -----------------------------------------------------------------------

    fn status_cli(dir: &Path, offline: bool, json: bool) -> StatusCli {
        StatusCli {
            offline,
            dir: dir.to_path_buf(),
            json,
        }
    }

    /// The whole contract in one test: the same repository that fails
    /// `rbx check` with exit 2 must leave `rbx status` at 0. A status command
    /// that fails a script is a check command with worse output.
    #[tokio::test]
    async fn status_exits_zero_on_the_repository_check_exits_two_on() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("rbxmeta.toml"),
            "[experience]\nuniverse_id = 1\nplace_id = 2\n\n[game]\nname = \"Declared\"\n",
        )
        .expect("write");

        run(cli(dir.path(), false), &global(dir.path(), None))
            .await
            .expect_err("declared but never synced is drift for check");

        let result = status(
            status_cli(dir.path(), false, false),
            &global(dir.path(), None),
        )
        .await;
        assert!(
            result.is_ok(),
            "status must never fail on drift: {result:#?}"
        );
    }

    /// An error is the other half: a config that cannot be read fails `check`
    /// with exit 1 and still leaves `status` at 0, because "one tool could not
    /// answer" is part of where the project stands.
    #[tokio::test]
    async fn status_exits_zero_even_when_a_check_could_not_answer() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("rbxshop.toml"),
            "this is not = valid toml [[[",
        )
        .expect("write");

        run(cli(dir.path(), false), &global(dir.path(), None))
            .await
            .expect_err("a broken config fails check");

        let result = status(
            status_cli(dir.path(), false, false),
            &global(dir.path(), None),
        )
        .await;
        assert!(result.is_ok(), "{result:#?}");
    }

    /// Read-only and useful with no credential: the offline half renders on
    /// its own rather than the whole command refusing.
    #[tokio::test]
    async fn status_renders_offline_with_no_api_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("rbxconfig.toml"), "[dev]\n[dev.entries]\n").expect("write");

        let result = status(
            status_cli(dir.path(), true, false),
            &global(dir.path(), Some("dev")),
        )
        .await;
        assert!(result.is_ok(), "{result:#?}");
    }

    /// The overview reports on the envs the run was asked about, so the report
    /// has to carry them: a repo whose checks are all repo-wide produces no
    /// env-tagged row at all, and counting rows would call it a repo with no
    /// envs.
    #[tokio::test]
    async fn the_report_carries_the_envs_the_run_targeted() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("rbxplace.toml"),
            "[prod]\nuniverse_id = 2\n\n[dev]\nuniverse_id = 1\n",
        )
        .expect("write");

        let report = status_report(
            &status_cli(dir.path(), true, false),
            &global(dir.path(), Some("all")),
        )
        .await;

        assert_eq!(report.envs, vec!["dev".to_string(), "prod".to_string()]);
        assert!(
            report.tools.iter().all(|row| row.env.is_none()),
            "no row names an env here, which is the whole point: {report:#?}"
        );
    }

    /// A repository that cannot be read at all is still a status: `--env all`
    /// against a places file with no envs is a mistake worth reporting, and
    /// reporting it is not the same as dying on it.
    #[tokio::test]
    async fn status_reports_a_repository_it_cannot_read_instead_of_failing() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("rbxplace.toml"),
            "# no envs here
",
        )
        .expect("write");

        let cli = status_cli(dir.path(), true, false);
        let flags = global(dir.path(), Some("all"));

        let report = status_report(&cli, &flags).await;
        assert_eq!(report.worst(), Outcome::Error, "{report:#?}");
        assert!(
            report
                .tools
                .iter()
                .any(|row| row.summary.contains("no envs")),
            "the reason must survive into the report: {report:#?}"
        );

        let result = status(cli, &flags).await;
        assert!(result.is_ok(), "{result:#?}");
    }

    /// The hole the contract had: a directory with no `rbxplace.toml` at all.
    /// `--env all` cannot expand, and that has to be a row rather than an
    /// exit code, or the one command documented as unable to kill a `set -e`
    /// script kills it whenever a prompt hook runs in the wrong directory.
    #[tokio::test]
    async fn status_exits_zero_with_no_places_file_and_says_what_happened() {
        let dir = tempfile::tempdir().expect("tempdir");

        let cli = status_cli(dir.path(), true, false);
        let flags = global(dir.path(), Some("all"));

        let report = status_report(&cli, &flags).await;
        assert_eq!(
            report
                .tools
                .iter()
                .map(|row| row.label())
                .collect::<Vec<_>>(),
            vec!["env/discovery".to_string()],
            "an unreadable repository is one error row, not silence"
        );
        assert_eq!(report.worst().exit_code(), 1, "check would exit 1 here");

        let result = status(cli, &flags).await;
        assert!(
            result.is_ok(),
            "rbx status must exit 0 on a directory it cannot read: {result:#?}"
        );
    }

    /// The message is the whole value of the row, so it is carried verbatim,
    /// causes included.
    #[test]
    fn a_discovery_failure_keeps_its_message_and_its_causes() {
        let err = anyhow::anyhow!("no such file").context("Failed to read rbxplace.toml");
        let report = discovery_failure(&err);

        assert_eq!(report.tools.len(), 1);
        assert_eq!(report.tools[0].outcome, Outcome::Error);
        assert_eq!(report.tools[0].summary, "Failed to read rbxplace.toml");
        assert_eq!(report.tools[0].details, vec!["no such file".to_string()]);
    }

    /// `status --json` is the same document as `check --json`, so a consumer
    /// can read either. `exit_code` carries what `check` would return, which
    /// is the only field where the two commands' *values* differ from each
    /// other's behaviour, and it is documented as such.
    #[test]
    fn the_status_document_is_the_check_document() {
        let doc = json_of(&sample_report());
        assert_eq!(doc["schema_version"], output::SCHEMA_VERSION);
        assert_eq!(
            doc["exit_code"], 2,
            "the field answers for `rbx check`, not for the status process"
        );
    }

    /// Discovery is the whole configuration story: a repo with no
    /// `rbxplace.toml` must not be told to write one.
    #[tokio::test]
    async fn a_tool_with_no_config_file_is_never_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("rbxapikey.toml"), "").expect("write");

        let result = run(cli(dir.path(), true), &global(dir.path(), None)).await;
        assert!(result.is_ok(), "{:#}", result.unwrap_err());
    }
}
