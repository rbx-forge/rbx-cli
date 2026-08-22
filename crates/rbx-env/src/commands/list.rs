use anyhow::{bail, Result};
use colored::Colorize;

use rbx_core::output::{self, OutputFormat};
use rbx_core::places::{Environment, PlacesFile};
use rbx_core::GlobalFlags;

use crate::json::ListDocument;

/// What a `rbx env list` invocation is actually asking for.
///
/// A single value rather than the three booleans clap parses, because only
/// four of the eight combinations exist: clap already rejects the rest with
/// `conflicts_with`, and encoding that once here keeps the printer from having
/// to re-derive which flag wins. The alternative (threading `names_only`,
/// `place_names_only` and `json` down) is how a fourth flag would silently
/// grow a precedence bug.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The default: the whole file in its own TOML shape, colored.
    Human,
    /// `--names`: env names, one per line.
    EnvNames,
    /// `--place-names`: place names, one per line.
    PlaceNames,
    /// `--json`: one document on stdout.
    Json,
}

impl Mode {
    /// Fold the parsed flags. `json` wins over the two name listings, which
    /// cannot both be set: clap rejects that pair before this runs.
    pub fn new(names: bool, place_names: bool, json: bool) -> Self {
        match (names, place_names, json) {
            (_, _, true) => Mode::Json,
            (true, _, _) => Mode::EnvNames,
            (_, true, _) => Mode::PlaceNames,
            _ => Mode::Human,
        }
    }
}

pub fn run(global: &GlobalFlags, mode: Mode) -> Result<()> {
    let places = PlacesFile::load(&global.places)?;
    let format = OutputFormat::from_json_flag(mode == Mode::Json);

    // `--env all` means "every env", which is already the default here, so it
    // is treated as no filter rather than rejected.
    let filter = match global.env.as_deref() {
        Some("all") | None => None,
        Some(name) => Some(name),
    };

    let names = match filter {
        // Resolve through `get` so an unknown name produces the shared
        // "Available: ..." error instead of an empty listing.
        Some(name) => {
            places.get(name)?;
            vec![name.to_string()]
        }
        None => places.env_names(),
    };

    if format.is_json() {
        // Ahead of the emptiness bail below, matching `--names` rather than
        // the human listing: both are read by scripts, and a file with no envs
        // is a fact to report (`.envs | length == 0`) rather than a failure.
        // The human listing keeps its error, which is the right answer for a
        // person who just asked to see the file.
        return output::emit(&ListDocument::new(&global.places, &places, &names)?);
    }

    if mode == Mode::EnvNames {
        for name in &names {
            println!("{name}");
        }
        return Ok(());
    }

    if mode == Mode::PlaceNames {
        // Deduplicated across envs, because a place name is a role (`main`,
        // `lobby`) that most files repeat in every env. `--place main` means
        // the same thing in each, so printing it once per env would be noise
        // in a completion menu and a lie in a script counting entries.
        let mut place_names: Vec<&str> = names
            .iter()
            .filter_map(|name| places.get(name).ok())
            .flat_map(|env| env.places.keys().map(String::as_str))
            .collect();
        place_names.sort_unstable();
        place_names.dedup();
        for name in place_names {
            println!("{name}");
        }
        return Ok(());
    }

    if names.is_empty() {
        bail!(
            "{} has no envs defined. Add at least one [<env>] section with universe_id.",
            global.places.display()
        );
    }

    println!("{}", global.places.display().to_string().dimmed());
    if let Some(owner) = places.owner {
        println!("owner = {} {}", owner.kind, owner.id.to_string().cyan());
    }

    for name in &names {
        let env = places.get(name)?;
        println!();
        print_env(name, env);
    }

    Ok(())
}

fn print_env(name: &str, env: &Environment) {
    let header = format!("[{name}]").bold();
    if env.confirm() {
        println!("{}  {}", header, "confirm".yellow());
    } else {
        println!("{header}");
    }

    let mut rows: Vec<(String, String)> =
        vec![("universe_id".to_string(), env.universe_id.to_string())];
    if let Some(env_type) = &env.env {
        rows.push(("env".to_string(), format!("\"{env_type}\"")));
    }
    if let Some(owner) = env.owner {
        rows.push(("owner".to_string(), format!("{} {}", owner.kind, owner.id)));
    }

    let mut place_names: Vec<&String> = env.places.keys().collect();
    place_names.sort();
    for place_name in place_names {
        let id = env.places[place_name];
        rows.push((format!("places.{place_name}"), id.to_string()));
    }

    let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (key, value) in &rows {
        println!("{key:width$} = {}", value.cyan());
    }

    if env.places.is_empty() {
        println!("{}", "(no places)".dimmed());
    }
}
