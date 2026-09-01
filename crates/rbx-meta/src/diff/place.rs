//! What changes on the root place, as opposed to the universe around it.

use serde_json::{json, Value};

use crate::config::{Game, ServerFill};
use crate::lockfile::GameLock;

use super::*;

pub(crate) fn build_place_legacy_patch(game: &Game, lock: &GameLock) -> Option<PlaceLegacyPatch> {
    let mut body = serde_json::Map::new();
    let mut descriptions: Vec<String> = Vec::new();

    if let Some(desired) = &game.server_fill {
        if lock.server_fill.as_ref() != Some(desired) {
            body.insert(
                "socialSlotType".to_string(),
                json!(desired.social_slot_type()),
            );
            if let Some(count) = desired.custom_count() {
                body.insert("customSocialSlotsCount".to_string(), json!(count));
            }
            descriptions.push(format!(
                "server_fill: {} → {:?}",
                lock.server_fill
                    .as_ref()
                    .map(format_server_fill)
                    .unwrap_or_else(|| "(unset)".to_string()),
                format_server_fill(desired)
            ));
        }
    }

    if let Some(desired) = game.allow_copying {
        if lock.allow_copying != Some(desired) {
            body.insert("copyingAllowed".to_string(), json!(desired));
            descriptions.push(format!(
                "allow_copying: {} → {}",
                show_opt(lock.allow_copying),
                desired
            ));
        }
    }

    if body.is_empty() {
        None
    } else {
        Some(PlaceLegacyPatch {
            body: Value::Object(body),
            descriptions,
        })
    }
}

pub(crate) fn format_server_fill(sf: &ServerFill) -> String {
    match sf {
        ServerFill::Automatic => "automatic".to_string(),
        ServerFill::Empty => "empty".to_string(),
        ServerFill::Custom { reserved_slots } => format!("custom(reserved={})", reserved_slots),
    }
}

pub(crate) fn build_place_patch(game: &Game, lock: &GameLock) -> Option<PlacePatch> {
    let mut body = serde_json::Map::new();
    let mut mask: Vec<&'static str> = Vec::new();
    let mut descriptions: Vec<String> = Vec::new();

    if let Some(name) = &game.name {
        if lock.name.as_ref() != Some(name) {
            body.insert("displayName".to_string(), json!(name));
            mask.push("displayName");
            descriptions.push(format!(
                "name: {} → {}",
                lock.name.as_deref().unwrap_or("(unset)"),
                name
            ));
        }
    }

    if let Some(description) = &game.description {
        if lock.description.as_ref() != Some(description) {
            body.insert("description".to_string(), json!(description));
            mask.push("description");
            descriptions.push(format!(
                "description: {} → {}",
                preview(lock.description.as_deref().unwrap_or("(unset)")),
                preview(description)
            ));
        }
    }

    if let Some(size) = game.server_size {
        if lock.server_size != Some(size) {
            body.insert("serverSize".to_string(), json!(size));
            mask.push("serverSize");
            descriptions.push(format!(
                "server size: {} → {}",
                show_opt(lock.server_size),
                size
            ));
        }
    }

    if mask.is_empty() {
        None
    } else {
        Some(PlacePatch {
            body: Value::Object(body),
            mask,
            descriptions,
        })
    }
}
