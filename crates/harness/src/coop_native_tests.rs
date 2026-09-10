// SPDX-License-Identifier: MIT

use super::*;
use serde_json::{Value, json};
use std::error::Error;

fn lineage() -> Result<CoopNativeLineage, CoopNativeIdentityError> {
    Ok(CoopNativeLineage::new(
        CoopNativeInstanceId::new("instance:native-test")?,
        CoopNativeSessionId::new("session:native-test")?,
        CoopNativeRunId::new("run:native-test")?,
        CoopNativeEpisodeId::new("episode:native-test")?,
        CoopNativeTrajectoryId::new("trajectory:native-test")?,
        CoopNativeRequestId::new("request:native-test")?,
        None,
        None,
        CoopNativeTraceId::new("trace:native-test")?,
        None,
        CoopNativeArtifactId::new("artifact:native-test")?,
    ))
}

fn golden(name: &str) -> &'static [u8] {
    match name {
        "observation-response" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/observation-response.json"
        ),
        "legal-catalog-request" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/legal-catalog-request.json"
        ),
        "legal-catalog-response" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/legal-catalog-response.json"
        ),
        "local-action-settled-request" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/local-action-settled-request.json"
        ),
        "local-action-settled-response" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/local-action-settled-response.json"
        ),
        "local-action-rejected-request" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/local-action-rejected-request.json"
        ),
        "local-action-rejected-response" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/local-action-rejected-response.json"
        ),
        "local-action-unknown-request" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/local-action-unknown-request.json"
        ),
        "local-action-unknown-response" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/local-action-unknown-response.json"
        ),
        "local-action-recovered-request" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/local-action-recovered-request.json"
        ),
        "local-action-recovered-response" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/local-action-recovered-response.json"
        ),
        "shared-vote-settled-request" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/shared-vote-settled-request.json"
        ),
        "shared-vote-settled-response" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/shared-vote-settled-response.json"
        ),
        "rejoin-pending-request" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/rejoin-pending-request.json"
        ),
        "rejoin-pending-response" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/rejoin-pending-response.json"
        ),
        "rejoin-recovered-request" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/rejoin-recovered-request.json"
        ),
        "rejoin-recovered-response" => include_bytes!(
            "../../../protocol-artifact/coop-native-v1/golden/rejoin-recovered-response.json"
        ),
        _ => &[],
    }
}

fn parse_request(name: &str) -> Result<CoopNativeEnvelope, CoopNativeEnvelopeError> {
    CoopNativeEnvelope::parse_request(golden(name))
}

fn parse_response(name: &str) -> Result<CoopNativeEnvelope, CoopNativeEnvelopeError> {
    CoopNativeEnvelope::parse_response(golden(name))
}

#[test]
fn accepted_component_artifact_is_exact_and_admitted_for_consumption() -> Result<(), Box<dyn Error>> {
    let state = verify_coop_native_candidate_artifact()?;
    assert_eq!(state.status(), CoopNativeArtifactStatus::AcceptedComponent);
    assert_eq!(state.admission(), CoopNativeAdmissionStatus::Component);
    assert!(state.is_admitted());
    assert!(state.producer_digest_matches_candidate());
    assert_eq!(verify_coop_native_artifact(), Ok(()));
    Ok(())
}

#[test]
fn all_seventeen_goldens_parse_in_their_declared_direction() -> Result<(), Box<dyn Error>> {
    for name in [
        "observation-response",
        "legal-catalog-request",
        "legal-catalog-response",
        "local-action-settled-request",
        "local-action-settled-response",
        "local-action-rejected-request",
        "local-action-rejected-response",
        "local-action-unknown-request",
        "local-action-unknown-response",
        "local-action-recovered-request",
        "local-action-recovered-response",
        "shared-vote-settled-request",
        "shared-vote-settled-response",
        "rejoin-pending-request",
        "rejoin-pending-response",
        "rejoin-recovered-request",
        "rejoin-recovered-response",
    ] {
        let bytes = golden(name);
        let value: Value = serde_json::from_slice(bytes)?;
        let is_request = match value.get("kind").and_then(Value::as_str) {
            Some(
                "legal_catalog_request"
                | "local_action_request"
                | "shared_vote_request"
                | "rejoin_request",
            ) => true,
            Some("recovery_response") => value.get("status").is_some_and(Value::is_null),
            _ => false,
        };
        let parsed = if is_request {
            CoopNativeEnvelope::parse_request(bytes)
        } else {
            CoopNativeEnvelope::parse_response(bytes)
        };
        assert!(parsed.is_ok(), "{name}: {parsed:?}");
    }
    Ok(())
}

#[test]
fn parser_rejects_stale_digest_and_duplicate_members() -> Result<(), Box<dyn Error>> {
    let stale = String::from_utf8(golden("observation-response").to_vec())?.replace(
        COOP_NATIVE_SCHEMA_DIGEST,
        "0000000000000000000000000000000000000000000000000000000000000000",
    );
    assert!(CoopNativeEnvelope::parse_response(stale.as_bytes()).is_err());

    let fixture = String::from_utf8(golden("observation-response").to_vec())?;
    let duplicate = fixture.replacen(
        "\"protocol_version\":\"coop-native-v1\"",
        "\"protocol_version\":\"coop-native-v1\",\"protocol_version\":\"coop-native-v1\"",
        1,
    );
    assert!(matches!(
        CoopNativeEnvelope::parse_response(duplicate.as_bytes()),
        Err(CoopNativeEnvelopeError::DuplicateMember)
    ));
    Ok(())
}

#[test]
fn parser_rejects_duplicate_peer_tokens_even_when_snapshots_differ() -> Result<(), Box<dyn Error>> {
    let mut value: Value = serde_json::from_slice(golden("observation-response"))?;
    let local = value["observation"]["peers"][0]["peer_token"].clone();
    value["observation"]["peers"][1]["peer_token"] = local;
    value["observation"]["peers"][1]["role"] = json!("ally");
    assert!(matches!(
        CoopNativeEnvelope::parse_response(&serde_json::to_vec(&value)?),
        Err(CoopNativeEnvelopeError::InvalidValue)
    ));
    Ok(())
}

#[test]
fn parser_rejects_non_settled_and_recovery_generation_drift() -> Result<(), Box<dyn Error>> {
    let mut unknown: Value = serde_json::from_slice(golden("local-action-unknown-response"))?;
    unknown["receipt"]["after_host_generation"] = json!(2);
    assert!(CoopNativeEnvelope::parse_response(&serde_json::to_vec(&unknown)?).is_err());

    let mut rejected: Value = serde_json::from_slice(golden("local-action-rejected-response"))?;
    rejected["receipt"]["before_host_generation"] = json!(0);
    assert!(CoopNativeEnvelope::parse_response(&serde_json::to_vec(&rejected)?).is_err());

    let mut recovered: Value = serde_json::from_slice(golden("local-action-recovered-response"))?;
    recovered["receipt"]["after_host_generation"] = json!(1);
    assert!(CoopNativeEnvelope::parse_response(&serde_json::to_vec(&recovered)?).is_err());

    let mut pending_rejoin: Value = serde_json::from_slice(golden("rejoin-pending-response"))?;
    pending_rejoin["receipt"]["after_host_generation"] = json!(2);
    assert!(CoopNativeEnvelope::parse_response(&serde_json::to_vec(&pending_rejoin)?).is_err());
    Ok(())
}

#[derive(Default)]
struct CountingPort {
    calls: usize,
}

impl CoopNativePort for CountingPort {
    fn consume(&mut self, _envelope: &CoopNativeEnvelope) -> Result<(), CoopNativePortError> {
        self.calls += 1;
        Ok(())
    }
}

#[test]
fn unknown_mutation_is_reconciled_without_retry() -> Result<(), Box<dyn Error>> {
    let request = parse_request("local-action-unknown-request")?;
    let unknown = parse_response("local-action-unknown-response")?;
    let reconcile_request = parse_request("local-action-recovered-request")?;
    let reconcile_response = parse_response("local-action-recovered-response")?;
    let operation = request.operation_id().cloned().ok_or("operation missing")?;
    let mut coordinator = CoopNativeCoordinator::new(CountingPort::default(), lineage()?)?;

    assert_eq!(
        coordinator.consume(request)?.state(),
        Some(CoopNativeOperationState::Requested)
    );
    assert_eq!(
        coordinator.consume(unknown)?.state(),
        Some(CoopNativeOperationState::Unknown)
    );
    assert_eq!(
        coordinator.retry_unknown(&operation),
        Err(CoopNativeCoordinatorError::NoBlindRetry)
    );
    assert_eq!(
        coordinator.reconcile(reconcile_request)?.event(),
        CoopNativeEventKind::ReconcileRequested
    );
    assert_eq!(
        coordinator.reconcile(reconcile_response)?.event(),
        CoopNativeEventKind::Reconciled
    );
    assert_eq!(
        coordinator.operation_state(&operation),
        Some(CoopNativeOperationState::Reconciled)
    );
    assert_eq!(coordinator.port().calls, 4);
    assert_eq!(coordinator.records().len(), 4);
    Ok(())
}

#[test]
fn response_receipt_must_match_the_request_generation_fence() -> Result<(), Box<dyn Error>> {
    let request = parse_request("local-action-unknown-request")?;
    let operation = request.operation_id().cloned().ok_or("operation missing")?;
    let mut value: Value = serde_json::from_slice(golden("local-action-unknown-response"))?;
    value["observation"]["host_generation"] = json!(2);
    value["receipt"]["before_host_generation"] = json!(2);
    let response = CoopNativeEnvelope::parse_response(&serde_json::to_vec(&value)?)?;
    let mut coordinator = CoopNativeCoordinator::new(CountingPort::default(), lineage()?)?;

    coordinator.consume(request)?;
    assert_eq!(
        coordinator.consume(response),
        Err(CoopNativeCoordinatorError::OperationConflict)
    );
    assert_eq!(coordinator.operation_state(&operation), Some(CoopNativeOperationState::Requested));
    assert_eq!(coordinator.port().calls, 1);
    assert_eq!(coordinator.records().len(), 1);
    Ok(())
}

#[test]
fn catalog_and_receipt_relations_are_checked() -> Result<(), Box<dyn Error>> {
    let catalog_request = parse_request("legal-catalog-request")?;
    let catalog_response = parse_response("legal-catalog-response")?;
    assert_eq!(catalog_request.kind(), CoopNativeKind::LegalCatalogRequest);
    assert_eq!(catalog_response.kind(), CoopNativeKind::LegalCatalogResponse);
    let response = catalog_response
        .legal_catalog_response()
        .ok_or("catalog response missing")?;
    assert_eq!(response.catalog().actions().len(), 2);
    assert_eq!(response.catalog().votes().len(), 1);
    assert_eq!(response.catalog().host_generation(), 1);

    let mut value: Value = serde_json::from_slice(golden("local-action-settled-response"))?;
    value["receipt"]["operation_id"] = json!("op:native:other");
    let bytes = serde_json::to_vec(&value)?;
    assert!(matches!(
        CoopNativeEnvelope::parse_response(&bytes),
        Err(CoopNativeEnvelopeError::InvalidValue)
    ));
    Ok(())
}
