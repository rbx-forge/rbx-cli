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

/// Read the file. An absent file is an error, not an empty set.
///
/// The distinction matters for `sync`: an empty file is a legitimate declaration
/// meaning "no templates", and publishing it clears whatever was there. A
/// missing file is somebody in the wrong directory, and treating it as empty
/// would offer to delete every template they have.
pub fn load(path: &Path) -> Result<Templates> {
    let text = std::fs::read_to_string(path).with_context(|| {
        format!(
            "Failed to read {}. Run `rbx rtbf init` to write one, or --config to point elsewhere.",
            path.display()
        )
    })?;
    let templates: Templates =
        toml::from_str(&text).with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(templates)
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

        let templates = load(&path).expect("the template must parse");
        assert!(
            templates.is_empty(),
            "every example is commented out, so a fresh file declares nothing"
        );
        templates.validate().expect("and validates");
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
        assert_eq!(load(&path).expect("read"), before);
    }

    /// A missing file must not read as "no templates": `sync` would then offer
    /// to clear every template the universe has, from the wrong directory.
    #[test]
    fn an_absent_file_is_an_error_that_names_init() {
        let d = dir();
        let err = format!("{:#}", load(&d.path().join(FILE)).unwrap_err());
        assert!(err.contains("rbx rtbf init"), "{err}");
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
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
        assert!(load(&path).expect("an empty file is valid").is_empty());
    }
}
