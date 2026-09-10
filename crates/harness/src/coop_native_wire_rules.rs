// SPDX-License-Identifier: MIT

use super::identity::{NativeId, validate_generation};

const OBSERVATION_FIELDS: [&str; 14] = [
    "host_authority_epoch",
    "authority_id",
    "run_id",
    "host_sequence_kind",
    "host_generation",
    "state_digest",
    "checkpoint_id",
    "checksum_algorithm",
    "checksum_status",
    "native_checksum",
    "host_digest_known",
    "host_loading",
    "host_divergent",
    "peers",
];
const PEER_FIELDS: [&str; 13] = [
    "peer_token",
    "authority_id",
    "role",
    "connected",
    "peer_generation",
    "state_digest",
    "rejoin_epoch",
    "authority_epoch",
    "checkpoint_id",
    "digest_known",
    "is_loading",
    "is_divergent",
    "checksum_status",
];
const ACTION_FIELDS: [&str; 3] = ["kind", "action_id", "target_peer"];
const VOTE_FIELDS: [&str; 3] = ["proposal_id", "voter_peer", "choice"];
const RECOVERY_FIELDS: [&str; 2] = ["kind", "rejoin_epoch"];
const CATALOG_FIELDS: [&str; 4] = ["host_generation", "actor_peer", "actions", "votes"];
const RECEIPT_FIELDS: [&str; 10] = [
    "operation_id",
    "status",
    "before_host_generation",
    "after_host_generation",
    "authority_id",
    "authority_epoch",
    "checkpoint_id",
    "state_digest",
    "native_checksum",
    "error_code",
];

fn parse_envelope(value: Value) -> Result<CoopNativeEnvelope, CoopNativeEnvelopeError> {
    let root = value
        .as_object()
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    exact_fields(root, &ROOT_FIELDS)?;
    let header = parse_header(root)?;
    let kind = text(root, "kind")
        .ok()
        .and_then(CoopNativeKind::parse)
        .ok_or(CoopNativeEnvelopeError::UnsupportedKind)?;
    let body = match kind {
        CoopNativeKind::Observation => parse_observation_envelope(root)?,
        CoopNativeKind::LegalCatalogRequest => parse_catalog_request(root)?,
        CoopNativeKind::LegalCatalogResponse => parse_catalog_response(root)?,
        CoopNativeKind::LocalActionRequest => parse_local_action(root)?,
        CoopNativeKind::SharedVoteRequest => parse_shared_vote(root)?,
        CoopNativeKind::RejoinRequest => parse_rejoin(root)?,
        CoopNativeKind::EffectResponse => parse_effect_response(root)?,
        CoopNativeKind::RecoveryResponse => parse_recovery_response(root)?,
    };
    Ok(CoopNativeEnvelope {
        header,
        kind,
        body,
        value,
    })
}

fn parse_header(root: &Map<String, Value>) -> Result<CoopNativeHeader, CoopNativeEnvelopeError> {
    if text(root, "protocol_version")? != COOP_NATIVE_PROTOCOL_VERSION
        || text(root, "schema_digest")? != COOP_NATIVE_SCHEMA_DIGEST
    {
        return Err(CoopNativeEnvelopeError::SchemaDigestMismatch);
    }
    let provenance = root
        .get("provenance")
        .and_then(Value::as_object)
        .ok_or(CoopNativeEnvelopeError::InvalidShape)?;
    exact_fields(provenance, &["artifact", "source", "generator"])?;
    if text(provenance, "artifact")? != COOP_NATIVE_ARTIFACT
        || text(provenance, "source")? != COOP_NATIVE_SCHEMA_SOURCE
        || text(provenance, "generator")? != COOP_NATIVE_GENERATOR
    {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    Ok(CoopNativeHeader {
        correlation_id: required_id(root, "correlation_id")?,
        instance_id: required_id(root, "instance_id")?,
        session_id: required_id(root, "session_id")?,
        lease_id: required_id(root, "lease_id")?,
        lease_epoch: generation(root, "lease_epoch")?,
    })
}

fn parse_observation_envelope(
    root: &Map<String, Value>,
) -> Result<CoopNativeBody, CoopNativeEnvelopeError> {
    for field in [
        "operation_id",
        "actor_peer",
        "expected_host_generation",
        "action",
        "vote",
        "status",
        "effect",
        "recovery",
        "catalog",
        "receipt",
    ] {
        require_null(root, field)?;
    }
    Ok(CoopNativeBody::Observation(parse_observation(
        root.get("observation")
            .ok_or(CoopNativeEnvelopeError::InvalidShape)?,
    )?))
}

fn parse_catalog_request(
    root: &Map<String, Value>,
) -> Result<CoopNativeBody, CoopNativeEnvelopeError> {
    require_null(root, "operation_id")?;
    for field in [
        "action",
        "vote",
        "status",
        "observation",
        "effect",
        "recovery",
        "catalog",
        "receipt",
    ] {
        require_null(root, field)?;
    }
    Ok(CoopNativeBody::LegalCatalogRequest(
        CoopNativeLegalCatalogRequest {
            actor_peer: required_peer(root, "actor_peer")?,
            expected_host_generation: generation(root, "expected_host_generation")?,
        },
    ))
}

fn parse_catalog_response(
    root: &Map<String, Value>,
) -> Result<CoopNativeBody, CoopNativeEnvelopeError> {
    require_null(root, "operation_id")?;
    for field in ["action", "vote", "status", "effect", "recovery", "receipt"] {
        require_null(root, field)?;
    }
    let actor_peer = required_peer(root, "actor_peer")?;
    let expected_host_generation = generation(root, "expected_host_generation")?;
    let observation = parse_observation(
        root.get("observation")
            .ok_or(CoopNativeEnvelopeError::InvalidShape)?,
    )?;
    let catalog = parse_catalog(
        root.get("catalog")
            .ok_or(CoopNativeEnvelopeError::InvalidShape)?,
    )?;
    if catalog.actor_peer != actor_peer
        || catalog.host_generation != expected_host_generation
        || observation.host_generation != expected_host_generation
    {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    Ok(CoopNativeBody::LegalCatalogResponse(
        CoopNativeLegalCatalogResponse {
            actor_peer,
            expected_host_generation,
            observation,
            catalog,
        },
    ))
}

fn parse_local_action(
    root: &Map<String, Value>,
) -> Result<CoopNativeBody, CoopNativeEnvelopeError> {
    require_null(root, "status")?;
    require_null(root, "observation")?;
    require_null(root, "effect")?;
    require_null(root, "vote")?;
    require_null(root, "recovery")?;
    require_null(root, "catalog")?;
    require_null(root, "receipt")?;
    Ok(CoopNativeBody::LocalActionRequest(CoopNativeLocalActionRequest {
        operation_id: required_id(root, "operation_id")?,
        actor_peer: required_peer(root, "actor_peer")?,
        expected_host_generation: generation(root, "expected_host_generation")?,
        action: parse_action(
            root.get("action")
                .ok_or(CoopNativeEnvelopeError::InvalidShape)?,
        )?,
    }))
}

fn parse_shared_vote(
    root: &Map<String, Value>,
) -> Result<CoopNativeBody, CoopNativeEnvelopeError> {
    require_null(root, "status")?;
    require_null(root, "observation")?;
    require_null(root, "effect")?;
    require_null(root, "action")?;
    require_null(root, "recovery")?;
    require_null(root, "catalog")?;
    require_null(root, "receipt")?;
    Ok(CoopNativeBody::SharedVoteRequest(CoopNativeSharedVoteRequest {
        operation_id: required_id(root, "operation_id")?,
        actor_peer: required_peer(root, "actor_peer")?,
        expected_host_generation: generation(root, "expected_host_generation")?,
        vote: parse_vote(
            root.get("vote")
                .ok_or(CoopNativeEnvelopeError::InvalidShape)?,
        )?,
    }))
}

fn parse_rejoin(root: &Map<String, Value>) -> Result<CoopNativeBody, CoopNativeEnvelopeError> {
    require_null(root, "status")?;
    require_null(root, "observation")?;
    require_null(root, "effect")?;
    require_null(root, "action")?;
    require_null(root, "vote")?;
    require_null(root, "catalog")?;
    require_null(root, "receipt")?;
    let recovery = parse_recovery(
        root.get("recovery")
            .ok_or(CoopNativeEnvelopeError::InvalidShape)?,
    )?;
    if recovery.kind != CoopNativeRecoveryKind::Rejoin {
        return Err(CoopNativeEnvelopeError::InvalidValue);
    }
    Ok(CoopNativeBody::RejoinRequest(CoopNativeRejoinRequest {
        operation_id: required_id(root, "operation_id")?,
        actor_peer: required_peer(root, "actor_peer")?,
        expected_host_generation: generation(root, "expected_host_generation")?,
        recovery,
    }))
}

include!("coop_native_wire_rules_responses.rs");
include!("coop_native_wire_rules_values.rs");
include!("coop_native_wire_rules_extra.rs");
