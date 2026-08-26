use std::path::Path;

use anyhow::{anyhow, bail, Result};

use rbx_core::output::{self, OutputFormat};
use rbx_core::places::{self, PlacesFile};
use rbx_core::GlobalFlags;

use crate::json::GetDocument;
use crate::Field;

pub fn run(global: &GlobalFlags, field: Field, json: bool) -> Result<()> {
    let format = OutputFormat::from_json_flag(json);

    // `all` and a group are the same shape here: several envs, one line each.
    // A single env is the third case, and keeps printing a bare value, because
    // that is what a shell substitution reads.
    let plural = match global.env.as_deref() {
        None => None,
        Some(value) => {
            let places = PlacesFile::load(&global.places)?;
            match places.selector(value)? {
                places::EnvSelector::One(_) => None,
                places::EnvSelector::Every => Some(places.env_names()),
                places::EnvSelector::Group { members, .. } => Some(members),
            }
        }
    };

    match (plural, global.env.as_deref()) {
        (Some(names), _) => {
            if names.is_empty() {
                bail!(
                    "{} has no envs defined. Add at least one [<env>] section with universe_id.",
                    global.places.display()
                );
            }
            if format.is_json() {
                // Every env resolved before anything is written, because a
                // document is only useful whole: a run that fails on the third
                // env must emit no document rather than a truncated one.
                let mut answers = Vec::with_capacity(names.len());
                for name in names {
                    let value =
                        resolve_field(&global.places, &name, global.place.as_deref(), field)?;
                    answers.push((name, value));
                }
                return output::emit(&GetDocument::every(field.as_str(), answers));
            }

            // The human form keeps streaming, as it always has: the lines
            // already printed when the fourth env fails are worth having.
            // Tab-separated so `rbx env get universe-id -e all | cut -f2` works.
            for name in names {
                let value = resolve_field(&global.places, &name, global.place.as_deref(), field)?;
                println!("{name}\t{value}");
            }
        }

        (None, Some(name)) => {
            let value = resolve_field(&global.places, name, global.place.as_deref(), field)?;
            if format.is_json() {
                return output::emit(&GetDocument::single(field.as_str(), Some(name), value));
            }
            println!("{value}");
        }

        // No `--env`: only the owner fields have an env-independent answer,
        // via the top-level `[owner]` block.
        (None, None) if field.needs_env() => {
            bail!(
                "`rbx env get {}` needs a target env. Pass --env <name>, or --env all \
                 to print one line per env.",
                field
            );
        }
        (None, None) => {
            let places = PlacesFile::load(&global.places)?;
            let owner = places.owner.ok_or_else(|| {
                anyhow!(
                    "No top-level [owner] block in {}. Pass --env <name> to read a \
                     per-env [<env>.owner] override instead.",
                    global.places.display()
                )
            })?;
            let value = owner_field(owner, field);
            if format.is_json() {
                // No `env` key on this one: the answer came from the top-level
                // `[owner]` block, and naming an env it did not come from
                // would be a lie a script could act on.
                return output::emit(&GetDocument::single(field.as_str(), None, value));
            }
            println!("{value}");
        }
    }

    Ok(())
}

/// Resolve one field of `rbxplace.toml` to its printable value.
///
/// Deliberately delegates to `rbx_core::places` rather than re-reading the
/// maps directly: the place-name defaulting rule (`--place` > `main` > the
/// only entry) must stay identical to what `rbx place`, `rbx meta`, and the
/// rest resolve, otherwise this command would report an id no other command
/// actually uses.
pub fn resolve_field(
    places_path: &Path,
    env: &str,
    place: Option<&str>,
    field: Field,
) -> Result<String> {
    match field {
        Field::UniverseId => Ok(places::resolve_universe_id(places_path, env)?.to_string()),

        Field::PlaceId => {
            let (_universe_id, place_id) = places::resolve(places_path, env, place)?;
            Ok(place_id.to_string())
        }

        Field::OwnerId | Field::OwnerType => {
            let places = PlacesFile::load(places_path)?;
            // Surface the "env not found / Available: ..." error before the
            // owner lookup, which tolerates unknown env names by design.
            places.get(env)?;
            let owner = places.resolve_owner(env).copied().ok_or_else(|| {
                anyhow!(
                    "No owner for env '{}': neither [{}.owner] nor a top-level [owner] \
                     block is set in {}.",
                    env,
                    env,
                    places_path.display()
                )
            })?;
            Ok(owner_field(owner, field))
        }
    }
}

fn owner_field(owner: rbx_core::owner::Owner, field: Field) -> String {
    match field {
        Field::OwnerType => owner.kind.to_string(),
        // `owner-id` is the only other owner-shaped field; the id fields never
        // reach here (they're handled above, and `run` gates them on --env).
        _ => owner.id.to_string(),
    }
}
