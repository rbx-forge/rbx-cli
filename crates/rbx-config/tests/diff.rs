#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use rbx_config::diff::{ChangeKind, Diff};
use serde_json::{json, Value as Json};

fn map(entries: &[(&str, Json)]) -> BTreeMap<String, Json> {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

#[test]
fn no_changes_when_identical() {
    let local = map(&[("foo", json!(42)), ("bar", json!("hello"))]);
    let remote = local.clone();
    let d = Diff::compute(&local, &remote);
    assert!(d.changes.is_empty());
    assert_eq!(d.unchanged.len(), 2);
}

#[test]
fn add_when_key_only_in_local() {
    let local = map(&[("new", json!(1))]);
    let remote = BTreeMap::new();
    let d = Diff::compute(&local, &remote);
    assert_eq!(d.changes.len(), 1);
    assert_eq!(d.changes[0].kind, ChangeKind::Add);
    assert_eq!(d.changes[0].key, "new");
    assert!(d.changes[0].old_value.is_none());
    assert_eq!(d.changes[0].new_value, Some(json!(1)));
}

#[test]
fn remove_when_key_only_in_remote() {
    let local = BTreeMap::new();
    let remote = map(&[("stale", json!("old"))]);
    let d = Diff::compute(&local, &remote);
    assert_eq!(d.changes.len(), 1);
    assert_eq!(d.changes[0].kind, ChangeKind::Remove);
    assert_eq!(d.changes[0].key, "stale");
    assert_eq!(d.changes[0].old_value, Some(json!("old")));
    assert!(d.changes[0].new_value.is_none());
}

#[test]
fn update_when_value_differs() {
    let local = map(&[("x", json!(10))]);
    let remote = map(&[("x", json!(5))]);
    let d = Diff::compute(&local, &remote);
    assert_eq!(d.changes.len(), 1);
    assert_eq!(d.changes[0].kind, ChangeKind::Update);
    assert_eq!(d.changes[0].old_value, Some(json!(5)));
    assert_eq!(d.changes[0].new_value, Some(json!(10)));
}

#[test]
fn changes_are_sorted_add_update_remove() {
    let local = map(&[("a_new", json!(1)), ("c_upd", json!("new"))]);
    let remote = map(&[("c_upd", json!("old")), ("b_rm", json!(2))]);
    let d = Diff::compute(&local, &remote);

    // Expected order: a_new (Add), c_upd (Update), b_rm (Remove)
    assert_eq!(d.changes.len(), 3);
    assert_eq!(d.changes[0].key, "a_new");
    assert_eq!(d.changes[0].kind, ChangeKind::Add);
    assert_eq!(d.changes[1].key, "c_upd");
    assert_eq!(d.changes[1].kind, ChangeKind::Update);
    assert_eq!(d.changes[2].key, "b_rm");
    assert_eq!(d.changes[2].kind, ChangeKind::Remove);
}

#[test]
fn changes_sorted_alphabetically_within_same_kind() {
    let local = map(&[("zoo", json!(1)), ("apple", json!(2)), ("middle", json!(3))]);
    let remote = BTreeMap::new();
    let d = Diff::compute(&local, &remote);
    let keys: Vec<&str> = d.changes.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys, vec!["apple", "middle", "zoo"]);
}

#[test]
fn equal_complex_values_treated_as_unchanged() {
    let local = map(&[("obj", json!({"a": 1, "b": [1, 2, 3]}))]);
    let remote = local.clone();
    let d = Diff::compute(&local, &remote);
    assert!(d.changes.is_empty());
    assert_eq!(d.unchanged, vec!["obj".to_string()]);
}

#[test]
fn key_order_in_object_doesnt_matter_for_equality() {
    // canonical() should sort object keys so `{a:1, b:2}` equals `{b:2, a:1}`.
    let local = map(&[("obj", json!({"a": 1, "b": 2}))]);
    let remote = map(&[("obj", json!({"b": 2, "a": 1}))]);
    let d = Diff::compute(&local, &remote);
    assert!(
        d.changes.is_empty(),
        "object key order should not produce a diff: {:?}",
        d.changes
    );
}

#[test]
fn nested_value_change_detected() {
    let local = map(&[("obj", json!({"a": 1, "b": 99}))]);
    let remote = map(&[("obj", json!({"a": 1, "b": 2}))]);
    let d = Diff::compute(&local, &remote);
    assert_eq!(d.changes.len(), 1);
    assert_eq!(d.changes[0].kind, ChangeKind::Update);
}
