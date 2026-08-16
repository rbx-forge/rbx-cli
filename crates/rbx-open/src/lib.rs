//! Launch Roblox Studio at a specific place via `rbxplace.toml`.
//!
//! Resolution order for env and place:
//! 1. The global `--env <name>` / `--place <name>` flags from `rbx-core`.
//! 2. Positional `<env> <place>` arguments to `rbx open`.
//! 3. Interactive picker (dialoguer) if either is still missing.

use std::process::Command;

use anyhow::{bail, Context, Result};

use clap::Args;

use colored::Colorize;

use dialoguer::Select;

use rbx_core::places::PlacesFile;

use rbx_core::GlobalFlags;

#[derive(Args, Debug)]
pub struct OpenCli {
    /// Environment name (e.g. `prod`, `staging`). Falls back to the global
    /// `--env` flag, then to an interactive picker.
    pub env: Option<String>,

    /// Place name within the env (e.g. `main`, `lobby`). Falls back to the
    /// global `--place` flag, then to an interactive picker (auto-pick when
    /// the env has exactly one place).
    pub place: Option<String>,
}

pub async fn run(cli: OpenCli, global: &GlobalFlags) -> Result<()> {
    if matches!(global.env.as_deref(), Some("all")) {
        bail!("`rbx open` operates on one place at a time. Pass --env <name> instead of `all`.");
    }

    // `--place-id` short-circuits the file entirely. This command builds a
    // `roblox-studio:` URI out of one number and makes no network call, so
    // requiring an rbxplace.toml to supply that number was the whole reason it
    // could not be used outside a configured project.
    if !global.place_id.is_empty() {
        let place_id = global.single_place()?;
        open_place(place_id)?;
        println!("{} Opening place {}", "✓".green(), place_id);
        return Ok(());
    }

    let places = PlacesFile::load(&global.places)?;

    // Env: positional > global --env > interactive picker
    let env_choice = cli.env.or_else(|| global.env.clone());
    let env_name = match env_choice {
        Some(name) => name,
        None => pick_env(&places)?,
    };
    let env = places.get(&env_name)?;

    // Place: positional > global --place > defaults > interactive picker
    let place_choice = cli.place.or_else(|| global.place.clone());
    let (place_name, place_id) = resolve_place(&env_name, env, place_choice)?;

    open_place(place_id)?;
    println!(
        "{} Opening {}/{} (place {})",
        "✓".green(),
        env_name.cyan(),
        place_name.cyan(),
        place_id
    );

    Ok(())
}

fn pick_env(places: &PlacesFile) -> Result<String> {
    let names = places.env_names();
    if names.is_empty() {
        bail!("No environments defined in rbxplace.toml.");
    }
    let selection = Select::new()
        .with_prompt("Select environment")
        .items(&names)
        .interact()?;
    Ok(names[selection].clone())
}

fn resolve_place(
    env_name: &str,
    env: &rbx_core::places::Environment,
    place_choice: Option<String>,
) -> Result<(String, u64)> {
    if let Some(name) = place_choice {
        let id = env.places.get(&name).copied().ok_or_else(|| {
            let mut available: Vec<&str> = env.places.keys().map(|s| s.as_str()).collect();
            available.sort();
            anyhow::anyhow!(
                "Place '{}' not found under [{}.places].\nAvailable: {}",
                name,
                env_name,
                available.join(", ")
            )
        })?;
        return Ok((name, id));
    }

    if env.places.is_empty() {
        bail!(
            "Environment '{}' has no [<env>.places] entries to pick from.",
            env_name
        );
    }

    if env.places.len() == 1 {
        let (k, v) = env
            .places
            .iter()
            .next()
            .expect("len == 1 just checked, entry must exist");
        return Ok((k.clone(), *v));
    }

    let mut names: Vec<&String> = env.places.keys().collect();
    names.sort();
    let display: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let selection = Select::new()
        .with_prompt("Select place")
        .items(&display)
        .interact()?;
    let chosen = names[selection].clone();
    let id = *env
        .places
        .get(&chosen)
        .expect("just selected from this map");
    Ok((chosen, id))
}

fn open_place(place_id: u64) -> Result<()> {
    let uri = format!(
        "roblox-studio:1+task:EditPlace+placeId:{}+universeId:0",
        place_id
    );

    #[cfg(target_os = "windows")]
    {
        // Hand the URI to `explorer`, which delegates protocol-URI launches to
        // the running desktop shell, and WAIT for that command to finish before
        // returning. Both details matter when `rbx` runs under a launcher that
        // tears down its child process tree on exit (e.g. the rokit trampoline):
        // a fire-and-forget `.spawn()` lets the launcher kill the helper process
        // before the hand-off to the shell completes, so Studio never appears.
        // Blocking on `.status()` until `explorer` has handed the URI off — then
        // letting the desktop shell start Studio out-of-tree — is exactly what
        // the proven-working ROpen does (a blocking `explorer "<uri>"` via a
        // shell). raw_arg keeps the URI's `+`/`:` intact inside quotes;
        // CREATE_NO_WINDOW suppresses the cmd console flash.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        Command::new("cmd")
            .raw_arg(format!("/C explorer \"{uri}\""))
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .context("Failed to launch Studio")?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&uri)
            .spawn()
            .context("Failed to launch Studio")?;
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(&uri)
            .spawn()
            .context("Failed to launch Studio")?;
    }

    Ok(())
}
