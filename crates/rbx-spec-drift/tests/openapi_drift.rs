//! Early-warning alarm for Roblox API drift.
//!
//! This test extracts every Roblox endpoint URL this workspace actually calls
//! and asserts each one still exists in the vendored OpenAPI document at
//! `spec/openapi.json`. When Roblox moves or removes an endpoint, this test
//! goes red in CI instead of a user hitting `Failed to parse response` at
//! runtime.
//!
//! It reads source files as plain text and the spec as plain JSON. It compiles
//! against nothing in this workspace on purpose, so it keeps working while
//! other crates are mid-refactor.
//!
//! # What this checks, honestly
//!
//! The vendored document is broader than its `cloud/v2` reputation suggests.
//! Its top-level `servers` entry is `https://apis.roblox.com`, but individual
//! operations override that with per-operation `servers` blocks naming the
//! legacy hosts: `develop.roblox.com`, `games.roblox.com`,
//! `groups.roblox.com`, `badges.roblox.com`, `thumbnails.roblox.com`,
//! `users.roblox.com`, `assetdelivery.roblox.com`, `economy.roblox.com` and
//! others. So a large part of our legacy surface *is* checkable here, and this
//! test checks it. Matching is host-aware: a path is only considered present
//! if the spec documents it **on the same host** we call it on. Without that,
//! our `games.roblox.com/v1/games` would spuriously "match" an unrelated
//! `/v1/games` documented on a different host.
//!
//! # The URL shapes it recognises
//!
//! Extraction is a text scan, so it only sees the shapes it was taught. These
//! are all of them, and a call written any other way contributes **nothing**
//! to this test while still compiling and running fine:
//!
//! | shape | example |
//! | --- | --- |
//! | an absolute URL in one literal | `"https://games.roblox.com/v1/games"` |
//! | a `const` base interpolated by name | `format!("{BASE}/v1/games/{id}")` |
//! | a `const` base interpolated positionally | `format!("{}/v1/games", BASE)` |
//! | a relative path within three lines of a `.join(` | `base.join(&format!("/cloud/v2/universes/{id}"))` |
//!
//! Two supporting conventions make those work: a `const` holding an absolute
//! URL is a *prefix*, never an endpoint on its own, and a non-default
//! `ApiBase` field named `<name>` is fed by a `const <NAME>_HOST` so that
//! `self.develop.join(...)` resolves to the right host rather than to Open
//! Cloud (see `join_receiver_host`).
//!
//! **Adding a new shape is a change to this file too.** A helper that
//! concatenates path fragments, a table of `const` endpoints, a builder: none
//! of them are recognised, and the crate that introduces one silently drops
//! out of the checked set. Two guards exist for that: `MINIMUM_ENDPOINTS`
//! catches gross breakage, and
//! [`every_crate_that_calls_roblox_contributes_an_endpoint`] catches the
//! narrow case of one crate, which is how the net actually erodes.
//!
//! # What this deliberately does not check
//!
//! - **`crates/rbx-probe/`** is skipped. It exists to send a raw request to an
//!   arbitrary user-supplied path; its paths are inputs, not call sites, so
//!   there is nothing to verify against the spec.
//! - **`crates/*/tests/`** is skipped. Those paths point at wiremock servers.
//! - **`#[cfg(test)]` items** are skipped, one item at a time: the attribute
//!   and the item it guards are blanked out, and the rest of the file is still
//!   read. Unit tests assert against fixed ids (`/cloud/v2/universes/1`),
//!   which are not real endpoint templates. This used to cut the file at the
//!   first marker and keep nothing after it, which cost every client that
//!   gated a `with_base_url` helper near the top of its module the whole of
//!   its coverage: `rbx-place`'s upload, rollback and download went with one
//!   such helper (`crates/rbx-place/src/api/mod.rs:70`, no longer
//!   `#[cfg(test)]`), and so did the configs client's publish and revisions
//!   calls, which now live in `crates/rbx-core/src/api/configs.rs`.
//! - **`www.roblox.com` and `create.roblox.com`** are skipped. Every use of
//!   those is a human-facing web link (a profile URL, a dashboard link) or an
//!   `Origin`/`Referer` header, not an API call.
//! - **Endpoints assembled from a `const` base plus runtime pieces** are only
//!   resolved for the simple, single-line `format!` shapes this codebase
//!   actually uses (`format!("{BASE}/x")` and `format!("{}/x", BASE)`). A
//!   `const` declaration on its own is treated as a prefix, never as an
//!   endpoint, because most of them are (`".../universes/v1"`). This is the
//!   main blind spot; see `MINIMUM_ENDPOINTS` for the guard against it
//!   silently widening.
//! - **Undocumented-by-Roblox endpoints** are listed in `KNOWN_UNDOCUMENTED`
//!   with a reason each. They are real endpoints we call that this document
//!   has never described, so they cannot be verified against it. They are not
//!   evidence of drift, and an alarm that is red on day one gets ignored.
//!
//! # Refreshing the spec
//!
//! Do not hand-edit `spec/openapi.json`. The `update-openapi` workflow
//! re-fetches it and updates `spec/source.json`. See `spec/NOTICE.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// HTTP methods that mark a key inside an OpenAPI path item as an operation.
const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

/// Hosts we build URLs for that are never API calls.
const NON_API_HOSTS: &[&str] = &["https://www.roblox.com", "https://create.roblox.com"];

/// Crate directories excluded from extraction, with the reason.
const SKIPPED_CRATES: &[(&str, &str)] = &[
    (
        "rbx-probe",
        "builds arbitrary user-supplied paths by design; its paths are inputs, not call sites",
    ),
    (
        "rbx-spec-drift",
        "this crate; the strings below are examples",
    ),
];

/// Endpoints we call that this document has never documented. Each entry is
/// `(host, path, reason)`. Paths are matched structurally, so `{}` here means
/// "any templated segment", exactly as in the extraction.
///
/// Adding to this list is a deliberate act: it says "Roblox does not describe
/// this endpoint, so we accept that we cannot get an early warning for it".
/// Removing an entry is free, if Roblox documents one of these later, the
/// test simply starts covering it, and a stale entry costs nothing but a line.
const KNOWN_UNDOCUMENTED: &[(&str, &str, &str)] = &[
    (
        "https://apis.roblox.com",
        "/api-keys/v1/introspect",
        "API key introspection; used by `rbx apikey`, never published in the OpenAPI document",
    ),
    (
        "https://apis.roblox.com",
        "/cloud-authentication/v1/apiKey",
        "API key create/update/delete, used by `rbx apikey`; the cookie-authenticated \
         management API behind the Creator Hub credentials page, never published in the \
         OpenAPI document (same family as /api-keys/v1/introspect above)",
    ),
    (
        "https://apis.roblox.com",
        "/cloud-authentication/v1/apiKeys",
        "plural, reached with a POST: the key listing behind the same page, undocumented \
         for the same reason",
    ),
    (
        "https://apis.roblox.com",
        "/universes/v1/places/{}/universe",
        "legacy place -> universe resolution; the document only describes \
         /universes/v1/{}/places/{}/versions",
    ),
    (
        "https://apis.roblox.com",
        "/universes/v1/universes/create",
        "legacy universe creation, used by `rbx init`; undocumented",
    ),
    (
        "https://apis.roblox.com",
        "/universes/v1/user/universes/{}/places",
        "legacy place creation, used by `rbx init`; undocumented",
    ),
    (
        "https://apis.roblox.com",
        "/experience-releases/v1beta1/experience_releases_api",
        "v1beta1 release-status API behind the creator dashboard; undocumented",
    ),
    (
        "https://apis.roblox.com",
        "/experience-releases/v1beta1/experience_releases_api/release_status",
        "v1beta1 release-status API behind the creator dashboard; undocumented",
    ),
    (
        "https://apis.roblox.com",
        "/experience-releases/v1beta1/experience_releases_api/release_status/{}",
        "v1beta1 release-status API behind the creator dashboard; undocumented",
    ),
    (
        "https://apis.roblox.com",
        "/developer-products/v2/universes/{}/developerproducts",
        "unhyphenated legacy spelling used by the public (unauthenticated) listing in \
         rbx-shop/src/api/public.rs; the document only describes the hyphenated \
         /developer-products/v2/universes/{}/developer-products, which rbx-shop/src/api/products.rs \
         already uses. Worth collapsing onto the documented spelling.",
    ),
    (
        "https://apis.roblox.com",
        "/analytics-query-api/{}",
        "not a fixed endpoint: this is the long-running-operation poll in rbx-analytics, whose \
         tail is an operation path handed back by Roblox at runtime. There is no static path \
         to verify. The analytics endpoints that *are* static \
         (/analytics-query-api/v1/universes/{}/metrics and .../dimension-values) are checked.",
    ),
    (
        "https://economy.roblox.com",
        "/v2/assets/{}/details",
        "economy.roblox.com is a documented host, but this particular path is not among the \
         operations it documents (only /v2/assets/batch, /v2/assets/{}/owners, \
         /v2/assets/{}/versions)",
    ),
];

/// A floor on how many endpoints extraction must find.
///
/// Without this, a refactor that changes how URLs are written would quietly
/// reduce the test to checking nothing, and it would still pass. If this trips
/// after a legitimate change, read the extraction rules above before lowering
/// it: the usual cause is a new URL-building shape that needs supporting, not
/// a number that needs editing.
const MINIMUM_ENDPOINTS: usize = 60;

/// Crates that depend on the HTTP layer and still contribute no endpoint to
/// this check, with the reason each is legitimate.
///
/// The floor above only catches gross breakage. The failure mode in between is
/// one crate: a new domain crate builds its URLs with a shape the extractor
/// does not recognise, contributes zero, and nothing goes red: the net
/// narrows by one crate while the total stays comfortably above the floor.
/// [`every_crate_that_calls_roblox_contributes_an_endpoint`] closes that, and
/// this list is where a genuine exception is admitted out loud.
///
/// An entry here is a claim that the crate calls no Roblox endpoint of its
/// own, not that its endpoints are hard to extract. If the extractor cannot
/// see a call the crate really makes, teach the extractor.
///
/// **Empty on purpose.** It held `rbx-config` for as long as that crate owned
/// the configs client, and the entry outlived the fact: the client moved to
/// `rbx_core::api::configs`, `rbx-config` stopped touching HTTP at all, and
/// what was left was an exemption covering a crate that could not call Roblox
/// while the configs surface itself went unchecked inside a crate that passed.
/// The endpoints are covered directly now, by
/// [`every_configs_endpoint_is_documented_in_the_spec`], which is what an
/// exemption should turn into.
const CRATES_WITHOUT_ENDPOINTS: &[(&str, &str)] = &[];

// ---------------------------------------------------------------------------
// Path normalisation
// ---------------------------------------------------------------------------

/// Collapses every `{...}` run in a segment to `*`, so that our Rust variable
/// names and the spec's parameter names never have to agree.
///
/// `{universe_id}` -> `*`, `{}` -> `*`, `{entry_id}:increment` -> `*:increment`,
/// `data-stores:snapshot` -> `data-stores:snapshot`.
fn normalise_segment(segment: &str) -> String {
    let mut out = String::new();
    let mut chars = segment.chars();
    while let Some(c) = chars.next() {
        if c == '{' {
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
            }
            out.push('*');
        } else {
            out.push(c);
        }
    }
    out
}

/// Length of the longest common substring. Used only to rank suggestions, so
/// that a one-segment rename (`user-restrictions` -> `player-restrictions`)
/// outranks an unrelated path of the same shape (`places`).
fn longest_common_substring(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut previous = vec![0usize; b.len() + 1];
    let mut best = 0;
    for i in 1..=a.len() {
        let mut current = vec![0usize; b.len() + 1];
        for j in 1..=b.len() {
            if a[i - 1] == b[j - 1] {
                current[j] = previous[j - 1] + 1;
                best = best.max(current[j]);
            }
        }
        previous = current;
    }
    best
}

/// Splits a path into normalised segments: literal segments must match
/// exactly, templated segments become wildcards.
fn normalise(path: &str) -> Vec<String> {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    path.trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .map(normalise_segment)
        .collect()
}

// ---------------------------------------------------------------------------
// The spec side
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/rbx-spec-drift is two levels below the workspace root")
        .to_path_buf()
}

/// `(host, normalised segments)` -> the spec path that produced it.
type SpecIndex = BTreeMap<(String, Vec<String>), String>;

fn load_spec(root: &Path) -> (SpecIndex, String) {
    let path = root.join("spec/openapi.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read the vendored spec at {}: {e}\n\
             It is committed to this repository; if it is missing, restore it with the \
             `update-openapi` workflow or `git checkout -- spec/`.",
            path.display()
        )
    });
    let spec: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));

    let default_host = spec["servers"][0]["url"]
        .as_str()
        .expect("the document declares a top-level servers[0].url")
        .trim_end_matches('/')
        .to_string();

    let paths = spec["paths"]
        .as_object()
        .expect("the document has a `paths` object");

    let mut index = SpecIndex::new();
    for (path, item) in paths {
        let segments = normalise(path);
        let Some(item) = item.as_object() else {
            continue;
        };
        for (key, operation) in item {
            if !METHODS.contains(&key.as_str()) {
                continue;
            }
            // An operation may override the document-level host. That is how
            // this document describes the legacy roblox.com services.
            let hosts: Vec<String> = match operation.get("servers").and_then(|s| s.as_array()) {
                Some(servers) => servers
                    .iter()
                    .filter_map(|s| s["url"].as_str())
                    .map(|u| u.trim_end_matches('/').to_string())
                    .collect(),
                None => vec![default_host.clone()],
            };
            for host in hosts {
                index
                    .entry((host, segments.clone()))
                    .or_insert_with(|| path.clone());
            }
        }
    }

    let provenance = fs::read_to_string(root.join("spec/source.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .map(|v| {
            format!(
                "{} @ {} ({})",
                v["repository"].as_str().unwrap_or("?"),
                v["commit"].as_str().unwrap_or("?"),
                v["commit_date"].as_str().unwrap_or("?")
            )
        })
        .unwrap_or_else(|| "unknown (spec/source.json missing or malformed)".to_string());

    (index, provenance)
}

// ---------------------------------------------------------------------------
// The source side
// ---------------------------------------------------------------------------

/// Every double-quoted literal on a line, with escapes skipped.
fn string_literals(line: &str) -> Vec<String> {
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

fn is_const_declaration(line: &str) -> bool {
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
fn strip_tests(src: &str) -> String {
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
fn opens_its_line(chars: &[char], at: usize) -> bool {
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
fn end_of_item(chars: &[char], from: usize) -> usize {
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
fn skip_string(chars: &[char], at: usize) -> usize {
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
fn skip_raw_string(chars: &[char], at: usize) -> Option<usize> {
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
fn skip_char_literal(chars: &[char], at: usize) -> usize {
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
fn crate_root_of(file: &Path) -> Option<PathBuf> {
    let mut current = file.parent()?;
    loop {
        if current.join("Cargo.toml").is_file() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

fn collect_consts(src: &str) -> BTreeMap<String, String> {
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
const DEFAULT_JOIN_HOST: &str = "https://apis.roblox.com";

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
fn join_receiver_host(line: &str, consts: &BTreeMap<String, String>) -> Option<String> {
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
fn resolve_const_prefix(
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
fn logical_lines(src: &str) -> Vec<(usize, String)> {
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
type CallSites = BTreeMap<(String, String), BTreeSet<(String, usize)>>;

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
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

fn collect_call_sites(root: &Path) -> CallSites {
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
fn crate_of(relative: &str) -> Option<&str> {
    relative.strip_prefix("crates/")?.split('/').next()
}

/// How many distinct endpoints each crate contributed.
fn endpoints_per_crate(sites: &CallSites) -> BTreeMap<String, usize> {
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
fn http_crates(root: &Path) -> BTreeSet<String> {
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

fn is_known_undocumented(host: &str, path: &str) -> Option<&'static str> {
    let segments = normalise(path);
    KNOWN_UNDOCUMENTED
        .iter()
        .find(|(h, p, _)| *h == host && normalise(p) == segments)
        .map(|(_, _, reason)| *reason)
}

// ---------------------------------------------------------------------------
// The other direction: documented and never called
// ---------------------------------------------------------------------------

/// The resource a path belongs to: its segments, with every parameterised one
/// dropped.
///
/// `/cloud/v2/universes/{universeId}/data-stores` and
/// `/cloud/v2/universes/{universeId}/data-stores/{dataStoreId}` share a family,
/// because the second is an operation on the resource the first lists. A
/// sub-resource does not: `.../data-stores/{dataStoreId}/entries` keeps
/// `entries` and is its own family, so calling the entries API says nothing
/// about whether the store API is covered.
///
/// A segment like `{dataStoreId}:undelete` normalises to `*:undelete`, which
/// carries a wildcard and is therefore dropped too. That is deliberate: the
/// `:undelete` action belongs to the resource it acts on, and it is one of the
/// endpoints this check was asked to surface.
fn family(segments: &[String]) -> Vec<String> {
    segments
        .iter()
        .filter(|segment| !segment.contains('*'))
        .cloned()
        .collect()
}

/// Endpoints this workspace **does** call, through a URL the extractor cannot
/// resolve, with the call site.
///
/// Every one of these is built by appending to a helper's return value, the
/// shape `format!("{}:increment", self.entry_url(entry))`. The extractor reads
/// string literals and consts; a URL assembled from a method call is a runtime
/// value, and teaching it to follow one means evaluating Rust.
///
/// Kept apart from [`NOT_CALLED_ON_PURPOSE`] because the two say opposite
/// things, and folding them together would bury the interesting half. An entry
/// here is not a decision about scope, it is **a hole in the sibling check**:
/// [`every_endpoint_we_call_still_exists_in_the_roblox_spec`] cannot see these
/// calls either, so if Roblox renames one of these paths nothing in CI notices.
/// This list is the record of which calls are unprotected, and it should shrink
/// by making the URLs literal rather than by growing.
const CALLED_THROUGH_A_HELPER: &[(&str, &str, &str)] = &[
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}/data-stores/{data_store_id}/scopes/{scope_id}/entries/{entry_id}:increment",
        "rbx-data/src/lib.rs, `format!(\"{}:increment\", self.entry_url(entry))`. \
         Reached by `rbx data increment`.",
    ),
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}/data-stores/{data_store_id}/scopes/{scope_id}/entries/{entry_id}:listRevisions",
        "rbx-data/src/lib.rs, `format!(\"{}:listRevisions?maxPageSize=100\", \
         self.entry_url(entry))`. Reached by `rbx data revisions`.",
    ),
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universeId}/secrets/{secretId}",
        "rbx-secret/src/lib.rs, `format!(\"{}/{}\", self.secrets_url(), \
         encode_query_value(id))`. Reached by `rbx secret set` (the PATCH half \
         of its POST-then-PATCH upsert) and `rbx secret remove`.",
    ),
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}/memory-store/sorted-maps/{sorted_map_id}/items/{item_id}",
        "rbx-memorystore/src/lib.rs, `fn item_url` appending an id to \
         `items_url()`. Reached by the get, update and delete of `rbx \
         memorystore`.",
    ),
];

/// Endpoints inside a family this workspace covers that it deliberately does
/// not call, each with the reason.
///
/// This is the counterpart of [`KNOWN_UNDOCUMENTED`] and works the same way: an
/// entry is a decision somebody made and can be argued with, where a silent
/// omission is a decision nobody recorded.
///
/// It stays short by construction. The scope is not curated here, it is read
/// off the code: an endpoint is only ever reported when the workspace already
/// calls something in the same family, so the hundreds of documented endpoints
/// in areas this tool has no business in (trading, private messages, avatar
/// customisation, localization tables) contribute nothing and need no entry.
const NOT_CALLED_ON_PURPOSE: &[(&str, &str, &str)] = &[
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}/data-stores/{data_store_id}",
        "Deleting a whole data store. A real gap rather than a decision, and \
         filed as #57: `rbx data` can bring a store into existence and cannot \
         remove one. Listed here so this check stays green while that is \
         designed, and the entry comes out with the command that closes it.",
    ),
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}/data-stores/{data_store_id}:undelete",
        "The undo of the one above, and the other half of #57.",
    ),
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}:restartServers",
        "A second way to do what `rbx restart` already does through \
         `/server-management/v1/universes/{id}/restarts`. The one in use also \
         forecasts and reports status, which this does not, so switching would \
         trade a three-endpoint command for a one-endpoint one. Worth \
         revisiting only if Roblox deprecates the server-management surface.",
    ),
    (
        "https://apis.roblox.com",
        "/assets/v1/assets/{assetId}/versions/{versionNumber}",
        "Metadata for one asset version. `rbx place` lists versions through the \
         sibling path and fetches the bytes of a chosen one through \
         `asset-delivery-api`, so the middle step has nothing to add: nothing \
         here asks a question that the listing has not already answered.",
    ),
    (
        "https://apis.roblox.com",
        "/ads-management/v1/billing-accounts/{id}",
        "One billing account by id. `rbx ads` lists them to let a campaign name \
         one, and never needs to look one up afterwards.",
    ),
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}:generateSpeechAsset",
        "Text to speech. Content creation, not deployment or operations, and \
         nothing else in this tool generates assets.",
    ),
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}:translateText",
        "Machine translation of a string. Same reason: this tool moves and \
         configures what a project already has.",
    ),
];

fn declined(host: &str, path: &str) -> Option<&'static str> {
    let segments = normalise(path);
    NOT_CALLED_ON_PURPOSE
        .iter()
        .chain(CALLED_THROUGH_A_HELPER)
        .find(|(h, p, _)| *h == host && normalise(p) == segments)
        .map(|(_, _, reason)| *reason)
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

/// An endpoint we call that the spec does not describe: `(host, path, call sites)`.
type Missing<'a> = (String, String, &'a BTreeSet<(String, usize)>);

#[test]
fn every_endpoint_we_call_still_exists_in_the_roblox_spec() {
    let root = repo_root();
    let (index, provenance) = load_spec(&root);
    let sites = collect_call_sites(&root);

    let mut checked = 0usize;
    let mut skipped = 0usize;
    let mut missing: Vec<Missing<'_>> = Vec::new();

    for ((host, path), locations) in &sites {
        if is_known_undocumented(host, path).is_some() {
            skipped += 1;
            continue;
        }
        checked += 1;
        let key = (host.clone(), normalise(path));
        if !index.contains_key(&key) {
            missing.push((host.clone(), path.clone(), locations));
        }
    }

    assert!(
        checked + skipped >= MINIMUM_ENDPOINTS,
        "The drift check only extracted {} endpoint(s) from crates/, which is below the \
         floor of {}.\n\n\
         This almost certainly means extraction is broken, not that the codebase shrank: \
         a new way of building URLs was introduced that the extractor in {} does not \
         recognise, so it is now silently checking almost nothing.\n\
         Read the module docs in that file (\"What this deliberately does not check\") and \
         teach it the new shape. Do not simply lower the floor.",
        checked + skipped,
        MINIMUM_ENDPOINTS,
        file!(),
    );

    if !missing.is_empty() {
        let mut report = String::new();
        report.push_str(&format!(
            "\n\nRoblox Open Cloud API drift detected.\n\
             \n{} endpoint(s) this workspace calls are NOT present in the vendored OpenAPI \
             document.\nVendored spec: {}\n\n",
            missing.len(),
            provenance,
        ));

        for (host, path, locations) in &missing {
            report.push_str(&format!("  {host}{path}\n"));
            for (file, line) in locations.iter() {
                report.push_str(&format!("      called from {file}:{line}\n"));
            }
            // Offer near misses: same path shape on a different host, or the
            // same host with a similar path. This usually names the rename.
            let segments = normalise(path);
            let mut scored: Vec<(usize, usize, String)> = index
                .iter()
                .filter_map(|((h, s), spec_path)| {
                    if *s == segments && h != host {
                        // Same shape, different host: Roblox moved the service.
                        return Some((usize::MAX, usize::MAX, format!("{h}{spec_path}")));
                    }
                    if h != host || s.len() != segments.len() {
                        return None;
                    }
                    let prefix = s.iter().zip(&segments).take_while(|(a, b)| a == b).count();
                    let suffix = s
                        .iter()
                        .rev()
                        .zip(segments.iter().rev())
                        .take_while(|(a, b)| a == b)
                        .count();
                    // Two shared segments is the point where a suggestion
                    // stops being noise.
                    if prefix + suffix < 2 {
                        return None;
                    }
                    // Among equally-shaped candidates, prefer the one whose
                    // differing segments most resemble ours: that is what a
                    // rename looks like.
                    let similarity: usize = s
                        .iter()
                        .zip(&segments)
                        .filter(|(a, b)| a != b)
                        .map(|(a, b)| longest_common_substring(a, b))
                        .sum();
                    Some((prefix + suffix, similarity, format!("{h}{spec_path}")))
                })
                .collect();
            scored.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| b.1.cmp(&a.1))
                    .then_with(|| a.2.cmp(&b.2))
            });
            let mut hints: Vec<String> = scored.into_iter().map(|(_, _, h)| h).collect();
            hints.dedup();
            hints.truncate(5);
            if !hints.is_empty() {
                report.push_str("      did Roblox mean one of these?\n");
                for hint in hints {
                    report.push_str(&format!("        {hint}\n"));
                }
            }
            report.push('\n');
        }

        report.push_str(
            "What to do:\n\
             \n  1. Check whether Roblox MOVED this endpoint (a rename or a new version) or\n\
             \x20    REMOVED it. The upstream changelog and the paths listed above are the\n\
             \x20    fastest way to tell.\n\
             \n  2. If it moved, update the call site(s) named above to the new path, and the\n\
             \x20    response types with it: a moved endpoint often reshapes its payload, which\n\
             \x20    is what produces `Failed to parse response` for users.\n\
             \n  3. If it was removed with no replacement, the feature that depends on it is\n\
             \x20    broken for every user. Decide whether to drop it or move to a different API.\n\
             \n  4. If Roblox merely stopped DOCUMENTING an endpoint that still works, add it to\n\
             \x20    KNOWN_UNDOCUMENTED in this file with a one-line reason. Do that only after\n\
             \x20    confirming it still works: that list is how we admit we have no early\n\
             \x20    warning for something, not a way to silence this test.\n\
             \n  5. If the spec was just refreshed and you did not expect any of this, check\n\
             \x20    `git log -1 -- spec/openapi.json` and diff the two revisions of the paths\n\
             \x20    above before changing any code.\n",
        );

        panic!("{report}");
    }

    println!(
        "Checked {checked} endpoint(s) against {provenance}; \
         {skipped} known-undocumented endpoint(s) skipped."
    );
}

/// Every crate that can call Roblox must contribute at least one endpoint.
///
/// The floor in the test above is a workspace-wide total, which one crate
/// cannot move: a domain crate whose URLs the extractor does not recognise
/// drops out silently while the total stays comfortably above 35. This is the
/// per-crate version of the same guard, and it fails on the day a crate stops
/// being covered rather than months later during an audit.
///
/// Two ways out when it goes red, and only one of them is usually right:
/// teach the extractor the new shape (see the module docs), or admit in
/// `CRATES_WITHOUT_ENDPOINTS` that the crate genuinely calls nothing.
#[test]
fn every_crate_that_calls_roblox_contributes_an_endpoint() {
    let root = repo_root();
    let sites = collect_call_sites(&root);
    let counts = endpoints_per_crate(&sites);
    let http = http_crates(&root);

    assert!(
        !http.is_empty(),
        "no crate under {}/crates depends on reqwest, which cannot be true: the manifest scan \
         in http_crates() is broken, so this test is checking nothing.",
        root.display()
    );

    let exempt = |krate: &str| {
        SKIPPED_CRATES.iter().any(|(name, _)| *name == krate)
            || CRATES_WITHOUT_ENDPOINTS
                .iter()
                .any(|(name, _)| *name == krate)
    };

    let silent: Vec<&String> = http
        .iter()
        .filter(|krate| !exempt(krate))
        .filter(|krate| counts.get(krate.as_str()).copied().unwrap_or(0) == 0)
        .collect();

    assert!(
        silent.is_empty(),
        "these crates depend on the HTTP layer but contributed no endpoint to the drift \
         check, so nothing they call is verified against the Roblox spec:\n\n{}\n\n\
         The usual cause is a URL-building shape the extractor in {} does not recognise: \
         a helper that concatenates paths, a const table, a different formatting macro. \
         Read the module docs (\"The URL shapes it recognises\") and teach it the shape.\n\
         If the crate really calls no Roblox endpoint of its own, add it to \
         CRATES_WITHOUT_ENDPOINTS with the reason.",
        silent
            .iter()
            .map(|krate| format!("  crates/{krate}"))
            .collect::<Vec<_>>()
            .join("\n"),
        file!(),
    );

    // The other direction: an exemption that stopped being true is a lie in a
    // file whose whole job is to be trusted.
    let stale: Vec<&str> = CRATES_WITHOUT_ENDPOINTS
        .iter()
        .map(|(krate, _)| *krate)
        .filter(|krate| counts.get(*krate).copied().unwrap_or(0) > 0)
        .collect();

    assert!(
        stale.is_empty(),
        "these crates are listed in CRATES_WITHOUT_ENDPOINTS but do contribute endpoints \
         now: {}. Remove them from the list.",
        stale.join(", ")
    );

    let unknown: Vec<&str> = CRATES_WITHOUT_ENDPOINTS
        .iter()
        .map(|(krate, _)| *krate)
        .filter(|krate| !http.contains(*krate))
        .collect();

    assert!(
        unknown.is_empty(),
        "these crates are listed in CRATES_WITHOUT_ENDPOINTS but no longer depend on the HTTP \
         layer (renamed, deleted, or the dependency was dropped): {}. Remove them from the \
         list: an exemption for a crate that cannot call Roblox anyway hides nothing and \
         outlives the reason it was written.",
        unknown.join(", ")
    );

    println!(
        "{} crate(s) with an HTTP dependency, {} contributing endpoints, {} exempt.",
        http.len(),
        http.iter()
            .filter(|k| counts.contains_key(k.as_str()))
            .count(),
        http.iter().filter(|k| exempt(k)).count(),
    );
}

// ---------------------------------------------------------------------------
// The configs client, which the extraction above cannot see
// ---------------------------------------------------------------------------

/// The one file whose URLs are reassembled by hand, and its endpoints.
///
/// `ConfigsClient` builds every path from a const plus two levels of
/// `format!`: `repo_url()` puts the universe and the repository on the const,
/// then each method appends its own suffix to that `String`. So no literal in
/// the file is a whole path, the generic extraction contributes nothing for
/// it, and while the client lived in `rbx-config` that was written off in
/// `CRATES_WITHOUT_ENDPOINTS`: the entire configs surface unchecked, under a
/// crate the per-crate guard reported as fine.
///
/// This reassembles those two levels for that one file. It stays specific to
/// this shape deliberately: teaching the general extractor to follow a
/// `String`-returning helper is a much larger change, and a narrow check that
/// runs beats a general one that does not exist.
const CONFIGS_CLIENT: &str = "crates/rbx-core/src/api/configs.rs";

/// A floor on the paths reassembled from `CONFIGS_CLIENT`: the repository
/// itself, `/draft`, `/draft:overwrite`, `/publish`, `/revisions` and a
/// revision restore. A rewrite of the client that stops matching the shape
/// below would otherwise leave this test checking nothing, quietly, which is
/// the failure this whole file exists to prevent.
const CONFIGS_ENDPOINTS: usize = 6;

/// Every path `CONFIGS_CLIENT` builds, as `(line, absolute path)`.
///
/// The shape it reads: one `const` holding a relative base, one `format!`
/// literal that starts with `{}/` and names `/repositories/` (the repository
/// URL every other call is built on), and one `{}/...` literal per call site
/// for the suffix. Query strings are dropped by `normalise`.
fn configs_endpoints(root: &Path) -> Vec<(usize, String)> {
    let file = root.join(CONFIGS_CLIENT);
    let src = fs::read_to_string(&file).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}\nIf the configs client moved, point CONFIGS_CLIENT at it \
             rather than deleting this test: the endpoints it builds are invisible to the \
             extraction above.",
            file.display()
        )
    });
    let src = strip_tests(&src);

    let bases: Vec<String> = src
        .lines()
        .filter(|line| is_const_declaration(line))
        .filter_map(|line| string_literals(line).into_iter().next())
        .filter(|value| value.starts_with('/'))
        .collect();
    assert_eq!(
        bases.len(),
        1,
        "expected exactly one relative base const in {CONFIGS_CLIENT}, found {bases:?}"
    );

    let mut suffixes: Vec<(usize, String)> = Vec::new();
    for (line_number, line) in logical_lines(&src) {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for literal in string_literals(&line) {
            if let Some(rest) = literal.strip_prefix("{}/") {
                suffixes.push((line_number, format!("/{rest}")));
            }
        }
    }

    let repository: Vec<(usize, String)> = suffixes
        .iter()
        .filter(|(_, suffix)| suffix.contains("/repositories/"))
        .cloned()
        .collect();
    assert_eq!(
        repository.len(),
        1,
        "expected exactly one repository-URL literal in {CONFIGS_CLIENT}, found {repository:?}"
    );
    let repository_url = format!("{}{}", bases[0], repository[0].1);

    let mut out = vec![(repository[0].0, repository_url.clone())];
    for (line_number, suffix) in suffixes {
        if suffix.contains("/repositories/") {
            continue;
        }
        out.push((line_number, format!("{repository_url}{suffix}")));
    }
    out
}

/// The configs endpoints, checked one by one against the spec.
///
/// This is what `CRATES_WITHOUT_ENDPOINTS` used to excuse. It caught a real
/// one on the day it was written: `restore_revision` built
/// `/revisions/{id}:restore`, a custom-method form this document defines
/// nowhere under `creator-configs-public-api`, so every rollback was a 404
/// reported as a failed restore.
#[test]
fn every_configs_endpoint_is_documented_in_the_spec() {
    let root = repo_root();
    let (spec, provenance) = load_spec(&root);
    let endpoints = configs_endpoints(&root);

    assert!(
        endpoints.len() >= CONFIGS_ENDPOINTS,
        "reassembled only {} path(s) from {CONFIGS_CLIENT}, expected at least \
         {CONFIGS_ENDPOINTS}: the client no longer matches the shape \
         `configs_endpoints` reads, so this test is checking almost nothing. \
         Teach it the new shape rather than lowering the floor.",
        endpoints.len()
    );

    let missing: Vec<String> = endpoints
        .iter()
        .filter(|(_, path)| !spec.contains_key(&(DEFAULT_JOIN_HOST.to_string(), normalise(path))))
        .map(|(line, path)| format!("  {CONFIGS_CLIENT}:{line}  {path}"))
        .collect();

    assert!(
        missing.is_empty(),
        "these configs endpoints are not in the vendored spec ({provenance}) on \
         {DEFAULT_JOIN_HOST}:\n\n{}\n\nEither Roblox moved them, or the path is misspelled: \
         a custom-method suffix (`:restore`) where the document has a plain segment \
         (`/restore`) is the one that shipped.",
        missing.join("\n")
    );

    println!(
        "{} configs endpoint(s) checked against {provenance}.",
        endpoints.len()
    );
}

// ---------------------------------------------------------------------------
// Request bodies
// ---------------------------------------------------------------------------

/// Request bodies this workspace sends, as `(source file, Rust struct, spec
/// schema, spec properties deliberately not sent)`.
///
/// Both directions are checked, and the second one is the one that was
/// missing. A key we send that the schema does not define is a rejected
/// request: these schemas carry `additionalProperties: false`, asserted below,
/// so a misspelled field is a 4xx rather than a field Roblox ignores. And a
/// property the schema defines that we neither send nor list here is the bug
/// that actually shipped: `UpdateDraftRequest.conditionalRules` was absent
/// from `OverwriteBody`, and on `PUT draft:overwrite` an absent
/// `conditionalRules` means "delete every published conditional rule".
///
/// So an entry in the fourth field is a deliberate statement that the property
/// is safe to omit, and the reason belongs in the doc comment of the struct
/// that omits it. An empty list means "we send all of it", which is the
/// strongest position and where both of these are.
const REQUEST_BODIES: &[(&str, &str, &str, &[&str])] = &[
    (CONFIGS_CLIENT, "OverwriteBody", "UpdateDraftRequest", &[]),
    (CONFIGS_CLIENT, "PublishBody", "PublishDraftRequest", &[]),
];

/// The JSON keys a `#[derive(Serialize)]` struct puts on the wire, read from
/// the source text.
///
/// Text rather than reflection because this crate compiles against nothing in
/// the workspace on purpose (see the module docs), so the alternative is no
/// check at all. It models the two things the request bodies here use, a bare
/// field name and `#[serde(rename = "...")]`, and refuses anything else it is
/// shown: a `rename_all` or a `flatten` changes the whole key set, and a
/// scanner that shrugged at one would compare a key set nobody sends.
fn serialised_keys(src: &str, struct_name: &str) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let declaration = format!("struct {struct_name}");
    let at = lines
        .iter()
        .position(|line| {
            let trimmed = line.trim_start();
            (trimmed.starts_with("struct ") || trimmed.starts_with("pub struct "))
                && trimmed.contains(&declaration)
        })
        .unwrap_or_else(|| panic!("no `{declaration}` in the source scanned for it"));

    for line in lines[..at].iter().rev() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("#[") {
            break;
        }
        assert!(
            !trimmed.contains("rename_all"),
            "{struct_name} carries {trimmed}, which renames every field: this scan reads \
             field names and `rename` attributes only, so it would report a key set that \
             is not the one being sent."
        );
    }

    let mut keys = Vec::new();
    let mut rename: Option<String> = None;
    for line in &lines[at + 1..] {
        let trimmed = line.trim();
        if trimmed.starts_with('}') {
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("#[") {
            assert!(
                !trimmed.contains("flatten"),
                "{struct_name} flattens a field, which lifts another struct's keys into \
                 this one: {trimmed}. Teach this scan to follow it before trusting the result."
            );
            if let Some((_, after)) = trimmed.split_once("rename = ") {
                rename = string_literals(after).into_iter().next();
            }
            continue;
        }
        let Some((name, _)) = trimmed.split_once(':') else {
            continue;
        };
        keys.push(rename.take().unwrap_or_else(|| name.trim().to_string()));
    }
    keys
}

/// What the tool sends and what the endpoint accepts, compared against the
/// document rather than against a mock the tool configures itself.
///
/// The regression that shipped was pinned only by a wiremock body matcher, so
/// renaming the serde attribute and the test literal together left the suite
/// green. This is the half that cannot be renamed into agreement.
#[test]
fn every_request_body_matches_the_schema_the_spec_documents() {
    let root = repo_root();
    let path = root.join("spec/openapi.json");
    let raw =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let spec: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));

    for (file, struct_name, schema_name, omitted) in REQUEST_BODIES {
        let source = root.join(file);
        let src = fs::read_to_string(&source)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", source.display()));
        let keys = serialised_keys(&strip_tests(&src), struct_name);
        assert!(
            !keys.is_empty(),
            "read no serialised key from {struct_name} in {file}: the scan found the struct \
             and nothing in it, so it is comparing empty sets."
        );

        let schema = &spec["components"]["schemas"][schema_name];
        let properties = schema["properties"].as_object().unwrap_or_else(|| {
            panic!(
                "components.schemas.{schema_name}.properties is not an object in the vendored \
                 spec. If Roblox renamed the schema, rename it here: an unresolvable schema \
                 must not read as an empty one."
            )
        });
        assert!(
            !properties.is_empty(),
            "components.schemas.{schema_name} documents no property, so this check would pass \
             on anything."
        );
        assert_eq!(
            schema["additionalProperties"],
            Value::Bool(false),
            "components.schemas.{schema_name} no longer refuses unknown properties. The \
             comparison below is written on the premise that it does: a key this document \
             does not define is a rejected request, not a field Roblox ignores."
        );

        let unknown: Vec<&String> = keys
            .iter()
            .filter(|key| !properties.contains_key(key.as_str()))
            .collect();
        assert!(
            unknown.is_empty(),
            "{struct_name} ({file}) sends {unknown:?}, which \
             components.schemas.{schema_name} does not define. With \
             `additionalProperties: false` that is a 4xx on every call, not an ignored \
             field. Known properties: {:?}",
            properties.keys().collect::<Vec<_>>()
        );

        let unsent: Vec<&String> = properties
            .keys()
            .filter(|property| !keys.contains(property))
            .filter(|property| !omitted.contains(&property.as_str()))
            .collect();
        assert!(
            unsent.is_empty(),
            "components.schemas.{schema_name} documents {unsent:?}, which {struct_name} \
             ({file}) neither sends nor declares as deliberately omitted. Read what the \
             property does when absent before adding it to the omission list in \
             REQUEST_BODIES: `conditionalRules` on `draft:overwrite` reads as \
             \"delete every published conditional rule\", and that is how this became a \
             test."
        );
    }
}

// ---------------------------------------------------------------------------
// Guards on the extractor itself
// ---------------------------------------------------------------------------

/// The bug this whole file's coverage depended on for months: a
/// `#[cfg(test)]` helper near the top of a client used to hide every call
/// below it.
#[test]
fn a_cfg_test_item_hides_itself_and_nothing_after_it() {
    let src = r#"
impl Client {
    #[cfg(test)]
    fn with_base_url(mut self, url: String) -> Self {
        self.base = ApiBase::new("https://mock.example/v1/nope");
        self
    }

    fn upload(&self) -> String {
        self.base.join("/universes/v1/places")
    }
}
"#;
    let stripped = strip_tests(src);
    assert!(
        !stripped.contains("mock.example"),
        "the gated item must be blanked out"
    );
    assert!(
        stripped.contains("/universes/v1/places"),
        "code after the gated item must survive: {stripped}"
    );
    assert_eq!(
        stripped.lines().count(),
        src.lines().count(),
        "line numbers are reported to the reader, so they must not shift"
    );
}

/// The `mod tests { ... }` at the bottom of a file, which is what the old
/// truncation handled correctly and this must keep handling.
#[test]
fn a_trailing_test_module_is_still_removed_whole() {
    let src = r#"
fn call() -> String {
    base.join("/cloud/v2/universes/{}/places")
}

#[cfg(test)]
mod tests {
    #[test]
    fn fixture() {
        assert_eq!(url(), "https://apis.roblox.com/cloud/v2/universes/1");
    }
}
"#;
    let stripped = strip_tests(src);
    assert!(stripped.contains("/cloud/v2/universes/{}/places"));
    assert!(
        !stripped.contains("universes/1"),
        "a unit test's fixed id is not an endpoint template: {stripped}"
    );
}

/// A brace inside a string or a comment is not a brace, and `'a` is not a
/// char literal. Getting either wrong swallows the rest of the file, which is
/// the failure this test exists to make loud.
#[test]
fn the_item_scanner_is_not_fooled_by_braces_in_strings_or_lifetimes() {
    let src = r#"
#[cfg(test)]
fn gated<'a>(name: &'a str) -> String {
    // } this brace is a comment
    format!("{{literal braces}} {name}")
}

fn real() -> String {
    base.join("/assets/v1/assets/{}/versions")
}
"#;
    let stripped = strip_tests(src);
    assert!(
        stripped.contains("/assets/v1/assets/{}/versions"),
        "the scanner stopped in the wrong place: {stripped}"
    );
    assert!(!stripped.contains("literal braces"));
}

/// Several files explain in prose why an item is `#[cfg(test)]`. Blanking
/// from a doc comment would delete the code it documents.
#[test]
fn a_mention_in_a_doc_comment_is_not_an_attribute() {
    let src = r#"
/// `#[cfg(test)]` rather than `#[doc(hidden)] pub`, because the module is
/// private.
fn call() -> String {
    base.join("/games/v1/games")
}
"#;
    assert!(strip_tests(src).contains("/games/v1/games"));
}

/// `http_crates` reads manifests, so it fails silently if the parse is wrong.
#[test]
fn the_manifest_scan_sees_dependencies_and_ignores_dev_dependencies() {
    let root = repo_root();
    let crates = http_crates(&root);

    assert!(
        crates.contains("rbx-place"),
        "rbx-place depends on reqwest: {crates:?}"
    );
    assert!(
        !crates.contains("rbx-spec-drift"),
        "this crate has no reqwest dependency at all and must not be counted: {crates:?}"
    );
}

/// The day this test goes red is the day `schemas/rbxavatar.schema.json` can
/// stop being maintained by hand.
///
/// That schema is the one file in `schemas/` with no freshness guarantee: it
/// describes the inside of `engineAvatarSettings`, and every other schema there
/// is regenerated from the serde model the CLI parses with, so CI fails when
/// one goes stale. This one has no model to regenerate from, because Roblox
/// types the field as an opaque string and publishes nothing about its
/// contents.
///
/// So this asserts the constraint still holds rather than the schema still
/// matches: the only checkable form of the question. If Roblox ever replaces
/// `"type": "string"` with a real object schema, this fails and says so, and
/// the hand-written file becomes a derived one like the rest.
///
/// It is deliberately not a test of *our* schema. Comparing our key names
/// against a document that describes none of them is not possible, and a test
/// that pretended otherwise would be worse than this one.
#[test]
fn engine_avatar_settings_is_still_an_opaque_string() {
    let spec: Value = serde_json::from_str(
        &fs::read_to_string(repo_root().join("spec/openapi.json")).expect("the vendored spec"),
    )
    .expect("the vendored spec is valid JSON");

    let field = &spec["components"]["schemas"]
        ["Roblox.Api.Develop.Models.UniverseSettingsRequestV2"]["properties"]
        ["engineAvatarSettings"];

    assert!(
        !field.is_null(),
        "engineAvatarSettings has left UniverseSettingsRequestV2. Either Roblox \
         removed the field it warned it might remove, or it moved. Check what \
         `game.engine_avatar_settings` should now send."
    );

    assert_eq!(
        field["type"], "string",
        "engineAvatarSettings is no longer an opaque JSON string in the Roblox \
         spec, which is the whole reason schemas/rbxavatar.schema.json is \
         hand-written and unverified.\n\nIf Roblox now documents its contents, \
         derive that schema from the spec instead and delete \
         crates/rbx-schema/src/engine_avatar.rs.\n\nField as vendored: {field}"
    );
}

/// What the vendored spec documents, in an area this workspace already works
/// in, and never calls.
///
/// The sibling check answers "does everything we call still exist". It cannot
/// answer this one, because an endpoint nobody calls contributes nothing to a
/// scan of call sites, so the spec can document a capability for months and
/// nothing says so. `Cloud_ListDataStores` sat there unimplemented until
/// somebody asked in conversation how to list an experience's data stores; it
/// shipped as `rbx data stores` in 0.6.0, and nothing in CI had ever mentioned
/// it.
///
/// **Scope is derived, not curated.** An endpoint is reported only when this
/// workspace already calls something in the same [`family`], so the several
/// hundred documented endpoints in areas this tool has no business in stay
/// silent without needing a line each. What that buys is the useful half: a
/// newly documented endpoint next to one already in use shows up once, as
/// "here is something you could now call", instead of never.
#[test]
fn every_documented_endpoint_in_a_covered_area_is_called_or_declined() {
    let root = repo_root();
    let (index, provenance) = load_spec(&root);
    let sites = collect_call_sites(&root);

    // The families this workspace works in, and the exact endpoints it calls.
    let mut covered: BTreeSet<(String, Vec<String>)> = BTreeSet::new();
    let mut called: BTreeSet<(String, Vec<String>)> = BTreeSet::new();
    for (host, path) in sites.keys() {
        let segments = normalise(path);
        covered.insert((host.clone(), family(&segments)));
        called.insert((host.clone(), segments));
    }

    let mut uncalled: Vec<(String, String)> = Vec::new();
    let mut listed = 0usize;

    for ((host, segments), spec_path) in &index {
        if !covered.contains(&(host.clone(), family(segments))) {
            continue;
        }
        if called.contains(&(host.clone(), segments.clone())) {
            continue;
        }
        if declined(host, spec_path).is_some() {
            listed += 1;
            continue;
        }
        uncalled.push((host.clone(), spec_path.clone()));
    }

    assert!(
        uncalled.is_empty(),
        "\n\n{} documented endpoint(s) sit in an area this workspace already \
         works in and are never called.\nVendored spec: {}\n\n{}\n\n\
         Each one is either a capability worth adding, or a deliberate pass \
         that belongs in NOT_CALLED_ON_PURPOSE with the reason written out. If \
         it is called through a URL built from a helper, it belongs in \
         CALLED_THROUGH_A_HELPER instead, which is a different statement. {} \
         endpoint(s) are already listed between the two.\n\n\
         This list only grows when Roblox documents something new next to \
         something already in use, which is exactly when it is worth reading.",
        uncalled.len(),
        provenance,
        uncalled
            .iter()
            .map(|(host, path)| format!("  {host}{path}"))
            .collect::<Vec<_>>()
            .join("\n"),
        listed,
    );
}

/// An entry that stops describing anything has to come out, the rule every
/// allowlist in this workspace follows. An endpoint listed as deliberately
/// skipped and then implemented is the good outcome; leaving the row behind
/// would exempt it again the day the call is deleted.
///
/// [`CALLED_THROUGH_A_HELPER`] is deliberately not checked this way. Those
/// endpoints *are* called, and the extractor not seeing them is the entire
/// point of the list, so the same assertion would fail every one of them.
/// Their stale-entry check is the opposite one, below.
#[test]
fn nothing_is_declined_while_being_called() {
    let root = repo_root();
    let sites = collect_call_sites(&root);
    let called: BTreeSet<(String, Vec<String>)> = sites
        .keys()
        .map(|(host, path)| (host.clone(), normalise(path)))
        .collect();

    let stale: Vec<String> = NOT_CALLED_ON_PURPOSE
        .iter()
        .filter(|(host, path, _)| called.contains(&((*host).to_string(), normalise(path))))
        .map(|(host, path, why)| format!("  {host}{path}\n    it was listed as: {why}"))
        .collect();

    assert!(
        stale.is_empty(),
        "{} NOT_CALLED_ON_PURPOSE entry/entries describe nothing:\n{}",
        stale.len(),
        stale.join("\n")
    );
}

/// A helper-built URL that became literal has to leave the list.
///
/// That is the good outcome for [`CALLED_THROUGH_A_HELPER`]: the extractor can
/// see the call, so the sibling drift check now protects it, and the entry is
/// describing a hole that no longer exists. Left behind, it would quietly
/// exempt the endpoint from this check for a reason that stopped being true.
#[test]
fn nothing_is_listed_as_helper_built_once_the_extractor_can_see_it() {
    let root = repo_root();
    let sites = collect_call_sites(&root);
    let called: BTreeSet<(String, Vec<String>)> = sites
        .keys()
        .map(|(host, path)| (host.clone(), normalise(path)))
        .collect();

    let visible: Vec<String> = CALLED_THROUGH_A_HELPER
        .iter()
        .filter(|(host, path, _)| called.contains(&((*host).to_string(), normalise(path))))
        .map(|(host, path, where_from)| {
            format!("  {host}{path}\n    was listed as unresolvable: {where_from}")
        })
        .collect();

    assert!(
        visible.is_empty(),
        "{} CALLED_THROUGH_A_HELPER entry/entries are extractable now, so the \
         sibling drift check covers them and the row is stale:\n{}",
        visible.len(),
        visible.join("\n")
    );
}

/// Both lists name paths the spec actually documents.
///
/// Without this, a Roblox rename turns an entry into a row matching nothing:
/// the endpoint quietly leaves the report, the reason stays in the file, and
/// the two never meet again. The sibling check catches a rename under a call
/// site; nothing would catch one under an allowlist row.
#[test]
fn every_listed_endpoint_still_exists_in_the_spec() {
    let root = repo_root();
    let (index, provenance) = load_spec(&root);

    let unknown: Vec<String> = NOT_CALLED_ON_PURPOSE
        .iter()
        .chain(CALLED_THROUGH_A_HELPER)
        .filter(|(host, path, _)| !index.contains_key(&((*host).to_string(), normalise(path))))
        .map(|(host, path, _)| format!("  {host}{path}"))
        .collect();

    assert!(
        unknown.is_empty(),
        "{} listed endpoint(s) are in neither list's spec any more.\n\
         Vendored spec: {}\n{}\n\n\
         Roblox renamed or removed them. Update the row to the new path, or \
         delete it if the endpoint is gone.",
        unknown.len(),
        provenance,
        unknown.join("\n")
    );
}
