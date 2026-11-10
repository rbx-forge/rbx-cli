//! The configs client, whose URLs are reassembled by hand and which the
//! extractor therefore cannot see.

use std::fs;
use std::path::Path;

use crate::{extract::*, paths::*, spec::*};
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
/// Relative to the rbx-core package root, not to this workspace: see
/// [`rbx_core_root`].
pub(crate) const CONFIGS_CLIENT: &str = "src/api/configs.rs";

/// Where the rbx-core sources this workspace compiles against live.
///
/// Asked of `cargo metadata` rather than written as `crates/rbx-core`, because
/// the root manifest decides where rbx-core comes from: a path in this
/// workspace or a git checkout pinned by tag. Either way the check reads the
/// code that is actually built.
pub(crate) fn rbx_core_root(root: &Path) -> std::path::PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("cargo metadata should run");
    assert!(
        out.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("cargo metadata should print JSON");
    let manifest = doc["packages"]
        .as_array()
        .and_then(|packages| packages.iter().find(|p| p["name"] == "rbx-core"))
        .and_then(|p| p["manifest_path"].as_str())
        .expect("rbx-core should be a dependency of this workspace");
    Path::new(manifest)
        .parent()
        .expect("a manifest path has a parent directory")
        .to_path_buf()
}

/// A floor on the paths reassembled from `CONFIGS_CLIENT`: the repository
/// itself, `/draft`, `/draft:overwrite`, `/publish`, `/revisions` and a
/// revision restore. A rewrite of the client that stops matching the shape
/// below would otherwise leave this test checking nothing, quietly, which is
/// the failure this whole file exists to prevent.
pub(crate) const CONFIGS_ENDPOINTS: usize = 6;

/// Every path `CONFIGS_CLIENT` builds, as `(line, absolute path)`.
///
/// The shape it reads: one `const` holding a relative base, one `format!`
/// literal that starts with `{}/` and names `/repositories/` (the repository
/// URL every other call is built on), and one `{}/...` literal per call site
/// for the suffix. Query strings are dropped by `normalise`.
pub(crate) fn configs_endpoints(root: &Path) -> Vec<(usize, String)> {
    let file = rbx_core_root(root).join(CONFIGS_CLIENT);
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
pub(crate) fn every_configs_endpoint_is_documented_in_the_spec() {
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
