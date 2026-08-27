//! Which tools this repo actually uses.
//!
//! Nothing to configure: a tool is checked when its config file is there. The
//! alternative (a `[check]` block listing what to run) is one more thing to
//! keep in sync with reality, and the reality is already on disk.

use std::path::{Path, PathBuf};

use rbx_core::places::{EnvSelector, PlacesFile, ALL_ENVS};

/// A config file the CLI knows how to check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// `rbxplace.toml`: env definitions, and the generated env module.
    Env,
    Shop,
    Meta,
    Config,
    /// `rbxrtbf.toml`: the right-to-be-forgotten deletion templates.
    Rtbf,
    Apikey,
}

impl Tool {
    /// Every tool, in the order `rbx check` runs them: local-only work first,
    /// so a repo that is going to fail on a byte comparison fails before
    /// spending a round trip on a remote diff.
    pub const ALL: [Tool; 6] = [
        Tool::Env,
        Tool::Shop,
        Tool::Meta,
        Tool::Rtbf,
        Tool::Config,
        Tool::Apikey,
    ];

    /// The file whose presence enables this tool.
    ///
    /// The per-tool `--config` flags can point elsewhere, but `rbx check` is
    /// the zero-configuration entry point and deliberately only knows the
    /// default names. A repo that renames them runs the per-tool checks.
    pub fn config_file(self) -> &'static str {
        match self {
            Tool::Env => "rbxplace.toml",
            Tool::Shop => "rbxshop.toml",
            Tool::Meta => "rbxmeta.toml",
            Tool::Config => "rbxconfig.toml",
            Tool::Rtbf => rbx_rtbf::config::FILE,
            Tool::Apikey => "rbxapikey.toml",
        }
    }
}

/// A tool found in the working directory.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub tool: Tool,
    pub path: PathBuf,
}

/// Find every known config file under `dir`.
///
/// `rbxplace.toml` is looked up at `places_path` instead, because `--places`
/// can move it and every other tool resolves envs through it.
pub fn discover(dir: &Path, places_path: &Path) -> Vec<Discovered> {
    Tool::ALL
        .into_iter()
        .filter_map(|tool| {
            let path = match tool {
                Tool::Env => places_path.to_path_buf(),
                _ => dir.join(tool.config_file()),
            };
            path.is_file().then_some(Discovered { tool, path })
        })
        .collect()
}

/// The envs to check, as names.
///
/// `--env all` expands through `rbxplace.toml`; no `--env` yields `None`, which
/// each tool reads as "your standalone config block", the same fallback the
/// per-tool commands already use.
///
/// A named env is checked against the file **when there is one**. Both halves
/// of that matter. `check` has to work in a project with no `rbxplace.toml` at
/// all, so an absent file still takes the name at its word. But a file that
/// exists and does not declare the name means a typo, and taking that at its
/// word is worse than refusing: `--env nosuchenv` used to answer
/// `! shop/lockfile [nosuchenv]  1 to create, 0 to update: run \`rbx shop
/// sync\``, which is a confident, actionable-looking row about an env that does
/// not exist, naming a `sync` that cannot work. A typo in a CI `--env` is
/// exactly how that gets read as real drift.
pub fn target_envs(env: Option<&str>, places_path: &Path) -> anyhow::Result<Vec<Option<String>>> {
    let Some(value) = env else {
        return Ok(vec![None]);
    };

    // Only a file that *loads* can expand a selector or spot a typo, and one
    // that does not load is `tools::env`'s row to report, not an abort here:
    // `rbx check` and `rbx status` promise an overview of every tool, and a
    // malformed `rbxplace.toml` used to appear as one failing row beside shop's
    // and meta's findings. Propagating the load error instead collapsed the
    // whole run to a raw TOML message and no rows at all.
    //
    // `load` already fails on an absent file, so this covers the
    // no-places-file project too, and a named env still stands there.
    let Ok(places) = PlacesFile::load(places_path) else {
        if value == ALL_ENVS {
            // `all` is the one selector that cannot stand on its own: there is
            // nothing to expand it against.
            anyhow::bail!(
                "--env all needs {} to expand. Point at it with --places, or name one env.",
                places_path.display()
            );
        }
        return Ok(vec![Some(value.to_string())]);
    };

    // `selector` resolves through `get`, so its "Available: ..." list is what
    // makes a typo obvious, and a group is expanded here rather than handed on:
    // every row this run produces is keyed by a real env.
    let names = match places.selector(value)? {
        EnvSelector::One(name) => vec![name],
        EnvSelector::Every => places.env_names(),
        EnvSelector::Group { members, .. } => members,
    };
    if names.is_empty() {
        anyhow::bail!(
            "--env all found no envs in {}. Add at least one [<env>] section with universe_id.",
            places_path.display()
        );
    }
    Ok(names.into_iter().map(Some).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), "").expect("write");
    }

    #[test]
    fn an_empty_directory_discovers_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let found = discover(dir.path(), &dir.path().join("rbxplace.toml"));
        assert!(found.is_empty());
    }

    #[test]
    fn only_the_files_that_exist_are_discovered() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(dir.path(), "rbxshop.toml");
        touch(dir.path(), "rbxmeta.toml");

        let found = discover(dir.path(), &dir.path().join("rbxplace.toml"));

        assert_eq!(
            found.iter().map(|d| d.tool).collect::<Vec<_>>(),
            vec![Tool::Shop, Tool::Meta]
        );
    }

    /// The env tool follows `--places`, so a repo keeping `rbxplace.toml`
    /// somewhere other than the working directory is still discovered.
    #[test]
    fn the_env_tool_is_found_through_the_places_flag() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("config");
        std::fs::create_dir(&nested).expect("mkdir");
        touch(&nested, "rbxplace.toml");

        let found = discover(dir.path(), &nested.join("rbxplace.toml"));

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].tool, Tool::Env);
        assert_eq!(found[0].path, nested.join("rbxplace.toml"));
    }

    #[test]
    fn a_directory_is_not_a_config_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("rbxshop.toml")).expect("mkdir");

        assert!(discover(dir.path(), &dir.path().join("rbxplace.toml")).is_empty());
    }

    /// The rule `Tool::ALL` states, asserted as a rule.
    ///
    /// It used to pin the literal file-name sequence, which meant inserting a
    /// tool in the wrong place was fixed by editing the expectation: the test
    /// then asserted an order its own name contradicted, and waved the next
    /// insertion through. What the rule actually says is that no tool needing
    /// the network runs before one that does not.
    #[test]
    fn tools_run_local_work_before_remote_work() {
        /// What a tool costs to run, which is what the ordering rule is about.
        #[derive(PartialEq, Eq)]
        enum Cost {
            /// Decidable from the files on disk.
            Local,
            /// Needs at least one request.
            Remote,
            /// Produces a fixed row and does no work at all, so it is exempt:
            /// `tools::apikey` is a documented gap that reports itself, and it
            /// sits last for that reason rather than for this rule's.
            None,
        }

        fn cost(tool: Tool) -> Cost {
            match tool {
                Tool::Env | Tool::Shop | Tool::Meta => Cost::Local,
                // `rtbf/templates` validates the file with no network at all,
                // which is why it belongs on the local side of the line even
                // though `rtbf/live` is remote.
                Tool::Rtbf => Cost::Local,
                // `config/live` is a Configs API read per env and nothing else.
                Tool::Config => Cost::Remote,
                Tool::Apikey => Cost::None,
            }
        }

        if let Some(boundary) = Tool::ALL.iter().position(|t| cost(*t) == Cost::Remote) {
            let late_local: Vec<&str> = Tool::ALL[boundary..]
                .iter()
                .filter(|t| cost(**t) == Cost::Local)
                .map(|t| t.config_file())
                .collect();
            assert!(
                late_local.is_empty(),
                "these do local-only work after a tool that needs the network: {late_local:?}"
            );
        }

        // And the whole set is covered, so a tool cannot escape the rule by
        // being left out of `cost`.
        assert_eq!(Tool::ALL.len(), 6);
    }

    /// A tool added to the enum and forgotten in `ALL` is a check that never
    /// runs and never says it did not, which is the one failure `rbx check`
    /// cannot have. Named per tool rather than counted, so the assertion says
    /// which one went missing.
    #[test]
    fn rtbf_is_both_listed_and_named() {
        assert!(Tool::ALL.contains(&Tool::Rtbf));
        assert_eq!(Tool::Rtbf.config_file(), rbx_rtbf::config::FILE);
    }

    /// The file name comes from the crate that owns it rather than a literal
    /// here, so renaming it there cannot leave `rbx check` looking for the old
    /// one. Pinned anyway, because the name is a user-facing contract.
    #[test]
    fn the_rtbf_file_is_the_one_the_crate_writes() {
        assert_eq!(rbx_rtbf::config::FILE, "rbxrtbf.toml");
    }

    #[test]
    fn the_rtbf_file_is_discovered_on_its_own() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(dir.path(), "rbxrtbf.toml");

        let found = discover(dir.path(), &dir.path().join("rbxplace.toml"));

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].tool, Tool::Rtbf);
    }

    #[test]
    fn no_env_flag_means_the_standalone_block() {
        let dir = tempfile::tempdir().expect("tempdir");
        let envs = target_envs(None, &dir.path().join("rbxplace.toml")).expect("targets");
        assert_eq!(envs, vec![None]);
    }

    /// A project with no `rbxplace.toml` is a project `check` still has useful
    /// things to say about, so with no file there is nothing to check the name
    /// against and it stands.
    #[test]
    fn a_named_env_is_taken_at_its_word_when_there_is_no_places_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let envs = target_envs(Some("prod"), &dir.path().join("absent.toml")).expect("targets");
        assert_eq!(envs, vec![Some("prod".to_string())]);
    }

    /// The typo check must not cost the overview. A `rbxplace.toml` that does
    /// not parse is one failing row from `tools::env` beside every other
    /// tool's findings; propagating the load error from here collapsed the
    /// whole run to a raw TOML message and no rows at all.
    #[test]
    fn a_places_file_that_does_not_load_is_a_row_elsewhere_not_an_abort_here() {
        let dir = tempfile::tempdir().expect("tempdir");
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(&places, "[dev]\nuniverse_id = \"not a number\"\n").expect("write");

        assert_eq!(
            target_envs(Some("dev"), &places).expect("a broken file is not this function's error"),
            vec![Some("dev".to_string())]
        );
        // And a name it could never have validated either way still stands.
        assert_eq!(
            target_envs(Some("whatever"), &places).expect("still not this function's error"),
            vec![Some("whatever".to_string())]
        );
    }

    /// The interesting half. A typo used to produce a row that read like real
    /// drift on an env nobody declared, which in CI is indistinguishable from
    /// the genuine article.
    #[test]
    fn an_env_the_places_file_does_not_declare_is_refused_with_the_alternatives() {
        let dir = tempfile::tempdir().expect("tempdir");
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(
            &places,
            "[dev]\nuniverse_id = 1\n\n[prod]\nuniverse_id = 2\n",
        )
        .expect("write");

        let err = format!(
            "{:#}",
            target_envs(Some("prd"), &places).expect_err("a typo is not an env")
        );
        assert!(err.contains("'prd' not found"), "{err}");
        assert!(err.contains("dev, prod"), "{err}");

        // And a name it does declare still passes through untouched.
        assert_eq!(
            target_envs(Some("prod"), &places).expect("targets"),
            vec![Some("prod".to_string())]
        );
    }

    #[test]
    fn env_all_expands_through_the_places_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(
            &places,
            "[dev]\nuniverse_id = 1\n\n[prod]\nuniverse_id = 2\n",
        )
        .expect("write");

        let mut envs = target_envs(Some("all"), &places).expect("targets");
        envs.sort();

        assert_eq!(
            envs,
            vec![Some("dev".to_string()), Some("prod".to_string())]
        );
    }

    #[test]
    fn env_all_against_a_places_file_with_no_envs_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(&places, "").expect("write");

        let err = target_envs(Some("all"), &places).expect_err("no envs to expand");
        assert!(format!("{err:#}").contains("no envs"), "{err:#}");
    }
}
