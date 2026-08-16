//! Launch Roblox Studio at a specific place via `rbxplace.toml`.
//!
//! Resolution order, most direct first:
//! 1. `--place-id <id>` : opened as given, no file and no network.
//! 2. `--universe-id <id>` : the universe's places are listed from Roblox, and
//!    a single-place universe opens without a prompt.
//! 3. `rbxplace.toml`, addressed by the global `--env` / `--place` flags, then
//!    by the positional `<env> <place>` arguments, then by a picker.

use std::process::Command;

use anyhow::{bail, Context, Result};

use clap::Args;

use colored::Colorize;

use dialoguer::Select;

use rbx_core::places::PlacesFile;

use rbx_core::universe::{UniversePlace, DEVELOP_HOST};

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

    // `--universe-id` is a global flag, so it already parsed here before this
    // branch existed — and was then silently dropped on the way to
    // `rbxplace.toml`, whose absence produced an error advising per-subcommand
    // flags that `open` does not have. Accepting a flag and ignoring it is
    // worse than rejecting it.
    //
    // The listing needs no credential (see `rbx_core::universe`), so this path
    // works in a bare directory, which is the point: adopting somebody else's
    // universe id is exactly when there is no project to read.
    if let Some(universe_id) = global.universe_id {
        // The explicit `--cookie` only, never `resolve_cookie()`. That helper
        // can go looking for a local Studio session and ask for consent to use
        // it, and asking to borrow a full-account credential for a call that
        // answers anonymously is a bad trade to offer.
        let places = rbx_core::universe::list_places(
            &rbx_core::api::build_client(),
            &rbx_core::api::ApiBase::new(DEVELOP_HOST),
            global.cookie.as_deref(),
            universe_id,
        )
        .await?;

        let chosen = pick_universe_place(universe_id, &places)?;
        open_place(chosen.id)?;
        println!(
            "{} Opening {} (place {})",
            "✓".green(),
            label(chosen).cyan(),
            chosen.id
        );
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

/// What to call a place in output. Roblox omits the name on some places, and
/// an entry rendered as an empty string is one the reader cannot pick from.
fn label(place: &UniversePlace) -> String {
    let name = place.name.trim();
    match (name.is_empty(), place.is_root) {
        (true, true) => format!("place {} (start place)", place.id),
        (true, false) => format!("place {}", place.id),
        (false, true) => format!("{name} (start place)"),
        (false, false) => name.to_string(),
    }
}

/// Choose among a universe's places: none is an error, one opens silently,
/// several ask.
fn pick_universe_place(universe_id: u64, places: &[UniversePlace]) -> Result<&UniversePlace> {
    match places {
        // `list_places` seeds the root, so an empty result means the universe
        // reported nothing at all rather than "no extra places".
        [] => bail!(
            "Universe {universe_id} reported no places. Check the id: a place id passed where \
             a universe id belongs is the usual mistake."
        ),
        [only] => Ok(only),
        several => {
            // The id goes on every row. Two places in one universe are allowed
            // to share a display name, and picking blind between two identical
            // rows is how the wrong one gets opened.
            let items: Vec<String> = several
                .iter()
                .map(|p| format!("{}  ({})", label(p), p.id))
                .collect();
            let selection = Select::new()
                .with_prompt(format!("Select a place in universe {universe_id}"))
                .default(0)
                .items(&items)
                .interact()?;
            Ok(&several[selection])
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn place(id: u64, name: &str, is_root: bool) -> UniversePlace {
        UniversePlace {
            id,
            name: name.to_string(),
            is_root,
        }
    }

    /// The whole point of the universe path: one place must not stop to ask.
    /// A prompt here would also hang a non-interactive run.
    #[test]
    fn a_single_place_universe_is_chosen_without_a_prompt() {
        let places = vec![place(111, "Start", true)];
        let chosen = pick_universe_place(7, &places).expect("one place is unambiguous");
        assert_eq!(chosen.id, 111);
    }

    /// `list_places` seeds the root, so an empty vec means the universe
    /// answered with nothing — most often a place id passed as a universe id.
    #[test]
    fn no_places_is_an_error_that_names_the_likely_mistake() {
        let error = pick_universe_place(7, &[]).expect_err("nothing to open");
        assert!(error.to_string().contains("place id"), "got: {error}");
    }

    #[test]
    fn a_nameless_place_is_still_identifiable() {
        assert_eq!(label(&place(111, "", false)), "place 111");
        assert_eq!(label(&place(111, "   ", true)), "place 111 (start place)");
    }

    #[test]
    fn the_start_place_is_marked_so_it_can_be_told_apart() {
        assert_eq!(label(&place(111, "Start", true)), "Start (start place)");
        assert_eq!(label(&place(222, "Lobby", false)), "Lobby");
    }
}
