//! `rbx apikey catalog regenerate [url]`: refresh the embedded scope catalog from
//! Roblox's live `openapi.json` and write `src/data/catalog.json`.
//!
//! The catalog is embedded at compile time (include_str!), so a rebuild is required for
//! the change to take effect: the command prints the next steps after writing.
//!
//! That makes this a maintainer command: it only means anything from a checkout, and it
//! refuses to run anywhere else rather than writing a file that can never be applied.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Result};
use chrono::Utc;
use colored::Colorize;
use serde_json::Value;

use crate::scope_catalog::{Catalog, ScopeInfo};

const DEFAULT_OPENAPI_URL: &str =
    "https://raw.githubusercontent.com/Roblox/creator-docs/main/content/en-us/reference/cloud/openapi.json";
const OUTPUT_FILE: &str = "src/data/catalog.json";

pub async fn regenerate(url: Option<&str>) -> Result<()> {
    let url = url.unwrap_or(DEFAULT_OPENAPI_URL);

    // Before the download, not after: the answer does not depend on the spec,
    // and a refusal should not cost a network round-trip.
    ensure_writable_from_here(Path::new(OUTPUT_FILE))?;

    println!("{}", "Regenerating scope catalog".cyan());
    println!();

    let body = fetch_openapi(url).await?;
    println!("{}", "✓ Downloaded".green());

    let mut scopes = parse_scopes(&body)?;
    if scopes.is_empty() {
        println!();
        bail!("No scopes found in JSON");
    }
    println!("{}", format!("✓ Parsed {} scopes", scopes.len()).green());

    // Descriptions only ever came from `components.securitySchemes`, which the
    // current spec no longer publishes. Without this, every regenerate would
    // quietly strip the descriptions the catalog already has.
    let carried = carry_over_descriptions(Path::new(OUTPUT_FILE), &mut scopes);
    if carried > 0 {
        println!(
            "{}",
            format!("✓ Kept {carried} descriptions from the previous catalog").green()
        );
    }

    let cat = Catalog {
        version: Utc::now().format("%Y-%m-%d").to_string(),
        source_url: provenance_for(url),
        scopes,
    };
    let json = serde_json::to_string_pretty(&cat)?;

    std::fs::write(OUTPUT_FILE, json)?;
    println!("{}", format!("✓ Wrote {}", OUTPUT_FILE).green());
    println!();
    println!("Next steps:");
    println!("  1. Review the generated file");
    println!("  2. Run `cargo build --release` to recompile the binary");
    Ok(())
}

/// Refuse unless the output directory is already there.
///
/// `OUTPUT_FILE` is relative to the working directory, so it only names
/// anything from a checkout of this repository. Creating the parents instead
/// (what this used to do) turned running the command from anywhere else into a
/// junk `./src/data/catalog.json` and a success message, which is the one
/// outcome nobody wants: the catalog is embedded with `include_str!`, so a
/// written file changes nothing until something rebuilds the binary.
fn ensure_writable_from_here(path: &Path) -> Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    match dir {
        Some(dir) if !dir.is_dir() => bail!(
            "no {} directory here, so this is not a checkout of rbx-forge/rbx-cli.\n\
             \n\
             `catalog regenerate` rewrites the catalog in the source tree, and the catalog is \
             compiled into the binary, so a rebuild is what applies it. Either:\n  \
             - Run this from the root of a checkout of https://github.com/rbx-forge/rbx-cli, \
             then `cargo build --release`\n  \
             - Or, to pick up scopes Roblox has added, upgrade to a release that ships them: \
             an installed binary cannot refresh its own catalog",
            dir.display()
        ),
        _ => Ok(()),
    }
}

/// What to record as the catalog's `source_url`.
///
/// A URL is its own provenance. A local path is not: regenerating from the
/// vendored `spec/openapi.json` (the offline, deterministic way to do it)
/// would otherwise stamp an absolute path from one developer's machine into a
/// committed file, which tells the next reader nothing about where the scopes
/// came from.
///
/// The vendored spec already carries its provenance in a `source.json`
/// alongside it, the same file `rbx-spec-drift` reads to report which upstream
/// revision it is testing against. When one is there, record the permalink it
/// describes; the bytes parsed are a byte-for-byte snapshot of exactly that
/// revision, so the permalink is the truthful answer, not an approximation of
/// one.
fn provenance_for(url: &str) -> String {
    let Some(path_str) = url.strip_prefix("file://") else {
        return url.to_string();
    };
    match permalink_beside(Path::new(path_str)) {
        Some(permalink) => permalink,
        None => {
            // Not fatal: the catalog is still correct, only its provenance line
            // is weak. Say so rather than committing the path silently.
            println!(
                "{}",
                format!(
                    "! no source.json beside {path_str}; recording the local path as the source"
                )
                .yellow()
            );
            url.to_string()
        }
    }
}

/// Build the upstream permalink from the `source.json` sitting next to a
/// vendored spec. `None` if it is missing, unreadable, or lacks a field.
fn permalink_beside(spec_path: &Path) -> Option<String> {
    let provenance = std::fs::read_to_string(spec_path.parent()?.join("source.json")).ok()?;
    let v: Value = serde_json::from_str(&provenance).ok()?;
    let repository = v.get("repository")?.as_str()?;
    let commit = v.get("commit")?.as_str()?;
    let document = v.get("document")?.as_str()?;
    Some(format!(
        "https://github.com/{repository}/blob/{commit}/{document}"
    ))
}

async fn fetch_openapi(url: &str) -> Result<String> {
    println!("Fetching {}...", url);

    if let Some(path_str) = url.strip_prefix("file://") {
        return std::fs::read_to_string(path_str)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path_str, e));
    }

    let client = reqwest::Client::new();
    let resp = client.get(url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        bail!("HTTP {}", status);
    }
    Ok(resp.text().await?)
}

fn parse_scopes(json_body: &str) -> Result<BTreeMap<String, ScopeInfo>> {
    let v: Value = serde_json::from_str(json_body)
        .map_err(|e| anyhow::anyhow!("Failed to parse JSON: {}", e))?;

    let mut scopes: BTreeMap<String, ScopeInfo> = BTreeMap::new();
    // Specifiers seen per scope type, collected across every path that
    // references it and applied once at the end. A scope type appears on many
    // operations and Roblox does not fill the field in consistently
    // (`universe` carries `universes` on some paths and nothing on others) so
    // deciding on first sight would make the catalog depend on map iteration
    // order. Empty strings are dropped at insert: the field is present but
    // says nothing, which is not the same as naming a target.
    let mut specifiers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    // 1. Extract from paths[*].<method>["x-roblox-scopes"]
    if let Some(paths) = v.get("paths").and_then(|p| p.as_object()) {
        for (_, path_item) in paths {
            if let Some(path_obj) = path_item.as_object() {
                for (_, op) in path_obj {
                    if let Some(roblox_scopes) =
                        op.get("x-roblox-scopes").and_then(|s| s.as_array())
                    {
                        for scope_ref in roblox_scopes {
                            let Some(raw) = scope_name(scope_ref) else {
                                continue;
                            };
                            let (scope_type, operations) = split_scope(raw);
                            if let Some(spec) = target_specifier(scope_ref) {
                                specifiers
                                    .entry(scope_type.to_string())
                                    .or_default()
                                    .insert(spec.to_string());
                            }
                            let entry =
                                scopes
                                    .entry(scope_type.to_string())
                                    .or_insert_with(|| ScopeInfo {
                                        operations: Vec::new(),
                                        target_type: String::new(),
                                        description: None,
                                    });
                            // The same scope type appears on many operations
                            // (`universe:read` on a few dozen paths), each
                            // contributing one operation. Union, don't replace.
                            for op in operations {
                                if !entry.operations.iter().any(|known| known == op) {
                                    entry.operations.push(op.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. components.securitySchemes.<name>.scopes
    //
    // Dead against the current spec: the three schemes Roblox publishes now
    // (roblox-api-key, roblox-legacy-cookie, roblox-oauth2) carry no `scopes`
    // key at all. Kept because it costs nothing and is the only source that
    // ever supplied descriptions, so an older or restored spec still works.
    if let Some(schemes) = v
        .pointer("/components/securitySchemes")
        .and_then(|s| s.as_object())
    {
        for (_, scheme) in schemes {
            if let Some(scope_map) = scheme.get("scopes").and_then(|s| s.as_object()) {
                for (scope_name, scope_desc) in scope_map {
                    scopes
                        .entry(scope_name.clone())
                        .or_insert_with(|| ScopeInfo {
                            operations: Vec::new(),
                            target_type: String::new(),
                            description: scope_desc.as_str().map(|s| s.to_string()),
                        });
                }
            }
        }
    }

    // Stable output: the file is committed, so unordered operations would
    // produce a diff on every regenerate even when nothing changed. Done last
    // so anything added by source 2 is sorted too.
    for (name, info) in scopes.iter_mut() {
        info.operations.sort();
        info.target_type = resolve_target_type(name, specifiers.get(name));
    }

    Ok(scopes)
}

/// Copy descriptions from the catalog already on disk onto the freshly parsed
/// scopes. Returns how many were carried over. A missing or unreadable file is
/// not an error: the first ever run has nothing to carry.
fn carry_over_descriptions(path: &Path, scopes: &mut BTreeMap<String, ScopeInfo>) -> usize {
    let Ok(existing) = std::fs::read_to_string(path) else {
        return 0;
    };
    let Ok(previous) = serde_json::from_str::<Catalog>(&existing) else {
        return 0;
    };

    let mut carried = 0;
    for (name, info) in scopes.iter_mut() {
        if info.description.is_some() {
            continue;
        }
        if let Some(description) = previous
            .scopes
            .get(name)
            .and_then(|p| p.description.clone())
        {
            info.description = Some(description);
            carried += 1;
        }
    }
    carried
}

/// Read one `x-roblox-scopes` entry.
///
/// Roblox changed this from a bare string to
/// `{"name": "universe:read", "targetResourceSpecifier": "universes"}`. The
/// old `as_str()` call silently returned `None` for every entry after that,
/// which is why `regenerate` reported "No scopes found" and the catalog stayed
/// frozen at its last working run. Both shapes are accepted so a restored or
/// vendored older spec still parses.
fn scope_name(scope_ref: &Value) -> Option<&str> {
    scope_ref
        .as_str()
        .or_else(|| scope_ref.get("name").and_then(|n| n.as_str()))
}

/// Read one entry's `targetResourceSpecifier`, if it names anything.
///
/// The bare-string form has no specifier at all. The object form often carries
/// `""`, which is treated as absent: the key exists but names no resource, so
/// it carries no more information than omitting it would.
fn target_specifier(scope_ref: &Value) -> Option<&str> {
    scope_ref
        .get("targetResourceSpecifier")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
}

/// Split `universe-datastores.objects:create,delete` into its type and its
/// operations. A name with no `:` is a type with no operations.
fn split_scope(raw: &str) -> (&str, Vec<&str>) {
    match raw.split_once(':') {
        Some((scope_type, operations)) => (
            scope_type,
            operations
                .split(',')
                .map(str::trim)
                .filter(|op| !op.is_empty())
                .collect(),
        ),
        None => (raw, Vec::new()),
    }
}

/// Decide a scope's target type, preferring what the spec states over what the
/// name suggests.
///
/// Roblox publishes `targetResourceSpecifier` next to each scope name, but
/// fills it in for four scope families out of forty-odd and writes `""` for
/// several more. So it is an override where it speaks, not a source: the name
/// heuristic still decides everything else.
///
/// Where it does speak it has been wrong-footing us: `developer-product` and
/// `game-pass` both say `universes` while the heuristic called them `none`,
/// which builds a key targeting `*` (every universe the owner has) instead of
/// the ones the config names.
fn resolve_target_type(scope_type: &str, spec_specifiers: Option<&BTreeSet<String>>) -> String {
    let from_spec = spec_specifiers
        .into_iter()
        .flatten()
        .find_map(|s| map_specifier(s));
    from_spec.unwrap_or_else(|| infer_target_type(scope_type))
}

/// Translate a spec `targetResourceSpecifier` into our target vocabulary.
///
/// `universes` is the only value Roblox currently emits. Anything else is
/// returned as `None` so an unrecognised value falls back to the heuristic
/// rather than being written into the catalog untranslated: a target type
/// `scope_builder` does not know produces a malformed key request, and that
/// fails at key creation rather than at compile time.
fn map_specifier(specifier: &str) -> Option<String> {
    match specifier {
        "universes" => Some("universe".into()),
        _ => None,
    }
}

fn infer_target_type(scope_type: &str) -> String {
    if scope_type.starts_with("universe-datastore") {
        "universe-datastore".into()
    } else if scope_type.starts_with("universe") {
        "universe".into()
    } else if scope_type.starts_with("memory-store") {
        // The spec says nothing about these, and the fall-through below would
        // call them `creator`: a key spanning every universe the owner has.
        // The Creator Hub's own key editor offers "Restrict by Experience" for
        // Memory Stores, which it would not if the grant were creator-wide, and
        // every memory-store path is rooted at
        // `/cloud/v2/universes/{universe_id}/memory-store/...`. Both point at a
        // universe target, so that is what we build.
        "universe".into()
    } else if scope_type.starts_with("developer-product") || scope_type.starts_with("game-pass") {
        // Called `none` until the spec was found to say `universes` for both.
        // `resolve_target_type` already prefers the spec, so this branch only
        // runs if Roblox stops publishing the field; it agrees with the spec so
        // that the answer does not silently revert if they do.
        "universe".into()
    } else if scope_type.starts_with("legacy") {
        "none".into()
    } else {
        "creator".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape Roblox publishes today. A regression here means `regenerate`
    /// silently produces an empty catalog again.
    const OBJECT_FORM: &str = r#"{
      "paths": {
        "/a": { "get": { "x-roblox-scopes": [
          { "name": "universe:read", "targetResourceSpecifier": "universes" }
        ] } },
        "/b": { "post": { "x-roblox-scopes": [
          { "name": "universe:write" },
          { "name": "universe.user-restriction:read" }
        ] } }
      }
    }"#;

    /// The shape Roblox used to publish, kept working on purpose.
    const STRING_FORM: &str = r#"{
      "paths": { "/a": { "get": { "x-roblox-scopes": ["universe:read"] } } }
    }"#;

    #[test]
    fn object_form_entries_are_parsed() {
        let scopes = parse_scopes(OBJECT_FORM).unwrap();
        assert_eq!(scopes["universe"].operations, vec!["read", "write"]);
        assert_eq!(scopes["universe.user-restriction"].operations, vec!["read"]);
    }

    #[test]
    fn string_form_entries_still_parse() {
        let scopes = parse_scopes(STRING_FORM).unwrap();
        assert_eq!(scopes["universe"].operations, vec!["read"]);
    }

    #[test]
    fn operations_from_different_paths_are_unioned_not_replaced() {
        let scopes = parse_scopes(OBJECT_FORM).unwrap();
        // `universe` is referenced by two paths, one read and one write.
        assert_eq!(scopes["universe"].operations.len(), 2);
    }

    #[test]
    fn operations_are_sorted_so_the_committed_file_does_not_churn() {
        let json = r#"{"paths":{"/a":{"get":{"x-roblox-scopes":[
            {"name":"universe-datastores.objects:update,create,list"}
        ]}}}}"#;
        let scopes = parse_scopes(json).unwrap();
        assert_eq!(
            scopes["universe-datastores.objects"].operations,
            vec!["create", "list", "update"]
        );
    }

    #[test]
    fn a_scope_without_operations_still_yields_its_type() {
        let json = r#"{"paths":{"/a":{"get":{"x-roblox-scopes":[{"name":"asset"}]}}}}"#;
        let scopes = parse_scopes(json).unwrap();
        assert!(scopes["asset"].operations.is_empty());
    }

    #[test]
    fn an_entry_that_is_neither_string_nor_named_object_is_skipped() {
        let json = r#"{"paths":{"/a":{"get":{"x-roblox-scopes":[
            {"noName":"x"}, {"name":"universe:read"}
        ]}}}}"#;
        let scopes = parse_scopes(json).unwrap();
        assert_eq!(scopes.len(), 1);
    }

    #[test]
    fn the_spec_specifier_overrides_the_name_heuristic() {
        // The heuristic alone calls `game-pass` a `none` target, which builds a
        // key over `*`. The spec says `universes`.
        let json = r#"{"paths":{"/a":{"get":{"x-roblox-scopes":[
            {"name":"game-pass:read","targetResourceSpecifier":"universes"}
        ]}}}}"#;
        let scopes = parse_scopes(json).unwrap();
        assert_eq!(scopes["game-pass"].target_type, "universe");
    }

    #[test]
    fn an_empty_specifier_is_not_a_target_and_leaves_the_heuristic_alone() {
        let json = r#"{"paths":{"/a":{"get":{"x-roblox-scopes":[
            {"name":"ad.campaign:read","targetResourceSpecifier":""}
        ]}}}}"#;
        let scopes = parse_scopes(json).unwrap();
        assert_eq!(scopes["ad.campaign"].target_type, "creator");
    }

    #[test]
    fn an_unrecognised_specifier_falls_back_rather_than_being_written_through() {
        let json = r#"{"paths":{"/a":{"get":{"x-roblox-scopes":[
            {"name":"asset:read","targetResourceSpecifier":"somethingNew"}
        ]}}}}"#;
        let scopes = parse_scopes(json).unwrap();
        assert_eq!(scopes["asset"].target_type, "creator");
    }

    /// Roblox fills the field in on some paths and not others for the same
    /// scope. Whichever path is visited first must not decide the answer.
    #[test]
    fn a_specifier_on_any_path_settles_the_scope_regardless_of_path_order() {
        let json = r#"{"paths":{
            "/a":{"get":{"x-roblox-scopes":[{"name":"game-pass:read"}]}},
            "/b":{"get":{"x-roblox-scopes":[
                {"name":"game-pass:write","targetResourceSpecifier":"universes"}
            ]}}
        }}"#;
        let scopes = parse_scopes(json).unwrap();
        assert_eq!(scopes["game-pass"].target_type, "universe");
    }

    /// The spec is silent on all three memory-store scopes, so the fall-through
    /// used to make them `creator`: one key over every universe the owner has.
    #[test]
    fn memory_store_scopes_target_a_universe_not_the_creator() {
        let json = r#"{"paths":{"/a":{"get":{"x-roblox-scopes":[
            {"name":"memory-store.sorted-map:read"},
            {"name":"memory-store.queue:add"},
            {"name":"memory-store:flush"}
        ]}}}}"#;
        let scopes = parse_scopes(json).unwrap();
        assert_eq!(scopes["memory-store.sorted-map"].target_type, "universe");
        assert_eq!(scopes["memory-store.queue"].target_type, "universe");
        assert_eq!(scopes["memory-store"].target_type, "universe");
    }

    #[test]
    fn datastore_scopes_keep_their_two_part_target() {
        let json = r#"{"paths":{"/a":{"get":{"x-roblox-scopes":[
            {"name":"universe-datastores.objects:read"}
        ]}}}}"#;
        let scopes = parse_scopes(json).unwrap();
        assert_eq!(
            scopes["universe-datastores.objects"].target_type,
            "universe-datastore"
        );
    }

    #[test]
    fn split_scope_handles_multiple_operations() {
        assert_eq!(
            split_scope("a.b:read,write"),
            ("a.b", vec!["read", "write"])
        );
    }

    #[test]
    fn split_scope_handles_no_operations() {
        assert_eq!(split_scope("a.b"), ("a.b", Vec::<&str>::new()));
    }

    #[test]
    fn descriptions_are_carried_over_from_the_previous_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.json");
        std::fs::write(
            &path,
            r#"{"version":"old","source_url":"u","scopes":{
                "universe":{"operations":["read"],"target_type":"universe","description":"kept"}
            }}"#,
        )
        .unwrap();

        let mut scopes = parse_scopes(OBJECT_FORM).unwrap();
        let carried = carry_over_descriptions(&path, &mut scopes);

        assert_eq!(carried, 1);
        assert_eq!(scopes["universe"].description.as_deref(), Some("kept"));
        assert!(scopes["universe.user-restriction"].description.is_none());
    }

    #[test]
    fn an_http_source_is_recorded_as_given() {
        let url = "https://example.invalid/openapi.json";
        assert_eq!(provenance_for(url), url);
    }

    /// Regenerating from the vendored spec must not stamp a developer's
    /// absolute path into the committed catalog.
    #[test]
    fn a_vendored_spec_is_recorded_as_its_upstream_permalink() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("source.json"),
            r#"{"repository":"Roblox/creator-docs","commit":"abc123",
                "document":"content/en-us/reference/cloud/openapi.json"}"#,
        )
        .unwrap();
        let spec = dir.path().join("openapi.json");
        std::fs::write(&spec, "{}").unwrap();

        assert_eq!(
            provenance_for(&format!("file://{}", spec.display())),
            "https://github.com/Roblox/creator-docs/blob/abc123/content/en-us/reference/cloud/openapi.json"
        );
    }

    #[test]
    fn a_local_spec_without_provenance_falls_back_to_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let spec = dir.path().join("openapi.json");
        std::fs::write(&spec, "{}").unwrap();

        let url = format!("file://{}", spec.display());
        assert_eq!(provenance_for(&url), url);
    }

    #[test]
    fn a_malformed_source_json_falls_back_rather_than_producing_a_broken_link() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("source.json"),
            r#"{"repository":"only-this"}"#,
        )
        .unwrap();
        let spec = dir.path().join("openapi.json");
        std::fs::write(&spec, "{}").unwrap();

        let url = format!("file://{}", spec.display());
        assert_eq!(provenance_for(&url), url);
    }

    /// The junk-file case: an empty directory is where an installed binary
    /// normally runs, and it must refuse rather than create `src/data/`.
    #[test]
    fn regenerating_outside_a_checkout_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("src/data/catalog.json");

        let err = ensure_writable_from_here(&path).unwrap_err().to_string();

        assert!(err.contains("rbx-forge/rbx-cli"), "{err}");
        assert!(
            !dir.path().join("src").exists(),
            "created the parents anyway"
        );
    }

    #[test]
    fn regenerating_from_a_checkout_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/data")).unwrap();

        ensure_writable_from_here(&dir.path().join("src/data/catalog.json")).unwrap();
    }

    /// A bare filename has no parent directory to require, and the working
    /// directory it lands in exists by definition.
    #[test]
    fn a_path_without_a_parent_directory_is_allowed() {
        ensure_writable_from_here(Path::new("catalog.json")).unwrap();
    }

    #[test]
    fn a_missing_previous_catalog_is_not_an_error() {
        let mut scopes = parse_scopes(OBJECT_FORM).unwrap();
        assert_eq!(
            carry_over_descriptions(Path::new("does/not/exist.json"), &mut scopes),
            0
        );
    }
}
