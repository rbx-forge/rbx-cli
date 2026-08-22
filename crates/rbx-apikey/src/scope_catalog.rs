//! Bundled scope catalog. Loaded once from `src/data/catalog.json` (embedded at compile time).
//! Lookups are advisory: unknown scopes emit warnings, not errors.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScopeInfo {
    pub operations: Vec<String>,
    pub target_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Catalog {
    pub version: String,
    pub source_url: String,
    pub scopes: BTreeMap<String, ScopeInfo>,
}

const EMBEDDED_CATALOG: &str = include_str!("data/catalog.json");

fn catalog() -> &'static Catalog {
    static C: OnceLock<Catalog> = OnceLock::new();
    C.get_or_init(|| {
        serde_json::from_str(EMBEDDED_CATALOG).expect("embedded src/data/catalog.json is invalid")
    })
}

pub fn version() -> &'static str {
    &catalog().version
}

pub fn source_url() -> &'static str {
    &catalog().source_url
}

#[derive(Debug, Clone)]
pub struct Lookup {
    pub known: bool,
    pub target_type: Option<String>,
    pub known_operations: Option<Vec<String>>,
}

pub fn lookup(scope_type: &str) -> Lookup {
    match catalog().scopes.get(scope_type) {
        Some(info) => Lookup {
            known: true,
            target_type: Some(info.target_type.clone()),
            known_operations: Some(info.operations.clone()),
        },
        None => Lookup {
            known: false,
            target_type: None,
            known_operations: None,
        },
    }
}

/// Operations the user asked for that the catalog doesn't list for this scope.
/// Returns an empty Vec for unknown scopes (callers should check `lookup().known`).
pub fn unknown_operations(scope_type: &str, requested: &[String]) -> Vec<String> {
    let info = match catalog().scopes.get(scope_type) {
        Some(i) => i,
        None => return Vec::new(),
    };
    let known: std::collections::HashSet<&str> =
        info.operations.iter().map(|s| s.as_str()).collect();
    requested
        .iter()
        .filter(|op| !known.contains(op.as_str()))
        .cloned()
        .collect()
}

pub fn all_scopes() -> Vec<String> {
    let mut v: Vec<String> = catalog().scopes.keys().cloned().collect();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_loads() {
        assert!(!version().is_empty());
        assert!(!all_scopes().is_empty());
    }

    #[test]
    fn known_scope_lookup() {
        let l = lookup("universe");
        assert!(l.known);
        assert_eq!(l.target_type.as_deref(), Some("universe"));
        assert!(l.known_operations.unwrap().contains(&"read".to_string()));
    }

    #[test]
    fn unknown_scope_lookup() {
        let l = lookup("definitely-not-a-real-scope");
        assert!(!l.known);
        assert!(l.target_type.is_none());
    }

    #[test]
    fn unknown_ops_for_known_scope() {
        let unk = unknown_operations("universe", &["read".into(), "destroy".into()]);
        assert_eq!(unk, vec!["destroy".to_string()]);
    }

    #[test]
    fn unknown_ops_for_unknown_scope_is_empty() {
        let unk = unknown_operations("not-real", &["x".into()]);
        assert!(unk.is_empty());
    }
}
