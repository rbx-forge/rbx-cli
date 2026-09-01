//! Rendering a value for the plan a human reads, and the guards that refuse a
//! plan nobody should be shown.

use std::path::Path;

use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::config::{AvatarScales, PaidAccess, Permissions};

/// Helpers
pub(crate) fn show_opt<T: std::fmt::Display>(v: Option<T>) -> String {
    match v {
        Some(x) => x.to_string(),
        None => "(unset)".to_string(),
    }
}

/// `{:?}` for a value that has one, `(unset)` for none. The enums here are
/// small and their `Debug` spelling is the config spelling with different
/// casing, which is close enough to read in a plan.
pub(crate) fn show_debug_opt<T: std::fmt::Debug>(value: Option<T>) -> String {
    match value {
        Some(v) => format!("{v:?}"),
        None => "(unset)".to_string(),
    }
}

pub(crate) fn show_permissions(p: Option<&Permissions>) -> String {
    match p {
        Some(p) => format!(
            "teleport={} asset={} purchase={} client={}",
            p.third_party_teleport, p.third_party_asset, p.third_party_purchase, p.client_teleport
        ),
        None => "(unset)".to_string(),
    }
}

pub(crate) fn show_paid_access(p: Option<&PaidAccess>) -> String {
    match p {
        Some(PaidAccess::Free) => "free".to_string(),
        Some(PaidAccess::Paid { price }) => format!("{price} Robux"),
        None => "(unset)".to_string(),
    }
}

/// Which legacy avatar field a section of `engineAvatarSettings` also describes.
///
/// Left column is the key this crate puts in the legacy body; middle is the
/// section of the modern document that says the same thing; right is the TOML
/// the reader actually wrote, because that is what they have to change.
const AVATAR_OVERLAPS: &[(&str, &str, &str)] = &[
    (
        "universeAvatarMinScales",
        "AvatarBodyRules",
        "game.avatar.min_scale",
    ),
    (
        "universeAvatarMaxScales",
        "AvatarBodyRules",
        "game.avatar.max_scale",
    ),
    (
        "universeAvatarAssetOverrides",
        "AvatarBodyRules",
        "game.avatar.asset_overrides",
    ),
];

/// Refuse to write the legacy avatar fields and `engineAvatarSettings` at once.
///
/// `engineAvatarSettings` is not an addition to the legacy avatar fields. It is
/// a *second description of the same settings*: `AvatarBodyRules` carries
/// `CustomHeightScale = { min, max }` where the legacy body carries
/// `universeAvatarMinScales.height` and `universeAvatarMaxScales.height`, and it
/// carries per-slot `Custom*Id` where the legacy body carries
/// `universeAvatarAssetOverrides`. Sending both in one PATCH tells Roblox the
/// same thing twice, in two shapes, with nothing making them agree.
///
/// Refusing rather than warning, for a reason specific to this field: **no GET
/// returns either of them.** Neither the v1 nor the v2 read echoes the scales or
/// the settings document back, which was verified against the vendored spec
/// (zero GET operations expose `ScaleModel`). So a project that writes a
/// contradiction has no way to discover it from this tool, from the API, or
/// from the Creator Hub. It surfaces when somebody opens Studio and reads
/// `AvatarSettings Error: Failed to deserialize properties`, which is exactly
/// how this was found, on a test universe that had been sent both.
///
/// Free to impose, too, which is why it is imposed now: the avatar fields have
/// never appeared in a release, so no existing project can be running a
/// combination this rejects.
pub(crate) fn refuse_overlapping_avatar_writes(
    body: &serde_json::Map<String, Value>,
    document: &str,
    path: &Path,
) -> Result<()> {
    // The document is this crate's own compact re-serialisation, so a parse
    // failure here would be a bug rather than bad input. Treating it as "no
    // sections found" keeps a hypothetical one from turning into a refusal of
    // a config that is fine.
    let Ok(parsed) = serde_json::from_str::<Value>(document) else {
        return Ok(());
    };

    let mut clashes: Vec<String> = Vec::new();
    for (legacy_key, section, toml_key) in AVATAR_OVERLAPS {
        if body.contains_key(*legacy_key) && find_section(&parsed, section) {
            clashes.push(format!(
                "  {toml_key}  ·  also set by {section} in the document"
            ));
        }
    }
    if clashes.is_empty() {
        return Ok(());
    }
    clashes.sort();
    clashes.dedup();

    bail!(
        "`engine_avatar_settings` describes the same settings as these fields, \
         and this sync would send both:\n\n{}\n\n\
         `{}` is not an extra layer on top of the avatar fields; it is another \
         way of writing them. `AvatarBodyRules.CustomHeightScale = {{min, max}}` \
         is `min_scale.height` and `max_scale.height` in one place.\n\n\
         Keep whichever one you maintain and remove the other from rbxmeta.toml. \
         Nothing here can reconcile them for you: Roblox returns neither on read, \
         so a disagreement between the two is invisible until Studio refuses to \
         load the settings.",
        clashes.join("\n"),
        path.display()
    )
}

/// Whether `name` appears as an object key anywhere in `value`.
///
/// A search rather than a fixed path, because the known-good document nests its
/// rule sections under wrappers this crate does not model and must not depend
/// on the shape of.
pub(crate) fn find_section(value: &Value, name: &str) -> bool {
    match value {
        Value::Object(map) => map.contains_key(name) || map.values().any(|v| find_section(v, name)),
        Value::Array(items) => items.iter().any(|v| find_section(v, name)),
        _ => false,
    }
}

/// The scale object Roblox takes.
///
/// **Five of the six fields.** `Roblox.Web.Responses.Avatar.ScaleModel` also
/// declares `depth`, and this omits it, which sits awkwardly beside the reason
/// [`AvatarScales`] requires all of its own fields: that Roblox reads the
/// object whole, so a key left out is a key it may read as zero.
///
/// It is omitted on precedent rather than on principle. Mantle's
/// `ExperienceAvatarScales` carries the same five and not `depth`, and Mantle
/// wrote avatar scales against real experiences for years. That is evidence,
/// not proof, and it is the strongest available: nothing here has sent this
/// object to Roblox, and `depth` appears in no avatar scaling UI to compare
/// against.
///
/// **If a synced experience comes back with squashed avatars, this is the first
/// place to look**: add `depth` to [`AvatarScales`] as a sixth required field
/// and it will travel with the rest.
pub(crate) fn scales_json(s: &AvatarScales) -> Value {
    json!({
        "height": s.height,
        "width": s.width,
        "head": s.head,
        "bodyType": s.body_type,
        "proportion": s.proportion,
    })
}

pub(crate) fn preview(s: &str) -> String {
    const MAX: usize = 60;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        let trimmed: String = s.chars().take(MAX).collect();
        format!("{}…", trimmed)
    }
}
