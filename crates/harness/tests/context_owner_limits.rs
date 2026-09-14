// SPDX-License-Identifier: MIT

#[path = "../examples/context_control_catalog/fixtures.rs"]
mod fixtures;

use serde_json::{Value, json};
use sts2_harness::management::{ContextBindingCatalog, ContextBindingDescriptor};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn descriptor_ranges_distinguish_decode_minima_and_ceilings() -> Result {
    let fixture: Value = serde_json::from_slice(&fixtures::bytes()?)?;
    let rows = fixture["descriptor_boundaries"]
        .as_array()
        .ok_or("missing boundary rows")?;
    assert_eq!(rows.len(), 25);
    for row in rows {
        let expected = match (row["field"].as_str(), row["case"].as_str()) {
            (Some("max_notes"), Some("below_minimum")) => "wire_decode_rejected",
            (_, Some("below_minimum" | "one_over_global")) => "context_effective_limits_invalid",
            _ => "valid",
        };
        assert_eq!(row["descriptor_result"], expected, "{row}");
    }
    Ok(())
}

#[test]
fn actual_catalogs_validate_and_disabled_is_discoverable_but_unusable() -> Result {
    let fixture: Value = serde_json::from_slice(&fixtures::bytes()?)?;
    let rows = fixture["catalogs"].as_array().ok_or("missing catalogs")?;
    assert_eq!(rows.len(), 4);
    for row in rows {
        let catalog: ContextBindingCatalog = serde_json::from_value(row["catalog"].clone())?;
        catalog.validate()?;
        let binding = catalog.descriptor_for("context.fixture.v1", "decide");
        if row["name"] == "disabled" {
            assert_eq!(catalog.descriptors.len(), 1);
            assert_eq!(
                binding.err().ok_or("disabled binding usable")?.code,
                "context_binding_unsupported"
            );
        } else {
            binding?;
        }
    }
    Ok(())
}

#[test]
fn every_limit_is_bound_by_descriptor_and_catalog_digest() -> Result {
    let original = fixtures::descriptor()?;
    let catalog = fixtures::catalog(original.clone())?;
    for (field, _, _, selected) in fixtures::FIELDS {
        let resealed = fixtures::with_limit(&original, field, json!(selected))?;
        resealed.validate()?;
        assert_ne!(original.digest, resealed.digest, "{field}");
        let mut tampered = resealed.clone();
        tampered.digest = original.digest.clone();
        assert_eq!(
            tampered.validate().err().ok_or("tamper admitted")?.code,
            "context_binding_digest_mismatch"
        );
        let mut stale_catalog = catalog.clone();
        stale_catalog.descriptors[0] = resealed;
        assert_eq!(
            stale_catalog
                .validate()
                .err()
                .ok_or("stale catalog admitted")?
                .code,
            "context_catalog_digest_mismatch"
        );
        let resealed_catalog = stale_catalog.seal()?;
        resealed_catalog.validate()?;
        assert_ne!(
            resealed_catalog.catalog_digest, catalog.catalog_digest,
            "{field}"
        );
    }
    Ok(())
}

#[test]
fn sealing_does_not_make_malformed_members_valid() -> Result {
    let original = fixtures::descriptor()?;
    let invalid = fixtures::with_limit(&original, "max_items", json!(65))?;
    let catalog = fixtures::catalog(invalid)?;
    assert_eq!(
        catalog
            .validate()
            .err()
            .ok_or("invalid member admitted")?
            .code,
        "context_effective_limits_invalid"
    );
    let mut duplicate = fixtures::catalog(original.clone())?;
    duplicate.descriptors.push(original.clone());
    assert_eq!(
        duplicate
            .seal()?
            .validate()
            .err()
            .ok_or("duplicate admitted")?
            .code,
        "context_binding_duplicate"
    );
    for field in ["missing", "unknown"] {
        let mut encoded = serde_json::to_value(&original)?;
        let limits = encoded["effective_limits"]
            .as_object_mut()
            .ok_or("limits not object")?;
        if field == "missing" {
            limits.remove("max_notes");
        } else {
            limits.insert("unlimited".to_owned(), json!(true));
        }
        assert!(serde_json::from_value::<ContextBindingDescriptor>(encoded).is_err());
    }
    Ok(())
}

#[test]
fn committed_fixture_is_exact_current_producer_output() -> Result {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/context-control/catalog-conformance.json");
    assert_eq!(fixtures::bytes()?, std::fs::read(path)?);
    Ok(())
}
