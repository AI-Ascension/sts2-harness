// SPDX-License-Identifier: MIT

//! #755's acceptance bar, in full: the rename reaches BOTH the Rust side and the contract, and the
//! contract says in its own text that the field is not an admission control.
//!
//! This is the "a test that fails on today's tree" the issue asked for. It lives in its own file
//! rather than in `effective_limits_conformance.rs` because that file is at its preferred size
//! budget, and a policy waiver would be a worse answer than a second test file.

#![allow(clippy::expect_used)]

use serde_json::Value;
use sts2_harness::provider_session::{CapabilityProvenance, NativeCapabilities};

/// #755's acceptance bar, in full: the rename reaches BOTH the Rust side and the contract, and
/// the contract says in its own text that the field is not an admission control.
///
/// This is the "a test that fails on today's tree" the issue asked for. Before the rename it fails
/// four ways, and each failure is the specific thing the decision required:
///
/// * the serialized descriptor still carries `evidence`, so `provenance` is absent from it;
/// * the schema still declares `evidence` and so does not accept `provenance` at all
///   (`additionalProperties` is `false`, so the old schema rejects the renamed field outright);
/// * the schema has no `description` on the field, so the non-admission role is undocumented;
/// * `policy.schema.json` is silent on the subject, so a reader of either contract could still
///   believe the field gates something.
#[test]
fn provenance_is_renamed_in_rust_and_schema_and_declared_not_to_gate() {
    let capability_schema: Value = serde_json::from_slice(include_bytes!(
        "../../../contracts/provider-session/capabilities.schema.json"
    ))
    .expect("session capabilities schema");
    let policy_schema: Value = serde_json::from_slice(include_bytes!(
        "../../../contracts/provider-session/policy.schema.json"
    ))
    .expect("session policy schema");

    let properties = capability_schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("capabilities schema properties");

    // 1. The Rust descriptor serializes under the new name.
    let descriptor = serde_json::to_value(NativeCapabilities::fixture()).expect("descriptor");
    assert!(
        descriptor.get("provenance").is_some(),
        "the Rust descriptor must serialize `provenance`; it still carries `evidence`"
    );
    assert!(
        descriptor.get("evidence").is_none(),
        "`evidence` is gone from the Rust descriptor: it is the name that invited a tier comparison"
    );

    // 2. The schema accepts the new name and no longer declares the old one.
    assert!(
        properties.contains_key("provenance"),
        "capabilities.schema.json must declare `provenance`"
    );
    assert!(
        !properties.contains_key("evidence"),
        "capabilities.schema.json must no longer declare `evidence`"
    );
    let validator = jsonschema::validator_for(&capability_schema).expect("capabilities validator");
    assert!(
        validator.is_valid(&descriptor),
        "the renamed descriptor must satisfy the renamed schema: {descriptor}"
    );

    // 3. The contract states the non-admission role rather than leaving it to be inferred.
    let description = properties
        .get("provenance")
        .and_then(|field| field.get("description"))
        .and_then(Value::as_str)
        .expect("provenance must carry a description");
    let described = description.to_ascii_lowercase();
    assert!(
        described.contains("not an admission control"),
        "the schema must say the field is not an admission control, so a reader does not infer a \
         tier from the variant names: {description}"
    );
    assert!(
        described.contains("provenance"),
        "the schema description must name the field's actual role: {description}"
    );

    // 4. The policy contract records the negative fact, so nobody adds a floor by assumption.
    let policy_text = policy_schema.to_string().to_ascii_lowercase();
    assert!(
        policy_text.contains("provenance"),
        "policy.schema.json is currently silent on `provenance`; it must record that there is \
         deliberately no policy field for it and why"
    );
    assert!(
        !policy_schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| properties.contains_key("provenance")),
        "policy.schema.json must not gain a `provenance` property: a floor here could not be \
         honestly evaluated, because `live_provider` asserts a runtime event outside the process"
    );
}

/// #755's third acceptance box: the rename must not weaken integrity.
///
/// `descriptor_digest()` covers the whole struct with `binding.descriptor_sha256` cleared, so
/// `provenance` is inside the digest exactly as `evidence` was. Relabelling and recomputing is a
/// legitimate new descriptor; relabelling *without* recomputing is tampering and stays refused.
///
/// This is the half that keeps the other test honest. `provenance_is_documented_as_ignored` in
/// `capabilities.rs` accepts a relabelled descriptor *after* recomputing the digest; if the digest
/// did not actually cover the field, that test would be passing because the digest check rejected
/// the descriptor for an unrelated reason, and the whole "provenance is ignored" claim would be
/// untested.
#[test]
fn descriptor_digest_still_covers_provenance_after_the_rename() {
    let mut capabilities = NativeCapabilities::reviewed_exo_lifecycle(
        "sts2-exo-lifecycle-v2",
        "1".repeat(64),
        "2".repeat(64),
        "3".repeat(64),
    )
    .expect("reviewed exo lifecycle descriptor");

    let before = capabilities.descriptor_digest();
    capabilities.provenance = CapabilityProvenance::LiveProvider;
    let after = capabilities.descriptor_digest();
    assert_ne!(
        before, after,
        "descriptor_digest() must cover provenance, or a descriptor could be relabelled without \
         invalidating its own digest and the integrity guarantee would be lost"
    );

    // Tampering without recomputing stays refused; this is the property that has to survive.
    assert!(
        capabilities.validate().is_err(),
        "a relabelled descriptor whose digest was not recomputed must be refused"
    );

    // And the honest form of the same relabelling is accepted, which is what makes the pair above a
    // statement about integrity rather than about the provenance value itself.
    capabilities.binding.descriptor_sha256 = after;
    assert!(
        capabilities.validate().is_ok(),
        "a relabelled descriptor with a recomputed digest is internally consistent and validates; \
         validate() deliberately does not consult provenance, per #755's option 3"
    );
}
