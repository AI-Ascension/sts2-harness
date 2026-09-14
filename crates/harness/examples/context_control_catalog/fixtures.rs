// SPDX-License-Identifier: MIT

//! Synthetic catalog fixtures built by the public producer, not consumer replicas.

use serde_json::{Value, json};
use sts2_harness::management::{
    CONTEXT_OWNER_BINDING_SCHEMA_VERSION, CONTEXT_OWNER_CATALOG_SCHEMA_VERSION,
    ContextBindingCatalog, ContextBindingContinuity, ContextBindingDescriptor,
    ContextBindingGrants, ContextBindingState, ContextEffectiveLimits,
};

pub const ORIGIN: &str = "997821420db5a32d40cd2f07ad8f0ebcafcda96c";
pub const FIELDS: [(&str, u64, u64, u64); 5] = [
    ("max_items", 1, 64, 8),
    ("max_notes", 0, 16, 4),
    ("max_context_bytes", 1, 131_072, 16_384),
    ("max_objective_bytes", 1, 512, 128),
    ("max_control_events", 1, 4096, 128),
];
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn descriptor() -> Result<ContextBindingDescriptor> {
    Ok(ContextBindingDescriptor {
        schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.to_owned(),
        binding_id: "fixture.context.binding.v1".to_owned(),
        version: 1,
        digest: String::new(),
        context_ref: "context.fixture.v1".to_owned(),
        node_kinds: vec!["analyze".to_owned(), "decide".to_owned()],
        sources: Vec::new(),
        operations: Vec::new(),
        effective_limits: ContextEffectiveLimits::default(),
        continuity: ContextBindingContinuity {
            survives_controller_restart: false,
            receipt_recovery: false,
            provider_session_continuity: false,
        },
        grants: ContextBindingGrants {
            metadata_read: true,
            ..ContextBindingGrants::default()
        },
        state: ContextBindingState::Available,
    }
    .seal()?)
}

pub fn catalog(descriptor: ContextBindingDescriptor) -> Result<ContextBindingCatalog> {
    Ok(ContextBindingCatalog {
        schema_version: CONTEXT_OWNER_CATALOG_SCHEMA_VERSION.to_owned(),
        owner_id: "fixture.context-owner".to_owned(),
        owner_version: "1.0.0".to_owned(),
        catalog_digest: String::new(),
        descriptors: vec![descriptor],
    }
    .seal()?)
}

pub fn with_limit(
    descriptor: &ContextBindingDescriptor,
    field: &str,
    value: Value,
) -> Result<ContextBindingDescriptor> {
    let mut encoded = serde_json::to_value(descriptor)?;
    encoded["effective_limits"][field] = value;
    Ok(serde_json::from_value::<ContextBindingDescriptor>(encoded)?.seal()?)
}

fn catalogs() -> Result<Vec<Value>> {
    let default = descriptor()?;
    let mut restricted = default.clone();
    for (field, _, _, selected) in FIELDS {
        restricted = with_limit(&restricted, field, json!(selected))?;
    }
    let zero_notes = with_limit(&default, "max_notes", json!(0))?;
    let mut disabled = default.clone();
    disabled.state = ContextBindingState::Disabled;
    disabled = disabled.seal()?;
    [
        ("default", default),
        ("restricted", restricted),
        ("zero_notes", zero_notes),
        ("disabled", disabled),
    ]
    .into_iter()
    .map(|(name, descriptor)| {
        let catalog = catalog(descriptor)?;
        catalog.validate()?;
        let usable = catalog
            .descriptor_for("context.fixture.v1", "decide")
            .is_ok();
        Ok(json!({"name": name, "catalog": catalog, "binding_usable": usable}))
    })
    .collect()
}

fn boundaries() -> Result<Vec<Value>> {
    let descriptor = descriptor()?;
    let mut rows = Vec::new();
    for (field, minimum, maximum, selected) in FIELDS {
        let below = minimum
            .checked_sub(1)
            .map_or(json!(-1), |value| json!(value));
        for (case, value) in [
            ("below_minimum", below),
            ("minimum", json!(minimum)),
            ("selected", json!(selected)),
            ("global_maximum", json!(maximum)),
            ("one_over_global", json!(maximum + 1)),
        ] {
            let outcome = match with_limit(&descriptor, field, value.clone()) {
                Err(_) => "wire_decode_rejected".to_owned(),
                Ok(candidate) => candidate
                    .validate()
                    .map_or_else(|error| error.code, |()| "valid".to_owned()),
            };
            rows.push(json!({
                "field": field, "case": case, "value": value,
                "descriptor_result": outcome
            }));
        }
    }
    Ok(rows)
}

pub fn bytes() -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(&json!({
        "fixture_schema": "ascension.context-control.catalog-conformance.fixture.v1",
        "license": "MIT",
        "origin_revision": ORIGIN,
        "evidence": "synthetic_descriptor_validation_only",
        "catalogs": catalogs()?,
        "descriptor_boundaries": boundaries()?
    }))?;
    bytes.push(b'\n');
    Ok(bytes)
}
