//! The vendored Roblox document, indexed by (host, normalised path).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::paths::*;
use crate::METHODS;

pub(crate) fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/rbx-spec-drift is two levels below the workspace root")
        .to_path_buf()
}

/// `(host, normalised segments)` -> the spec path that produced it.
pub(crate) type SpecIndex = BTreeMap<(String, Vec<String>), String>;

pub(crate) fn load_spec(root: &Path) -> (SpecIndex, String) {
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
