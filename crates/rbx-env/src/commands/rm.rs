//! `rbx env rm <name>` — take an env out of every file that mentions it.
//!
//! ## Why this is not `destroy`
//!
//! The obvious name for this, borrowed from the infrastructure tools this one
//! resembles, would be `destroy`: tear down what was deployed. Roblox does not
//! let a tool keep that promise. A game pass and a developer product cannot be
//! deleted, ever — the most a creator can do is take them off sale, and every
//! player who already owns one still owns it. A badge can be disabled, not
//! removed. A universe can be deactivated, and it is still there. A command
//! called `destroy` would be describing something it did not do, on resources
//! people paid money for.
//!
//! So this destroys nothing on Roblox and never opens a connection. What it
//! removes is the *env*: the block in `rbxplace.toml` and every overlay,
//! lockfile section and generated module keyed by it. That is the part that
//! really can be deleted, and doing it by hand means editing four or five
//! files and forgetting one — usually a lockfile, which then keeps ids for an
//! env nothing targets any more.
//!
//! ## Why it edits documents rather than reserialising models
//!
//! `rbxplace.toml`, `rbxmeta.toml` and `rbxshop.toml` are files people write
//! by hand, with comments in them. Loading each through serde and writing the
//! model back would drop those comments, reorder keys, and silently delete any
//! key this build does not model — the exact failure `rbx-shop`'s
//! `toml_write` and `docs/env.md` both already record. `toml_edit` removes one
//! table and leaves every byte around it alone.
//!
//! The lockfiles are machine-written and could safely be reserialised, but
//! they are edited the same way here so there is one rule rather than two.
//!
//! ## Why `rbxapikey.toml` is handled apart
//!
//! Every other file keys an env by giving it a table: `[envs.dev]`, removed
//! whole. `rbxapikey.toml` names envs *inside arrays* instead — `[settings]
//! default_envs`, and each key's `envs`, itself either one array or a table of
//! named group arrays. There is no table to drop, only a string to pull out of
//! a list, which is why it cannot be another `Target`.
//!
//! Skipping it was not a tidiness question. `resolve_universe_ids` errors
//! rather than silently dropping an env it cannot find, so a `rbxapikey.toml`
//! still naming a removed env makes the *next* `rbx apikey` run fail outright,
//! on a file the user never edited.
//!
//! Emptying one of those arrays is not neutral either, and the command says so
//! rather than doing it quietly: `effective_envs` falls back to `[settings]
//! default_envs` when a key's own list is empty, so a key that named only the
//! removed env would silently start targeting whatever the defaults are. That
//! is a removal *widening* a key's reach, which no one would predict. An
//! emptied group is the other shape of the same problem: group names are key
//! identity, so what is left is a key declaration targeting nothing.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colored::Colorize;
use toml_edit::DocumentMut;

use rbx_core::confirm::confirm_always;
use rbx_core::places::PlacesFile;

/// A file this command knows how to remove an env from.
///
/// Named rather than discovered: the set of files an env can appear in is a
/// fact about this tool's own formats, and globbing for `*.toml` would happily
/// mangle somebody's unrelated config.
struct Target {
    /// File name, resolved beside `rbxplace.toml`.
    file: &'static str,
    /// Where the env lives in that document.
    at: Location,
    /// How to describe it when reporting.
    what: &'static str,
}

enum Location {
    /// A top-level table named after the env, as in `rbxplace.toml`.
    TopLevel,
    /// A child of the `envs` table, as in every other file here.
    UnderEnvs,
}

const TARGETS: &[Target] = &[
    Target {
        file: "rbxmeta.toml",
        at: Location::UnderEnvs,
        what: "experience metadata overlay",
    },
    Target {
        file: "rbxmeta.lock.toml",
        at: Location::UnderEnvs,
        what: "experience metadata lock",
    },
    Target {
        file: "rbxshop.toml",
        at: Location::UnderEnvs,
        what: "shop overlay",
    },
    Target {
        file: "rbxshop.lock.toml",
        at: Location::UnderEnvs,
        what: "shop lock",
    },
    // The two lockfiles this command used to walk past. `rbx-config`'s own
    // loader already treats a section for an unknown env as a problem — it
    // tells the reader to "Delete the [envs.<name>] section ... if intentional"
    // — so leaving one behind meant this command created the very state
    // another command complains about.
    Target {
        file: "rbxconfig.lock.toml",
        at: Location::UnderEnvs,
        what: "universe config lock",
    },
    Target {
        file: "rbxapikey.lock.toml",
        at: Location::UnderEnvs,
        what: "api key lock",
    },
];

/// Where the env name was found in `rbxapikey.toml`, and what emptying it
/// costs the reader.
#[derive(Default)]
struct ApikeyEdit {
    /// Arrays the env name was pulled out of, for the plan.
    sites: Vec<String>,
    /// Those the removal left empty, which changes what a key targets.
    emptied: Vec<String>,
}

impl ApikeyEdit {
    /// Record one array the env came out of, and whether that emptied it.
    fn note(&mut self, site: String, empty: bool) {
        if empty {
            self.emptied.push(site.clone());
        }
        self.sites.push(site);
    }
}

/// Drop every occurrence of `env` from a TOML array. Returns whether it was
/// there.
///
/// The prefix dance keeps the file readable. `retain` removes a value together
/// with its own leading whitespace, so taking the head out of
/// `["dev", "prod"]` promotes the space that used to follow the comma and
/// writes `[ "prod"]` — a space nobody typed, in a file somebody maintains by
/// hand. Restoring whatever decor the original head carried reproduces the
/// author's own style instead of imposing one: a list written `["a", "b"]`
/// stays tight, and one written `[ "a", "b" ]` keeps its padding.
fn drop_from_array(arr: &mut toml_edit::Array, env: &str) -> bool {
    let before = arr.len();
    let head_prefix = arr
        .get(0)
        .and_then(|v| v.decor().prefix())
        .and_then(|p| p.as_str())
        .map(str::to_owned);
    arr.retain(|v| v.as_str() != Some(env));
    if arr.len() == before {
        return false;
    }
    if let (Some(prefix), Some(head)) = (head_prefix, arr.get_mut(0)) {
        head.decor_mut().set_prefix(prefix);
    }
    true
}

/// Take `env` out of every array in `rbxapikey.toml` that names it.
///
/// Both shapes of `envs` are walked: the array form (`envs = ["dev", "prod"]`,
/// one key spanning the list) and the group form (`[keys.deploy.envs]` with one
/// array per named group, one key each). Missing either would leave the
/// dangling reference this exists to prevent.
fn strip_apikey_refs(doc: &mut DocumentMut, env: &str) -> ApikeyEdit {
    let mut edit = ApikeyEdit::default();

    if let Some(arr) = doc
        .get_mut("settings")
        .and_then(|s| s.get_mut("default_envs"))
        .and_then(|d| d.as_array_mut())
    {
        if drop_from_array(arr, env) {
            let empty = arr.is_empty();
            edit.note("[settings] default_envs".to_string(), empty);
        }
    }

    let Some(keys) = doc.get_mut("keys").and_then(|k| k.as_table_like_mut()) else {
        return edit;
    };
    for (name, item) in keys.iter_mut() {
        let name = name.get().to_string();
        let Some(envs) = item.get_mut("envs") else {
            continue;
        };
        if let Some(arr) = envs.as_array_mut() {
            if drop_from_array(arr, env) {
                let empty = arr.is_empty();
                edit.note(format!("[keys.{name}] envs"), empty);
            }
        } else if let Some(groups) = envs.as_table_like_mut() {
            for (group, item) in groups.iter_mut() {
                let group = group.get().to_string();
                let Some(arr) = item.as_array_mut() else {
                    continue;
                };
                if drop_from_array(arr, env) {
                    let empty = arr.is_empty();
                    edit.note(format!("[keys.{name}.envs] {group}"), empty);
                }
            }
        }
    }
    edit
}

/// One removal the run intends to make.
struct Removal {
    path: PathBuf,
    what: String,
}

/// Load a TOML document, or `None` when the file is simply not there.
///
/// A project with no shop has no `rbxshop.toml`, and that is not an error to
/// report — it is the ordinary case.
fn load(path: &Path) -> Result<Option<DocumentMut>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let doc = text
        .parse::<DocumentMut>()
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(Some(doc))
}

/// Whether `doc` holds `env` at `at`, without changing anything.
fn holds(doc: &DocumentMut, at: &Location, env: &str) -> bool {
    match at {
        Location::TopLevel => doc.get(env).is_some(),
        // `as_table_like`, not `as_table`: `envs = { dev = { … } }` is an
        // inline table, which `as_table` reports as absent. A file written that
        // way would have been skipped in silence while the command printed a
        // plan without it and reported success — the opposite of what a command
        // promising "every file that mentions it" owes the reader.
        Location::UnderEnvs => doc
            .get("envs")
            .and_then(|envs| envs.as_table_like())
            .is_some_and(|envs| envs.get(env).is_some()),
    }
}

/// Remove `env` at `at`. Returns whether anything was there to remove.
fn strip(doc: &mut DocumentMut, at: &Location, env: &str) -> bool {
    match at {
        Location::TopLevel => doc.remove(env).is_some(),
        Location::UnderEnvs => {
            let Some(envs) = doc.get_mut("envs").and_then(|e| e.as_table_like_mut()) else {
                return false;
            };
            let removed = envs.remove(env).is_some();
            // An `[envs]` table left behind with nothing in it is noise in a
            // hand-edited file, and in a lockfile it round-trips to the same
            // thing either way. Only dropped when this command emptied it.
            if removed && envs.is_empty() {
                doc.remove("envs");
            }
            removed
        }
    }
}

/// The per-env module `rbx shop codegen` writes, if this project generates
/// one.
///
/// Generated files are the one place where leaving the env behind is not
/// merely untidy: `<out>/dev.luau` keeps returning ids for an env that no
/// longer exists, and `rbx shop codegen --check` would go on accepting it
/// because the folder still matches a lockfile that no longer mentions the
/// env. The aggregate modules — `init.luau`, the type module, and whatever
/// `rbx env gen-module` writes — are *regenerated* rather than deleted, so
/// they are named in the closing hint instead of touched here.
fn shop_env_module(dir: &Path, env: &str) -> Result<Option<PathBuf>> {
    let Some(doc) = load(&dir.join("rbxshop.toml"))? else {
        return Ok(None);
    };
    let Some(output) = doc
        .get("codegen")
        .and_then(|c| c.get("output"))
        .and_then(|o| o.as_str())
    else {
        return Ok(None);
    };
    let module = dir.join(output).join(format!("{env}.luau"));
    Ok(module.exists().then_some(module))
}

pub fn run(places_path: &Path, env: &str, dry_run: bool, yes: bool) -> Result<()> {
    let dir = places_path.parent().unwrap_or(Path::new("."));

    // The env has to exist in `rbxplace.toml` before anything is removed
    // anywhere. Without this check a typo would quietly delete nothing and
    // report success, which is the worst possible answer for a command whose
    // whole job is deletion.
    //
    // Reading through `PlacesFile` rather than looking for a table of that
    // name also keeps `[owner]` and `[codegen]` from being mistaken for envs.
    let places = PlacesFile::load(places_path)?;
    if !places.environments.contains_key(env) {
        let mut known: Vec<&str> = places.environments.keys().map(|s| s.as_str()).collect();
        known.sort_unstable();
        bail!(
            "No env named '{env}' in {}.\nDefined: {}",
            places_path.display(),
            if known.is_empty() {
                "none".to_string()
            } else {
                known.join(", ")
            }
        );
    }

    // Everything is planned before anything is written. A run that removed the
    // env from three files and then failed to parse the fourth would leave the
    // project in a state no command describes.
    let mut removals = Vec::new();
    let mut edits: Vec<(PathBuf, DocumentMut)> = Vec::new();

    let mut places_doc = load(places_path)?
        .with_context(|| format!("{} disappeared while reading it", places_path.display()))?;
    if strip(&mut places_doc, &Location::TopLevel, env) {
        removals.push(Removal {
            path: places_path.to_path_buf(),
            what: "env definition".to_string(),
        });
        edits.push((places_path.to_path_buf(), places_doc));
    }

    for target in TARGETS {
        let path = dir.join(target.file);
        let Some(mut doc) = load(&path)? else {
            continue;
        };
        if !holds(&doc, &target.at, env) {
            continue;
        }
        strip(&mut doc, &target.at, env);
        removals.push(Removal {
            path: path.clone(),
            what: target.what.to_string(),
        });
        edits.push((path, doc));
    }

    // Handled after TARGETS rather than inside it: the env lives in arrays
    // here, not in a table of its own. See the module header.
    let apikey_path = dir.join("rbxapikey.toml");
    let mut emptied: Vec<String> = Vec::new();
    if let Some(mut doc) = load(&apikey_path)? {
        let edit = strip_apikey_refs(&mut doc, env);
        if !edit.sites.is_empty() {
            removals.push(Removal {
                path: apikey_path.clone(),
                what: format!("api key targeting: {}", edit.sites.join(", ")),
            });
            edits.push((apikey_path, doc));
            emptied = edit.emptied;
        }
    }

    let module = shop_env_module(dir, env)?;
    if let Some(path) = &module {
        removals.push(Removal {
            path: path.clone(),
            what: "generated env module".to_string(),
        });
    }

    // "would be" only under --dry-run. On the real path the same list is the
    // plan the confirmation prompt is about, and reading "would be removed"
    // immediately above a prompt that removes it is one tense too many.
    println!(
        "{} '{}' {} removed from:\n",
        "env".bold(),
        env.bold(),
        if dry_run { "would be" } else { "will be" }
    );
    for removal in &removals {
        println!("  {} — {}", removal.path.display(), removal.what.dimmed());
    }

    // Printed before the dry-run exit and before the prompt, because it is the
    // one consequence of this command that outlives it: every other line above
    // describes something being deleted, and this one describes a key whose
    // meaning changes as a result.
    if !emptied.is_empty() {
        println!(
            "\n{} this empties {} in {}:",
            "!".yellow().bold(),
            if emptied.len() == 1 {
                "one list".to_string()
            } else {
                format!("{} lists", emptied.len())
            },
            "rbxapikey.toml".bold()
        );
        for site in &emptied {
            println!("  {site}");
        }
        println!(
            "{}",
            "  A key with an empty `envs` falls back to [settings] default_envs, so it\n  \
             may now target envs it never named; an empty group targets nothing at all.\n  \
             Decide what each one should do before the next `rbx apikey sync`."
                .dimmed()
        );
    }

    if dry_run {
        println!("\nDry run — nothing was changed.");
        return Ok(());
    }

    // Always asked, never merely "if configured". Unlike a sync, there is no
    // remote state to compare against afterwards and no `pull` that brings the
    // env back: the files are the only record it existed.
    confirm_always(
        &format!("Remove env '{env}' from {} file(s)?", removals.len()),
        yes,
    )?;

    for (path, doc) in edits {
        std::fs::write(&path, doc.to_string())
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    if let Some(path) = module {
        std::fs::remove_file(&path)
            .with_context(|| format!("Failed to remove {}", path.display()))?;
    }

    println!("{} env '{}' removed.", "✓".green(), env);
    println!(
        "{}",
        "  Regenerate the aggregate modules: rbx shop codegen, rbx env gen-module.".dimmed()
    );

    Ok(())
}
