// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::PrivateRoot;
use crate::config::{
    PROVIDER_ENDPOINT, SUPPORTED_CONTEXT_MODES, SUPPORTED_DECISIONS, SUPPORTED_PROFILES,
    SYNTHETIC_MODEL, UNSUPPORTED_DECISIONS, UNSUPPORTED_PROFILE_CODE, UNSUPPORTED_PROFILES,
    UNSUPPORTED_RECOVERY_CODE, capability_fields, provider_route_admitted,
    synthetic_route_admitted, unsupported_profile_axis,
};
use serde_json::{Value, json};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use sts2_harness::{EXO_SOURCE_REVISION, ExoBridgeRequestEnvelope, parse_bridge_request_envelope};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const REQUEST: &[u8] =
    include_bytes!("../../../../../protocol-artifact/exo-bridge-v1/golden/request.json");

fn request_envelope() -> ExoBridgeRequestEnvelope {
    let value = json!({
        "wire_version": "sts2.exo-bridge-wire-v1",
        "request_id": "request-1",
        "turn_id": "turn-1",
        "request": serde_json::from_slice::<Value>(REQUEST).expect("golden request is JSON")
    });
    let bytes = serde_json::to_vec(&value).expect("envelope serializes");
    parse_bridge_request_envelope(&bytes, 131_072).expect("golden envelope parses")
}

/// A complete executor receipt whose only variable part is the decision or failure code.
fn receipt(decision: Option<&str>, error_code: Option<&str>) -> Vec<u8> {
    let mut value = json!({
        "version": "sts2.exo-executor-receipt-v2",
        "request_id": "request-1",
        "host_turn_id": "turn-1",
        "exo_agent_id": "11111111-1111-4111-8111-111111111111",
        "exo_conversation_id": "22222222-2222-4222-8222-222222222222",
        "exo_turn_id": "33333333-3333-4333-8333-333333333333",
        "exo_session_id": "44444444-4444-4444-8444-444444444444",
        "event_cursor": "55555555-5555-4555-8555-555555555555",
        "fetch_attempts": 1,
        "forwarded_requests": 1,
        "denied_requests": 0
    });
    value["decision"] = decision.map_or(Value::Null, |text| json!(text));
    value["error_code"] = error_code.map_or(Value::Null, |text| json!(text));
    serde_json::to_vec(&value).expect("receipt serializes")
}

/// `response` is the shipped guard: `Ok` is the only dispatchable outcome, and it carries exactly
/// one correlated decision envelope.
#[test]
fn only_a_legal_correlated_decision_becomes_a_dispatchable_response() {
    let envelope = request_envelope();
    let legal = &envelope.request.legal_action_ids[0];
    let accepted =
        format!(r#"{{"decision":"action","action_id":"{legal}","rationale":"synthetic"}}"#);
    let output = super::response(&envelope, &receipt(Some(&accepted), None))
        .expect("a legal action decision is dispatchable");
    let decoded: Value = serde_json::from_slice(&output).expect("response is JSON");
    assert_eq!(decoded["request_id"], json!("request-1"));
    assert_eq!(decoded["turn_id"], json!("turn-1"));
    assert_eq!(decoded["outcome"], json!("decision"));
    assert_eq!(decoded["decision"]["action_id"], json!(legal));
}

/// Every negative receipt/decision must fail closed rather than produce an envelope.
#[test]
fn negative_receipts_and_decisions_never_produce_a_dispatchable_response() {
    let envelope = request_envelope();
    let legal = envelope.request.legal_action_ids[0].clone();
    let cases: [(&str, Vec<u8>); 13] = [
        (
            "illegal_action_id",
            receipt(
                Some(r#"{"decision":"action","action_id":"invented","rationale":"x"}"#),
                None,
            ),
        ),
        (
            "illegal_plan_id",
            receipt(
                Some(r#"{"decision":"plan","action_ids":["invented"],"rationale":"x"}"#),
                None,
            ),
        ),
        (
            "unsupported_recovery",
            receipt(
                Some(r#"{"decision":"recovery","recovery_kind":"reobserve","rationale":"x"}"#),
                None,
            ),
        ),
        (
            "unknown_decision_field",
            receipt(
                Some(r#"{"decision":"wait","rationale":"x","extra":1}"#),
                None,
            ),
        ),
        ("multiple_json", receipt(Some("{}{}"), None)),
        ("truncated_json", receipt(Some(r#"{"decision":"#), None)),
        ("empty_decision", receipt(Some(""), None)),
        ("oversized_decision", receipt(Some(&"x".repeat(8193)), None)),
        ("missing_decision", receipt(None, None)),
        ("executor_error_code", receipt(None, Some("remote_failure"))),
        ("malformed_receipt", b"{".to_vec()),
        ("wrong_receipt_identity", {
            let mut value: Value = serde_json::from_slice(&receipt(
                Some(r#"{"decision":"wait","rationale":"x"}"#),
                None,
            ))
            .expect("receipt is JSON");
            value["request_id"] = json!("other-request");
            serde_json::to_vec(&value).expect("receipt serializes")
        }),
        ("nil_receipt_identity", {
            let mut value: Value = serde_json::from_slice(&receipt(
                Some(r#"{"decision":"wait","rationale":"x"}"#),
                None,
            ))
            .expect("receipt is JSON");
            value["exo_turn_id"] = json!("00000000-0000-0000-0000-000000000000");
            serde_json::to_vec(&value).expect("receipt serializes")
        }),
    ];
    for (name, bytes) in cases {
        assert!(
            super::response(&envelope, &bytes).is_err(),
            "{name} produced a dispatchable response"
        );
    }
    // A legal decision through the same path still succeeds, so this is not blanket denial.
    let accepted = format!(r#"{{"decision":"wait","rationale":"{legal}"}}"#);
    assert!(super::response(&envelope, &receipt(Some(&accepted), None)).is_ok());
}

/// The advertised capability document must describe exactly what the guard enforces.
#[test]
fn advertisement_matches_the_shipped_guard() {
    let fields = capability_fields();
    assert_eq!(fields["profiles"], json!(SUPPORTED_PROFILES));
    assert_eq!(fields["context_modes"], json!(SUPPORTED_CONTEXT_MODES));
    assert_eq!(fields["decisions"], json!(SUPPORTED_DECISIONS));
    assert_eq!(
        fields["unsupported_profile_code"],
        json!(UNSUPPORTED_PROFILE_CODE)
    );
    assert_eq!(
        fields["unsupported_recovery_code"],
        json!(UNSUPPORTED_RECOVERY_CODE)
    );
    for profile in UNSUPPORTED_PROFILES {
        assert_eq!(fields["profile_support"][profile], json!("unsupported"));
    }
    for decision in UNSUPPORTED_DECISIONS {
        assert_eq!(fields["decision_support"][decision], json!("unsupported"));
    }
    // The guard refuses exactly the axes the advertisement calls unsupported.
    let envelope = request_envelope();
    assert_eq!(unsupported_profile_axis(&envelope.request), None);
    assert_eq!(envelope.request.provider_revision, EXO_SOURCE_REVISION);
}

/// The synthetic smoke route must be structurally unable to reach an external provider.
#[test]
fn synthetic_route_cannot_reach_a_real_provider() {
    // The advertised capability document is itself non-inferencing.
    assert_eq!(capability_fields()["profiles"], json!(["standard"]));

    // Admitted: literal loopback with the synthetic-only model binding.
    assert!(synthetic_route_admitted(
        "http://127.0.0.1:8080",
        SYNTHETIC_MODEL
    ));
    assert!(synthetic_route_admitted(
        "http://127.0.0.1:1",
        SYNTHETIC_MODEL
    ));
    // Refused: the real provider route, any other host, port 0, and any other model.
    for (endpoint, model) in [
        (PROVIDER_ENDPOINT, SYNTHETIC_MODEL),
        ("https://api.openai.com/v1", "gpt-5"),
        ("http://127.0.0.1:8080", "gpt-5"),
        ("http://127.0.0.1:0", SYNTHETIC_MODEL),
        ("http://localhost:8080", SYNTHETIC_MODEL),
        ("http://127.0.0.2:8080", SYNTHETIC_MODEL),
        ("https://127.0.0.1:8080", SYNTHETIC_MODEL),
        ("http://127.0.0.1:8080/v1", SYNTHETIC_MODEL),
    ] {
        assert!(
            !synthetic_route_admitted(endpoint, model),
            "synthetic mode admitted {endpoint} with {model}"
        );
    }
    // The real route is admitted only as the exact reviewed HTTPS endpoint.
    assert!(provider_route_admitted(PROVIDER_ENDPOINT));
    for endpoint in [
        "http://api.openai.com/v1",
        "https://api.openai.com",
        "https://api.openai.com/v1/",
        "https://example.com/v1",
        "http://127.0.0.1:8080",
    ] {
        assert!(
            !provider_route_admitted(endpoint),
            "real mode admitted non-reviewed endpoint {endpoint}"
        );
    }
}

struct Parent(PathBuf);

impl Parent {
    fn new() -> Result<Self> {
        let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .canonicalize()?;
        let path = target.join(format!("exo-private-root-test-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Parent {
    fn drop(&mut self) {
        // Only this test's exclusively created parent is owned by this guard.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn relative_temporary_parent_is_rejected() {
    assert!(PrivateRoot::create_under(Path::new("relative"), uuid::Uuid::new_v4()).is_err());
}

#[test]
fn symlink_temporary_parent_is_rejected() -> Result {
    let parent = Parent::new()?;
    let real = parent.0.join("real");
    std::fs::create_dir(&real)?;
    let link = parent.0.join("link");
    symlink(&real, &link)?;
    assert!(PrivateRoot::create_under(&link, uuid::Uuid::new_v4()).is_err());
    assert_eq!(std::fs::read_dir(real)?.count(), 0);
    Ok(())
}

#[test]
fn existing_private_child_is_not_reused_or_removed() -> Result {
    let parent = Parent::new()?;
    let identity = uuid::Uuid::new_v4();
    let existing = parent.0.join(format!("sts2-exo-{identity}"));
    std::fs::create_dir(&existing)?;
    let marker = existing.join("marker");
    std::fs::write(&marker, b"existing child")?;
    assert!(PrivateRoot::create_under(&parent.0, identity).is_err());
    let linked_identity = uuid::Uuid::new_v4();
    let link = parent.0.join(format!("sts2-exo-{linked_identity}"));
    symlink(&existing, &link)?;
    assert!(PrivateRoot::create_under(&parent.0, linked_identity).is_err());
    assert!(std::fs::symlink_metadata(link)?.file_type().is_symlink());
    assert_eq!(std::fs::read(marker)?, b"existing child");
    Ok(())
}

#[test]
fn cleanup_is_confined_to_the_owned_private_child() -> Result {
    let parent = Parent::new()?;
    let sibling = parent.0.join("sibling");
    std::fs::write(&sibling, b"preserved")?;
    let private = PrivateRoot::create_under(&parent.0, uuid::Uuid::new_v4())?;
    for path in [
        private.0.clone(),
        private.0.join("state"),
        private.0.join("temp"),
    ] {
        assert_eq!(std::fs::metadata(path)?.permissions().mode() & 0o777, 0o700);
    }
    private.remove()?;
    assert!(!private.0.exists());
    assert_eq!(std::fs::read(sibling)?, b"preserved");
    Ok(())
}
