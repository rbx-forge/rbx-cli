mod completions;

// Lives in the binary crate because `Cli` is private to it, and the point is
// to parse without running: an integration test could only reach the built
// executable, and a documented command that parses would then execute.
#[cfg(test)]
mod docs_drift;

// The other direction: every flag the tree has must appear in a page. Same
// crate, same reason.
#[cfg(test)]
mod docs_coverage;

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
    about = "Declarative deployment for Roblox",
    long_about = "Declarative deployment for Roblox. Describe an experience in TOML files you \
                  commit, and rbx makes Roblox match them. Per-subcommand help: \
                  `rbx <subcommand> --help`."
)]
struct Cli {
    #[command(flatten)]
    global: GlobalFlags,

    #[command(subcommand)]
    tool: Tool,
}

// Order chosen to read top-to-bottom like a user journey: bootstrap →
// routine operations on the universe → meta CLI features. Not alphabetical,
// and clap renders subcommands in declaration order, so this list *is* the
// help output.
//
// Every command here reconciles state declared in a committed file, or reads
// the file itself. Acting on a running game, managing API keys, opening Studio
// and downloading assets are separate tools: see the post-mortem in TODO.md
// for why they left.
#[derive(Subcommand)]
enum Tool {
    /// Bootstrap groups, universes, and places
    Init(rbx_init::InitCli),
    /// Adopt an existing universe: write every config and lockfile from it
    Import(rbx_import::ImportCli),
    /// Read rbxplace.toml: list envs, print an id, generate a module
    Env(rbx_env::EnvCli),
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
    // Straight after `config`, because it is the question `config` does not
    // ask: `config` carries the values a universe may hold, `rtbf` the ones it
    // must be able to delete. It is a declaration in a committed file, not an
    // operation on a live store.
    /// Data store keys holding a user's data, for right-to-be-forgotten
    Rtbf(rbx_rtbf::RtbfCli),
    /// Game passes, badges, and developer products
    Shop(rbx_shop::ShopCli),
    /// Send a raw authenticated request to an Open Cloud path
    ///
    /// Hidden from the command list on purpose. This is the tool for working
    /// out what an undocumented endpoint returns while writing a typed client
    /// for it. Fully supported: `rbx probe --help` works, and it is documented
    /// in <https://rbx-forge.github.io/rbx-deploy/probe/>.
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
    /// powershell:  rbx completions powershell -o (Join-Path (Split-Path $PROFILE) rbx-completions.ps1)
    ///              then dot-source that file from $PROFILE
    /// ```
    ///
    /// Never `-o $PROFILE`: `-o` replaces the file, profile included.
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
        Tool::Check(c) => rbx_check::run(c, &cli.global).await,
        Tool::Status(c) => rbx_check::status(c, &cli.global).await,
        Tool::Place(c) => rbx_place::run(c, &cli.global).await,
        Tool::Meta(c) => rbx_meta::run(c, &cli.global).await,
        Tool::Config(c) => rbx_config::run(c, &cli.global).await,
        Tool::Rtbf(c) => rbx_rtbf::run(c, &cli.global).await,
        Tool::Shop(c) => rbx_shop::run(c, &cli.global).await,
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
