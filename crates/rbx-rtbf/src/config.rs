//! `rbxrtbf.toml`: user-editable, safe to commit, the desired set of templates.
//!
//! Deliberately flat. The whole file is the two template arrays, with no
//! `[settings]`, no `[experience]` and no `[envs.*]`: there is nothing to
//! configure about a template beyond the template, and the universe comes from
//! `--env` through `rbxplace.toml` like everywhere else in this suite.

use std::path::Path;

use anyhow::{Context, Result};

use crate::model::Templates;

/// The file's name, and the default value of `--config`.
pub const FILE: &str = "rbxrtbf.toml";

/// What `rbx rtbf init` writes.
///
/// Commented rather than bare, and the comments carry the two facts that are
/// invisible from the shape of the file: the token is case-sensitive, and a
/// pattern that matches nothing is accepted and silently deletes nothing.
pub const TEMPLATE: &str = r#"# rbxrtbf.toml
#
# Which data store keys hold a user's data, so Roblox can delete them when a
# right-to-be-forgotten request arrives. Push with `rbx rtbf sync --env prod`.
#
# `{UserId}` is replaced with the requester's id. It is CASE-SENSITIVE:
# `{userId}` is stored happily by Roblox and matches nothing at all. That is the
# failure this file exists to prevent, and `rbx rtbf verify` is what proves a
# pattern matches something real before a legal request depends on it.

# A key inside a named store. `store` is an exact name, not a pattern.
# [[key]]
# store = "PlayerInventory"
# pattern = "User_{UserId}"
# scope = "Scope_{UserId}"   # optional; omitted means the default scope, "global"

# An ordered data store, same shape.
# [[key]]
# store = "PlayerLeaderboard"
# pattern = "User_{UserId}"
# ordered = true

# A whole store, named by pattern. Standard stores only: Roblox does not support
# deleting an entire ordered store.
# [[store]]
# pattern = "Player_{UserId}_Save"
"#;

/// What `load` read: the declaration, and the root tables it gave no meaning to.
///
/// The second field is not decoration. It is the only thing that tells an empty
/// declaration somebody wrote on purpose from an empty declaration a one
/// character typo produced, and `sync` cannot be safe without that distinction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// What the file declares, as this build reads it.
    pub templates: Templates,
    /// Root tables this build reads nothing from, sorted. See
    /// [`unknown_root_keys`].
    pub unknown_root_keys: Vec<String>,
}

impl Loaded {
    /// Refuse a declaration a misspelled table emptied, before anything acts on
    /// it.
    ///
    /// The distinction this rests on: an empty file has no unknown root keys,
    /// while `[[keys]]` (the plural, which is what the Rust field is called and
    /// what the `--json` output shows) has one. So an empty file stays the
    /// legitimate declaration `load` describes below, and only an empty
    /// declaration arrived at by typo is refused.
    ///
    /// Nothing else catches it, which is why this exists rather than being
    /// left to the layers that look like they would. `Templates::default()`
    /// validates clean on purpose (`model.rs:772`), `sync`'s read-before-write
    /// bails only on live templates this build cannot parse
    /// (`commands/sync.rs:66-78`), and `--yes` returns from the prompt whose
    /// `0 templates` count was the last remaining signal
    /// (`rbx-core/src/confirm.rs:27-32`). What that left was
    /// `user_data_templates: []` published over a live compliance artefact,
    /// exit 0, green in CI.
    pub fn refuse_if_emptied_by_a_typo(&self, path: &Path) -> Result<()> {
        if !self.templates.is_empty() || self.unknown_root_keys.is_empty() {
            return Ok(());
        }
        anyhow::bail!(
            "{} declares no templates, and names {} root table(s) this release gives no \
             meaning to: {}. The tables this file is made of are {}, and a plural or a \
             misspelling parses to nothing at all.\n  \
             Refused before anything acted on it: an empty declaration replaces the whole \
             published set, and Roblox's only undo is restoring a revision. Fix the \
             spelling, or delete the table if declaring nothing was the intent.",
            path.display(),
            self.unknown_root_keys.len(),
            self.unknown_root_keys.join(", "),
            ROOT_KEYS.join(", ")
        );
    }
}

/// Read the file. An absent file is an error, not an empty set.
///
/// The distinction matters for `sync`: an empty file is a legitimate declaration
/// meaning "no templates", and publishing it clears whatever was there. A
/// missing file is somebody in the wrong directory, and treating it as empty
/// would offer to delete every template they have.
///
/// The unknown root keys come back with the templates rather than being warned
/// about and dropped, because a caller that is about to publish has to be able
/// to tell those two empty declarations apart: see
/// [`Loaded::refuse_if_emptied_by_a_typo`].
pub fn load(path: &Path) -> Result<Loaded> {
    let text = std::fs::read_to_string(path).with_context(|| {
        format!(
            "Failed to read {}. Run `rbx rtbf init` to write one, or --config to point elsewhere.",
            path.display()
        )
    })?;
    let templates: Templates =
        toml::from_str(&text).with_context(|| format!("Failed to parse {}", path.display()))?;
    let unknown_root_keys = unknown_root_keys(&text);
    warn_unknown_root_keys(path, &unknown_root_keys);
    Ok(Loaded {
        templates,
        unknown_root_keys,
    })
}

/// The two tables this file gives meaning to.
const ROOT_KEYS: &[&str] = &["key", "store"];

/// Top-level keys in `content` that this build reads nothing from, sorted.
///
/// Public, pure, and taking the text rather than a path, the way
/// `rbx_shop::config::unknown_root_keys` (`crates/rbx-shop/src/config.rs:36-47`)
/// is: this is the half with a rule in it, and the same logic inlined into an
/// `eprintln!` was a rule no test could reach.
///
/// Read off the raw document rather than the deserialized struct, so the answer
/// does not depend on which `#[serde]` attributes happen to be on `Templates`
/// today.
pub fn unknown_root_keys(content: &str) -> Vec<String> {
    let Ok(toml::Value::Table(root)) = content.parse::<toml::Value>() else {
        // The typed parse in `load` already failed or will; it owns that
        // message, and it can give a line number this cannot.
        return Vec::new();
    };
    let mut found: Vec<String> = root
        .keys()
        .filter(|key| !ROOT_KEYS.contains(&key.as_str()))
        .cloned()
        .collect();
    found.sort();
    found
}

/// Name a root table this build does not read, on stderr, without refusing.
///
/// **Warned, not rejected**, which is the position stated in `rbx-schema`'s
/// module docs and held by a test: a key from a newer release has to stay
/// loadable, or adopting a field would mean upgrading every machine in the same
/// instant. `deny_unknown_fields` here would also make the generated schema
/// stricter than the tool, which trains people to stop reading the squiggles.
///
/// This is not the guard, and its doc comment used to claim it was. A line on
/// stderr does not change an exit status and `--yes` reads no output, so the
/// refusal lives in [`Loaded::refuse_if_emptied_by_a_typo`]. What is left here
/// is the case that refusal deliberately allows: real templates and a table
/// from a newer release in the same file, where the only thing owed to the
/// reader is being told which lines were ignored.
fn warn_unknown_root_keys(path: &Path, unknown: &[String]) {
    if unknown.is_empty() {
        return;
    }
    eprintln!(
        "warning: {} has {} key(s) this release gives no meaning to: {}.\n  \
         known tables: {}. Either one is misspelled, or it comes from a newer \
         `rbx`. Nothing in it is published.",
        path.display(),
        unknown.len(),
        unknown.join(", "),
        ROOT_KEYS.join(", ")
    );
}

/// Write the file, replacing it whole.
///
/// `pull` is the only caller, and it is authoritative by construction: what
/// Roblox has is what the file should say. There is no comment-preserving edit
/// here (the `toml_edit` dance `rbx meta pull` performs) because there is no
/// partial update to make: the templates are the file.
pub fn save(path: &Path, templates: &Templates) -> Result<()> {
    let body = toml::to_string_pretty(templates).context("serialising the templates")?;
    let text = format!("{}\n{}", HEADER.trim_end(), body);
    std::fs::write(path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// Written above a pulled file, because a file this tool generated should say so
/// rather than look hand-written.
const HEADER: &str = "# rbxrtbf.toml, written by `rbx rtbf pull`.\n\
                      #\n\
                      # `{UserId}` is case-sensitive. `rbx rtbf verify` checks these against the\n\
                      # data stores that actually exist.\n";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KeyTemplate, StoreTemplate};

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn the_init_template_is_a_valid_empty_file() {
        let d = dir();
        let path = d.path().join(FILE);
        std::fs::write(&path, TEMPLATE).unwrap();

        let loaded = load(&path).expect("the template must parse");
        assert!(
            loaded.templates.is_empty(),
            "every example is commented out, so a fresh file declares nothing"
        );
        loaded.templates.validate().expect("and validates");
        assert!(
            loaded.unknown_root_keys.is_empty(),
            "and names nothing this build ignores, or `sync` would refuse the file it wrote"
        );
    }

    #[test]
    fn a_file_round_trips_through_save_and_load() {
        let d = dir();
        let path = d.path().join(FILE);
        let before = Templates {
            keys: vec![
                KeyTemplate {
                    store: "PlayerInventory".into(),
                    pattern: "User_{UserId}".into(),
                    scope: Some("Scope_{UserId}".into()),
                    ordered: false,
                },
                KeyTemplate {
                    store: "PlayerLeaderboard".into(),
                    pattern: "User_{UserId}".into(),
                    scope: None,
                    ordered: true,
                },
            ],
            stores: vec![StoreTemplate {
                pattern: "Player_{UserId}_Save".into(),
            }],
        };

        save(&path, &before).expect("write");
        assert_eq!(load(&path).expect("read").templates, before);
    }

    /// A missing file must not read as "no templates": `sync` would then offer
    /// to clear every template the universe has, from the wrong directory.
    #[test]
    fn an_absent_file_is_an_error_that_names_init() {
        let d = dir();
        let err = format!("{:#}", load(&d.path().join(FILE)).unwrap_err());
        assert!(err.contains("rbx rtbf init"), "{err}");
    }

    /// Collected off the raw document, so a test can hold the rule rather than
    /// hold an `eprintln!`. The unparseable case is the reason for the early
    /// return: the typed parse owns that failure and can give a line number.
    #[test]
    fn unknown_root_keys_names_what_this_build_ignores_and_nothing_else() {
        assert_eq!(
            unknown_root_keys("[[keys]]\nstore = \"A\"\n"),
            vec!["keys".to_string()]
        );
        // Sorted, so the message is stable whatever order the file used. A
        // field inside a known table is not a root key and must not be listed:
        // `deny_unknown_fields` on the template structs owns that one, with a
        // line number.
        assert_eq!(
            unknown_root_keys("zebra = 1\napple = 2\n[[key]]\nstore = \"A\"\n"),
            vec!["apple".to_string(), "zebra".to_string()]
        );
        assert!(unknown_root_keys("[[key]]\nstore = \"A\"\n").is_empty());
        assert!(unknown_root_keys("").is_empty());
        assert!(
            unknown_root_keys("this is not toml at all").is_empty(),
            "the typed parse owns that message; guessing here would double it"
        );
    }

    /// Named, not refused: a key from a newer release has to stay loadable.
    /// What matters is that the caller can see it, because `[[keys]]` parses to
    /// an empty declaration and `sync --yes` would publish that as a wipe.
    #[test]
    fn an_unknown_root_key_is_named_and_the_file_still_loads() {
        let d = dir();
        let path = d.path().join(FILE);
        std::fs::write(
            &path,
            "[[keys]]\nstore = \"A\"\npattern = \"User_{UserId}\"\n",
        )
        .unwrap();

        let loaded = load(&path).expect("a newer release's key must not break the load");
        assert!(
            loaded.templates.is_empty(),
            "the misspelled table declares nothing, which is why naming it matters"
        );
        assert_eq!(loaded.unknown_root_keys, vec!["keys".to_string()]);
    }

    /// The one character typo, and the whole point of handing the keys back:
    /// `[[keys]]` is an empty declaration nobody asked for.
    #[test]
    fn a_declaration_emptied_by_a_typo_is_refused_and_names_both_spellings() {
        let d = dir();
        let path = d.path().join(FILE);
        std::fs::write(
            &path,
            "[[keys]]\nstore = \"A\"\npattern = \"User_{UserId}\"\n",
        )
        .unwrap();

        let loaded = load(&path).unwrap();
        let err = format!(
            "{:#}",
            loaded.refuse_if_emptied_by_a_typo(&path).unwrap_err()
        );
        assert!(err.contains("keys"), "the misspelling must be named: {err}");
        assert!(
            err.contains("key, store"),
            "and what it should have been: {err}"
        );
    }

    /// The refusal must not cost the design recorded on `load`: an empty file
    /// is a legitimate declaration meaning "delete nothing", and it has no
    /// unknown root keys, which is exactly what makes the distinction safe.
    #[test]
    fn an_empty_file_and_a_readable_one_pass_the_refusal() {
        let d = dir();
        let path = d.path().join(FILE);

        std::fs::write(&path, "").unwrap();
        load(&path)
            .unwrap()
            .refuse_if_emptied_by_a_typo(&path)
            .expect("an empty file declares nothing on purpose");

        // A table from a newer release alongside real templates: nothing is
        // being emptied, so this loads, warns and publishes what it read.
        std::fs::write(
            &path,
            "[[key]]\nstore = \"A\"\npattern = \"User_{UserId}\"\n\n[[wildcard]]\nx = 1\n",
        )
        .unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.unknown_root_keys, vec!["wildcard".to_string()]);
        loaded
            .refuse_if_emptied_by_a_typo(&path)
            .expect("a newer release's table next to real templates is not a wipe");
    }

    #[test]
    fn an_unknown_field_inside_a_template_is_refused() {
        let d = dir();
        let path = d.path().join(FILE);
        // `deny_unknown_fields` on the template structs: a misspelled
        // `patern` would otherwise be a template that silently declares
        // nothing, which is the whole class of bug this crate exists for.
        std::fs::write(
            &path,
            "[[key]]\nstore = \"A\"\npatern = \"User_{UserId}\"\n",
        )
        .unwrap();
        let err = format!("{:#}", load(&path).unwrap_err());
        assert!(err.contains("Failed to parse"), "{err}");
    }

    #[test]
    fn an_empty_file_is_a_declaration_of_no_templates() {
        let d = dir();
        let path = d.path().join(FILE);
        std::fs::write(&path, "").unwrap();
        assert!(load(&path)
            .expect("an empty file is valid")
            .templates
            .is_empty());
    }
}
