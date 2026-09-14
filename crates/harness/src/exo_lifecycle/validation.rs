// SPDX-License-Identifier: MIT

use super::*;
use crate::provider_session::{BindingState, NativeOperationKind, NativeOperationState};

pub(super) fn input(
    manifest: &InvocationManifest,
    bytes: &[u8],
) -> Result<crate::ExoDecisionRequest, LifecycleError> {
    manifest.validate()?;
    if bytes.len() != manifest.input_length || crate::sha256_hex(bytes) != manifest.input_digest {
        return Err(LifecycleError::Invalid);
    }
    let envelope = crate::parse_bridge_request_envelope(bytes, MAX_INPUT_BYTES)
        .map_err(|_| LifecycleError::Invalid)?;
    let request = envelope.request;
    if envelope.request_id != manifest.request_id
        || envelope.turn_id != manifest.host_turn_id
        || request.model_execution_id != manifest.execution_id
        || request.provider_revision != manifest.model_revision
        || request.state_id != manifest.authority.state_id
        || request.generation != manifest.authority.generation
        || crate::sha256_hex(
            serde_json::to_vec(&request.legal_action_ids).map_err(|_| LifecycleError::Invalid)?,
        ) != manifest.authority.catalog_digest
    {
        return Err(LifecycleError::Invalid);
    }
    Ok(request)
}

pub(super) fn response(
    manifest: &InvocationManifest,
    input_bytes: &[u8],
    bytes: &[u8],
) -> Result<crate::BoundDecision, LifecycleError> {
    let request = input(manifest, input_bytes)?;
    crate::parse_bridge_decision_envelope(bytes, &manifest.request_id, &manifest.host_turn_id)
        .map_err(|_| LifecycleError::Invalid)?
        .bind(&request.legal_action_ids)
        .map_err(|_| LifecycleError::Invalid)
}

impl LifecycleOwner {
    pub(super) fn validate_broker(
        &self,
        manifest: &InvocationManifest,
        expected: NativeOperationState,
    ) -> Result<(), LifecycleError> {
        let a = &manifest.authority;
        let binding = self
            .broker
            .binding(&manifest.binding_id)
            .map_err(|_| LifecycleError::Held)?;
        let operation = self
            .broker
            .operation(&manifest.operation_id)
            .map_err(|_| LifecycleError::Held)?;
        let request_digest = crate::sha256_hex(
            serde_json::to_vec(&serde_json::json!({
                "prepared_id": manifest.prepared_id, "suffix_sha256": manifest.input_digest,
            }))
            .map_err(|_| LifecycleError::Invalid)?,
        );
        if manifest.scope != *self.broker.scope()
            || a.owner_epoch != self.broker.owner_epoch()
            || a.auth_epoch != a.owner_epoch
            || a.revocation_epoch != self.broker.revocation_epoch()
            || binding.state != BindingState::Active
            || !binding.game_dispatch_capability
            || binding.owner_epoch != a.owner_epoch
            || binding.session_epoch != a.session_epoch
            || binding.history_epoch != a.history_epoch
            || binding.compaction_epoch != a.compaction_epoch
            || binding.profile_sha256 != manifest.profile_digest
            || operation.kind != NativeOperationKind::Turn
            || operation.request_sha256 != request_digest
            || operation.state != expected
            || operation.binding_id != manifest.binding_id
            || operation.scope != manifest.scope
            || operation.owner_epoch != a.owner_epoch
            || operation.session_epoch != a.session_epoch
            || !operation.generation_permission
            || !operation.generation_class
            || operation.automatic_retry
            || operation.auto_resume
            || operation.game_effects != 0
        {
            return Err(LifecycleError::Held);
        }
        Ok(())
    }
}
