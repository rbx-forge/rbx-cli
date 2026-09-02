//! `rbx env set`: write one local setting into `rbxplace.toml`.
//!
//! ## Why only these five
//!
//! Every other file in this suite can be produced from Roblox: `shop init
//! --from-remote`, `meta init --from-remote`, `config pull`. What those
//! commands cannot recover is the part of `rbxplace.toml` that has no
//! counterpart on Roblox at all:
//!
//! - `[owner]`, when the group was made on the website rather than by `rbx
//!   init create-group --record`. `create-universe` only ever *reads* this
//!   block, so nothing writes it for a group that already existed.
//! - `[codegen] output`, `[groups]`, and the per-env `codegen` / `confirm`
//!   booleans. These are policy: which module to generate, which envs to fan a
//!   command out over, which env is dangerous enough to ask first. No import
//!   can ever bring them back, because Roblox has never heard of them.
//!
//! That is the whole rule, and it is why this is not a general-purpose setter.
//! `universe_id` and `places.*` are deliberately absent: `rbx init` writes
//! those when it creates the thing they name, and a command that lets you type
//! an id by hand is a command that lets you point prod at a test universe.
//!
//! ## Why it re-parses before it writes
//!
//! A `[groups]` entry naming an env that does not exist makes the whole file
//! unloadable: `PlacesFile::validate_groups` refuses it, so *every* later
//! command fails on a file this one produced. Rather than duplicate those
//! rules here and let the two drift, the new document is built in memory,
//! handed to [`PlacesFile::parse`], and only written if it comes back clean.
//! The refusals the user sees are therefore the same ones the loader gives,
//! including its "Available: ..." list.
//!
//! ## Why an unchanged value writes nothing
//!
//! This is meant to be run from a bootstrap script that may be re-run after a
//! failure halfway through. Setting a value the file already has is reported
//! and skipped rather than rewritten, so a second run is a no-op instead of a
//! diff. Replacing a value that differs asks first, unless `--yes`.

use anyhow::{bail, Context, Result};
use colored::Colorize;
use toml_edit::{value, Array, DocumentMut, Item, Table};

use rbx_core::confirm::confirm_always;
use rbx_core::owner::Owner;
use rbx_core::places::PlacesFile;
use rbx_core::GlobalFlags;

use crate::{SetCli, SetCommands};

/// What applying a setting did, so the caller reports it without re-reading.
#[derive(Debug, PartialEq, Eq)]
enum Change {
    /// The file already says this. Nothing is written.
    Unchanged(String),
    /// The setting was absent.
    Added(String),
    /// It was there, with something else.
    Replaced { old: String, new: String },
}

pub fn run(global: &GlobalFlags, cli: SetCli) -> Result<()> {
    let path = &global.places;

    // The two top-level scalars may bring the file into existence: pointing a
    // fresh repository at a group that already exists is the case `rbx init
    // create-group --record` cannot serve, because there is no group to
    // create. Everything else names an env, so it needs a file that has some.
    let may_create = matches!(
        cli.what,
        SetCommands::Owner { .. } | SetCommands::CodegenOutput { .. }
    );
    if !path.exists() && !may_create {
        bail!(
            "{} does not exist, so there is no env to set this on. Create it first \
             (`rbx init create-universe --env <name>`, or `rbx import`).",
            path.display()
        );
    }

    let content = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("Failed to read {}", path.display())),
    };

    // The current state is read through the shared loader, so "what it says
    // now" is what every other command would say it says: a per-env `codegen`
    // that is absent reads as its default rather than as missing.
    let before = PlacesFile::parse(&content, path)?;
    let mut doc = content
        .parse::<DocumentMut>()
        .with_context(|| format!("Failed to parse {}", path.display()))?;

    let change = apply(&mut doc, &before, global.env.as_deref(), &cli.what)?;

    if let Change::Unchanged(what) = &change {
        println!(
            "{} already sets {}. Nothing to write.",
            path.display(),
            what.bold()
        );
        return Ok(());
    }

    // Built, then checked: a document this command would not be able to read
    // back is not one it may leave on disk.
    let rendered = doc.to_string();
    PlacesFile::parse(&rendered, path)
        .context("this change would leave rbxplace.toml in a state no rbx command can read")?;

    if let Change::Replaced { old, new } = &change {
        allow_replacement(
            &format!(
                "Replace {} with {} in {}?",
                old.bold(),
                new.bold(),
                path.display()
            ),
            cli.yes,
        )?;
    }

    std::fs::write(path, rendered)
        .with_context(|| format!("Failed to write {}", path.display()))?;

    match change {
        Change::Added(what) => println!(
            "{} {} to {}",
            "Added".green().bold(),
            what.bold(),
            path.display()
        ),
        Change::Replaced { old, new } => println!(
            "{} {} -> {} in {}",
            "Replaced".green().bold(),
            old,
            new.bold(),
            path.display()
        ),
        Change::Unchanged(_) => unreachable!("returned above"),
    }
    Ok(())
}

/// Edit the document, and say what that amounted to.
///
/// Whether there is anything to do is decided against the **effective** value,
/// which is what makes `set codegen true` on an env that never mentioned it a
/// no-op: `true` is already what the loader reports there, and writing the
/// default would be noise in the diff. See [`describe_defaulted`] for why the
/// settings that have a default then need a second question asked of them.
fn apply(
    doc: &mut DocumentMut,
    before: &PlacesFile,
    env_flag: Option<&str>,
    what: &SetCommands,
) -> Result<Change> {
    match what {
        SetCommands::Owner { kind, id } => {
            let new = Owner {
                kind: *kind,
                id: *id,
            };
            let change = describe(
                before.owner.map(|o| format!("[owner] {o}")),
                format!("[owner] {new}"),
            );
            if !matches!(change, Change::Unchanged(_)) {
                let table = ensure_table(doc, "owner");
                table["type"] = value(kind.to_string());
                // Roblox ids are far below i64::MAX; TOML has no u64.
                table["id"] = value(*id as i64);
            }
            Ok(change)
        }

        SetCommands::CodegenOutput { path } => {
            let change = describe(
                // A `[codegen]` table with no `output` reads as "not set"
                // rather than as an empty path: that is what `gen-module`
                // does with it, which is the behaviour worth agreeing with.
                before
                    .codegen
                    .as_ref()
                    .and_then(|c| c.output.as_ref())
                    .map(|out| format!("[codegen] output = \"{}\"", out.display())),
                format!("[codegen] output = \"{path}\""),
            );
            if !matches!(change, Change::Unchanged(_)) {
                ensure_table(doc, "codegen")["output"] = value(path);
            }
            Ok(change)
        }

        SetCommands::Group { name, members } => {
            if members.is_empty() {
                // Same refusal the loader gives for a group already on disk,
                // worded the same way: meeting it here or there should not
                // read as two different problems.
                bail!(
                    "group '{name}' would name no envs. A group that targets nothing is never \
                     what was meant: list its envs, comma-separated."
                );
            }
            let change = describe(
                before
                    .group(name)
                    .map(|m| format!("[groups] {name} = [{}]", quoted(m))),
                format!("[groups] {name} = [{}]", quoted(members)),
            );
            if !matches!(change, Change::Unchanged(_)) {
                let mut array = Array::new();
                for member in members {
                    array.push(member.as_str());
                }
                ensure_table(doc, "groups")[name] = value(array);
            }
            Ok(change)
        }

        SetCommands::Codegen { enabled } => {
            let env = target_env(before, env_flag)?;
            let change = describe_defaulted(
                spelled_out(doc, &env, "codegen"),
                format!("[{env}] codegen = {}", before.get(&env)?.codegen),
                format!("[{env}] codegen = {enabled}"),
            );
            if !matches!(change, Change::Unchanged(_)) {
                env_table(doc, &env)?["codegen"] = value(*enabled);
            }
            Ok(change)
        }

        SetCommands::Confirm { enabled } => {
            let env = target_env(before, env_flag)?;
            let change = describe_defaulted(
                spelled_out(doc, &env, "confirm"),
                format!("[{env}] confirm = {}", before.get(&env)?.confirm()),
                format!("[{env}] confirm = {enabled}"),
            );
            if !matches!(change, Change::Unchanged(_)) {
                env_table(doc, &env)?["confirm"] = value(*enabled);
            }
            Ok(change)
        }
    }
}

/// Which of the three outcomes a write is, from the old rendering and the new.
///
/// For the settings modelled as an `Option`: absent really is absent, so the
/// question of a default does not arise.
fn describe(old: Option<String>, new: String) -> Change {
    match old {
        Some(old) if old == new => Change::Unchanged(new),
        Some(old) => Change::Replaced { old, new },
        None => Change::Added(new),
    }
}

/// The same three outcomes for a setting that has a default.
///
/// `codegen` and `confirm` are never *absent* as far as the loader is
/// concerned: an env that does not mention `codegen` still reports `true`. So
/// two different questions have to be asked, and both matter. The **effective**
/// value decides whether there is anything to do, which is what makes `set
/// codegen true` on an env that never mentioned it a no-op. Whether the key is
/// **written in the file** decides whether this is an addition or an overwrite,
/// which is what stops `set confirm true` on a fresh env from stopping to ask
/// permission to replace a `false` nobody ever typed.
fn describe_defaulted(spelled_out: bool, old: String, new: String) -> Change {
    if old == new {
        Change::Unchanged(new)
    } else if spelled_out {
        Change::Replaced { old, new }
    } else {
        Change::Added(new)
    }
}

/// Whether the env's table actually carries this key, as opposed to inheriting
/// its default.
fn spelled_out(doc: &DocumentMut, env: &str, key: &str) -> bool {
    doc.get(env)
        .and_then(|table| table.as_table_like())
        .is_some_and(|table| table.contains_key(key))
}

fn quoted(members: &[String]) -> String {
    members
        .iter()
        .map(|m| format!("\"{m}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The single env `--env` names, refusing the plural selectors.
///
/// `all` and a group name both resolve to several envs, and there is no
/// sensible way to set one env's `confirm` on all of them at once: the flag
/// people leave set in a shell for a whole session must not silently fan a
/// write out. [`rbx_core::places::EnvSelector::single`] is the same refusal every other
/// single-env command gives.
fn target_env(places: &PlacesFile, env_flag: Option<&str>) -> Result<String> {
    let value = env_flag
        .ok_or_else(|| anyhow::anyhow!("this setting belongs to one env. Pass --env <name>."))?;
    Ok(places.selector(value)?.single("envs")?.to_string())
}

/// A top-level table, created as an explicit one when it is absent.
///
/// `Table::new()` rather than an implicit table, so a fresh block renders as
/// `[owner]` rather than being folded into a dotted key somewhere above it.
fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Item {
    if !doc.contains_key(key) {
        doc[key] = Item::Table(Table::new());
    }
    &mut doc[key]
}

/// The env's own table, which the loader has already proved is there.
fn env_table<'a>(doc: &'a mut DocumentMut, env: &str) -> Result<&'a mut Item> {
    if !doc.contains_key(env) {
        bail!("[{env}] is not a table in rbxplace.toml");
    }
    Ok(&mut doc[env])
}

/// Ask before overwriting, and refuse rather than hang when nobody can answer.
///
/// A bare `confirm_always` off a terminal fails inside `dialoguer` with an IO
/// error about a terminal, which tells somebody reading CI logs nothing they
/// can act on. The same fix `rbx config`'s `resolve_message` carries: name the
/// flag that answers the question.
fn allow_replacement(prompt: &str, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    if !rbx_core::output::is_interactive() {
        bail!("{prompt}\n  Pass --yes to replace it. There is nobody to ask here.");
    }
    confirm_always(prompt, false)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::path::Path;

    use super::*;
    use rbx_core::owner::OwnerType;

    fn doc_of(content: &str) -> (DocumentMut, PlacesFile) {
        (
            content.parse::<DocumentMut>().unwrap(),
            PlacesFile::parse(content, Path::new("rbxplace.toml")).unwrap(),
        )
    }

    fn apply_to(content: &str, env: Option<&str>, what: SetCommands) -> (String, Change) {
        let (mut doc, before) = doc_of(content);
        let change = apply(&mut doc, &before, env, &what).unwrap();
        (doc.to_string(), change)
    }

    // ---------------------------------------------------------------
    // owner
    // ---------------------------------------------------------------

    /// The case this command exists for: a group made on the website, and a
    /// repository with nothing in it yet.
    #[test]
    fn owner_is_written_into_an_empty_document() {
        let (out, change) = apply_to(
            "",
            None,
            SetCommands::Owner {
                kind: OwnerType::Group,
                id: 1234567890,
            },
        );
        assert!(matches!(change, Change::Added(_)));
        assert!(out.contains("[owner]"), "{out}");
        assert!(out.contains("type = \"group\""), "{out}");
        assert!(out.contains("id = 1234567890"), "{out}");

        let parsed = PlacesFile::parse(&out, Path::new("p")).unwrap();
        assert_eq!(parsed.owner.unwrap().id, 1234567890);
        assert_eq!(parsed.owner.unwrap().kind, OwnerType::Group);
    }

    /// Re-running a bootstrap script must not produce a diff.
    #[test]
    fn setting_the_owner_it_already_has_changes_nothing() {
        let before = "[owner]\ntype = \"group\"\nid = 1234567890\n";
        let (out, change) = apply_to(
            before,
            None,
            SetCommands::Owner {
                kind: OwnerType::Group,
                id: 1234567890,
            },
        );
        assert!(matches!(change, Change::Unchanged(_)));
        assert_eq!(out, before, "an unchanged value must not rewrite the file");
    }

    #[test]
    fn a_different_owner_is_reported_as_a_replacement_naming_both() {
        let (out, change) = apply_to(
            "[owner]\ntype = \"group\"\nid = 111\n",
            None,
            SetCommands::Owner {
                kind: OwnerType::User,
                id: 222,
            },
        );
        match change {
            Change::Replaced { old, new } => {
                assert!(old.contains("group 111"), "{old}");
                assert!(new.contains("user 222"), "{new}");
            }
            other => panic!("expected a replacement, got {other:?}"),
        }
        assert!(out.contains("type = \"user\""), "{out}");
        assert!(out.contains("id = 222"), "{out}");
    }

    #[test]
    fn writing_the_owner_keeps_the_comments_around_it() {
        let before = "\
# Our envs. Ask before touching prod.

[prod]
universe_id = 200
places.main = 2001
";
        let (out, _) = apply_to(
            before,
            None,
            SetCommands::Owner {
                kind: OwnerType::Group,
                id: 7,
            },
        );
        assert!(
            out.starts_with("# Our envs. Ask before touching prod."),
            "{out}"
        );
        assert!(out.contains("universe_id = 200"), "{out}");
    }

    // ---------------------------------------------------------------
    // codegen output
    // ---------------------------------------------------------------

    #[test]
    fn codegen_output_is_added_and_then_left_alone() {
        let (out, change) = apply_to(
            "[prod]\nuniverse_id = 1\n",
            None,
            SetCommands::CodegenOutput {
                path: "generated/shared/Environments.luau".into(),
            },
        );
        assert!(matches!(change, Change::Added(_)));
        assert!(out.contains("[codegen]"), "{out}");

        let (_, again) = apply_to(
            &out,
            None,
            SetCommands::CodegenOutput {
                path: "generated/shared/Environments.luau".into(),
            },
        );
        assert!(matches!(again, Change::Unchanged(_)));
    }

    // ---------------------------------------------------------------
    // groups
    // ---------------------------------------------------------------

    #[test]
    fn a_group_is_written_as_an_array_of_env_names() {
        let (out, change) = apply_to(
            "[dev]\nuniverse_id = 1\n\n[prod]\nuniverse_id = 2\n",
            None,
            SetCommands::Group {
                name: "shops".into(),
                members: vec!["dev".into(), "prod".into()],
            },
        );
        assert!(matches!(change, Change::Added(_)));

        let parsed = PlacesFile::parse(&out, Path::new("p")).unwrap();
        assert_eq!(parsed.group("shops").unwrap(), ["dev", "prod"]);
    }

    /// The reason the command re-parses before writing: this document loads
    /// nowhere, so it must never reach the disk. The refusal is the loader's
    /// own, which is what makes it name the available envs.
    #[test]
    fn a_group_naming_an_absent_env_is_caught_by_the_loader() {
        let (out, _) = apply_to(
            "[dev]\nuniverse_id = 1\n",
            None,
            SetCommands::Group {
                name: "shops".into(),
                members: vec!["dev".into(), "qa".into()],
            },
        );
        let err = PlacesFile::parse(&out, Path::new("rbxplace.toml"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("qa"), "{err}");
        assert!(err.contains("not an env in this file"), "{err}");
    }

    #[test]
    fn an_empty_group_is_refused_before_anything_is_edited() {
        let (mut doc, before) = doc_of("[dev]\nuniverse_id = 1\n");
        let err = apply(
            &mut doc,
            &before,
            None,
            &SetCommands::Group {
                name: "shops".into(),
                members: vec![],
            },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("would name no envs"), "{err}");
    }

    // ---------------------------------------------------------------
    // per-env booleans
    // ---------------------------------------------------------------

    /// An env that never mentioned `codegen` is having the key *added*, not a
    /// `true` replaced: nothing is being overwritten, so a bootstrap script
    /// must not have to pass `--yes` for it.
    #[test]
    fn codegen_false_is_written_onto_the_named_env_only() {
        let (out, change) = apply_to(
            "[ci]\nuniverse_id = 1\n\n[prod]\nuniverse_id = 2\n",
            Some("ci"),
            SetCommands::Codegen { enabled: false },
        );
        assert!(matches!(change, Change::Added(_)), "got {change:?}");

        let parsed = PlacesFile::parse(&out, Path::new("p")).unwrap();
        assert!(!parsed.get("ci").unwrap().codegen);
        assert!(
            parsed.get("prod").unwrap().codegen,
            "prod must be untouched"
        );
    }

    /// `codegen` defaults to true, so setting it to true on an env that never
    /// mentioned it is a no-op rather than a line of noise in the diff.
    #[test]
    fn setting_a_boolean_to_its_default_writes_nothing() {
        let before = "[ci]\nuniverse_id = 1\n";
        let (out, change) = apply_to(before, Some("ci"), SetCommands::Codegen { enabled: true });
        assert!(matches!(change, Change::Unchanged(_)));
        assert_eq!(out, before);
    }

    /// The other half of the same rule: a value written in the file *is*
    /// overwritten, and that one does ask first.
    #[test]
    fn overwriting_a_boolean_the_file_spells_out_is_a_replacement() {
        let (_, change) = apply_to(
            "[ci]\nuniverse_id = 1\ncodegen = false\n",
            Some("ci"),
            SetCommands::Codegen { enabled: true },
        );
        match change {
            Change::Replaced { old, new } => {
                assert!(old.contains("codegen = false"), "{old}");
                assert!(new.contains("codegen = true"), "{new}");
            }
            other => panic!("expected a replacement, got {other:?}"),
        }
    }

    #[test]
    fn confirm_true_lands_on_prod() {
        let (out, _) = apply_to(
            "[prod]\nuniverse_id = 2\n",
            Some("prod"),
            SetCommands::Confirm { enabled: true },
        );
        let parsed = PlacesFile::parse(&out, Path::new("p")).unwrap();
        assert!(parsed.get("prod").unwrap().confirm());
    }

    /// `--env all` and a group name both name several envs. Setting one env's
    /// flag on all of them because a shell had `--env all` exported is exactly
    /// the accident `single` exists to stop.
    #[test]
    fn a_plural_env_selector_is_refused() {
        let content = "\
[groups]
nonprod = [\"dev\"]

[dev]
universe_id = 1

[prod]
universe_id = 2
";
        // `all` and a group name are the two shapes of "several envs", and
        // both have to be refused: a group is exactly as plural as `all`.
        for selector in ["all", "nonprod"] {
            let (mut doc, before) = doc_of(content);
            let err = apply(
                &mut doc,
                &before,
                Some(selector),
                &SetCommands::Confirm { enabled: true },
            )
            .unwrap_err()
            .to_string();
            assert!(
                err.contains("one env") || err.contains("acts on one"),
                "{err}"
            );
        }
    }

    #[test]
    fn a_missing_env_flag_says_which_flag_is_missing() {
        let (mut doc, before) = doc_of("[dev]\nuniverse_id = 1\n");
        let err = apply(
            &mut doc,
            &before,
            None,
            &SetCommands::Codegen { enabled: false },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("--env"), "{err}");
    }
}
