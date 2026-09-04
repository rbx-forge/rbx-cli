//! The pass itself: resolve, lay down `rbxplace.toml`, then run each domain.
//!
//! Order is load-bearing. `rbxplace.toml` comes first because every domain
//! resolves `--env` against it; if it were written last, each domain would need
//! its own `--universe-id` path and the second import into the same directory
//! would have nothing to layer onto.
//!
//! Within a domain the choice is between `init --from-remote` and `pull`, and
//! it is decided by whether the config file exists, not by whether this is the
//! first import. `init` builds a config from nothing and refuses to overwrite;
//! `pull` layers a new env onto a config that already describes another. Using
//! `init` on an existing file would erase the env imported before it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::Colorize;

use rbx_core::GlobalFlags;

use crate::discover::{self, Hosts, Universe};
use crate::places_file;
use crate::report::{self, Gap};
use crate::{Domain, ImportCli};

/// How one domain's state gets imported.
///
/// Production drives the domain crate's own command ([`Domains`]); tests
/// substitute a recorder. The seam exists because the domain crates inject
/// their API host per-client and gate it behind `cfg(test)`: a deliberate
/// choice documented in `rbx_core::api::base`, and one that leaves no way to
/// point `rbx_shop::run` at a mock server from outside its own crate. Without
/// this trait, everything below (the order, the init-versus-pull decision,
/// the env wiring, what happens when one domain fails) would be untested.
pub(crate) trait DomainImporter {
    async fn import(&self, domain: Domain, global: &GlobalFlags, dir: &Path) -> Result<()>;
}

/// The real thing: each domain imported by the command that already knows how.
pub(crate) struct Domains;

impl DomainImporter for Domains {
    async fn import(&self, domain: Domain, global: &GlobalFlags, dir: &Path) -> Result<()> {
        match domain {
            Domain::Shop => import_shop(global, dir).await,
            Domain::Meta => import_meta(global, dir).await,
            Domain::Config => import_config(global, dir).await,
        }
    }
}

pub async fn run(cli: ImportCli, global: &GlobalFlags, hosts: Hosts) -> Result<()> {
    run_with(cli, global, hosts, &Domains).await
}

pub(crate) async fn run_with(
    cli: ImportCli,
    global: &GlobalFlags,
    hosts: Hosts,
    importer: &impl DomainImporter,
) -> Result<()> {
    // `--env` is the global flag every other command resolves against, so it
    // is read from there rather than duplicated on this subcommand.
    let env = global.env.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "--env <name> is required: it names the section this universe gets in \
             rbxplace.toml, and every domain resolves against it afterwards."
        )
    })?;
    // Checked before anything is resolved or written: this name becomes a
    // section in `rbxplace.toml`, and the reserved ones are read as something
    // else entirely by every later command: `all` as "every env", `owner` and
    // `codegen` as the top-level tables they name.
    if rbx_core::places::is_reserved_env_name(&env) {
        anyhow::bail!(
            "--env {} is reserved: {} already mean something in rbxplace.toml, so a \
             section under that name would not be read back as an env. Pass --env <name>.",
            env,
            rbx_core::places::RESERVED_ENV_NAMES.join(", ")
        );
    }
    let domains = cli.only.clone().unwrap_or_else(|| Domain::ALL.to_vec());
    let dir = &cli.dir;
    std::fs::create_dir_all(dir).with_context(|| format!("Failed to create {}", dir.display()))?;

    let api_key = rbx_core::api::require_api_key(global.api_key.as_deref())?.to_string();
    // Explicit only, deliberately: the place listing below never auto-detects,
    // which `docs/cookie.md` states as a rule of this command. The domains it
    // runs afterwards are ordinary invocations and resolve the usual way.
    let cookie = global.cookie.clone();
    let client = rbx_core::api::build_client();

    // ── resolve, before anything is written ──
    println!(
        "Resolving universe {}...",
        cli.universe_id.to_string().bold()
    );
    let (display_name, owner) =
        discover::fetch_universe(&client, &hosts.cloud, &api_key, cli.universe_id).await?;
    let places =
        discover::fetch_places(&client, &hosts.develop, cookie.as_deref(), cli.universe_id).await?;

    let universe = Universe {
        id: cli.universe_id,
        display_name,
        owner,
        places,
    };

    println!(
        "  {} {}, {} place{}{}",
        "✓".green(),
        universe
            .display_name
            .as_deref()
            .unwrap_or("(unnamed)")
            .bold(),
        universe.places.len(),
        if universe.places.len() == 1 { "" } else { "s" },
        match &universe.owner {
            Some(o) => format!(", owned by {} {}", o.kind, o.id),
            None => String::new(),
        }
    );

    let places_path = places_path_for(global, dir);

    if cli.dry_run {
        print_plan(&cli, &env, &universe, &places_path, &domains);
        return Ok(());
    }

    // ── rbxplace.toml ──
    let written = places_file::write_env(&places_path, &env, &universe)?;
    report_places_write(&places_path, &env, &written);
    warn_existing_configs(&existing_configs(dir, &domains));

    // Every domain below resolves `--env` through this file, so it has to be
    // the one just written even when the caller's `--places` pointed elsewhere.
    let env_global = global_for_env(global, &env, &places_path);

    // ── domains ──
    // `resolve_cookie()`, not `env_global.cookie`: the meta step runs the whole
    // chain, Studio included, so the gap has to follow what it will actually
    // have. See `meta_gaps`.
    let mut gaps = meta_gaps(env_global.resolve_cookie().as_deref(), &domains);

    for domain in &domains {
        let outcome = importer.import(*domain, &env_global, dir).await;
        match outcome {
            Ok(()) => {}
            Err(err) if !cli.strict => {
                // Recorded rather than raised: a key missing one scope should
                // not leave a half-adopted directory behind.
                println!("  {} {} skipped", "!".yellow(), domain);
                // Name the command that would actually run, which is the same
                // question `import` just answered: `pull` layers onto a config
                // that exists, `init --from-remote` creates one that does not.
                // Hardcoding `pull` sent people to the one command that cannot
                // work in the case that just happened: a failed `init` leaves
                // no file, and `pull` refuses without one.
                let remedy = if dir.join(config_file(*domain)).exists() {
                    format!("fix the cause and run `rbx {domain} pull --env {env}`")
                } else {
                    format!(
                        "fix the cause and run `rbx {domain} init --from-remote --env {env}`                          : {} was never written, so there is nothing to pull into yet",
                        config_file(*domain)
                    )
                };
                gaps.push(
                    Gap::new(*domain, format!("{domain} import"), format!("{err:#}"))
                        .with_remedy(remedy),
                );
            }
            Err(err) => return Err(err.context(format!("importing {domain}"))),
        }
    }

    println!();
    report::print(&gaps);
    println!(
        "\n{} Run {} to confirm there is no drift.",
        "→".cyan(),
        check_hint(dir, &env).bold()
    );

    Ok(())
}

/// Where `rbxplace.toml` lives for this run.
///
/// The rule itself lives in `rbx_core::places` so `rbx check --dir` resolves
/// the same default this does; see `resolve_places_path` there.
fn places_path_for(global: &GlobalFlags, dir: &Path) -> PathBuf {
    rbx_core::places::resolve_places_path(&global.places, dir)
}

/// A `GlobalFlags` pointed at the env this import just created.
///
/// Constructed rather than mutated: the caller's flags describe the `import`
/// invocation, and the domains need to be told about an env that did not exist
/// when those flags were parsed.
fn global_for_env(global: &GlobalFlags, env: &str, places: &Path) -> GlobalFlags {
    GlobalFlags {
        api_key: global.api_key.clone(),
        cookie: global.cookie.clone(),
        no_auto_cookie: global.no_auto_cookie,
        auto_cookie: global.auto_cookie,
        env: Some(env.to_string()),
        place: None,
        places: places.to_path_buf(),
        // Cleared on purpose: the domains must resolve through the env, or a
        // second import would write its resources into the first env's section.
        // `place_id` goes with it for the same reason: meta resolves the root
        // place from the env, and an id carried over from the import
        // invocation would pin every domain to one place.
        universe_id: None,
        place_id: Vec::new(),
    }
}

/// What the meta step will leave unset, decided by the cookie it will actually
/// have rather than by the one that was typed.
///
/// Meta is the only domain with fields no API key can reach, so the gap is
/// only worth reporting when meta is in this run at all.
///
/// The resolution matters more than it looks. `import` passes `--cookie`
/// straight through to its own place listing and auto-detects nothing there,
/// but the meta step it launches is an ordinary `rbx meta` invocation and goes
/// through the whole chain in `docs/cookie.md`, Studio auto-detection
/// included. Reading `global.cookie` here therefore told everyone with Studio
/// signed in to re-run with `--cookie` for fields meta had just read
/// correctly, which is advice to redo finished work.
///
/// Hence the resolved cookie as a parameter rather than the flags to resolve
/// it from. The distinction this function got wrong once now lives at its one
/// call site, in the words `resolve_cookie()`, where re-introducing the bug
/// means writing `global.cookie` on a line that plainly reads as the wrong
/// thing. It used to be testable from in here, because the per-tool cookie
/// variable that outlived the merge into one binary was a source reachable
/// from outside `rbx-core`; it was retired in 0.9.0, and the Studio lookup it
/// stood in for has no seam this crate can reach.
fn meta_gaps(cookie: Option<&str>, domains: &[Domain]) -> Vec<Gap> {
    if !domains.contains(&Domain::Meta) {
        return Vec::new();
    }
    report::cookie_only_meta_gaps(cookie.is_some())
}

/// The file a domain layers onto when it is already there.
fn config_file(domain: Domain) -> &'static str {
    match domain {
        Domain::Shop => "rbxshop.toml",
        Domain::Meta => "rbxmeta.toml",
        Domain::Config => "rbxconfig.toml",
    }
}

/// The domain configs this run will pull into rather than create.
fn existing_configs(dir: &Path, domains: &[Domain]) -> Vec<&'static str> {
    domains
        .iter()
        .map(|d| config_file(*d))
        .filter(|file| dir.join(file).exists())
        .collect()
}

/// Say so before a `pull` resolves somebody's unsynced edits away.
///
/// The domain pulls run with `accept_remote` and `yes` set, so every conflict
/// between a local edit and what Roblox holds is settled remote-wards with no
/// prompt. That is the right default for adoption (the whole point is that
/// the live game is authoritative) and the wrong surprise for somebody
/// re-importing an env they have been editing. `--dry-run` cannot preview it
/// either: it returns before the domains run, so it never reaches the
/// resolution.
///
/// Changing the resolution is a larger question than this line. Naming it
/// before it happens is not.
fn warn_existing_configs(existing: &[&str]) {
    if existing.is_empty() {
        return;
    }
    println!(
        "  {} {} already exist{}: this env is layered onto {}, and a local edit that \
         disagrees with Roblox is resolved to the remote value without asking. Commit or \
         sync local edits first if you have any.",
        "!".yellow(),
        existing.join(", "),
        if existing.len() == 1 { "s" } else { "" },
        if existing.len() == 1 { "it" } else { "them" },
    );
}

fn report_places_write(path: &Path, env: &str, written: &places_file::PlacesWrite) {
    if written.env_created {
        println!("  {} [{}] added to {}", "✓".green(), env, path.display());
    } else {
        println!(
            "  {} [{}] already in {}: left as it is",
            "=".dimmed(),
            env,
            path.display()
        );
    }
    if !written.places_added.is_empty() {
        println!(
            "  {} places: {}",
            "✓".green(),
            written.places_added.join(", ")
        );
    }
    if written.owner_written {
        println!("  {} [owner] added", "✓".green());
    }
    if let Some(on_file) = written.existing_universe_id {
        println!(
            "  {} [{}] already points at universe {}: kept. Nothing was retargeted; \
             use a different --env if you meant to add this universe.",
            "!".yellow(),
            env,
            on_file
        );
    }
}

// ---------------------------------------------------------------------------
// The domains
// ---------------------------------------------------------------------------

/// Which command brings a domain's state in.
///
/// The single most load-bearing decision in this file. `init --from-remote`
/// builds a config from nothing and refuses to overwrite one; `pull` layers a
/// new env onto a config that already describes another. Choosing `init` for
/// the second env would erase the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    Init,
    Pull,
}

/// Decided by the file on disk, not by whether this is the first import: a
/// directory can already be under management for reasons that have nothing to
/// do with a previous `import`.
pub(crate) fn entry_for(config: &Path) -> Entry {
    if config.exists() {
        Entry::Pull
    } else {
        Entry::Init
    }
}

/// `shop init --from-remote` on a fresh directory, `shop pull` on one that
/// already has a config.
///
/// `init` writes `[experience]`-free config plus the env's lock section; `pull`
/// discovers the same resources and records the ones that diverge from base as
/// an `[envs.<name>]` overlay. Both leave the lockfile in the state `shop
/// check` expects, which is the only thing this function is really asserting.
async fn import_shop(global: &GlobalFlags, dir: &Path) -> Result<()> {
    use rbx_shop::{ShopCli, ShopCommands};

    let config = dir.join("rbxshop.toml");
    let command = if config.exists() {
        println!(
            "\n{} rbxshop.toml exists: pulling this env into it",
            "→".cyan()
        );
        ShopCommands::Pull {
            dry_run: false,
            accept_remote: true,
            accept_local: false,
            yes: true,
        }
    } else {
        println!("\n{} importing passes, badges and products", "→".cyan());
        ShopCommands::Init {
            from_remote: true,
            universe_id: None,
            gift_label: None,
            dry_run: false,
        }
    };

    rbx_shop::run(ShopCli { command, config }, global).await
}

/// `meta init --from-remote`, then a `pull --accept-remote` to bring the icon
/// and thumbnails down.
///
/// Two calls because `init` writes the config and lockfile but records no media
/// hashes: it never downloads. Without the second call the files exist and
/// `check` is green, but `media.dir` is empty and the first `sync` would upload
/// nothing over the real icon.
async fn import_meta(global: &GlobalFlags, dir: &Path) -> Result<()> {
    use rbx_meta::{MetaCli, MetaCommands};

    let config = dir.join("rbxmeta.toml");
    if entry_for(&config) == Entry::Init {
        println!("\n{} importing universe and place metadata", "→".cyan());
        rbx_meta::run(
            MetaCli {
                command: MetaCommands::Init {
                    from_remote: true,
                    universe_id: None,
                    place_id: None,
                },
                config: config.clone(),
            },
            global,
        )
        .await?;
    } else {
        println!(
            "\n{} rbxmeta.toml exists: pulling this env into it",
            "→".cyan()
        );
    }

    rbx_meta::run(
        MetaCli {
            command: MetaCommands::Pull {
                dry_run: false,
                accept_remote: true,
                accept_local: false,
                yes: true,
            },
            config,
        },
        global,
    )
    .await
}

/// `config pull` handles both cases on its own: it creates `rbxconfig.toml`
/// when absent and replaces only this env's section when present.
async fn import_config(global: &GlobalFlags, dir: &Path) -> Result<()> {
    use rbx_config::{ConfigCli, ConfigCommands};

    println!("\n{} importing the live config", "→".cyan());
    rbx_config::run(
        ConfigCli {
            command: ConfigCommands::Pull { yes: true },
            config: dir.join("rbxconfig.toml"),
            // `InExperienceConfig`, the repository an import has always
            // mirrored. An import is a starting point, and naming another one
            // here would be this command deciding something the user has not
            // said anything about yet.
            repository: None,
        },
        global,
    )
    .await
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

/// The one command that confirms the import landed.
///
/// It used to chain each domain's own check with `&&`. `rbx check` discovers
/// the same tools from the files this import just wrote, runs them in one
/// pass, and aggregates into one exit code (0 clean, 2 drift, 1 error) so
/// the chain is now three round trips and three exit codes for one answer.
///
/// `--dir` is repeated when it is not the default: `rbx check` resolves the
/// config paths, `rbxplace.toml` included, the same way this import wrote
/// them, so the hint has to name the directory the files went into.
fn check_hint(dir: &Path, env: &str) -> String {
    if dir == Path::new(".") {
        format!("rbx check --env {env}")
    } else {
        format!("rbx check --dir {} --env {env}", dir.display())
    }
}

fn print_plan(
    cli: &ImportCli,
    env: &str,
    universe: &Universe,
    places_path: &Path,
    domains: &[Domain],
) {
    println!("\nDry run: nothing written.\n");
    println!("Would write:");
    println!(
        "  {} [{}] universe_id = {}",
        places_path.display(),
        env,
        universe.id
    );
    for place in &universe.places {
        println!("      places.{} = {}", place.key, place.id);
    }
    if let Some(owner) = &universe.owner {
        println!("      [owner] type = \"{}\", id = {}", owner.kind, owner.id);
    }
    for domain in domains {
        let file = match domain {
            Domain::Shop => "rbxshop.toml + rbxshop.lock.toml",
            Domain::Meta => "rbxmeta.toml + rbxmeta.lock.toml",
            Domain::Config => "rbxconfig.toml + rbxconfig.lock.toml",
        };
        println!("  {}/{}", cli.dir.display(), file);
    }
    // The one thing a dry run cannot show by running: it returns before the
    // domains, so the conflict resolution never happens for it to preview.
    // The warning is the preview.
    warn_existing_configs(&existing_configs(&cli.dir, domains));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    // No `#[allow(unsafe_code)]`: the one test that needed it set a cookie
    // env var to reach a source `--cookie` could not spell, and `meta_gaps`
    // now takes the resolved cookie instead of resolving one.
    use std::sync::Mutex;

    use serde_json::json;
    use wiremock::matchers::{method, path as path_matcher};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNIVERSE: u64 = 99887766554;
    const ROOT_PLACE: u64 = 55501;

    /// Stands in for the four domain crates so the orchestration around them
    /// can be exercised: what runs, in what order, with which flags, and what
    /// happens when one fails.
    #[derive(Default)]
    struct Recorder {
        /// One entry per call: the domain, the env it was given, and whether
        /// `rbxplace.toml` already existed when it ran.
        calls: Mutex<Vec<(Domain, String, bool)>>,
        /// Domains that should fail instead of succeeding.
        fail: Vec<Domain>,
    }

    impl Recorder {
        fn failing(domains: &[Domain]) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail: domains.to_vec(),
            }
        }

        fn calls(&self) -> Vec<(Domain, String, bool)> {
            self.calls.lock().unwrap().clone()
        }

        fn domains(&self) -> Vec<Domain> {
            self.calls().into_iter().map(|(d, _, _)| d).collect()
        }
    }

    impl DomainImporter for Recorder {
        async fn import(&self, domain: Domain, global: &GlobalFlags, _dir: &Path) -> Result<()> {
            self.calls.lock().unwrap().push((
                domain,
                global.env.clone().unwrap_or_default(),
                global.places.exists(),
            ));
            if self.fail.contains(&domain) {
                anyhow::bail!("{domain} is unavailable");
            }
            Ok(())
        }
    }

    /// A universe with a root place and one extra place.
    async fn mock_universe(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path_matcher(format!("/cloud/v2/universes/{UNIVERSE}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({ "displayName": "Tower Defence", "group": "groups/456" }),
                ),
            )
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path_matcher(format!("/v1/universes/{UNIVERSE}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "rootPlaceId": ROOT_PLACE })),
            )
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path_matcher(format!("/v1/universes/{UNIVERSE}/places")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "id": 77702, "name": "Lobby" }],
                "nextPageCursor": ""
            })))
            .mount(server)
            .await;
    }

    fn cli(dir: &Path) -> ImportCli {
        ImportCli {
            universe_id: UNIVERSE,
            dir: dir.to_path_buf(),
            dry_run: false,
            strict: false,
            only: None,
        }
    }

    async fn import(
        cli: ImportCli,
        env: &str,
        server: &MockServer,
        importer: &impl DomainImporter,
    ) -> Result<()> {
        run_with(
            cli,
            &flags_for("rbxplace.toml", Some(env)),
            Hosts::with_base_url(server.uri()),
            importer,
        )
        .await
    }

    // ── the decision that keeps a second import from erasing the first ──

    #[test]
    fn a_missing_config_is_imported_and_an_existing_one_is_pulled_into() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("rbxshop.toml");
        assert_eq!(entry_for(&config), Entry::Init);
        std::fs::write(&config, "").unwrap();
        assert_eq!(
            entry_for(&config),
            Entry::Pull,
            "init would refuse to overwrite, or worse, overwrite"
        );
    }

    // ── names rbxplace.toml spells for itself ──

    /// A reserved `--env` has to be refused before the first write: the file
    /// would parse afterwards, but no command would read the section back as
    /// the env that was asked for.
    async fn refuses_reserved_env(env: &str) {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        mock_universe(&server).await;
        let recorder = Recorder::default();

        let err = import(cli(dir.path()), env, &server, &recorder)
            .await
            .unwrap_err();

        assert!(format!("{err:#}").contains("reserved"), "{err:#}");
        assert!(recorder.calls().is_empty());
        assert!(!dir.path().join("rbxplace.toml").exists());
    }

    #[tokio::test]
    async fn env_all_is_refused_rather_than_written_as_a_section() {
        refuses_reserved_env("all").await;
    }

    #[tokio::test]
    async fn env_owner_is_refused_rather_than_written_as_a_section() {
        refuses_reserved_env("owner").await;
    }

    #[tokio::test]
    async fn env_codegen_is_refused_rather_than_written_as_a_section() {
        refuses_reserved_env("codegen").await;
    }

    // ── orchestration ──

    /// Every domain resolves `--env` through `rbxplace.toml`, so it has to be
    /// on disk before the first one runs.
    #[tokio::test]
    async fn the_place_file_exists_before_any_domain_runs() {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        mock_universe(&server).await;
        let recorder = Recorder::default();

        import(cli(dir.path()), "prod", &server, &recorder)
            .await
            .unwrap();

        assert_eq!(recorder.domains(), Domain::ALL.to_vec());
        for (domain, env, places_existed) in recorder.calls() {
            assert_eq!(env, "prod", "{domain} was given the wrong env");
            assert!(places_existed, "{domain} ran before rbxplace.toml existed");
        }

        // And the file it resolves against is the one this import wrote.
        let places = rbx_core::places::PlacesFile::load(&dir.path().join("rbxplace.toml")).unwrap();
        let env = places.get("prod").unwrap();
        assert_eq!(env.universe_id, UNIVERSE);
        assert_eq!(env.places.get("main"), Some(&ROOT_PLACE));
        assert_eq!(env.places.get("lobby"), Some(&77702));
        assert_eq!(places.owner.as_ref().unwrap().id, 456);
    }

    /// A key that covers two domains but not the third should still adopt the
    /// game. Failing the run would leave a half-written directory and no
    /// record of why.
    #[tokio::test]
    async fn one_failing_domain_does_not_stop_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        mock_universe(&server).await;
        let recorder = Recorder::failing(&[Domain::Meta]);

        import(cli(dir.path()), "prod", &server, &recorder)
            .await
            .unwrap();

        assert_eq!(
            recorder.domains(),
            Domain::ALL.to_vec(),
            "config must still be imported after meta failed"
        );
    }

    /// `--strict` is for CI, where a partial import is worse than a red build.
    #[tokio::test]
    async fn strict_fails_on_the_first_failing_domain() {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        mock_universe(&server).await;
        let recorder = Recorder::failing(&[Domain::Shop]);

        let mut c = cli(dir.path());
        c.strict = true;
        let err = import(c, "prod", &server, &recorder).await.unwrap_err();

        assert!(format!("{err:#}").contains("shop"), "{err:#}");
        assert_eq!(
            recorder.domains(),
            vec![Domain::Shop],
            "strict must stop, not carry on"
        );
    }

    #[tokio::test]
    async fn only_restricts_which_domains_run() {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        mock_universe(&server).await;
        let recorder = Recorder::default();

        let mut c = cli(dir.path());
        c.only = Some(vec![Domain::Shop]);
        import(c, "prod", &server, &recorder).await.unwrap();

        assert_eq!(recorder.domains(), vec![Domain::Shop]);
        // rbxplace.toml is not a domain: it is written regardless.
        assert!(dir.path().join("rbxplace.toml").exists());
    }

    /// The whole point of a dry run: it resolves, so it can fail on a bad id
    /// or a missing scope, but it leaves the directory alone.
    #[tokio::test]
    async fn a_dry_run_writes_nothing_and_imports_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        mock_universe(&server).await;
        let recorder = Recorder::default();

        let mut c = cli(dir.path());
        c.dry_run = true;
        import(c, "prod", &server, &recorder).await.unwrap();

        assert!(recorder.calls().is_empty());
        assert!(!dir.path().join("rbxplace.toml").exists());
    }

    /// The second import is the one that decides whether the command is
    /// usable: a new env alongside the first, and every domain run again for
    /// it, without the first env being touched.
    #[tokio::test]
    async fn a_second_import_adds_an_env_beside_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        mock_universe(&server).await;

        import(cli(dir.path()), "prod", &server, &Recorder::default())
            .await
            .unwrap();
        let after_first = std::fs::read_to_string(dir.path().join("rbxplace.toml")).unwrap();

        let recorder = Recorder::default();
        import(cli(dir.path()), "staging", &server, &recorder)
            .await
            .unwrap();

        assert_eq!(recorder.domains(), Domain::ALL.to_vec());
        let after_second = std::fs::read_to_string(dir.path().join("rbxplace.toml")).unwrap();
        assert!(
            after_second.starts_with(&after_first),
            "the first env must survive verbatim:\n{after_second}"
        );
        let places = rbx_core::places::PlacesFile::load(&dir.path().join("rbxplace.toml")).unwrap();
        assert_eq!(places.env_names(), vec!["prod", "staging"]);
    }

    /// An import that cannot even resolve the universe must not have created
    /// anything: the failure has to happen before the first write.
    #[tokio::test]
    async fn a_refused_universe_leaves_the_directory_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher(format!("/cloud/v2/universes/{UNIVERSE}")))
            .respond_with(ResponseTemplate::new(403).set_body_string("no scope"))
            .mount(&server)
            .await;
        let recorder = Recorder::default();

        assert!(import(cli(dir.path()), "prod", &server, &recorder)
            .await
            .is_err());

        assert!(recorder.calls().is_empty());
        assert!(!dir.path().join("rbxplace.toml").exists());
    }

    use super::*;

    fn flags(places: &str) -> GlobalFlags {
        flags_for(places, None)
    }

    fn flags_for(places: &str, env: Option<&str>) -> GlobalFlags {
        GlobalFlags {
            api_key: Some("test-key".into()),
            cookie: None,
            no_auto_cookie: true,
            auto_cookie: false,
            env: env.map(str::to_string),
            place: None,
            places: places.into(),
            universe_id: None,
            place_id: Vec::new(),
        }
    }

    /// The default `--places` means "next to everything else this import
    /// writes", not "in the process's working directory".
    #[test]
    fn the_default_places_path_follows_dir() {
        let global = flags("rbxplace.toml");
        assert_eq!(
            places_path_for(&global, Path::new("/tmp/game")),
            PathBuf::from("/tmp/game/rbxplace.toml")
        );
    }

    /// An explicit `--places` is a shared env file that may well live outside
    /// the import directory, so it wins.
    #[test]
    fn an_explicit_places_path_wins_over_dir() {
        let global = flags("/shared/envs.toml");
        assert_eq!(
            places_path_for(&global, Path::new("/tmp/game")),
            PathBuf::from("/shared/envs.toml")
        );
    }

    /// The domains must resolve through the env, or a second import would
    /// write its resources into the first env's section.
    #[test]
    fn the_domain_flags_name_the_env_and_drop_the_universe_override() {
        let mut global = flags("rbxplace.toml");
        global.universe_id = Some(999);
        let derived = global_for_env(&global, "staging", Path::new("/tmp/rbxplace.toml"));
        assert_eq!(derived.env.as_deref(), Some("staging"));
        assert_eq!(derived.universe_id, None);
        assert_eq!(derived.places, PathBuf::from("/tmp/rbxplace.toml"));
        assert_eq!(derived.api_key.as_deref(), Some("test-key"));
    }

    /// The cookie-only fields are reported as missing when the meta step will
    /// have no cookie, and not when it will have one, whichever source it came
    /// from.
    ///
    /// Total now that the cookie arrives resolved: both inputs are covered,
    /// and no source is privileged over another because the function cannot
    /// tell them apart. What this can no longer assert is that the *caller*
    /// resolves before asking, which the retired per-tool cookie variable used
    /// to make reachable from here. That moved to the call site in `run_with`,
    /// where it is a visible `resolve_cookie()` rather than a habit buried in
    /// a helper.
    #[test]
    fn the_meta_gap_follows_the_cookie_the_meta_step_will_resolve() {
        assert_eq!(
            meta_gaps(None, &Domain::ALL).len(),
            1,
            "nothing to resolve, so the cookie-only fields really are unreadable"
        );
        assert!(
            meta_gaps(Some("from-anywhere"), &Domain::ALL).is_empty(),
            "the meta step resolves one, so the fields are readable and \
             `re-run with --cookie` is advice to redo finished work"
        );
    }

    /// No meta, no report about meta: the fields belong to a domain this run
    /// never touched.
    #[test]
    fn a_run_without_meta_reports_no_cookie_gap_at_all() {
        assert!(meta_gaps(None, &[Domain::Shop]).is_empty());
    }

    /// The warning names the configs this run will pull into, and only those:
    /// a file for a domain `--only` excluded is not a file this run touches,
    /// and a warning about it would be noise the next one teaches people to
    /// skip.
    #[test]
    fn only_the_domain_configs_this_run_would_pull_into_are_named() {
        let dir = tempfile::tempdir().unwrap();
        assert!(existing_configs(dir.path(), &Domain::ALL).is_empty());

        std::fs::write(dir.path().join("rbxmeta.toml"), "").unwrap();
        std::fs::write(dir.path().join("rbxconfig.toml"), "").unwrap();

        assert_eq!(
            existing_configs(dir.path(), &Domain::ALL),
            vec!["rbxmeta.toml", "rbxconfig.toml"]
        );
        assert!(existing_configs(dir.path(), &[Domain::Shop]).is_empty());
    }

    /// One command, one exit code. The chain of per-tool checks it replaced
    /// was three round trips and three exit codes for one answer.
    #[test]
    fn the_check_hint_is_the_one_aggregated_check() {
        assert_eq!(check_hint(Path::new("."), "prod"), "rbx check --env prod");
    }

    /// An import that wrote somewhere else has to hand back a command that
    /// reads from there, or the hint checks the wrong directory.
    #[test]
    fn the_check_hint_carries_dir_when_it_is_not_the_default() {
        assert_eq!(
            check_hint(Path::new("game"), "prod"),
            "rbx check --dir game --env prod"
        );
    }
}
