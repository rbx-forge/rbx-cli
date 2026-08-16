#![allow(clippy::unwrap_used)]

use rbx_config::value::{canonical, compact, json_to_toml, toml_to_json, type_label};
use serde_json::json;
use toml::Value as Toml;

#[test]
fn toml_int_to_json_int() {
    let t = Toml::Integer(42);
    assert_eq!(toml_to_json(t), json!(42));
}

#[test]
fn toml_string_to_json_string() {
    let t = Toml::String("hello".into());
    assert_eq!(toml_to_json(t), json!("hello"));
}

#[test]
fn toml_array_to_json_array() {
    let t: Toml = toml::from_str::<Toml>("v = [1, 2, 3]")
        .unwrap()
        .as_table()
        .unwrap()
        .get("v")
        .unwrap()
        .clone();
    assert_eq!(toml_to_json(t), json!([1, 2, 3]));
}

#[test]
fn toml_table_to_json_object() {
    let t: Toml = toml::from_str::<Toml>("[obj]\na = 1\nb = \"x\"")
        .unwrap()
        .get("obj")
        .unwrap()
        .clone();
    let j = toml_to_json(t);
    assert_eq!(j, json!({"a": 1, "b": "x"}));
}

#[test]
fn json_to_toml_round_trip_primitives() {
    let cases = vec![json!(1), json!("hi"), json!(true), json!(2.5)];
    for j in cases {
        let t = json_to_toml(j.clone());
        let back = toml_to_json(t);
        assert_eq!(j, back);
    }
}

#[test]
fn json_to_toml_handles_arrays_and_objects() {
    let j = json!({"a": 1, "b": [1, 2, 3], "c": {"d": "x"}});
    let t = json_to_toml(j.clone());
    let back = toml_to_json(t);
    assert_eq!(j, back);
}

#[test]
fn compact_short_values_returned_inline() {
    assert_eq!(compact(&json!(42)), "42");
    assert_eq!(compact(&json!("hi")), "\"hi\"");
    assert_eq!(compact(&json!(true)), "true");
}

#[test]
fn compact_truncates_long_values() {
    let long_string = json!("a".repeat(300));
    let out = compact(&long_string);
    assert!(out.len() < 300, "compact should truncate long values");
}

#[test]
fn type_label_returns_human_readable_type() {
    assert_eq!(type_label(&json!(42)), "number");
    assert_eq!(type_label(&json!(2.5)), "number");
    assert_eq!(type_label(&json!("x")), "string");
    assert_eq!(type_label(&json!(true)), "bool");
    assert_eq!(type_label(&json!(null)), "null");
    assert_eq!(type_label(&json!([])), "array");
    assert_eq!(type_label(&json!({})), "object");
}

#[test]
fn canonical_sorts_object_keys() {
    let a = canonical(&json!({"b": 1, "a": 2}));
    let b = canonical(&json!({"a": 2, "b": 1}));
    assert_eq!(a, b);
}

#[test]
fn canonical_recurses_into_arrays() {
    let a = canonical(&json!({"x": [{"b": 1, "a": 2}]}));
    let b = canonical(&json!({"x": [{"a": 2, "b": 1}]}));
    assert_eq!(a, b);
}

#[test]
fn canonical_different_values_produce_different_strings() {
    let a = canonical(&json!({"a": 1}));
    let b = canonical(&json!({"a": 2}));
    assert_ne!(a, b);
}
