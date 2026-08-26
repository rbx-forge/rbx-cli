mod completions;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

use rbx_core::generated::{Drift, DRIFT_EXIT_CODE};
use rbx_core::GlobalFlags;

#[derive(Parser)]
#[command(
    name = "rbx",
    version,
    about = "Roblox developer toolkit",
    long_about = "Unified Roblox Open Cloud CLI. Each subcommand maps to a domain \
                  (bootstrap, API keys, place files, metadata, config, monetization, Studio launch). \
                  Per-subcommand help: `rbx <subcommand> --help`."
)]
struct Cli {
    #[command(flatten)]
    global: GlobalFlags,

    #[command(subcommand)]
    tool: Tool,
}

// Order chosen to read top-to-bottom like a user journey: bootstrap →
// authentication → routine operations on the universe → local Studio
// utilities → live operations → meta CLI features. Not alphabetical, and clap
// renders subcommands in declaration order, so this list *is* the help output.
//
// The live-operations commands come last and say so in their first word. They
// used to be a second binary, `rbx-ops`, which made the boundary visible in the
// command name itself. That stopped being tenable: Rokit resolves one artifact
// per repository, so only one of the two could ever be installed through it.
// The boundary was never enforced by the split anyway: a key is bound to its
// scopes at creation, and a deploy key cannot ban somebody no matter which
// binary calls it. What the split bought was a signal, and the signal now lives
// in the `Live:` prefix and in the ordering here.
#[derive(Subcommand)]
enum Tool {
    /// Bootstrap groups, universes, and places
    Init(rbx_init::InitCli),
    /// Adopt an existing universe: write every config and lockfile from it
    Import(rbx_import::ImportCli),
    /// Read rbxplace.toml: list envs, print an id, generate a module
    Env(rbx_env::EnvCli),
    /// API key creation and management
    Apikey(rbx_apikey::ApikeyCli),
    // Directly after `apikey`, because it is the command that answers for what
    // `apikey` set up: which credential is live, whether it is still valid, and
    // whether it covers the tools configured here. "Why doesn't it work" gets
    // asked in the first hour, so it is listed where somebody scanning the help
    // for it will find it.
    /// Diagnose credentials, key validity, and scope coverage
    Doctor(rbx_doctor::DoctorCli),
    /// Run every configured tool's check in one pass (CI contract)
    Check(rbx_check::CheckCli),
    // Straight after `check`, because it is the same engine: one gathering
    // pass, two contracts. `check` answers "may this build continue" and exits
    // 2 on drift; `status` answers "where does this project stand" and always
    // exits 0.
    /// Where every environment stands, grouped by env, always exit 0
    Status(rbx_check::StatusCli),
    /// Place files: upload, download, promote between envs
    Place(rbx_place::PlaceCli),
    /// Universe and place metadata
    Meta(rbx_meta::MetaCli),
    /// Game configuration flags
    Config(rbx_config::ConfigCli),
    // Straight after `config`, and for the same reason `doctor` sits after
    // `apikey`: these are the two halves of "what this universe is configured
    // with". `config` carries the values a repository can hold, `secret` the
    // ones it must never hold, and somebody looking for where to put an API
    // token will scan past `config` first.
    /// Secrets HttpService:GetSecret reads, written encrypted
    Secret(rbx_secret::SecretCli),
    // Last of the three, because it is the question the other two do not ask:
    // `config` carries the values a universe may hold, `secret` the ones it
    // must never hold, and `rtbf` the ones it must be able to delete. It sits
    // here rather than beside `data` because it is a declaration in a
    // committed file, not an operation on a live store.
    /// Data store keys holding a user's data, for right-to-be-forgotten
    Rtbf(rbx_rtbf::RtbfCli),
    /// Game passes, badges, and developer products
    Shop(rbx_shop::ShopCli),
    /// Launch Roblox Studio at a specific place
    Open(rbx_open::OpenCli),
    /// Download Roblox assets by id (public or Open Cloud)
    Download(rbx_download::DownloadCli),

    /// Live: servers currently up, and how the stopped ones ended
    Servers(rbx_servers::ServersCli),
    /// Live: query the experience's own analytics metrics
    Analytics(rbx_analytics::AnalyticsCli),
    /// Live: inspect and change player restrictions
    Ban(rbx_ban::BanCli),
    /// Live: forecast and launch a rolling server restart
    Restart(rbx_restart::RestartCli),
    /// Live: read and overwrite a data store entry
    Data(rbx_data::DataCli),
    /// Live: read and write memory store sorted map items
    Memorystore(rbx_memorystore::MemoryStoreCli),
    /// Live: push a MessagingService message to every running server
    ///
    /// Named `message`, not `publish`. "Publish" already means a deploy
    /// everywhere else in this tool (`place upload --published` publishes a
    /// place, `config sync` publishes a config, `place rollback` republishes
    /// live) and somebody reaching for "how do I publish my place from the
    /// CLI" would find this first and get far enough in to be confused.
    Message(rbx_message::MessageCli),
    /// Live: launch and steer ad campaigns
    ///
    /// Last of the live commands because it is the odd one: it spends real
    /// money and reads no results back. `/ads-management/v1` has no reporting
    /// endpoint, so campaign performance is read in Ads Manager.
    Ads(rbx_ads::AdsCli),
    /// Live: send a raw authenticated request to an Open Cloud path
    ///
    /// Hidden from the command list on purpose. This is the tool for working
    /// out what an undocumented endpoint returns while writing a typed client
    /// for it. Fully supported: `rbx probe --help` works, and it is documented
    /// in docs/ops/probe.md.
    #[command(hide = true)]
    Probe(rbx_probe::ProbeCli),
    /// Generate shell completions
    ///
    /// Without `--output`, prints the completion script to stdout (typically
    /// redirected via `>` or piped into your shell's profile loader).
    ///
    /// Usage examples:
    /// ```text
    /// bash:        rbx completions bash       -o ~/.local/share/bash-completion/completions/rbx
    /// zsh:         rbx completions zsh        -o "${fpath\[1\]}/_rbx"
    /// fish:        rbx completions fish       -o ~/.config/fish/completions/rbx.fish
    /// powershell:  rbx completions powershell -o $PROFILE
    /// ```
    ///
    /// The script completes `--env` and `--place` with the names in the
    /// `rbxplace.toml` of whatever directory you are in, by calling
    /// `rbx env list --names` and `rbx env list --place-names` when you press
    /// TAB. Outside a project, or with a file that does not parse, both
    /// complete to nothing and say nothing. Pass `--no-dynamic` to leave that
    /// out and get a script that never spawns a process.
    Completions {
        /// Target shell.
        shell: Shell,
        /// Write the completion script to this file instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Emit the static script only, with no callback into the binary.
        ///
        /// The values from `rbxplace.toml` are lost (`--env` and `--place`
        /// fall back to completing file names) in exchange for a completion
        /// that starts no subprocess.
        #[arg(long)]
        no_dynamic: bool,
    },
}

/// Exit codes: `0` success, `1` failure, `2` a `--check` run found drift.
///
/// The drift code is separate so a CI job can tell "regenerate and commit"
/// from "something broke". It sits on 2 rather than 1 so that every other
/// command's exit status stays exactly what it was.
#[tokio::main]
async fn main() -> ExitCode {
    match dispatch().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err:?}");
            if err.chain().any(|cause| cause.is::<Drift>()) {
                ExitCode::from(DRIFT_EXIT_CODE)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

async fn dispatch() -> Result<()> {
    let cli = Cli::parse();
    match cli.tool {
        Tool::Init(c) => rbx_init::run(c, &cli.global).await,
        Tool::Import(c) => rbx_import::run(c, &cli.global).await,
        Tool::Env(c) => rbx_env::run(c, &cli.global).await,
        Tool::Apikey(c) => rbx_apikey::run(c, &cli.global).await,
        Tool::Doctor(c) => rbx_doctor::run(c, &cli.global).await,
        Tool::Check(c) => rbx_check::run(c, &cli.global).await,
        Tool::Status(c) => rbx_check::status(c, &cli.global).await,
        Tool::Place(c) => rbx_place::run(c, &cli.global).await,
        Tool::Meta(c) => rbx_meta::run(c, &cli.global).await,
        Tool::Config(c) => rbx_config::run(c, &cli.global).await,
        Tool::Secret(c) => rbx_secret::run(c, &cli.global).await,
        Tool::Rtbf(c) => rbx_rtbf::run(c, &cli.global).await,
        Tool::Shop(c) => rbx_shop::run(c, &cli.global).await,
        Tool::Open(c) => rbx_open::run(c, &cli.global).await,
        Tool::Download(c) => rbx_download::run(c, &cli.global).await,

        Tool::Servers(c) => rbx_servers::run(c, &cli.global).await,
        Tool::Analytics(c) => rbx_analytics::run(c, &cli.global).await,
        Tool::Ban(c) => rbx_ban::run(c, &cli.global).await,
        Tool::Restart(c) => rbx_restart::run(c, &cli.global).await,
        Tool::Data(c) => rbx_data::run(c, &cli.global).await,
        Tool::Memorystore(c) => rbx_memorystore::run(c, &cli.global).await,
        Tool::Message(c) => rbx_message::run(c, &cli.global).await,
        Tool::Ads(c) => rbx_ads::run(c, &cli.global).await,
        Tool::Probe(c) => rbx_probe::run(c, &cli.global).await,
        Tool::Completions {
            shell,
            output,
            no_dynamic,
        } => {
            let mut cmd = Cli::command();
            let script = completions::script(shell, &mut cmd, !no_dynamic);
            match output {
                Some(path) => {
                    completions::write(&path, &script)?;
                    eprintln!("wrote completions to {}", path.display());
                }
                None => print!("{script}"),
            }
            Ok(())
        }
    }
}
