//! Reading endpoints back out of this workspace's own Rust.
//!
//! A hand-rolled scanner over source text, and the most suspect thing in this
//! file for exactly that reason: it reads string literals and consts, so a URL
//! assembled from a method call is invisible to it. NOT_CALLED_ON_PURPOSE''s
//! sibling list records the calls that have already got away.
//!
//! Kept in one module so the boundary is explicit. Everything else here is
//! comparison; this is parsing, and parsing is where the silent failures are.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::paths::*;
use crate::{NON_API_HOSTS, SKIPPED_CRATES};

/// Every double-quoted literal on a line, with escapes skipped.
pub(crate) fn string_literals(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            i += 1;
            let mut literal = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i += 1;
                } else {
                    literal.push(chars[i]);
                }
                i += 1;
            }
            out.push(literal);
        }
        i += 1;
    }
    out
}

pub(crate) fn is_const_declaration(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("const ") || trimmed.starts_with("pub const ")
}

/// Blanks out every `#[cfg(test)]` item, so a test's fixed ids and mock hosts
/// are not read as endpoints this workspace calls.
///
/// It used to cut the file at the *first* `#[cfg(test)]` and keep nothing
/// after it. That is right for the `mod tests` at the bottom of a file and
/// catastrophic for a `#[cfg(test)] fn with_base_url` near the top, which is
/// where the clients of the day kept theirs: every endpoint below it (place
/// upload, rollback, download, config publish, revisions) was invisible to
/// this check while the workspace total stayed comfortably above
/// `MINIMUM_ENDPOINTS`. That is the exact erosion
/// [`every_crate_that_calls_roblox_contributes_an_endpoint`] now guards.
///
/// Removed regions are replaced by whitespace rather than deleted, so every
/// reported line number still matches the file on disk.
pub(crate) fn strip_tests(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut kept: Vec<char> = chars.clone();
    let marker: Vec<char> = "#[cfg(test)]".chars().collect();

    let mut i = 0;
    while i + marker.len() <= chars.len() {
        // Only an attribute that opens its line is one: several files mention
        // `#[cfg(test)]` in a doc comment explaining why an item is gated, and
        // blanking from there would delete real code.
        if chars[i..i + marker.len()] != marker[..] || !opens_its_line(&chars, i) {
            i += 1;
            continue;
        }
        let end = end_of_item(&chars, i + marker.len());
        for slot in kept.iter_mut().take(end).skip(i) {
            if *slot != '\n' {
                *slot = ' ';
            }
        }
        i = end;
    }
    kept.into_iter().collect()
}

/// Whether only whitespace separates `at` from the start of its line.
pub(crate) fn opens_its_line(chars: &[char], at: usize) -> bool {
    chars[..at]
        .iter()
        .rev()
        .take_while(|c| **c != '\n')
        .all(|c| c.is_whitespace())
}

/// Where the item starting at `from` ends: the matching `}` of its first
/// block, or the `;` of a blockless item such as `#[cfg(test)] use x;`.
///
/// Braces inside strings, char literals and comments do not count, which is
/// the whole reason this is a scanner and not a `find('}')`.
pub(crate) fn end_of_item(chars: &[char], from: usize) -> usize {
    let mut depth = 0usize;
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            'r' if matches!(chars.get(i + 1), Some('"') | Some('#')) => {
                match skip_raw_string(chars, i) {
                    Some(after) => i = after,
                    None => i += 1,
                }
            }
            '"' => i = skip_string(chars, i),
            '\'' => i = skip_char_literal(chars, i),
            '{' => {
                depth += 1;
                i += 1;
            }
            // A `}` before any `{` closes the block this attribute sits in,
            // which means the item ended without one (an unparsed shape).
            // Stopping here blanks the attribute and nothing else.
            '}' if depth == 0 => return i,
            '}' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return i;
                }
            }
            ';' if depth == 0 => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

/// Index just past the closing quote of the string starting at `at`.
pub(crate) fn skip_string(chars: &[char], at: usize) -> usize {
    let mut i = at + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// Index just past `r"..."` / `r#"..."#`, or `None` if this `r` starts an
/// identifier rather than a raw string.
pub(crate) fn skip_raw_string(chars: &[char], at: usize) -> Option<usize> {
    let mut i = at + 1;
    let mut hashes = 0;
    while chars.get(i) == Some(&'#') {
        hashes += 1;
        i += 1;
    }
    if chars.get(i) != Some(&'"') {
        return None;
    }
    i += 1;
    while i < chars.len() {
        if chars[i] == '"' {
            let closing = (1..=hashes).all(|n| chars.get(i + n) == Some(&'#'));
            if closing {
                return Some(i + hashes + 1);
            }
        }
        i += 1;
    }
    Some(chars.len())
}

/// Index just past a char literal, or just past the `'` when it opens a
/// lifetime (`'a`), which is not a literal and must not swallow the code
/// after it.
pub(crate) fn skip_char_literal(chars: &[char], at: usize) -> usize {
    match chars.get(at + 1) {
        Some('\\') => {
            let mut i = at + 2;
            while i < chars.len() && chars[i] != '\'' {
                i += 1;
            }
            i + 1
        }
        Some(_) if chars.get(at + 2) == Some(&'\'') => at + 3,
        _ => at + 1,
    }
}

/// The `crates/<name>` directory a file belongs to, used to scope const
/// resolution to one crate.
pub(crate) fn crate_root_of(file: &Path) -> Option<PathBuf> {
    let mut current = file.parent()?;
    loop {
        if current.join("Cargo.toml").is_file() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

pub(crate) fn collect_consts(src: &str) -> BTreeMap<String, String> {
    let mut consts = BTreeMap::new();
    for line in src.lines() {
        if !is_const_declaration(line) {
            continue;
        }
        let Some((_, after_const)) = line.trim_start().split_once("const ") else {
            continue;
        };
        let Some((name, _)) = after_const.split_once(':') else {
            continue;
        };
        if let Some(value) = string_literals(line).into_iter().next() {
            if value.starts_with("https://") {
                consts.insert(name.trim().to_string(), value);
            }
        }
    }
    consts
}

/// Where a bare `/path` lands when `ApiBase::join` is the only clue.
///
/// `ApiBase::default()` is the Open Cloud host, and that is what almost every
/// client in the workspace joins against.
pub(crate) const DEFAULT_JOIN_HOST: &str = "https://apis.roblox.com";

/// The host a `.join(...)` call resolves against, when it is not the default.
///
/// A client that talks to more than one host keeps one `ApiBase` per host:
/// `rbx-place` has `self.base` for Open Cloud and `self.develop` for the
/// `develop` family. Assuming every join meant Open Cloud attributed
/// `/v1/universes/{}/places` to `apis.roblox.com`, where it does not exist,
/// and reported drift that was not there.
///
/// The convention this relies on: a non-default base named `<name>` is fed by
/// a `const <NAME>_HOST`. That is how the crates that need a second host are
/// written, and a base whose host cannot be resolved this way falls back to
/// the default rather than being silently dropped.
pub(crate) fn join_receiver_host(line: &str, consts: &BTreeMap<String, String>) -> Option<String> {
    let before = &line[..line.find(".join(")?];
    let receiver: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if receiver.is_empty() {
        return None;
    }
    consts
        .get(&format!("{}_HOST", receiver.to_uppercase()))
        .cloned()
}

/// Resolves `format!("{BASE}/x")` and `format!("{}/x", BASE)` where `BASE` is a
/// `const` holding an absolute URL. Returns the joined absolute URL.
pub(crate) fn resolve_const_prefix(
    literal: &str,
    line: &str,
    consts: &BTreeMap<String, String>,
) -> Option<String> {
    if !literal.starts_with('{') {
        return None;
    }
    let close = literal.find('}')?;
    let name = &literal[1..close];
    let rest = &literal[close + 1..];

    let base = if name.is_empty() {
        // Positional `{}`: the const is the first argument after the literal.
        let index = line.find(literal)?;
        let after = &line[index + literal.len()..];
        let after = after.trim_start().strip_prefix('"')?.trim_start();
        let after = after.strip_prefix(',')?.trim_start();
        let arg: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        consts.get(&arg)?
    } else {
        consts.get(name)?
    };

    Some(format!("{base}{rest}"))
}

/// Splices Rust string-literal line continuations (a trailing `\`) so a URL
/// split across two source lines is seen whole. The reported line number stays
/// the line the literal started on.
pub(crate) fn logical_lines(src: &str) -> Vec<(usize, String)> {
    let raw: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let start = i;
        let mut current = raw[i].to_string();
        while current.trim_end().ends_with('\\') && i + 1 < raw.len() {
            let trimmed = current.trim_end();
            current = format!(
                "{}{}",
                &trimmed[..trimmed.len() - 1],
                raw[i + 1].trim_start()
            );
            i += 1;
        }
        out.push((start + 1, current));
        i += 1;
    }
    out
}

/// A `(host, path)` we call, and every source location that calls it.
pub(crate) type CallSites = BTreeMap<(String, String), BTreeSet<(String, usize)>>;

pub(crate) fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            // `tests/` holds paths aimed at mock servers; `target/` is build output.
            if name == "tests" || name == "target" {
                continue;
            }
            if SKIPPED_CRATES.iter().any(|(krate, _)| *krate == name) {
                continue;
            }
            collect_rust_files(&path, out);
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

pub(crate) fn collect_call_sites(root: &Path) -> CallSites {
    let mut files = Vec::new();
    collect_rust_files(&root.join("crates"), &mut files);
    files.sort();

    // Host consts are a crate-level fact, not a file-level one. A client whose
    // `ApiBase` fields live in `api/mod.rs` while its calls live in
    // `api/groups.rs` would otherwise have every host resolve to the default,
    // and `rbx-init` is exactly that shape. Collected per crate, consulted
    // after the file's own so a local const still wins and two crates using the
    // same name cannot reach across each other.
    let mut crate_consts: BTreeMap<PathBuf, BTreeMap<String, String>> = BTreeMap::new();
    for file in &files {
        let Ok(src) = fs::read_to_string(file) else {
            continue;
        };
        let src = strip_tests(&src);
        let Some(krate) = crate_root_of(file) else {
            continue;
        };
        crate_consts
            .entry(krate)
            .or_default()
            .extend(collect_consts(&src));
    }

    let mut sites = CallSites::new();
    for file in files {
        let Ok(src) = fs::read_to_string(&file) else {
            continue;
        };
        // Every `#[cfg(test)]` item is blanked out, line numbers intact.
        let src = strip_tests(&src);
        let src = src.as_str();
        let mut consts = crate_root_of(&file)
            .and_then(|k| crate_consts.get(&k).cloned())
            .unwrap_or_default();
        // The file's own declarations win over its crate's.
        consts.extend(collect_consts(src));
        let relative = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");

        let lines = logical_lines(src);
        for (position, (line_number, line)) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }

            // A bare `/path` literal becomes an absolute URL only via
            // `ApiBase::join`, which may sit up to a few lines above the
            // literal in a multi-line `format!`.
            let context_start = position.saturating_sub(3);
            let join_line = lines[context_start..=position]
                .iter()
                .rev()
                .find(|(_, l)| l.contains(".join("))
                .map(|(_, l)| l.as_str());
            let joins = join_line.is_some();
            let join_host = join_line
                .and_then(|l| join_receiver_host(l, &consts))
                .unwrap_or_else(|| DEFAULT_JOIN_HOST.to_string());

            let mut found: Vec<(String, String)> = Vec::new();
            for literal in string_literals(line) {
                let absolute = if literal.starts_with("https://") {
                    Some(literal.clone())
                } else if let Some(resolved) = resolve_const_prefix(&literal, line, &consts) {
                    Some(resolved)
                } else if joins && literal.starts_with('/') && !is_const_declaration(line) {
                    Some(format!("{join_host}{literal}"))
                } else {
                    None
                };

                let Some(absolute) = absolute else { continue };
                // A const declaration on its own is a prefix, not an endpoint.
                if is_const_declaration(line) && literal.starts_with("https://") {
                    continue;
                }
                let Some(rest) = absolute.strip_prefix("https://") else {
                    continue;
                };
                let Some((host, path)) = rest.split_once('/') else {
                    continue;
                };
                if !host.ends_with("roblox.com") {
                    continue;
                }
                let host = format!("https://{host}");
                if NON_API_HOSTS.contains(&host.as_str()) {
                    continue;
                }
                let path = format!("/{path}");
                let path = path.split(['?', '#']).next().unwrap_or(&path).to_string();
                let path = path.trim_end_matches('/').to_string();
                // A path whose first segment is an unresolved interpolation
                // could not be reconstructed; do not guess at it.
                let segments = normalise(&path);
                if segments.len() < 2 || segments[0].contains('*') {
                    continue;
                }
                found.push((host, path));
            }

            for (host, path) in found {
                sites
                    .entry((host, path))
                    .or_default()
                    .insert((relative.clone(), *line_number));
            }
        }
    }
    sites
}

/// The crate a `crates/<name>/...` call site belongs to.
pub(crate) fn crate_of(relative: &str) -> Option<&str> {
    relative.strip_prefix("crates/")?.split('/').next()
}

/// How many distinct endpoints each crate contributed.
pub(crate) fn endpoints_per_crate(sites: &CallSites) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for locations in sites.values() {
        let mut credited = BTreeSet::new();
        for (file, _) in locations {
            if let Some(krate) = crate_of(file) {
                credited.insert(krate.to_string());
            }
        }
        for krate in credited {
            *counts.entry(krate).or_insert(0) += 1;
        }
    }
    counts
}

/// Crates whose `[dependencies]` include `reqwest`: everything that can reach
/// Roblox at all.
///
/// Read from `Cargo.toml` rather than from a hand-maintained list, so a new
/// crate is in scope the day it is added instead of the day somebody remembers
/// this file. `[dev-dependencies]` does not count: a wiremock-only dependency
/// is a test fixture, not a call site.
pub(crate) fn http_crates(root: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let Ok(entries) = fs::read_dir(root.join("crates")) else {
        return found;
    };
    for entry in entries.flatten() {
        let manifest = entry.path().join("Cargo.toml");
        let Ok(text) = fs::read_to_string(&manifest) else {
            continue;
        };
        let mut in_dependencies = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_dependencies = line == "[dependencies]";
                continue;
            }
            if in_dependencies
                && (line == "reqwest"
                    || line.starts_with("reqwest ")
                    || line.starts_with("reqwest.")
                    || line.starts_with("reqwest="))
            {
                found.insert(entry.file_name().to_string_lossy().into_owned());
                break;
            }
        }
    }
    found
}
