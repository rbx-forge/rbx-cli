//! Launch Roblox Studio at a specific place via `rbxplace.toml`.
//!
//! Resolution order, most direct first:
//! 0. `--new` : Roblox's baseplate template, the way Studio's own "New
//!    Experience" button opens it. No id to supply, and nothing created.
//! 1. `--place-id <id>` : opened as given, no file and no network.
//! 2. `--universe-id <id>` : the universe's places are listed from Roblox, and
//!    a single-place universe opens without a prompt.
//! 3. `rbxplace.toml`, addressed by the global `--env` / `--place` flags, then
//!    by the positional `<env> <place>` arguments, then by a picker.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use clap::Args;

use colored::Colorize;

use dialoguer::Select;

use rbx_core::api::ApiBase;

use rbx_core::places::PlacesFile;

use rbx_core::templates::{list_templates, StudioTemplate, DEFAULT_TEMPLATE_PLACE_ID, GAMES_HOST};

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

    /// Open a new, empty place, like Studio's "New Experience" button.
    ///
    /// Lists Roblox's templates and asks which one; `--baseplate` takes the
    /// stock one without asking, `--template <id>` names one outright.
    ///
    /// Needs no project, no id and no credential. Studio fetches the
    /// template's content and then unbinds the session from it, so the place
    /// exists only in Studio until you save it to Roblox: nothing is created
    /// on Roblox by this flag. To create an experience outright instead, see
    /// `rbx init create-universe`.
    #[arg(long = "new")]
    pub new_place: bool,

    /// Template place id for `--new`, instead of picking from the list. Any
    /// place Roblox lets Studio open works, including one of your own.
    #[arg(long, requires = "new_place")]
    pub template: Option<u64>,

    /// A `.rbxl` / `.rbxlx` to open from disk instead of a published place.
    /// The same as passing the path as the first argument; this spelling is
    /// for paths that do not carry the extension.
    #[arg(long)]
    pub file: Option<PathBuf>,

    /// Take Roblox's baseplate without opening the picker, the way Studio's
    /// own "New Experience" button does. Skips the template listing, so it
    /// needs neither a terminal nor a network call.
    #[arg(long, requires = "new_place", conflicts_with = "template")]
    pub baseplate: bool,
}

pub async fn run(cli: OpenCli, global: &GlobalFlags) -> Result<()> {
    if matches!(global.env.as_deref(), Some("all")) {
        bail!("`rbx open` operates on one place at a time. Pass --env <name> instead of `all`.");
    }

    // A path is the most direct target there is: no id, no config, no network.
    // Recognised by extension rather than by "does this file exist", so an env
    // named `prod` can never be captured by a stray file of the same name.
    if let Some(path) = place_file_target(cli.file.as_deref(), cli.env.as_deref()) {
        if cli.new_place
            || !global.place_id.is_empty()
            || global.universe_id.is_some()
            || cli.place.is_some()
        {
            bail!(
                "A place file is opened on its own: `rbx open {}` names the file to edit, so \
                 there is nothing left for another target to say.",
                path.display()
            );
        }
        open_file(&path)?;
        println!(
            "{} Opening {}",
            "\u{2713}".green(),
            path.display().to_string().cyan()
        );
        return Ok(());
    }

    // `--new` comes first because it is the only path that needs nothing at
    // all: no file, no id, no network, no credential. Pairing it with a target
    // is a contradiction rather than a preference, so it is refused instead of
    // silently winning.
    if cli.new_place {
        let decided = new_target(
            cli.template,
            cli.baseplate,
            !global.place_id.is_empty(),
            global.universe_id.is_some(),
            cli.env.as_deref(),
            cli.place.as_deref(),
        )?;

        let (place_id, name) = match decided {
            Some(place_id) => (place_id, None),
            None => {
                let templates =
                    list_templates(&rbx_core::api::build_client(), &ApiBase::new(GAMES_HOST))
                        .await?;
                let chosen = pick_template(&templates)?;
                (chosen.place_id, Some(chosen.name.clone()))
            }
        };

        open_place(place_id)?;
        match name {
            Some(name) => println!(
                "{} Opening a new {} place (template {})",
                "\u{2713}".green(),
                name.cyan(),
                place_id
            ),
            None => println!(
                "{} Opening a new place from template {}",
                "\u{2713}".green(),
                place_id.to_string().cyan()
            ),
        }
        // Said out loud because the window looks exactly like an opened place.
        // Studio loads the template's content and then sets the session's place
        // id back to 0, so there is nothing on Roblox for a save to go to yet.
        println!("  Not on Roblox yet: the first save there creates the experience.");
        return Ok(());
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
            &ApiBase::new(DEVELOP_HOST),
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

/// Choose among Studio's stock templates.
///
/// Roblox returns them newest-first, so the list is shown as `list_templates`
/// ordered it: baseplate first, then the rest as Roblox sees them. The id goes
/// on every row for the same reason it does in the universe picker — it is the
/// only thing guaranteed to tell two rows apart.
fn pick_template(templates: &[StudioTemplate]) -> Result<&StudioTemplate> {
    if templates.is_empty() {
        bail!(
            "Roblox returned no Studio templates. Pass `--baseplate` for the stock one, or \
             `--template <place-id>` for a specific one."
        );
    }

    // A picker with nowhere to draw itself hangs a script forever. `--baseplate`
    // is the escape hatch, so the error names it rather than choosing for them.
    if !rbx_core::output::is_interactive() {
        bail!(
            "`--new` picks a template interactively, and this is not a terminal. Pass \
             `--baseplate`, or `--template <place-id>`."
        );
    }

    let items: Vec<String> = templates
        .iter()
        .map(|t| format!("{}  ({})", t.name, t.place_id))
        .collect();
    let selection = Select::new()
        .with_prompt("Select a template")
        .default(0)
        .items(&items)
        .interact()?;
    Ok(&templates[selection])
}

/// The place file this invocation names, if it names one.
///
/// `--file` is explicit. The positional is taken only when it carries a place
/// file's extension: `rbx open prod` must stay an environment even in a folder
/// that happens to hold a file called `prod`, so existence on disk is the
/// wrong question to ask.
fn place_file_target(file: Option<&Path>, env: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = file {
        return Some(path.to_path_buf());
    }
    let candidate = env?;
    let lowered = candidate.to_ascii_lowercase();
    (lowered.ends_with(".rbxl") || lowered.ends_with(".rbxlx")).then(|| PathBuf::from(candidate))
}

/// What `--new` opens, and which company it refuses to keep.
///
/// `--new` names no place, so every other way of naming one contradicts it.
/// Picking a winner silently is the failure this crate has already been bitten
/// by once: a flag that parses, is dropped, and leaves the user reading an
/// error about something else entirely.
fn new_target(
    template: Option<u64>,
    baseplate: bool,
    has_place_id: bool,
    has_universe_id: bool,
    env: Option<&str>,
    place: Option<&str>,
) -> Result<Option<u64>> {
    let conflict = if has_place_id {
        Some("--place-id")
    } else if has_universe_id {
        Some("--universe-id")
    } else if env.is_some() || place.is_some() {
        Some("an env/place argument")
    } else {
        None
    };

    if let Some(other) = conflict {
        bail!(
            "`--new` opens a brand-new place, so it cannot be combined with {other}. \
             Drop one: `rbx open --new` for an empty place, or the target on its own \
             to open a place that already exists."
        );
    }

    if let Some(id) = template {
        return Ok(Some(id));
    }
    if baseplate {
        return Ok(Some(DEFAULT_TEMPLATE_PLACE_ID));
    }
    // Nothing decided it, so the list gets to. `None` means "ask", not "use the
    // baseplate": defaulting quietly here would make `--baseplate` a flag that
    // changes nothing.
    Ok(None)
}

fn open_place(place_id: u64) -> Result<()> {
    launch(&format!(
        "roblox-studio:1+task:EditPlace+placeId:{place_id}+universeId:0"
    ))
}

/// Open a `.rbxl` / `.rbxlx` from disk.
///
/// The path is handed to the desktop's opener rather than wrapped in a
/// `roblox-studio:` URI, and deliberately: that URI is parsed by splitting on
/// `+` and `:`, which a Windows path (`C:\\...`) and any filename containing a
/// `+` would both break. The file association reaches the same place — Studio
/// logs `createAndShowIDEDoc with task EditFile` either way.
///
/// Absolute, because the opener does not inherit our working directory.
fn open_file(path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("No such place file: {}", path.display());
    }
    let absolute = path
        .canonicalize()
        .with_context(|| format!("Failed to resolve {}", path.display()))?;

    // Windows canonicalize() returns a `\\?\C:\...` extended-length path, which
    // the shell hands to Studio verbatim and Studio does not open. The prefix
    // is stripped because it is an API detail, not part of the path.
    let mut target = absolute.to_string_lossy().into_owned();
    if let Some(stripped) = target.strip_prefix(r#"\\?\"#) {
        target = stripped.to_string();
    }
    launch(&target)
}

/// Hand one target — a `roblox-studio:` URI or a path — to the desktop.
fn launch(target: &str) -> Result<()> {
    let uri = target;

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
        // Studio has no Linux build, so a Linux `rbx` is either running under
        // WSL — where the target has to cross to the Windows host — or has
        // nothing to launch at all. `xdg-open` is tried first anyway: a desktop
        // Linux user with Studio under Wine or Proton has an association for
        // it, and that is theirs to keep.
        if is_wsl() || !spawned("xdg-open", &[uri]) {
            open_on_windows_host(uri)?;
        }
    }

    Ok(())
}

/// Whether this Linux is really a Windows machine wearing a kernel.
///
/// Checked before `xdg-open` rather than after: under WSL the command often
/// exists and "succeeds" without a desktop to hand anything to, so a failure
/// to spawn is not a reliable signal on its own.
#[cfg(target_os = "linux")]
fn is_wsl() -> bool {
    if std::env::var_os("WSL_DISTRO_NAME").is_some()
        || std::env::var_os("WSL_INTEROP").is_some()
        || std::env::var_os("WSLENV").is_some()
    {
        return true;
    }
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|release| {
            let release = release.to_ascii_lowercase();
            release.contains("microsoft") || release.contains("wsl")
        })
        .unwrap_or(false)
}

/// Did the program start at all? Distinguishes "no such binary" from "the
/// binary ran and reported something", which is all the caller needs to decide
/// whether to try the next one.
#[cfg(target_os = "linux")]
fn spawned(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Cross from WSL to the Windows host, which is where Studio actually lives.
///
/// Several routes are tried because which ones exist varies by distro and by
/// how interop was set up. Independent implementation; the gap it closes was
/// pointed out by ROpen 1.3.2, which fixed the same thing in Luau.
#[cfg(target_os = "linux")]
fn open_on_windows_host(target: &str) -> Result<()> {
    // A Linux path means nothing to a Windows program. A `roblox-studio:` URI
    // is not a path and must be handed over untouched.
    let target = if target.starts_with('/') {
        Command::new("wslpath")
            .args(["-w", target])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| {
                let converted = String::from_utf8_lossy(&out.stdout).trim().to_string();
                (!converted.is_empty()).then_some(converted)
            })
            .unwrap_or_else(|| target.to_string())
    } else {
        target.to_string()
    };

    // PowerShell first: Start-Process handles protocol URIs and paths alike.
    // The quote doubling is PowerShell's own escape for a single-quoted string.
    let quoted = target.replace('\'', "''");
    for powershell in [
        "powershell.exe",
        "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe",
    ] {
        if spawned(
            powershell,
            &[
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &format!("Start-Process '{quoted}'"),
            ],
        ) {
            return Ok(());
        }
    }

    for rundll in ["rundll32.exe", "/mnt/c/Windows/System32/rundll32.exe"] {
        if spawned(rundll, &["url.dll,FileProtocolHandler", &target]) {
            return Ok(());
        }
    }

    // `start` reads its first quoted argument as a window title, so it needs a
    // throwaway one before the target or it opens a console instead.
    for cmd in ["cmd.exe", "/mnt/c/Windows/System32/cmd.exe"] {
        if spawned(cmd, &["/c", "start", "rbx", &target]) {
            return Ok(());
        }
    }

    if spawned("wslview", &[&target]) {
        return Ok(());
    }

    bail!(
        "Could not reach Roblox Studio from this Linux. Under WSL, check that interop is \
         enabled (`powershell.exe`, `rundll32.exe` or `wslview` must be runnable). Studio \
         has no native Linux build."
    );
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

    #[test]
    fn a_place_file_is_recognised_by_extension_either_way_it_is_cased() {
        for arg in ["game.rbxl", "Game.RBXLX", "./sub/dir/place.rbxlx"] {
            assert_eq!(
                place_file_target(None, Some(arg)),
                Some(PathBuf::from(arg)),
                "{arg}"
            );
        }
    }

    /// The reason extension beats existence: an env is a name, and a folder
    /// holding a file of that name must not change what the name means.
    #[test]
    fn an_env_name_is_never_mistaken_for_a_file() {
        assert_eq!(place_file_target(None, Some("prod")), None);
        assert_eq!(place_file_target(None, Some("staging.toml")), None);
    }

    #[test]
    fn the_explicit_flag_needs_no_extension() {
        assert_eq!(
            place_file_target(Some(Path::new("weird-name")), None),
            Some(PathBuf::from("weird-name"))
        );
    }

    /// Bare `--new` decides nothing on its own: the picker does. Returning the
    /// baseplate here instead would make `--baseplate` a flag that changes
    /// nothing.
    #[test]
    fn new_on_its_own_defers_to_the_picker() {
        let chosen = new_target(None, false, false, false, None, None).expect("nothing to refuse");
        assert_eq!(chosen, None);
    }

    #[test]
    fn baseplate_asks_nobody() {
        let chosen =
            new_target(None, true, false, false, None, None).expect("--baseplate stands alone");
        assert_eq!(chosen, Some(DEFAULT_TEMPLATE_PLACE_ID));
    }

    #[test]
    fn an_explicit_template_wins_over_the_picker() {
        let chosen = new_target(Some(777), false, false, false, None, None)
            .expect("--template stands alone");
        assert_eq!(chosen, Some(777));
    }

    /// Each of these names a place, which is exactly what `--new` says there
    /// isn't one of yet. Accepting the pair and quietly preferring one is how
    /// `--universe-id` used to behave, and it is what the error is for.
    #[test]
    fn new_refuses_every_way_of_naming_a_place() {
        for (case, error) in [
            (
                "--place-id",
                new_target(None, false, true, false, None, None),
            ),
            (
                "--universe-id",
                new_target(None, false, false, true, None, None),
            ),
            (
                "env",
                new_target(None, false, false, false, Some("prod"), None),
            ),
            (
                "place",
                new_target(None, false, false, false, None, Some("main")),
            ),
        ] {
            let error = error.expect_err("naming a place contradicts --new");
            let message = error.to_string();
            assert!(message.contains("--new"), "{case}: {message}");
            assert!(message.contains("cannot be combined"), "{case}: {message}");
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
