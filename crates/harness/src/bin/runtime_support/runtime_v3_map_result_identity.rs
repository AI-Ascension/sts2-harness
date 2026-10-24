// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_harness::{DecisionInput, EpisodeLegalActionSet, RecipeInvocationBinding};
use sts2_protocol::{RuntimeMapV1Message, RuntimeMapV1MessageKind};

use super::super::support::sha256_bytes;
use super::map_collection_error::MapCollectionError;
use super::{MAP_PROFILE, RPC_CORRELATION_ID};

pub(super) fn typed_result_digest(
    value: &Value,
    binding: &RecipeInvocationBinding,
    actions: &EpisodeLegalActionSet,
) -> Result<String, MapCollectionError> {
    let message: RuntimeMapV1Message = serde_json::from_value(value.clone()).map_err(|_| {
        MapCollectionError::failed("validated runtime map response could not be typed")
    })?;
    message
        .validate_response()
        .map_err(|_| MapCollectionError::failed("validated runtime map response is malformed"))?;
    let context = binding.context();
    if message.protocol_version.as_str() != MAP_PROFILE
        || message.schema_digest.as_str() != sts2_harness::RUNTIME_MAP_SCHEMA_DIGEST
        || message.kind != RuntimeMapV1MessageKind::SnapshotResponse
        || message.correlation_id.as_str() != RPC_CORRELATION_ID
        || message.instance_id.as_str() != context.instance_id.as_str()
        || message.session_id.as_str() != context.gateway_session_id.as_str()
        || message.lease_id.as_str() != context.lease_id.as_str()
        || message.lease_epoch != context.lease_epoch
        || message.generation != context.generation
        || message.snapshot.is_none()
    {
        return Err(MapCollectionError::failed(
            "runtime map response identity does not match its invocation",
        ));
    }
    let snapshot_digest = DecisionInput::canonical_runtime_map_snapshot_digest(
        value,
        &context.state_id,
        context.generation,
        actions,
    )
    .ok_or_else(|| {
        MapCollectionError::owner_snapshot_invalid("runtime map snapshot failed owner validation")
    })?;
    Ok(map_result_digest(&snapshot_digest))
}

pub(super) fn map_result_digest(snapshot_digest: &str) -> String {
    let mut material = Vec::with_capacity(160);
    material.extend_from_slice(b"sts2-harness:recipe-map-result:v2\0");
    for field in [
        MAP_PROFILE,
        sts2_harness::RUNTIME_MAP_SCHEMA_DIGEST,
        snapshot_digest,
    ] {
        material.extend_from_slice(field.as_bytes());
        material.push(0);
    }
    sha256_bytes(&material)
}
