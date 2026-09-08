// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::Digest;
use sts2_harness::{EpisodeLegalAction, OperationIntent, OperationState};

use super::DurableHandle;
use super::support::{response_evidence, sha256_bytes, sha256_json};

#[path = "runtime_v3_durable_decisions.rs"]
mod decisions;

#[cfg(test)]
const TEST_ORIGINAL_CONTEXT_RAW: &[u8] = br#"{"deployment_id":"33333333-3333-4333-8333-333333333333","instance_id":"44444444-4444-4444-8444-444444444444","instance_incarnation":"55555555-5555-4555-8555-555555555555","boot_id":"66666666-6666-4666-8666-666666666666","authority_generation":1,"lease_id":"77777777-7777-4777-8777-777777777777","lease_epoch":1}"#;

pub(in super::super) struct OperationCatalogEvidence<'a> {
    pub(in super::super) input: &'a Value,
    pub(in super::super) raw: &'a [u8],
}

pub(in super::super) struct OperationIntentEvidence<'a> {
    pub(in super::super) operation_id: &'a str,
    pub(in super::super) state_id: &'a str,
    pub(in super::super) generation: u64,
    pub(in super::super) action: &'a EpisodeLegalAction,
    pub(in super::super) payload: &'a Value,
    pub(in super::super) catalog: OperationCatalogEvidence<'a>,
    pub(in super::super) original_context_raw: Option<&'a [u8]>,
}

impl DurableHandle {
    #[cfg(test)]
    pub(in super::super) fn operation_intent(
        &self,
        operation_id: &str,
        state_id: &str,
        generation: u64,
        action: &EpisodeLegalAction,
        payload: &Value,
        input: &Value,
    ) -> Result<String, String> {
        let catalog_raw = input
            .get("legal_actions")
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|error| format!("cannot encode runtime-v3 operation catalog: {error}"))?;
        let Some(catalog_raw) = catalog_raw else {
            return Err(String::from(
                "runtime-v3 operation intent omitted legal-action catalog",
            ));
        };
        self.persist_operation_intent(OperationIntentEvidence {
            operation_id,
            state_id,
            generation,
            action,
            payload,
            catalog: OperationCatalogEvidence {
                input,
                raw: &catalog_raw,
            },
            original_context_raw: Some(TEST_ORIGINAL_CONTEXT_RAW),
        })
    }

    pub(in super::super) fn persist_operation_intent(
        &self,
        evidence: OperationIntentEvidence<'_>,
    ) -> Result<String, String> {
        let action_payload = super::super::super::runtime_v3_wire::canonical_action_bytes(
            evidence.action.action_id(),
            evidence.payload,
        )?;
        let payload_digest = format!("{:x}", sha2::Sha256::digest(&action_payload));
        let input_digest = sha256_json(evidence.catalog.input)?;
        let catalog_digest = Some(sha256_bytes(evidence.catalog.raw));
        let intent = OperationIntent::new_with_action_and_catalog(
            self.lineage.clone(),
            evidence.operation_id,
            evidence.state_id,
            evidence.generation,
            evidence.action.action_id(),
            super::super::super::runtime_v3_wire::action_kind_name(evidence.action.kind()),
            action_payload,
            payload_digest.clone(),
            input_digest,
            catalog_digest,
            Some(evidence.catalog.raw.to_vec()),
        )
        .and_then(|intent| {
            intent.with_original_context(evidence.original_context_raw.map(<[u8]>::to_vec))
        })
        .map_err(|error| format!("runtime-v3 operation intent is invalid: {error}"))?;
        super::try_lock(&self.store)?
            .record_operation_intent(&intent)
            .map_err(|error| format!("cannot persist runtime-v3 operation intent: {error}"))?;
        Ok(payload_digest)
    }

    pub(in super::super) fn operation_dispatched(
        &self,
        operation_id: &str,
        payload_digest: &str,
    ) -> Result<(), String> {
        super::try_lock(&self.store)?
            .mark_operation_dispatched(operation_id, payload_digest)
            .map(|_| ())
            .map_err(|error| format!("cannot persist runtime-v3 dispatch intent: {error}"))
    }

    pub(in super::super) fn operation_result(
        &self,
        operation_id: &str,
        payload_digest: &str,
        status: OperationState,
        response: Option<&Value>,
    ) -> Result<(), String> {
        let evidence = response
            .map(|value| response_evidence(operation_id, value))
            .transpose()?;
        super::try_lock(&self.store)?
            .record_operation_result(
                operation_id,
                payload_digest,
                status,
                evidence.as_ref().map(|(reference, _)| reference.as_str()),
                evidence.as_ref().map(|(_, digest)| digest.as_str()),
            )
            .map(|_| ())
            .map_err(|error| format!("cannot persist runtime-v3 operation result: {error}"))
    }

    pub(in super::super) fn operation_state(
        &self,
        operation_id: &str,
    ) -> Result<OperationState, String> {
        super::try_lock(&self.store)?
            .operation(operation_id)
            .map(|operation| operation.state)
            .map_err(|error| format!("cannot read runtime-v3 operation state: {error}"))
    }

    pub(in super::super) fn operation_payload_digest(
        &self,
        operation_id: &str,
    ) -> Result<String, String> {
        super::try_lock(&self.store)?
            .operation(operation_id)
            .map(|operation| operation.intent.payload_digest)
            .map_err(|error| format!("cannot read runtime-v3 operation digest: {error}"))
    }

    /// Applies a result from the authoritative recovery endpoint.  A recovery result resolves
    /// the original operation in one durable transition; it never creates a replacement ID.
    pub(in super::super) fn reconcile_response(
        &self,
        operation_id: &str,
        resolved_state: OperationState,
        response: &Value,
    ) -> Result<(), String> {
        let operation = super::try_lock(&self.store)?
            .operation(operation_id)
            .map_err(|error| {
                format!("cannot read runtime-v3 operation for reconciliation: {error}")
            })?;
        let (result_ref, result_digest) = response_evidence(operation_id, response)?;
        super::try_lock(&self.store)?
            .reconcile_operation(
                operation_id,
                &operation.intent.payload_digest,
                resolved_state,
                &result_ref,
                &result_digest,
            )
            .map(|_| ())
            .map_err(|error| format!("cannot reconcile runtime-v3 operation: {error}"))
    }
}
