//! The differential algorithm: which of the base block and the env overlay a
//! remote value belongs in.

use serde::Serialize;

use crate::config::SocialLink;

/// A value as `rbxmeta.toml` spells it, for the summary `pull` prints.
///
/// `Debug` printed the Rust names (`Limited`, `{Playtesters}`, `R15`), which
/// are not what the user finds in the file a moment later. Going through the
/// same serde attributes that read the file gives the spelling they will see.
/// `Debug` stays as the fallback for a value TOML cannot represent.
pub(super) fn toml_spelling<T: Serialize + std::fmt::Debug>(v: &T) -> String {
    match toml::Value::try_from(v) {
        Ok(value) => value.to_string(),
        Err(_) => format!("{v:?}"),
    }
}

/// Apply differential algorithm for an `Option<T>` field.
///   - remote=None: do nothing (we don't pull "absence")
///   - base=None && remote=Some: promote to base, clear overlay
///   - remote==base: clear overlay
///   - else: set overlay = remote
pub(super) fn diff_apply_opt<T: Clone + PartialEq + std::fmt::Debug + Serialize>(
    base: &mut Option<T>,
    overlay: &mut Option<T>,
    remote: Option<T>,
    label: &str,
    changes: &mut Vec<String>,
) {
    let Some(r) = remote else { return };
    match base {
        None => {
            *base = Some(r.clone());
            if overlay.is_some() {
                *overlay = None;
            }
            changes.push(format!("{}: base ← {}", label, toml_spelling(&r)));
        }
        Some(b) if *b == r => {
            if overlay.take().is_some() {
                changes.push(format!("{}: cleared override (matches base)", label));
            }
        }
        Some(_) => {
            let was = overlay.replace(r.clone());
            if was.as_ref() != Some(&r) {
                changes.push(format!("{}: override ← {}", label, toml_spelling(&r)));
            }
        }
    }
}

pub(super) fn diff_apply_social(
    base: &mut Option<SocialLink>,
    overlay: &mut Option<SocialLink>,
    remote: Option<SocialLink>,
    platform: &str,
    changes: &mut Vec<String>,
) {
    let Some(r) = remote else { return };
    match base {
        None => {
            *base = Some(r.clone());
            if overlay.is_some() {
                *overlay = None;
            }
            changes.push(format!("social.{}: base ← '{}'", platform, r.title));
        }
        Some(b) if *b == r => {
            if overlay.take().is_some() {
                changes.push(format!("social.{}: cleared override", platform));
            }
        }
        Some(_) => {
            let was = overlay.replace(r.clone());
            if was.as_ref() != Some(&r) {
                changes.push(format!("social.{}: override ← '{}'", platform, r.title));
            }
        }
    }
}

#[cfg(test)]
mod spelling_tests {
    use super::toml_spelling;
    use crate::config::{Audience, AvatarType, Genre, PaidAccess, ServerFill, Visibility};
    use std::collections::BTreeSet;

    /// Every line of a real pull summary, as the file spells it.
    #[test]
    fn the_summary_uses_the_spelling_of_the_file() {
        let audience: BTreeSet<Audience> = [Audience::Playtesters].into();
        let cases = [
            (toml_spelling(&"staging".to_string()), r#""staging""#),
            (toml_spelling(&Visibility::Limited), r#""limited""#),
            (toml_spelling(&audience), r#"["playtesters"]"#),
            (toml_spelling(&false), "false"),
            (toml_spelling(&AvatarType::R15), r#""r15""#),
            (toml_spelling(&Genre::All), r#""all""#),
            (toml_spelling(&PaidAccess::Free), r#"{ mode = "free" }"#),
            (
                toml_spelling(&ServerFill::Automatic),
                r#"{ mode = "automatic" }"#,
            ),
        ];
        for (got, want) in cases {
            assert_eq!(got, want);
        }
    }
}

#[cfg(test)]
mod not_confirmed_tests {
    use super::diff_apply_opt;

    /// `None` means "not confirmed", and nothing may move on it.
    ///
    /// This is the property the `beta_mode` change relies on. That read used to
    /// fall back to the *lockfile's* value on failure and hand it here, which
    /// writes into the config and lists it under "Config updates", so a user
    /// who had edited `beta_mode` and pulled while the endpoint was down had
    /// their edit silently replaced by an old value, and was told Roblox had
    /// said so. Yielding `None` instead is only safe because of what is
    /// asserted below.
    #[test]
    fn an_unconfirmed_read_touches_neither_the_config_nor_the_report() {
        let mut base = Some(true);
        let mut overlay = Some(false);
        let mut changes = Vec::new();

        diff_apply_opt(&mut base, &mut overlay, None, "beta_mode", &mut changes);

        assert_eq!(base, Some(true), "the user's edit must survive");
        assert_eq!(overlay, Some(false), "so must their env override");
        assert!(
            changes.is_empty(),
            "nothing was read, so nothing may be reported as pulled: {changes:?}"
        );
    }

    /// The control: a value that *was* confirmed still lands, or the change
    /// above would have turned the field off rather than made it honest.
    #[test]
    fn a_confirmed_read_still_applies() {
        let mut base = Some(true);
        let mut overlay = None;
        let mut changes = Vec::new();

        diff_apply_opt(
            &mut base,
            &mut overlay,
            Some(false),
            "beta_mode",
            &mut changes,
        );

        assert_eq!(overlay, Some(false));
        assert_eq!(changes.len(), 1, "{changes:?}");
    }
}
