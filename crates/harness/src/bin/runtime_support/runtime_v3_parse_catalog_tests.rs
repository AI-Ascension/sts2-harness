// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::catalog::raw_catalog;

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).expect("catalog fixture is valid JSON")
}

#[test]
fn catalog_retention_preserves_noncanonical_wire_bytes() {
    let text = r#"{"legal_actions" : [ {"action_id":"a","action":{"kind":"end_turn","label":"\u0061"}} ]}"#;
    let value = parsed(text);
    let semantic = value["legal_actions"].clone();
    assert_eq!(
        raw_catalog(Some(text), &semantic).expect("raw catalog is retained"),
        br#"[ {"action_id":"a","action":{"kind":"end_turn","label":"\u0061"}} ]"#
    );
}

#[test]
fn catalog_retention_rejects_duplicate_members_and_trailing_bytes() {
    let duplicate_root = r#"{"legal_actions":[],"legal_actions":[]}"#;
    assert!(raw_catalog(Some(duplicate_root), &json!([])).is_err());

    let duplicate_nested =
        r#"{"legal_actions":[{"action":{"kind":"end_turn","kind":"end_turn"}}]}"#;
    assert!(raw_catalog(Some(duplicate_nested), &json!([])).is_err());

    let trailing = r#"{"legal_actions":[]} trailing"#;
    assert!(raw_catalog(Some(trailing), &json!([])).is_err());
}

#[test]
fn catalog_retention_rejects_malformed_deep_and_oversized_input() {
    assert!(raw_catalog(Some(r#"{"legal_actions":[}"#), &json!([])).is_err());

    let deep = format!(
        r#"{{"legal_actions":{}{}}}"#,
        "[".repeat(65),
        "]".repeat(65)
    );
    assert!(raw_catalog(Some(&deep), &json!([])).is_err());

    let large_string = "x".repeat(sts2_harness::MAX_CATALOG_BYTES);
    let large = format!(r#"{{"legal_actions":["{large_string}"]}}"#);
    let value = parsed(&large);
    assert!(raw_catalog(Some(&large), &value["legal_actions"]).is_err());
}

#[test]
fn catalog_retention_rejects_semantic_mismatch_without_reserializing() {
    let text = r#"{"legal_actions":[1]}"#;
    assert!(raw_catalog(Some(text), &json!([2])).is_err());
}
