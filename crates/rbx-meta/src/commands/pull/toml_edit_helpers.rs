//! The primitives every write_* is built from: reach a table, set a value,
//! remove one that is no longer set.

use std::path::{Path, PathBuf};

use toml_edit::{value, Array, DocumentMut, Item, Table};

use crate::config::{Genre, ServerFill, SocialLink, Visibility};

pub(super) fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Table {
    if !doc.contains_key(key) {
        doc.insert(key, Item::Table(Table::new()));
    }
    doc[key].as_table_mut().expect("section is not a table")
}

/// Walk a dotted parent path (e.g. "envs.dev") to get a mutable subtable, then
/// create-or-fetch the child. Used for `envs.<name>.<sub>` style.
pub(super) fn ensure_subtable<'a>(
    doc: &'a mut DocumentMut,
    parent: &str,
    child: &str,
) -> &'a mut Table {
    let parts: Vec<&str> = parent.split('.').collect();
    let mut current: &mut Table = ensure_path(doc, &parts);
    if !current.contains_key(child) {
        current.insert(child, Item::Table(Table::new()));
    }
    current = current[child]
        .as_table_mut()
        .expect("subsection is not a table");
    current
}

pub(super) fn ensure_subtable_dotted<'a>(
    doc: &'a mut DocumentMut,
    parent: &str,
    child: &str,
) -> &'a mut Table {
    ensure_subtable(doc, parent, child)
}

pub(super) fn ensure_path<'a>(doc: &'a mut DocumentMut, parts: &[&str]) -> &'a mut Table {
    let mut current = doc.as_table_mut();
    for p in parts {
        if !current.contains_key(p) {
            current.insert(p, Item::Table(Table::new()));
        }
        current = current[*p]
            .as_table_mut()
            .expect("intermediate is not a table");
    }
    current
}

pub(super) fn remove_subtable(doc: &mut DocumentMut, parent: &str, child: &str) {
    let parts: Vec<&str> = parent.split('.').collect();
    let mut current = doc.as_table_mut();
    for p in &parts {
        match current.get_mut(p).and_then(|i| i.as_table_mut()) {
            Some(t) => current = t,
            None => return,
        }
    }
    current.remove(child);
}

pub(super) fn remove_subtable_dotted(doc: &mut DocumentMut, parent: &str, child: &str) {
    remove_subtable(doc, parent, child)
}

/// Assign `item` to `t[key]`, carrying over the decor of whatever was there.
///
/// `toml_edit` stores a value's surrounding trivia (the whitespace before it
/// and, crucially, any trailing `# comment`) on the value itself. A plain
/// `t[key] = value(..)` therefore drops the user's inline comment on every key
/// pull rewrites, which defeats the point of maintaining this mirror at all.
pub(super) fn set_value(t: &mut Table, key: &str, mut item: Item) {
    if let Some(decor) = t
        .get(key)
        .and_then(|i| i.as_value())
        .map(|v| v.decor().clone())
    {
        if let Some(v) = item.as_value_mut() {
            *v.decor_mut() = decor;
        }
    }
    t[key] = item;
}

pub(super) fn set_opt_str(t: &mut Table, key: &str, val: Option<&str>) {
    match val {
        Some(v) => set_value(t, key, value(v.to_string())),
        None => {
            t.remove(key);
        }
    }
}

pub(super) fn set_opt_int(t: &mut Table, key: &str, val: Option<i64>) {
    match val {
        Some(v) => set_value(t, key, value(v)),
        None => {
            t.remove(key);
        }
    }
}

pub(super) fn set_opt_bool(t: &mut Table, key: &str, val: Option<bool>) {
    match val {
        Some(v) => set_value(t, key, value(v)),
        None => {
            t.remove(key);
        }
    }
}

pub(super) fn set_opt_path(t: &mut Table, key: &str, val: Option<&Path>) {
    match val {
        Some(p) => set_value(t, key, value(path_to_toml_str(p))),
        None => {
            t.remove(key);
        }
    }
}

pub(super) fn set_path_array(t: &mut Table, key: &str, paths: &[PathBuf]) {
    if paths.is_empty() {
        t.remove(key);
    } else {
        let mut arr = Array::new();
        for p in paths {
            arr.push(path_to_toml_str(p));
        }
        set_value(t, key, value(arr));
    }
}

pub(super) fn set_social(
    doc: &mut DocumentMut,
    parent: &str,
    platform: &str,
    link: &Option<SocialLink>,
) {
    match link {
        Some(l) => {
            let social = ensure_subtable(doc, parent, "social_links");
            if !social.contains_key(platform) {
                social.insert(platform, Item::Table(Table::new()));
            }
            let platform_t = social[platform]
                .as_table_mut()
                .expect("platform entry is not a table");
            set_value(platform_t, "title", value(l.title.clone()));
            set_value(platform_t, "url", value(l.url.clone()));
        }
        None => {
            // Walk to the social_links sub-table and remove the platform.
            let parts: Vec<&str> = parent.split('.').collect();
            let mut current = doc.as_table_mut();
            for p in &parts {
                match current.get_mut(p).and_then(|i| i.as_table_mut()) {
                    Some(t) => current = t,
                    None => return,
                }
            }
            if let Some(social) = current
                .get_mut("social_links")
                .and_then(|i| i.as_table_mut())
            {
                social.remove(platform);
            }
        }
    }
}

pub(super) fn path_to_toml_str(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// The config spelling of a genre. Kept beside the other `*_str` helpers
/// rather than as a `Display` impl, because these exist to write TOML and the
/// enums are also printed by `Debug` in sync plans, where the two spellings
/// are allowed to differ.
pub(super) fn genre_str(v: Genre) -> &'static str {
    match v {
        Genre::All => "all",
        Genre::Tutorial => "tutorial",
        Genre::Scary => "scary",
        Genre::TownAndCity => "town_and_city",
        Genre::War => "war",
        Genre::Funny => "funny",
        Genre::Fantasy => "fantasy",
        Genre::Adventure => "adventure",
        Genre::SciFi => "sci_fi",
        Genre::Pirate => "pirate",
        Genre::Fps => "fps",
        Genre::Rpg => "rpg",
        Genre::Sports => "sports",
        Genre::Ninja => "ninja",
        Genre::WildWest => "wild_west",
    }
}

pub(super) fn visibility_str(v: Visibility) -> &'static str {
    match v {
        Visibility::Public => "public",
        Visibility::Private => "private",
    }
}

pub(super) fn server_fill_mode_str(sf: &ServerFill) -> &'static str {
    match sf {
        ServerFill::Automatic => "automatic",
        ServerFill::Empty => "empty",
        ServerFill::Custom { .. } => "custom",
    }
}
