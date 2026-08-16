//! One state-gathering pass per tool.
//!
//! Every probe here answers the same question — does what this repo declares
//! still match what it recorded — and returns [`ToolReport`]s rather than
//! printing. Nothing here calls another command.
//!
//! The reason not to compose is stdout, not exit codes: every per-tool check
//! now returns `Err(Drift)` on drift, but they all print as they decide, and
//! under `--json` stdout belongs to the document — a probe that shelled into
//! them would emit something `jq` cannot read. A row also carries more than a
//! status: an env label, a summary and per-change details, none of which
//! survive a process exit code.
//!
//! So each probe rebuilds the comparison from the same public pieces the
//! command uses — the renderers and plan builders themselves, not a
//! re-description of them. No check body is called from here; see
//! `docs/check.md`.

use std::path::Path;

use anyhow::Result;
use rbx_core::env::DEFAULT_ENV;
use rbx_core::generated::{compare, GeneratedFile, Verdict};
use rbx_core::places::PlacesFile;
use rbx_core::GlobalFlags;

use crate::report::{Outcome, ToolReport};

/// Compare rendered files against disk and describe the result.
///
/// The generated-file checks are a byte comparison, so the verdict is the same
/// one `CheckReport` reaches — computed here only because `CheckReport::finish`
/// prints as it decides, and stdout is not always ours to write to.
fn compare_generated(
    files: &[GeneratedFile],
    stale: &[std::path::PathBuf],
) -> (Outcome, String, Vec<String>) {
    let mut details = Vec::new();

    for file in files {
        match compare(&file.content, &file.path) {
            Ok(verdict) if !verdict.is_drift() => {}
            Ok(Verdict::Missing) => details.push(format!("{}: missing", file.path.display())),
            Ok(Verdict::Formatting) => details.push(format!(
                "{}: whitespace only — something else is rewriting this file \
                 (a formatter, or an editor trimming on save). Regenerating will not stop it.",
                file.path.display()
            )),
            Ok(Verdict::Differs(diff)) => details.push(format!(
                "{}: differs at line {}",
                file.path.display(),
                diff.line
            )),
            Ok(Verdict::Match) => {}
            // `compare` cannot return this today — `Stale` is recorded by the
            // caller that knows a file is left over, never by a byte
            // comparison. It is mapped rather than ignored because
            // `Stale::is_drift()` is true, so a future `compare` that started
            // returning it would otherwise have its drift swallowed here by an
            // arm that reads as "nothing to report". Same wording as the
            // `stale` loop below: one verdict, one meaning.
            Ok(Verdict::Stale) => details.push(stale_detail(&file.path)),
            Err(err) => return (Outcome::Error, one_line(&err), Vec::new()),
        }
    }
    // A leftover module from a deleted env is drift too: it still looks
    // generated, and game code can still require it.
    for path in stale {
        details.push(stale_detail(path));
    }

    if details.is_empty() {
        let count = files.len();
        return (
            Outcome::Clean,
            format!(
                "{count} generated file{} up to date",
                if count == 1 { "" } else { "s" }
            ),
            Vec::new(),
        );
    }
    let drifted = details.len();
    (
        Outcome::Drift,
        format!(
            "{drifted} generated file{} no longer match{}",
            if drifted == 1 { "" } else { "s" },
            if drifted == 1 { "es" } else { "" }
        ),
        details,
    )
}

/// What a file the current inputs no longer produce reads as in the report.
fn stale_detail(path: &Path) -> String {
    format!("{}: not produced by the current inputs", path.display())
}

/// Collapse an error to a single line: a summary column that wraps is not a
/// summary. The full text is still printed by whatever produced it.
fn one_line(err: &anyhow::Error) -> String {
    let text = format!("{err}");
    match text.split('\n').next() {
        Some(first) if !first.trim().is_empty() => first.trim().to_string(),
        _ => text.trim().to_string(),
    }
}

/// The env label to show for a target. `None` means the tool's standalone
/// block, which every tool records under the `default` section name.
fn env_label(env: Option<&str>) -> String {
    env.unwrap_or(DEFAULT_ENV).to_string()
}

// ---------------------------------------------------------------------------
// env / rbxplace.toml
// ---------------------------------------------------------------------------

/// The comparison `rbx env gen-module --check` runs.
///
/// Offline and credential-free by construction: it re-renders from
/// `rbxplace.toml` and compares bytes.
pub fn env(places_path: &Path) -> Vec<ToolReport> {
    let places = match PlacesFile::load(places_path) {
        Ok(p) => p,
        Err(err) => {
            return vec![ToolReport::new(
                "env",
                "gen-module",
                Outcome::Error,
                one_line(&err),
            )]
        }
    };

    // Not every repo generates the module, and `gen-module` bails when there
    // is nowhere to write. "Not configured" is not a failing check.
    if places
        .codegen
        .as_ref()
        .and_then(|c| c.output.as_ref())
        .is_none()
    {
        return vec![ToolReport::new(
            "env",
            "gen-module",
            Outcome::Skipped,
            "no [codegen].output in rbxplace.toml",
        )];
    }

    let out_path = match rbx_env::resolve_env_module_out(&places, None, places_path) {
        Ok(p) => p,
        Err(err) => {
            return vec![ToolReport::new(
                "env",
                "gen-module",
                Outcome::Error,
                one_line(&err),
            )]
        }
    };
    let file = match rbx_env::render_env_module(&places, &out_path) {
        Ok(f) => f,
        Err(err) => {
            return vec![ToolReport::new(
                "env",
                "gen-module",
                Outcome::Error,
                one_line(&err),
            )]
        }
    };

    let (outcome, summary, mut details) = compare_generated(std::slice::from_ref(&file), &[]);

    // The one cause where regenerating is the wrong move: an ignored key is
    // one this binary did not apply, so the render just compared against is
    // itself the misreading. `gen-module --check` carries the same caveat.
    if outcome == Outcome::Drift && !places.unknown.is_empty() {
        details.push(format!(
            "{} key{} in {} ignored — if one of them was meant to change what is generated, \
             the committed file may be the correct side and regenerating would bake the \
             misreading in. Upgrade rbx, or fix the spelling, first.",
            places.unknown.len(),
            if places.unknown.len() == 1 {
                " is"
            } else {
                "s are"
            },
            places_path.display(),
        ));
    }

    vec![ToolReport::new("env", "gen-module", outcome, summary).details(details)]
}

// ---------------------------------------------------------------------------
// shop / rbxshop.toml
// ---------------------------------------------------------------------------

pub fn shop(config_path: &Path, envs: &[Option<String>]) -> Vec<ToolReport> {
    use rbx_shop::config::Config;
    use rbx_shop::diff::{build_sync_plan, Action};
    use rbx_shop::lockfile::{EnvLock, Lockfile, LOCKFILE_NAME};

    let mut reports = Vec::new();
    let config_dir = config_path.parent().unwrap_or(Path::new("."));

    let config = match Config::load_merged(config_path) {
        Ok(c) => c,
        Err(err) => {
            return vec![ToolReport::new(
                "shop",
                "lockfile",
                Outcome::Error,
                one_line(&err),
            )]
        }
    };

    let lockfile_path = config_dir.join(LOCKFILE_NAME);
    let lockfile = if lockfile_path.exists() {
        match Lockfile::load(&lockfile_path) {
            Ok(l) => l,
            Err(err) => {
                return vec![ToolReport::new(
                    "shop",
                    "lockfile",
                    Outcome::Error,
                    one_line(&err),
                )]
            }
        }
    } else {
        Lockfile::default()
    };

    for env in envs {
        let label = env_label(env.as_deref());
        let resolved = match config.resolve_env(env.as_deref()) {
            Ok(r) => r,
            Err(err) => {
                reports.push(
                    ToolReport::new("shop", "lockfile", Outcome::Error, one_line(&err)).env(&label),
                );
                continue;
            }
        };

        if let Err(err) = Config::validate_icon_paths(&resolved, config_dir) {
            reports.push(
                ToolReport::new("shop", "lockfile", Outcome::Error, one_line(&err)).env(&label),
            );
            continue;
        }

        let default_lock = EnvLock::default();
        let env_lock = lockfile.env(&label).unwrap_or(&default_lock);

        let plan = match build_sync_plan(&resolved, env_lock, config_dir) {
            Ok(p) => p,
            Err(err) => {
                reports.push(
                    ToolReport::new("shop", "lockfile", Outcome::Error, one_line(&err)).env(&label),
                );
                continue;
            }
        };

        let mut creates = 0;
        let mut updates = 0;
        for action in plan.passes.iter().chain(&plan.badges).chain(&plan.products) {
            match &action.action {
                Action::Create => creates += 1,
                Action::Update { .. } => updates += 1,
                Action::Skip => {}
            }
        }

        let report = if creates == 0 && updates == 0 {
            ToolReport::new("shop", "lockfile", Outcome::Clean, "everything in sync")
        } else {
            ToolReport::new(
                "shop",
                "lockfile",
                Outcome::Drift,
                format!("{creates} to create, {updates} to update — run `rbx shop sync`"),
            )
        };
        reports.push(report.env(&label).details(plan.warnings.clone()));
    }

    reports.push(shop_codegen(&config, &lockfile, config_dir, &lockfile_path));
    reports
}

/// The comparison `rbx shop codegen --check` runs.
fn shop_codegen(
    config: &rbx_shop::config::Config,
    lockfile: &rbx_shop::lockfile::Lockfile,
    config_dir: &Path,
    lockfile_path: &Path,
) -> ToolReport {
    if config.codegen.output.is_none() {
        return ToolReport::new(
            "shop",
            "codegen",
            Outcome::Skipped,
            "no [codegen].output in rbxshop.toml",
        );
    }
    // Codegen reads asset ids out of the lockfile, so without one there is
    // nothing it could have generated — and nothing to have drifted.
    if !lockfile_path.exists() {
        return ToolReport::new(
            "shop",
            "codegen",
            Outcome::Skipped,
            "no lockfile yet — run `rbx shop sync` once",
        );
    }

    match rbx_shop::codegen::plan(config, lockfile, config_dir) {
        Err(err) => ToolReport::new("shop", "codegen", Outcome::Error, one_line(&err)),
        Ok(None) => ToolReport::new(
            "shop",
            "codegen",
            Outcome::Skipped,
            "no envs in the lockfile yet — run `rbx shop sync` once",
        ),
        Ok(Some(plan)) => {
            let (outcome, summary, details) = compare_generated(&plan.files, &plan.stale);
            ToolReport::new("shop", "codegen", outcome, summary).details(details)
        }
    }
}

// ---------------------------------------------------------------------------
// meta / rbxmeta.toml
// ---------------------------------------------------------------------------

pub fn meta(config_path: &Path, envs: &[Option<String>]) -> Vec<ToolReport> {
    use rbx_meta::config::Config;
    use rbx_meta::diff::build_plan;
    use rbx_meta::lockfile::{Lockfile, LOCKFILE_NAME};

    let config_dir = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let config = match Config::load(config_path) {
        Ok(c) => c,
        Err(err) => {
            return vec![ToolReport::new(
                "meta",
                "lockfile",
                Outcome::Error,
                one_line(&err),
            )]
        }
    };

    let lockfile = match Lockfile::load(&config_dir.join(LOCKFILE_NAME)) {
        Ok(l) => l,
        Err(err) => {
            return vec![ToolReport::new(
                "meta",
                "lockfile",
                Outcome::Error,
                one_line(&err),
            )]
        }
    };

    let mut reports = Vec::new();
    for env in envs {
        let label = env_label(env.as_deref());
        let (game, media) = config.resolve_env(Some(&label));

        if let Err(err) = Config::validate_invariants(&game) {
            reports.push(
                ToolReport::new("meta", "lockfile", Outcome::Error, one_line(&err)).env(&label),
            );
            continue;
        }
        if let Err(err) = Config::validate_media_paths(&media, &config_dir) {
            reports.push(
                ToolReport::new("meta", "lockfile", Outcome::Error, one_line(&err)).env(&label),
            );
            continue;
        }

        let env_lock = lockfile.env_view(&label);
        let plan = match build_plan(&game, &media, &env_lock.game, &env_lock.media, &config_dir) {
            Ok(p) => p,
            Err(err) => {
                reports.push(
                    ToolReport::new("meta", "lockfile", Outcome::Error, one_line(&err)).env(&label),
                );
                continue;
            }
        };

        let report = if plan.is_empty() {
            ToolReport::new("meta", "lockfile", Outcome::Clean, "everything in sync")
        } else {
            ToolReport::new(
                "meta",
                "lockfile",
                Outcome::Drift,
                format!(
                    "{} pending change{} — run `rbx meta sync`",
                    meta_change_count(&plan),
                    if meta_change_count(&plan) == 1 {
                        ""
                    } else {
                        "s"
                    }
                ),
            )
            .details(meta_change_descriptions(&plan))
        };
        reports.push(report.env(&label));
    }
    reports
}

fn meta_change_count(plan: &rbx_meta::diff::SyncPlan) -> usize {
    meta_change_descriptions(plan).len()
}

/// The plan's own descriptions, which the tool already writes for humans.
/// Reusing them keeps one wording for one change.
fn meta_change_descriptions(plan: &rbx_meta::diff::SyncPlan) -> Vec<String> {
    use rbx_meta::diff::IconPlan;

    let mut out = Vec::new();
    for patch in [&plan.universe_patch].into_iter().flatten() {
        out.extend(patch.descriptions.iter().cloned());
    }
    for patch in [&plan.place_patch].into_iter().flatten() {
        out.extend(patch.descriptions.iter().cloned());
    }
    for patch in [&plan.place_legacy_patch].into_iter().flatten() {
        out.extend(patch.descriptions.iter().cloned());
    }
    for patch in [&plan.universe_legacy_patch].into_iter().flatten() {
        out.extend(patch.descriptions.iter().cloned());
    }
    if let Some(v) = plan.visibility_change {
        out.push(format!("visibility: → {v:?}"));
    }
    if let Some(b) = plan.beta_mode_change {
        out.push(format!("beta_mode: → {b}"));
    }
    if let IconPlan::Upload { path, .. } = &plan.icon {
        out.push(format!("icon: upload {}", path.display()));
    }
    for id in &plan.thumbnails.deletes {
        out.push(format!("thumbnail: delete image {id}"));
    }
    for upload in &plan.thumbnails.uploads {
        out.push(format!("thumbnail: upload {}", upload.path.display()));
    }
    if plan.thumbnails.needs_reorder {
        out.push("thumbnails: reorder to match config order".to_string());
    }
    out
}

// ---------------------------------------------------------------------------
// config / rbxconfig.toml
// ---------------------------------------------------------------------------

/// The one remote check: local entries against the live config on Roblox.
pub async fn config(
    config_path: &Path,
    envs: &[Option<String>],
    global: &GlobalFlags,
    offline: bool,
) -> Vec<ToolReport> {
    use rbx_config::api::RbxConfigClient;
    use rbx_config::config::ConfigsFile;
    use rbx_config::diff::Diff;

    if offline {
        return vec![ToolReport::new(
            "config",
            "live",
            Outcome::Skipped,
            "--offline: comparing against Roblox needs an API key",
        )];
    }

    let local = match ConfigsFile::load(config_path) {
        Ok(c) => c,
        Err(err) => {
            return vec![ToolReport::new(
                "config",
                "live",
                Outcome::Error,
                one_line(&err),
            )]
        }
    };

    // Built up front so the key is read once, but not *demanded* until an env
    // has something to compare — see the gates below.
    let client = global.api_key.clone().map(RbxConfigClient::new);

    let mut reports = Vec::new();
    for env in envs {
        // rbxconfig.toml is env-keyed with no standalone fallback, so there is
        // no meaningful target without a named env.
        let Some(name) = env.as_deref() else {
            reports.push(ToolReport::new(
                "config",
                "live",
                Outcome::Skipped,
                "no --env: rbxconfig.toml is env-keyed, pass --env <name> or --env all",
            ));
            continue;
        };

        if !local.environments.contains_key(name) {
            reports.push(
                ToolReport::new(
                    "config",
                    "live",
                    Outcome::Skipped,
                    format!("no [{name}] section in rbxconfig.toml"),
                )
                .env(name),
            );
            continue;
        }

        // Only now is a key actually needed. Asking for one before the gates
        // above turned a run that would have skipped into an exit 1, which
        // pushes a keyless pre-commit hook onto `--offline` for nothing.
        let Some(client) = client.as_ref() else {
            reports.push(
                ToolReport::new(
                    "config",
                    "live",
                    Outcome::Error,
                    "no API key: pass --api-key, set RBX_API_KEY, or run with --offline",
                )
                .env(name),
            );
            continue;
        };

        let universe_id = match resolve_universe(global, name) {
            Ok(id) => id,
            Err(err) => {
                reports.push(
                    ToolReport::new("config", "live", Outcome::Error, one_line(&err)).env(name),
                );
                continue;
            }
        };

        let entries = match local.entries_as_json(name) {
            Ok(e) => e,
            Err(err) => {
                reports.push(
                    ToolReport::new("config", "live", Outcome::Error, one_line(&err)).env(name),
                );
                continue;
            }
        };

        let snapshot = match client.get_config(universe_id).await {
            Ok(s) => s,
            Err(err) => {
                reports.push(
                    ToolReport::new("config", "live", Outcome::Error, one_line(&err)).env(name),
                );
                continue;
            }
        };

        let diff = Diff::compute(&entries, &snapshot.entries);
        let report = if diff.is_empty() {
            ToolReport::new("config", "live", Outcome::Clean, "local matches live")
        } else {
            ToolReport::new(
                "config",
                "live",
                Outcome::Drift,
                format!(
                    "{} entr{} differ{} — run `rbx config sync --env {name}`",
                    diff.changes.len(),
                    if diff.changes.len() == 1 { "y" } else { "ies" },
                    if diff.changes.len() == 1 { "s" } else { "" },
                ),
            )
            .details(
                diff.changes
                    .iter()
                    .map(|c| c.key.clone())
                    .collect::<Vec<_>>(),
            )
        };
        reports.push(report.env(name));
    }
    reports
}

fn resolve_universe(global: &GlobalFlags, env: &str) -> Result<u64> {
    if let Some(id) = global.universe_id {
        return Ok(id);
    }
    rbx_core::places::resolve_universe_id(&global.places, env)
}

// ---------------------------------------------------------------------------
// apikey / rbxapikey.toml
// ---------------------------------------------------------------------------

/// Reported, not run.
///
/// `rbx apikey status` classifies key health (expiry, orphan lockfile entries,
/// missing secrets) inside a private module and returns `Ok(())` whatever it
/// finds, which the other per-tool checks no longer do. Wiring it in would mean
/// either widening `rbx-apikey` internals or re-implementing that classifier
/// here — a second copy of the rules, which is exactly the failure mode this
/// tool exists to catch. Left as a named gap instead. See `docs/check.md`.
pub fn apikey() -> Vec<ToolReport> {
    vec![ToolReport::new(
        "apikey",
        "status",
        Outcome::Skipped,
        "not yet wired — run `rbx apikey status` (see docs/check.md)",
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generated(dir: &Path, name: &str, on_disk: &str, rendered: &str) -> GeneratedFile {
        std::fs::write(dir.join(name), on_disk).expect("write");
        GeneratedFile::new(dir.join(name), rendered)
    }

    #[test]
    fn matching_generated_files_are_clean() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = generated(dir.path(), "a.luau", "return 1\n", "return 1\n");

        let (outcome, summary, details) = compare_generated(&[file], &[]);

        assert_eq!(outcome, Outcome::Clean);
        assert_eq!(summary, "1 generated file up to date");
        assert!(details.is_empty());
    }

    #[test]
    fn an_edited_generated_file_is_drift_and_names_the_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = generated(dir.path(), "a.luau", "return 2\n", "return 1\n");

        let (outcome, _, details) = compare_generated(&[file], &[]);

        assert_eq!(outcome, Outcome::Drift);
        assert!(details[0].contains("differs at line 1"), "{details:?}");
    }

    #[test]
    fn a_missing_generated_file_is_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = GeneratedFile::new(dir.path().join("absent.luau"), "return 1\n");

        let (outcome, _, details) = compare_generated(&[file], &[]);

        assert_eq!(outcome, Outcome::Drift);
        assert!(details[0].contains("missing"), "{details:?}");
    }

    /// A formatter rewriting the generated path is drift that regenerating
    /// will not fix, so the detail says so instead of repeating the standard
    /// advice.
    #[test]
    fn a_whitespace_only_difference_says_regenerating_will_not_help() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = generated(dir.path(), "a.luau", "return   1\n", "return 1\n");

        let (outcome, _, details) = compare_generated(&[file], &[]);

        assert_eq!(outcome, Outcome::Drift);
        assert!(details[0].contains("whitespace only"), "{details:?}");
        assert!(details[0].contains("will not stop it"), "{details:?}");
    }

    /// A module left behind by a deleted env still looks generated and can
    /// still be required, so it is drift on its own.
    #[test]
    fn a_stale_file_is_drift_even_when_every_rendered_file_matches() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = generated(dir.path(), "a.luau", "return 1\n", "return 1\n");

        let (outcome, _, details) =
            compare_generated(&[file], &[dir.path().join("removed_env.luau")]);

        assert_eq!(outcome, Outcome::Drift);
        assert!(
            details[0].contains("not produced by the current inputs"),
            "{details:?}"
        );
    }

    #[test]
    fn a_summary_is_collapsed_to_its_first_line() {
        let err = anyhow::anyhow!("first line\n  second line\n  third");
        assert_eq!(one_line(&err), "first line");
    }

    #[test]
    fn the_standalone_block_is_labelled_default() {
        assert_eq!(env_label(None), "default");
        assert_eq!(env_label(Some("prod")), "prod");
    }

    #[test]
    fn apikey_is_reported_as_a_named_gap_rather_than_silently_omitted() {
        let reports = apikey();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].outcome, Outcome::Skipped);
        assert!(reports[0].summary.contains("rbx apikey status"));
    }

    // -----------------------------------------------------------------------
    // config/live — when the key is demanded
    // -----------------------------------------------------------------------

    /// A repo with an env-keyed config and no credential anywhere.
    fn keyless(dir: &Path) -> GlobalFlags {
        std::fs::write(dir.join("rbxconfig.toml"), "[dev]\n[dev.entries]\n").expect("write");
        GlobalFlags {
            api_key: None,
            cookie: None,
            no_auto_cookie: true,
            auto_cookie: false,
            env: None,
            place: None,
            places: dir.join("rbxplace.toml"),
            universe_id: None,
            place_id: Vec::new(),
        }
    }

    /// The check would have skipped whether or not a key was present, so
    /// asking for one turns a no-op into exit 1 — and pushes a keyless
    /// pre-commit hook onto `--offline` for a check that never runs.
    #[tokio::test]
    async fn no_env_is_skipped_rather_than_failing_for_a_key_it_would_not_have_used() {
        let dir = tempfile::tempdir().expect("tempdir");
        let global = keyless(dir.path());

        let reports = config(&dir.path().join("rbxconfig.toml"), &[None], &global, false).await;

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].outcome, Outcome::Skipped);
        assert!(reports[0].summary.contains("no --env"), "{reports:?}");
    }

    /// An env the config never declares is nothing to compare either, so the
    /// missing key is still not the thing to report.
    #[tokio::test]
    async fn an_env_with_no_section_is_skipped_rather_than_failing_for_a_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let global = keyless(dir.path());

        let reports = config(
            &dir.path().join("rbxconfig.toml"),
            &[Some("staging".to_string())],
            &global,
            false,
        )
        .await;

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].outcome, Outcome::Skipped);
        assert!(
            reports[0].summary.contains("section in rbxconfig.toml"),
            "{reports:?}"
        );
    }

    /// The other half of the same rule: once there is something to compare,
    /// a missing key is a failure and not a quiet skip.
    #[tokio::test]
    async fn a_declared_env_with_no_key_is_still_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let global = keyless(dir.path());

        let reports = config(
            &dir.path().join("rbxconfig.toml"),
            &[Some("dev".to_string())],
            &global,
            false,
        )
        .await;

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].outcome, Outcome::Error);
        assert!(reports[0].summary.contains("no API key"), "{reports:?}");
        assert_eq!(reports[0].env.as_deref(), Some("dev"));
    }
}
