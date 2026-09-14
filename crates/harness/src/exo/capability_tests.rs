// SPDX-License-Identifier: MIT

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

use super::*;

const REVISION: &str = "7801005e6a1ab77008a05dbba80e0a2a7a56e35d";
const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn expectation() -> ExoPreflightExpectation<'static> {
    ExoPreflightExpectation {
        provider_revision: REVISION,
        package_digest: DIGEST,
        platform: "linux",
        required_projection: "standard",
    }
}

fn descriptor() -> serde_json::Value {
    serde_json::json!({
        "schema": EXO_CAPABILITY_SCHEMA,
        "contract_version": EXO_CONTRACT_VERSION,
        "provider_revision": REVISION,
        "package_digest": DIGEST,
        "decision_kinds": ["action", "plan", "wait", "reobserve", "recovery"],
        "projections": ["standard", "map"],
        "context_modes": ["fresh", "managed"],
        "platform": "linux",
        "limits": {
            "max_request_bytes": 131072,
            "max_map_request_bytes": 393443,
            "max_decision_bytes": 8192,
            "max_turn_millis": 120000,
            "max_concurrency": 1
        },
        "lifecycle": {
            "cancellation": true,
            "restart_recovery": true,
            "idempotent_replay": true
        },
        "evidence": "synthetic"
    })
}

fn bytes(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("test descriptor serializes")
}

#[test]
fn accepts_a_well_formed_descriptor() -> Result<(), ExoPreflightError> {
    let validated = preflight(&bytes(&descriptor()), &expectation())?;
    assert_eq!(validated.provider_revision, REVISION);
    assert_eq!(validated.limits.max_map_request_bytes, 393_443);
    assert!(validated.lifecycle.idempotent_replay);
    Ok(())
}

#[test]
fn rejects_malformed_and_unknown_and_missing_fields() {
    assert_eq!(
        preflight(b"{", &expectation()),
        Err(ExoPreflightError::Malformed)
    );
    let mut unknown = descriptor();
    unknown["extra"] = serde_json::json!(1);
    assert_eq!(
        preflight(&bytes(&unknown), &expectation()),
        Err(ExoPreflightError::UnknownField)
    );
    let mut missing = descriptor();
    missing.as_object_mut().expect("object").remove("limits");
    assert_eq!(
        preflight(&bytes(&missing), &expectation()),
        Err(ExoPreflightError::MissingField)
    );
    let mut nested_unknown = descriptor();
    nested_unknown["limits"]["extra"] = serde_json::json!(1);
    assert_eq!(
        preflight(&bytes(&nested_unknown), &expectation()),
        Err(ExoPreflightError::UnknownField)
    );
}

#[test]
fn rejects_wrong_schema_contract_platform_revision_and_package() {
    let mut schema = descriptor();
    schema["schema"] = serde_json::json!("sts2.exo-capability-v2");
    assert_eq!(
        preflight(&bytes(&schema), &expectation()),
        Err(ExoPreflightError::UnsupportedSchema)
    );
    let mut contract = descriptor();
    contract["contract_version"] = serde_json::json!("sts2.exo-bridge-v2");
    assert_eq!(
        preflight(&bytes(&contract), &expectation()),
        Err(ExoPreflightError::UnsupportedContract)
    );
    let mut platform = descriptor();
    platform["platform"] = serde_json::json!("windows");
    assert_eq!(
        preflight(&bytes(&platform), &expectation()),
        Err(ExoPreflightError::UnsupportedPlatform)
    );
    let mut revision = descriptor();
    revision["provider_revision"] = serde_json::json!("b06869ab789dee3f80ca474b5fa89dbe47ccb859");
    assert_eq!(
        preflight(&bytes(&revision), &expectation()),
        Err(ExoPreflightError::WrongRevision)
    );
    let mut swapped = descriptor();
    swapped["package_digest"] =
        serde_json::json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    assert_eq!(
        preflight(&bytes(&swapped), &expectation()),
        Err(ExoPreflightError::SwappedPackage)
    );
}

#[test]
fn rejects_unsupported_capabilities() {
    let mut kind = descriptor();
    kind["decision_kinds"] = serde_json::json!(["action", "plan", "wait", "reobserve"]);
    assert_eq!(
        preflight(&bytes(&kind), &expectation()),
        Err(ExoPreflightError::UnsupportedDecisionKind)
    );
    let mut kind_unknown = descriptor();
    kind_unknown["decision_kinds"] =
        serde_json::json!(["action", "plan", "wait", "reobserve", "recovery", "shell"]);
    assert_eq!(
        preflight(&bytes(&kind_unknown), &expectation()),
        Err(ExoPreflightError::UnsupportedDecisionKind)
    );
    let mut projection = descriptor();
    projection["projections"] = serde_json::json!(["map"]);
    assert_eq!(
        preflight(&bytes(&projection), &expectation()),
        Err(ExoPreflightError::UnsupportedProjection)
    );
    let mut mode = descriptor();
    mode["context_modes"] = serde_json::json!(["managed"]);
    assert_eq!(
        preflight(&bytes(&mode), &expectation()),
        Err(ExoPreflightError::UnsupportedContextMode)
    );
    let mut evidence = descriptor();
    evidence["evidence"] = serde_json::json!("claimed");
    assert_eq!(
        preflight(&bytes(&evidence), &expectation()),
        Err(ExoPreflightError::UnsupportedEvidence)
    );
}

#[test]
fn rejects_out_of_bound_limits() {
    let mut oversized = descriptor();
    oversized["limits"]["max_map_request_bytes"] = serde_json::json!(400_000u64);
    assert_eq!(
        preflight(&bytes(&oversized), &expectation()),
        Err(ExoPreflightError::LimitExceeded)
    );
    let mut zero = descriptor();
    zero["limits"]["max_decision_bytes"] = serde_json::json!(0);
    assert_eq!(
        preflight(&bytes(&zero), &expectation()),
        Err(ExoPreflightError::LimitExceeded)
    );
    let mut long = descriptor();
    long["limits"]["max_turn_millis"] = serde_json::json!(120_001u64);
    assert_eq!(
        preflight(&bytes(&long), &expectation()),
        Err(ExoPreflightError::LimitExceeded)
    );
}

#[test]
fn requires_the_requested_map_projection() {
    let map_expectation = ExoPreflightExpectation {
        required_projection: "map",
        ..expectation()
    };
    let mut standard_only = descriptor();
    standard_only["projections"] = serde_json::json!(["standard"]);
    assert_eq!(
        preflight(&bytes(&standard_only), &map_expectation),
        Err(ExoPreflightError::UnsupportedProjection)
    );
    assert!(preflight(&bytes(&descriptor()), &map_expectation).is_ok());
}

#[test]
fn rejects_duplicate_keys_and_unbounded_concurrency() {
    let serialized = bytes(&descriptor());
    let mut duplicated = Vec::with_capacity(serialized.len() + 16);
    duplicated.push(b'{');
    duplicated.extend_from_slice(br#""schema":"x","#);
    duplicated.extend_from_slice(&serialized[1..]);
    assert_eq!(
        preflight(&duplicated, &expectation()),
        Err(ExoPreflightError::Malformed)
    );

    let serialized = String::from_utf8(bytes(&descriptor())).expect("utf8 descriptor");
    let nested = serialized.replacen("\"limits\":{", "\"limits\":{\"max_concurrency\":9,", 1);
    assert_ne!(nested, serialized, "nested insert point must exist");
    assert_eq!(
        preflight(nested.as_bytes(), &expectation()),
        Err(ExoPreflightError::Malformed)
    );

    let mut concurrent = descriptor();
    concurrent["limits"]["max_concurrency"] = serde_json::json!(u64::MAX);
    assert_eq!(
        preflight(&bytes(&concurrent), &expectation()),
        Err(ExoPreflightError::LimitExceeded)
    );
}
