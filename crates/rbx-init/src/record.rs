//! Recording freshly created resources into `rbxplace.toml`.
//!
//! Two concerns live here: deciding *what* to record, and writing it *without*
//! reformatting the file.
//!
//! **Deciding** always happens before the Roblox call that creates the
//! resource. Creating a universe or a place is irreversible and costs a real
//! resource, so a Ctrl-C at the prompt, or an ambiguous config we refuse to
//! guess at: must not be able to leave something created but unrecorded.
//! Every function here is therefore side-effect-free until the caller has its
//! ids in hand.
//!
//! **Writing** is deliberately line-level rather than a serde round-trip.
//! Reserializing through `toml::to_string_pretty` drops every comment,
//! reorders keys, and silently deletes any field the struct does not model,
//! which is how the removed `gen-rbxplace` came to eat `env` overrides. The
//! helpers below only ever insert lines, so existing content (comments, key
//! order, CRLF endings, fields this crate never heard of)
//! survives byte-for-byte.

use std::io::IsTerminal;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use dialoguer::theme::ColorfulTheme;
use dialoguer::Input;

use rbx_core::places::{is_reserved_env_name, Environment, PlacesFile, RESERVED_ENV_NAMES};

/// Place name assumed when none is given. The whole toolkit already treats
/// `main` as the default entry (`rbx_core::places::resolve` prefers it over
/// any other key), so asking the user for it would be a question with one
/// obvious answer.
const DEFAULT_PLACE: &str = "main";

/// A new `[<env>]` block to append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEnv {
    pub env: String,
    pub place: String,
}

/// A new place key to insert under an existing `[<env>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPlace {
    pub env: String,
    pub place: String,
}

// ---------------------------------------------------------------------------
// Name suggestions
// ---------------------------------------------------------------------------

/// Slug for an env name: strip leading `[PREFIX]`, lowercase, alphanumeric only.
/// Returns the prefix when present, otherwise the slugified full name.
fn suggest_env_name(universe_name: &str) -> String {
    let trimmed = universe_name.trim();
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some(close) = rest.find(']') {
            let prefix = &rest[..close];
            let slug = slugify(prefix);
            if !slug.is_empty() {
                return slug;
            }
        }
    }
    let slug = slugify(trimmed);
    if slug.is_empty() {
        "env".to_string()
    } else {
        slug
    }
}

/// Slug for a place name. Returns None if the slug would be empty.
fn suggest_place_name(place_name: &str) -> Option<String> {
    let slug = slugify(place_name);
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_under = true;
    for c in s.chars() {
        let lower = c.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            prev_under = false;
        } else if !prev_under {
            out.push('_');
            prev_under = true;
        }
    }
    out.trim_end_matches('_').to_string()
}

// ---------------------------------------------------------------------------
// Deciding what to record
// ---------------------------------------------------------------------------

/// Reject an `--env` that names something `rbxplace.toml` already spells for
/// itself.
///
/// The list comes from `rbx_core::places` rather than from a literal here, so
/// this crate and rbx-import cannot end up refusing different names: see
/// [`rbx_core::places::RESERVED_ENV_NAMES`].
fn ensure_recordable_env(env: &str) -> Result<()> {
    if is_reserved_env_name(env) {
        bail!(
            "`--env {}` is not a recording target: {} are reserved names in \
             rbxplace.toml. Pass --env <name>.",
            env,
            RESERVED_ENV_NAMES.join(", ")
        );
    }
    Ok(())
}

/// Decide which env a universe about to be created should be recorded as.
///
/// `env_flag` is the global `--env`: for a *create* there is no existing env to
/// read, so the flag unambiguously names the entry to write. Returns `None`
/// when nothing should be recorded.
pub fn choose_new_env(
    places_path: &Path,
    env_flag: Option<&str>,
    place_flag: Option<&str>,
    universe_name: Option<&str>,
    no_record: bool,
    yes: bool,
) -> Result<Option<NewEnv>> {
    if no_record {
        return Ok(None);
    }

    let place = place_flag.unwrap_or(DEFAULT_PLACE).to_string();

    if let Some(env) = env_flag {
        ensure_recordable_env(env)?;
        if !places_path.exists() {
            bail!(
                "--env {} was passed but {} does not exist. Create it first (a \
                 minimal file is a [<env>] section with a universe_id, plus the \
                 shared [owner] block) or point --places at an existing one.",
                env,
                places_path.display()
            );
        }
        let places = PlacesFile::load(places_path)?;
        ensure_env_absent(&places, env, places_path)?;
        return Ok(Some(NewEnv {
            env: env.to_string(),
            place,
        }));
    }

    // No explicit target: only prompt when there is a file to add to and a
    // human to ask. `--yes` means "don't ask me anything", so it must skip:
    // otherwise a scripted `-y` invocation that used to run unattended would
    // start hanging on a prompt.
    if yes || !places_path.exists() || !std::io::stdin().is_terminal() {
        return Ok(None);
    }

    let places = PlacesFile::load(places_path)?;
    let suggested = suggest_env_name(universe_name.unwrap_or_default());
    let env = prompt_env_name(&places, &suggested)?;
    Ok(Some(NewEnv { env, place }))
}

/// Decide where a place about to be created should be recorded.
///
/// The env is derived from the universe it is being created in, so there is no
/// second question to ask: only the place's key name is prompted for.
pub fn choose_new_place(
    places_path: &Path,
    env_flag: Option<&str>,
    place_flag: Option<&str>,
    universe_id: u64,
    place_display_name: Option<&str>,
    no_record: bool,
    yes: bool,
) -> Result<Option<NewPlace>> {
    if no_record {
        return Ok(None);
    }

    // A flag naming the target is an explicit request: failing to honor it is
    // an error, whereas the implicit path just stays quiet.
    let explicit = env_flag.is_some() || place_flag.is_some();

    if !places_path.exists() {
        if explicit {
            bail!(
                "{} does not exist, so there is nothing to record into. Run \
                 create it first, or pass --no-record.",
                places_path.display()
            );
        }
        return Ok(None);
    }

    let places = PlacesFile::load(places_path)?;
    let Some(env) =
        resolve_env_for_universe(&places, env_flag, universe_id, explicit, places_path)?
    else {
        return Ok(None);
    };
    let entry = places.get(&env)?;

    let place = match place_flag {
        Some(name) => {
            ensure_place_absent(entry, &env, name)?;
            name.to_string()
        }
        None => {
            if yes || !std::io::stdin().is_terminal() {
                return Ok(None);
            }
            let suggested = place_display_name
                .and_then(suggest_place_name)
                .unwrap_or_else(|| DEFAULT_PLACE.to_string());
            prompt_place_name(entry, &env, &suggested)?
        }
    };

    Ok(Some(NewPlace { env, place }))
}

/// Map a universe id back to the env that points at it. An explicit `--env`
/// must match that universe: recording a place under the wrong env would be
/// worse than not recording it at all.
fn resolve_env_for_universe(
    places: &PlacesFile,
    env_flag: Option<&str>,
    universe_id: u64,
    explicit: bool,
    places_path: &Path,
) -> Result<Option<String>> {
    if let Some(name) = env_flag {
        ensure_recordable_env(name)?;
        let entry = places.get(name)?;
        if entry.universe_id != universe_id {
            bail!(
                "[{}] in {} points at universe {}, but the place is being created in \
                 universe {}. Pass the matching --env, or --no-record.",
                name,
                places_path.display(),
                entry.universe_id,
                universe_id
            );
        }
        return Ok(Some(name.to_string()));
    }

    let mut matches: Vec<&String> = places
        .environments
        .iter()
        .filter(|(_, e)| e.universe_id == universe_id)
        .map(|(name, _)| name)
        .collect();
    matches.sort();

    match matches.len() {
        1 => Ok(Some(matches[0].clone())),
        0 if explicit => bail!(
            "No env in {} points at universe {}. Add a [<env>] section for it, \
             or pass --no-record.",
            places_path.display(),
            universe_id
        ),
        _ if explicit => bail!(
            "Several envs in {} point at universe {} ({}). Pass --env <name> to pick one.",
            places_path.display(),
            universe_id,
            matches
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        // Nothing to attach to, and nobody asked: stay quiet.
        _ => Ok(None),
    }
}

fn ensure_env_absent(places: &PlacesFile, env: &str, places_path: &Path) -> Result<()> {
    if places.environments.contains_key(env) {
        bail!(
            "Env '{}' already exists in {}. Pick another name, or pass --no-record.",
            env,
            places_path.display()
        );
    }
    Ok(())
}

fn ensure_place_absent(entry: &Environment, env: &str, place: &str) -> Result<()> {
    if entry.places.contains_key(place) {
        bail!(
            "Place '{}' already exists under [{}.places]. Pick another name, or pass \
             --no-record.",
            place,
            env
        );
    }
    Ok(())
}

/// Prompt for the resource's display name when `--name` was omitted.
///
/// Empty input keeps the template's own name, which is exactly what omitting
/// the flag does today, so a bare Enter reproduces the previous behavior.
/// Asking here also feeds the env/place suggestion downstream, which is why
/// this runs before [`choose_new_env`].
pub fn prompt_display_name(label: &str, yes: bool) -> Result<Option<String>> {
    if yes || !std::io::stdin().is_terminal() {
        return Ok(None);
    }
    let theme = ColorfulTheme::default();
    let value: String = Input::with_theme(&theme)
        .with_prompt(format!("{label} (empty to keep the template's)"))
        .allow_empty(true)
        .interact_text()
        .map_err(|e| anyhow!("Prompt error: {}", e))?;

    let trimmed = value.trim();
    Ok(if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    })
}

fn prompt_env_name(places: &PlacesFile, suggested: &str) -> Result<String> {
    let theme = ColorfulTheme::default();
    loop {
        let candidate: String = Input::with_theme(&theme)
            .with_prompt("Env name in rbxplace.toml")
            .default(suggested.to_string())
            .interact_text()
            .map_err(|e| anyhow!("Prompt error: {}", e))?;

        let trimmed = candidate.trim().to_string();
        if trimmed.is_empty() {
            eprintln!("  Env name cannot be empty.");
            continue;
        }
        if places.environments.contains_key(&trimmed) {
            eprintln!("  Env '{trimmed}' already exists. Pick a different name.");
            continue;
        }
        return Ok(trimmed);
    }
}

fn prompt_place_name(entry: &Environment, env: &str, suggested: &str) -> Result<String> {
    let theme = ColorfulTheme::default();
    loop {
        let candidate: String = Input::with_theme(&theme)
            .with_prompt(format!("Place name under [{env}.places]"))
            .default(suggested.to_string())
            .interact_text()
            .map_err(|e| anyhow!("Prompt error: {}", e))?;

        let trimmed = candidate.trim().to_string();
        if trimmed.is_empty() {
            eprintln!("  Place name cannot be empty.");
            continue;
        }
        if entry.places.contains_key(&trimmed) {
            eprintln!("  Place '{trimmed}' already exists under [{env}.places].");
            continue;
        }
        return Ok(trimmed);
    }
}

// ---------------------------------------------------------------------------
// Writing, without reformatting
// ---------------------------------------------------------------------------

/// Append a brand-new env block at the end of the file. A `[header]` always
/// opens a fresh table, so this is a pure append: nothing already on disk is
/// re-read, re-parsed, or rewritten.
pub fn append_env(
    path: &Path,
    env: &str,
    universe_id: u64,
    place: &str,
    place_id: u64,
) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let updated = append_env_str(&content, env, universe_id, place, place_id);
    std::fs::write(path, updated).with_context(|| format!("Failed to write {}", path.display()))
}

/// Insert a place key under an existing env, leaving every other line intact.
pub fn insert_place(path: &Path, env: &str, place: &str, place_id: u64) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let updated = insert_place_str(&content, env, place, place_id)?;
    std::fs::write(path, updated).with_context(|| format!("Failed to write {}", path.display()))
}

/// String form of [`append_env`], so the formatting is directly testable.
pub fn append_env_str(
    content: &str,
    env: &str,
    universe_id: u64,
    place: &str,
    place_id: u64,
) -> String {
    let newline = detect_newline(content);
    let mut out = content.to_string();

    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(newline);
    }
    // One blank line between blocks, but not at the top of an empty file.
    if !out.trim().is_empty() {
        out.push_str(newline);
    }

    out.push_str(&format!("[{env}]{newline}"));
    out.push_str(&format!("universe_id = {universe_id}{newline}"));
    out.push_str(&format!("places.{place} = {place_id}{newline}"));
    out
}

/// String form of [`insert_place`], so the line surgery is directly testable.
pub fn insert_place_str(content: &str, env: &str, place: &str, place_id: u64) -> Result<String> {
    let newline = detect_newline(content);
    let mut lines: Vec<String> = content.split_inclusive('\n').map(str::to_string).collect();

    let header = lines
        .iter()
        .position(|l| header_name(l).as_deref() == Some(env))
        .ok_or_else(|| anyhow!("[{}] not found in rbxplace.toml.", env))?;

    // A `[<env>.places]` sub-table wins when the file already uses one, so the
    // new key lands next to its siblings instead of introducing a second,
    // conflicting style for the same data.
    let sub_prefix = format!("{env}.");
    let region_end = (header + 1..lines.len())
        .find(|&i| header_name(&lines[i]).is_some_and(|n| n != env && !n.starts_with(&sub_prefix)))
        .unwrap_or(lines.len());
    let places_header = (header + 1..region_end)
        .find(|&i| header_name(&lines[i]).as_deref() == Some(&format!("{env}.places")));

    let (block_start, new_key) = match places_header {
        Some(i) => (i, format!("{place} = {place_id}")),
        None => (header, format!("places.{place} = {place_id}")),
    };
    let block_end = table_end(&lines, block_start);

    // Anchor on the last key line so trailing blanks and comments (which
    // usually introduce whatever comes next) stay below the insertion.
    let anchor = (block_start + 1..block_end)
        .rfind(|&i| is_key_line(&lines[i]))
        .unwrap_or(block_start);

    // The anchor may be the file's final line with no terminator of its own.
    if !lines[anchor].ends_with('\n') {
        lines[anchor].push_str(newline);
    }
    lines.insert(anchor + 1, format!("{new_key}{newline}"));

    Ok(lines.concat())
}

/// Preserve the file's existing line endings: rewriting a CRLF file with LF
/// would show up as a whole-file diff.
fn detect_newline(content: &str) -> &'static str {
    if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// The table name a line opens, if it is a table header. Quotes are stripped
/// so `["dev"]` matches `dev`; `[[array]]` returns its inner name and still
/// counts as a boundary.
fn header_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    let inner = match inner.strip_prefix('[') {
        Some(rest) => rest.strip_suffix(']').unwrap_or(rest),
        None => inner,
    };
    Some(inner.trim().replace(['"', '\''], ""))
}

/// Index just past the last line of the table opened at `start`.
fn table_end(lines: &[String], start: usize) -> usize {
    (start + 1..lines.len())
        .find(|&i| header_name(&lines[i]).is_some())
        .unwrap_or(lines.len())
}

fn is_key_line(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && !trimmed.starts_with('#') && trimmed.contains('=')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every write must still parse as the shared rbxplace.toml shape: the
    /// line surgery is only safe if the result round-trips.
    fn parsed(content: &str) -> PlacesFile {
        toml::from_str(content).unwrap_or_else(|e| panic!("result must parse: {e}\n---\n{content}"))
    }

    const WITH_COMMENTS: &str = "\
# Shared env map. Keep prod last.
[owner]
type = \"group\"
id = 1234567

[dev]
universe_id = 100
places.main = 1001

# Production: do not point this at a test universe.
[prod]
universe_id = 200
[prod.places]
main = 2001
";

    // -----------------------------------------------------------------
    // append_env_str
    // -----------------------------------------------------------------

    #[test]
    fn append_env_leaves_existing_bytes_untouched() {
        let out = append_env_str(WITH_COMMENTS, "test", 300, "main", 3001);
        assert!(
            out.starts_with(WITH_COMMENTS),
            "append must be a pure suffix, got:\n{out}"
        );
        assert!(out.ends_with("[test]\nuniverse_id = 300\nplaces.main = 3001\n"));

        let places = parsed(&out);
        assert_eq!(places.get("test").unwrap().universe_id, 300);
        assert_eq!(places.get("test").unwrap().places["main"], 3001);
        // The pre-existing envs and owner survive.
        assert_eq!(places.get("dev").unwrap().universe_id, 100);
        assert_eq!(places.owner.unwrap().id, 1234567);
    }

    #[test]
    fn append_env_separates_blocks_with_one_blank_line() {
        let out = append_env_str("[dev]\nuniverse_id = 100\n", "test", 300, "main", 3001);
        assert_eq!(
            out,
            "[dev]\nuniverse_id = 100\n\n[test]\nuniverse_id = 300\nplaces.main = 3001\n"
        );
    }

    #[test]
    fn append_env_to_an_empty_file_has_no_leading_blank() {
        let out = append_env_str("", "test", 300, "main", 3001);
        assert_eq!(out, "[test]\nuniverse_id = 300\nplaces.main = 3001\n");
        parsed(&out);
    }

    #[test]
    fn append_env_terminates_a_file_that_lacks_a_final_newline() {
        let out = append_env_str("[dev]\nuniverse_id = 100", "test", 300, "main", 3001);
        assert!(out.contains("universe_id = 100\n\n[test]"), "got:\n{out}");
        parsed(&out);
    }

    #[test]
    fn append_env_honors_a_custom_place_key() {
        let out = append_env_str("[dev]\nuniverse_id = 100\n", "test", 300, "lobby", 3002);
        assert_eq!(parsed(&out).get("test").unwrap().places["lobby"], 3002);
    }

    #[test]
    fn append_env_preserves_crlf() {
        let out = append_env_str("[dev]\r\nuniverse_id = 100\r\n", "test", 300, "main", 3001);
        assert!(out.ends_with("[test]\r\nuniverse_id = 300\r\nplaces.main = 3001\r\n"));
        assert!(!out.contains("\n\n"), "must not mix endings, got {out:?}");
        parsed(&out);
    }

    // -----------------------------------------------------------------
    // insert_place_str
    // -----------------------------------------------------------------

    #[test]
    fn insert_place_extends_an_existing_places_sub_table() {
        let out = insert_place_str(WITH_COMMENTS, "prod", "lobby", 2002).unwrap();
        assert!(out.contains("main = 2001\nlobby = 2002\n"), "got:\n{out}");

        let places = parsed(&out);
        assert_eq!(places.get("prod").unwrap().places["lobby"], 2002);
        assert_eq!(places.get("prod").unwrap().places["main"], 2001);
    }

    #[test]
    fn insert_place_matches_the_dotted_style_when_the_file_uses_it() {
        let out = insert_place_str(WITH_COMMENTS, "dev", "lobby", 1002).unwrap();
        assert!(
            out.contains("places.main = 1001\nplaces.lobby = 1002\n"),
            "got:\n{out}"
        );
        assert_eq!(parsed(&out).get("dev").unwrap().places["lobby"], 1002);
    }

    #[test]
    fn insert_place_keeps_every_comment() {
        let out = insert_place_str(WITH_COMMENTS, "dev", "lobby", 1002).unwrap();
        assert!(out.contains("# Shared env map. Keep prod last."));
        assert!(out.contains("# Production: do not point this at a test universe."));
    }

    #[test]
    fn insert_place_lands_above_a_trailing_comment_not_below_it() {
        // The comment introduces the *next* section, so the new key must not
        // be pushed past it.
        let content = "\
[dev]
universe_id = 100
places.main = 1001

# prod is deliberately last
[prod]
universe_id = 200
";
        let out = insert_place_str(content, "dev", "lobby", 1002).unwrap();
        assert!(
            out.contains("places.main = 1001\nplaces.lobby = 1002\n\n# prod is deliberately last"),
            "got:\n{out}"
        );
        parsed(&out);
    }

    #[test]
    fn insert_place_creates_the_first_place_of_an_env() {
        let content = "[dev]\nuniverse_id = 100\n\n[prod]\nuniverse_id = 200\n";
        let out = insert_place_str(content, "dev", "main", 1001).unwrap();
        assert_eq!(
            out,
            "[dev]\nuniverse_id = 100\nplaces.main = 1001\n\n[prod]\nuniverse_id = 200\n"
        );
    }

    #[test]
    fn insert_place_stops_at_a_sibling_sub_table() {
        // `[dev.places]` ends where `[dev.owner]` begins: the new key must
        // not slide into the owner table.
        let content = "\
[dev]
universe_id = 100
[dev.places]
main = 1001
[dev.owner]
type = \"user\"
id = 42
";
        let out = insert_place_str(content, "dev", "lobby", 1002).unwrap();
        assert!(
            out.contains("main = 1001\nlobby = 1002\n[dev.owner]"),
            "got:\n{out}"
        );

        let places = parsed(&out);
        assert_eq!(places.get("dev").unwrap().places["lobby"], 1002);
        assert_eq!(places.get("dev").unwrap().owner.unwrap().id, 42);
    }

    #[test]
    fn insert_place_finds_a_sub_table_declared_after_other_ones() {
        let content = "\
[dev]
universe_id = 100
[dev.owner]
type = \"user\"
id = 42
[dev.places]
main = 1001
";
        let out = insert_place_str(content, "dev", "lobby", 1002).unwrap();
        assert!(out.ends_with("main = 1001\nlobby = 1002\n"), "got:\n{out}");
        assert_eq!(parsed(&out).get("dev").unwrap().places["lobby"], 1002);
    }

    #[test]
    fn insert_place_does_not_touch_the_following_env() {
        let out = insert_place_str(WITH_COMMENTS, "dev", "lobby", 1002).unwrap();
        let places = parsed(&out);
        assert_eq!(places.get("prod").unwrap().universe_id, 200);
        assert_eq!(places.get("prod").unwrap().places.len(), 1);
    }

    #[test]
    fn insert_place_terminates_a_file_that_lacks_a_final_newline() {
        let out = insert_place_str("[dev]\nuniverse_id = 100", "dev", "main", 1001).unwrap();
        assert_eq!(out, "[dev]\nuniverse_id = 100\nplaces.main = 1001\n");
        parsed(&out);
    }

    #[test]
    fn insert_place_preserves_crlf() {
        let content = "[dev]\r\nuniverse_id = 100\r\nplaces.main = 1001\r\n";
        let out = insert_place_str(content, "dev", "lobby", 1002).unwrap();
        assert_eq!(
            out,
            "[dev]\r\nuniverse_id = 100\r\nplaces.main = 1001\r\nplaces.lobby = 1002\r\n"
        );
    }

    #[test]
    fn insert_place_rejects_an_unknown_env() {
        let err = insert_place_str(WITH_COMMENTS, "nope", "main", 1).unwrap_err();
        assert!(err.to_string().contains("[nope]"), "got {err}");
    }

    #[test]
    fn insert_place_is_not_confused_by_the_owner_table() {
        // `[owner]` is a reserved top-level key, not an env; targeting the
        // first env must not anchor on it.
        let out = insert_place_str(WITH_COMMENTS, "dev", "lobby", 1002).unwrap();
        assert!(out.contains("[owner]\ntype = \"group\"\nid = 1234567\n"));
    }

    #[test]
    fn append_then_insert_compose() {
        let appended = append_env_str(WITH_COMMENTS, "test", 300, "main", 3001);
        let out = insert_place_str(&appended, "test", "lobby", 3002).unwrap();

        let places = parsed(&out);
        let test = places.get("test").unwrap();
        assert_eq!(test.universe_id, 300);
        assert_eq!(test.places["main"], 3001);
        assert_eq!(test.places["lobby"], 3002);
        assert_eq!(places.env_names(), vec!["dev", "prod", "test"]);
    }

    // -----------------------------------------------------------------
    // Deciding what to record
    //
    // These tests never exercise the prompt itself: every case below pins the
    // interactive branch off (via `yes`, `no_record`, an explicit flag, or a
    // missing file) so the suite can't block on stdin when `cargo test` is run
    // from a terminal.
    // -----------------------------------------------------------------

    fn temp_places(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rbxplace.toml");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn no_record_wins_over_everything() {
        let (_d, path) = temp_places(WITH_COMMENTS);
        let chosen = choose_new_env(&path, Some("test"), None, Some("X"), true, false).unwrap();
        assert_eq!(chosen, None);
    }

    #[test]
    fn explicit_env_defaults_the_place_key_to_main() {
        let (_d, path) = temp_places(WITH_COMMENTS);
        let chosen = choose_new_env(&path, Some("test"), None, None, false, false)
            .unwrap()
            .unwrap();
        assert_eq!(chosen.env, "test");
        assert_eq!(chosen.place, "main");
    }

    #[test]
    fn explicit_env_honors_a_place_override() {
        let (_d, path) = temp_places(WITH_COMMENTS);
        let chosen = choose_new_env(&path, Some("test"), Some("lobby"), None, false, false)
            .unwrap()
            .unwrap();
        assert_eq!(chosen.place, "lobby");
    }

    #[test]
    fn explicit_env_refuses_to_shadow_an_existing_one() {
        let (_d, path) = temp_places(WITH_COMMENTS);
        let err = choose_new_env(&path, Some("prod"), None, None, false, false).unwrap_err();
        assert!(err.to_string().contains("already exists"), "got {err}");
    }

    #[test]
    fn explicit_env_without_a_file_says_how_to_create_one() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("rbxplace.toml");
        let err = choose_new_env(&missing, Some("test"), None, None, false, false).unwrap_err();
        assert!(err.to_string().contains("Create it first"), "got {err}");
    }

    #[test]
    fn env_all_is_not_a_recording_target() {
        let (_d, path) = temp_places(WITH_COMMENTS);
        let err = choose_new_env(&path, Some("all"), None, None, false, false).unwrap_err();
        assert!(err.to_string().contains("--env <name>"), "got {err}");
    }

    /// `all` is not the only name rbxplace.toml spells for itself, and the
    /// list comes from rbx-core so import and init cannot refuse different
    /// ones.
    #[test]
    fn the_reserved_table_names_are_not_recording_targets_either() {
        let (_d, path) = temp_places(WITH_COMMENTS);
        for name in ["owner", "codegen"] {
            let err = choose_new_env(&path, Some(name), None, None, false, false).unwrap_err();
            assert!(err.to_string().contains("reserved"), "got {err}");
        }
    }

    #[test]
    fn yes_skips_recording_so_scripts_never_block() {
        let (_d, path) = temp_places(WITH_COMMENTS);
        assert_eq!(
            choose_new_env(&path, None, None, Some("X"), false, true).unwrap(),
            None
        );
    }

    #[test]
    fn a_missing_file_is_not_created_implicitly() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("rbxplace.toml");
        assert_eq!(
            choose_new_env(&missing, None, None, Some("X"), false, false).unwrap(),
            None
        );
    }

    #[test]
    fn place_recording_derives_the_env_from_the_universe_id() {
        let (_d, path) = temp_places(WITH_COMMENTS);
        let chosen = choose_new_place(&path, None, Some("lobby"), 200, None, false, false)
            .unwrap()
            .unwrap();
        assert_eq!(chosen.env, "prod");
        assert_eq!(chosen.place, "lobby");
    }

    #[test]
    fn place_recording_rejects_an_env_pointing_elsewhere() {
        let (_d, path) = temp_places(WITH_COMMENTS);
        // [dev] is universe 100, not 200.
        let err = choose_new_place(&path, Some("dev"), Some("lobby"), 200, None, false, false)
            .unwrap_err();
        assert!(
            err.to_string().contains("points at universe 100"),
            "got {err}"
        );
    }

    #[test]
    fn place_recording_rejects_a_duplicate_key() {
        let (_d, path) = temp_places(WITH_COMMENTS);
        let err = choose_new_place(&path, None, Some("main"), 200, None, false, false).unwrap_err();
        assert!(err.to_string().contains("already exists"), "got {err}");
    }

    #[test]
    fn place_recording_errors_when_an_explicit_universe_maps_to_nothing() {
        let (_d, path) = temp_places(WITH_COMMENTS);
        let err =
            choose_new_place(&path, None, Some("lobby"), 999, None, false, false).unwrap_err();
        assert!(err.to_string().contains("No env"), "got {err}");
    }

    #[test]
    fn place_recording_stays_quiet_when_nothing_was_asked_for() {
        let (_d, path) = temp_places(WITH_COMMENTS);
        // Unmapped universe, no flags: skip rather than fail the creation.
        assert_eq!(
            choose_new_place(&path, None, None, 999, None, false, true).unwrap(),
            None
        );
    }

    #[test]
    fn place_recording_refuses_to_guess_between_duplicate_envs() {
        let (_d, path) = temp_places("[a]\nuniverse_id = 100\n[b]\nuniverse_id = 100\n");
        let err =
            choose_new_place(&path, None, Some("lobby"), 100, None, false, false).unwrap_err();
        assert!(err.to_string().contains("Several envs"), "got {err}");
        assert!(err.to_string().contains("a, b"), "got {err}");
    }

    // -----------------------------------------------------------------
    // header parsing
    // -----------------------------------------------------------------

    #[test]
    fn header_name_reads_plain_quoted_and_dotted_forms() {
        assert_eq!(header_name("[dev]").as_deref(), Some("dev"));
        assert_eq!(
            header_name("  [dev.places]  ").as_deref(),
            Some("dev.places")
        );
        assert_eq!(header_name("[\"dev\"]").as_deref(), Some("dev"));
        assert_eq!(header_name("[[matrix]]").as_deref(), Some("matrix"));
    }

    #[test]
    fn header_name_ignores_key_lines() {
        assert_eq!(header_name("universe_id = 100"), None);
        assert_eq!(header_name("places.main = 1001"), None);
        assert_eq!(header_name(""), None);
        assert_eq!(header_name("# [dev]"), None);
    }
}
