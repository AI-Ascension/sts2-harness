// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_harness::{
    Decision, DecisionInput, DecisionReference, EpisodeLegalAction, OperationIntent,
    OperationState, ProviderFailureClass, ProviderReservation,
};

use super::support::{decision_digest, decision_input_digest, response_evidence, sha256_json};
use super::{DurableHandle, PROVIDER_RESERVATION_UNITS, ProviderReservationToken};

impl DurableHandle {
    pub(in super::super) fn operation_intent(
        &self,
        operation_id: &str,
        state_id: &str,
        generation: u64,
        action: &EpisodeLegalAction,
        payload: &Value,
        input: &Value,
    ) -> Result<String, String> {
        let payload_digest = sha256_json(payload)?;
        let input_digest = sha256_json(input)?;
        let intent = OperationIntent::new(
            self.lineage.clone(),
            operation_id,
            state_id,
            generation,
            action.action_id(),
            payload_digest.clone(),
            input_digest,
        )
        .map_err(|error| format!("runtime-v3 operation intent is invalid: {error}"))?;
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .record_operation_intent(&intent)
            .map_err(|error| format!("cannot persist runtime-v3 operation intent: {error}"))?;
        Ok(payload_digest)
    }

    pub(in super::super) fn operation_dispatched(
        &self,
        operation_id: &str,
        payload_digest: &str,
    ) -> Result<(), String> {
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
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
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
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
        self.store
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .operation(operation_id)
            .map(|operation| operation.state)
            .map_err(|error| format!("cannot read runtime-v3 operation state: {error}"))
    }

    pub(in super::super) fn operation_payload_digest(
        &self,
        operation_id: &str,
    ) -> Result<String, String> {
        self.store
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
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
        let operation = self
            .store
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .operation(operation_id)
            .map_err(|error| {
                format!("cannot read runtime-v3 operation for reconciliation: {error}")
            })?;
        let (result_ref, result_digest) = response_evidence(operation_id, response)?;
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
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

    pub(in super::super) fn decision_admission(
        &self,
        input: &DecisionInput,
    ) -> Result<Option<ProviderReservationToken>, String> {
        self.store
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .resume_for_decision(&self.lineage.episode_id, &self.fingerprint)
            .map_err(|error| format!("runtime-v3 decision admission is blocked: {error}"))?;
        let input_fingerprint = decision_input_digest(input)?;
        let execution_id = input.execution_id.to_string();
        let reference = DecisionReference::new(
            self.lineage.clone(),
            execution_id.clone(),
            input_fingerprint,
            self.model_revision.clone(),
            self.config_digest.clone(),
        )
        .map_err(|error| format!("runtime-v3 decision reference is invalid: {error}"))?;
        let stored = self
            .store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .record_decision(&reference)
            .map_err(|error| format!("cannot persist runtime-v3 decision reference: {error}"))?;
        if stored.completed {
            return Ok(None);
        }
        if stored.unknown {
            return Err(String::from(
                "runtime-v3 provider decision is unknown and cannot be reused",
            ));
        }
        let reservation_id = format!("provider-reservation-{execution_id}");
        let provider_execution_id = format!("provider-execution-{execution_id}");
        let reservation = ProviderReservation::new(
            self.lineage.clone(),
            reservation_id.clone(),
            execution_id,
            provider_execution_id,
            PROVIDER_RESERVATION_UNITS,
        )
        .map_err(|error| format!("runtime-v3 provider reservation is invalid: {error}"))?;
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .reserve_provider(&reservation)
            .map_err(|error| format!("cannot reserve runtime-v3 provider usage: {error}"))?;
        Ok(Some(ProviderReservationToken { reservation_id }))
    }

    pub(in super::super) fn complete_decision(
        &self,
        token: &ProviderReservationToken,
        decision: &Decision,
    ) -> Result<(), String> {
        let result_digest = decision_digest(decision);
        let result_ref = format!("decision-result-{}", token.reservation_id);
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .complete_provider(
                &token.reservation_id,
                &result_ref,
                &result_digest,
                PROVIDER_RESERVATION_UNITS,
            )
            .map(|_| ())
            .map_err(|error| format!("cannot persist runtime-v3 provider completion: {error}"))
    }

    pub(in super::super) fn fail_decision(
        &self,
        token: &ProviderReservationToken,
        failure: ProviderFailureClass,
    ) -> Result<(), String> {
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .fail_provider(&token.reservation_id, failure, None)
            .map(|_| ())
            .map_err(|error| format!("cannot persist runtime-v3 provider failure: {error}"))
    }

    pub(in super::super) fn unknown_decision(
        &self,
        token: &ProviderReservationToken,
        failure: ProviderFailureClass,
    ) -> Result<(), String> {
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .mark_provider_unknown(&token.reservation_id, failure, None)
            .map(|_| ())
            .map_err(|error| format!("cannot persist unknown runtime-v3 provider result: {error}"))
    }
}
