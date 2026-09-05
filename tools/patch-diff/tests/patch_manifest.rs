// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

const SCHEMA: &str = include_str!("../../../patch-manifest.schema.json");
const MANIFEST: &str =
    include_str!("../../../docs/evidence/runtime-v3-preparation/data/build-manifest.json");

#[test]
fn canonical_manifest_conforms_to_full_draft_2020_12_schema_and_stays_quarantined()
-> Result<(), Box<dyn std::error::Error>> {
    let schema: Value = serde_json::from_str(SCHEMA)?;
    jsonschema::meta::validate(&schema).map_err(|error| error.to_string())?;
    let validator = jsonschema::draft202012::new(&schema)?;
    let manifest: Value = serde_json::from_str(MANIFEST)?;
    let errors: Vec<_> = validator
        .iter_errors(&manifest)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "schema errors: {errors:?}");
    assert_eq!(manifest["quarantine"]["status"], "quarantined");
    Ok(())
}

#[test]
fn schema_gate_rejects_nested_contract_violations() -> Result<(), Box<dyn std::error::Error>> {
    let schema: Value = serde_json::from_str(SCHEMA)?;
    let validator = jsonschema::draft202012::new(&schema)?;
    let canonical: Value = serde_json::from_str(MANIFEST)?;
    for (pointer, invalid) in [
        ("/manifest_version", json!("unknown")),
        ("/evidence_status", json!("passed")),
        ("/provenance/generator", json!("different-generator")),
        ("/provenance/created_on", json!("not-a-date")),
        ("/provenance/licensed_runtime_available", json!("false")),
        ("/base/repository", json!("")),
        ("/candidate/revision", json!("x".repeat(2049))),
        (
            "/base/artifact_digests/runtime-v3-gameplay-schema",
            json!("fnv1a64-invalid"),
        ),
        ("/diffs/build/observed", json!("false")),
        ("/diffs/actions/entries/0/change", json!("promoted")),
        ("/diffs/actions/entries/0/status", json!("unknown")),
        (
            "/diffs/schema/entries/0/after_digest",
            json!("A".repeat(64)),
        ),
        ("/diffs/build/entries", json!(vec![json!({}); 257])),
        ("/quarantine/status", json!("approved")),
        ("/quarantine/required_checks", json!(vec!["check"; 65])),
    ] {
        let mut candidate = canonical.clone();
        *candidate
            .pointer_mut(pointer)
            .ok_or("invalid test pointer")? = invalid;
        assert!(
            !validator.is_valid(&candidate),
            "accepted invalid {pointer}"
        );
    }
    for pointer in [
        "",
        "/base",
        "/candidate",
        "/provenance",
        "/diffs",
        "/diffs/build",
        "/diffs/actions/entries/0",
        "/quarantine",
    ] {
        let mut extra = canonical.clone();
        extra
            .pointer_mut(pointer)
            .and_then(Value::as_object_mut)
            .ok_or("invalid object pointer")?
            .insert("unexpected".to_owned(), json!(true));
        assert!(
            !validator.is_valid(&extra),
            "accepted extra field at {pointer}"
        );
    }
    for (pointer, field) in [
        ("", "base"),
        ("/base", "revision"),
        ("/diffs", "ui"),
        ("/diffs/actions/entries/0", "path"),
        ("/quarantine", "reason"),
    ] {
        let mut missing = canonical.clone();
        missing
            .pointer_mut(pointer)
            .and_then(Value::as_object_mut)
            .ok_or("invalid object pointer")?
            .remove(field);
        assert!(
            !validator.is_valid(&missing),
            "accepted missing {pointer}/{field}"
        );
    }
    Ok(())
}
