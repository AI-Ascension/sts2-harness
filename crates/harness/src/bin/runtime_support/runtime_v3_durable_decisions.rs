// SPDX-License-Identifier: MIT

use sts2_harness::{
    Decision, DecisionInput, DecisionReference, ProviderFailureClass, ProviderReservation,
};

use super::super::super::DecisionAdmission;
use super::super::super::decision_replay;
use super::super::support::decision_input_digest;
use super::super::{DurableHandle, PROVIDER_RESERVATION_UNITS, ProviderReservationToken};

impl DurableHandle {
    pub(in super::super::super) fn decision_admission_with_reuse(
        &self,
        input: &DecisionInput,
    ) -> Result<DecisionAdmission, String> {
        super::super::try_lock(&self.store)?
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
        if let Some(stored) = super::super::try_lock(&self.store)?
            .reuse_completed_decision(
                &self.lineage,
                &reference.execution_id,
                &reference.input_fingerprint,
                &reference.model_revision,
                &reference.config_digest,
            )
            .map_err(|error| format!("cannot inspect reusable runtime-v3 decision: {error}"))?
        {
            return Ok(DecisionAdmission::Reused(replay_stored_decision(&stored)?));
        }
        let stored = super::super::try_lock(&self.store)?
            .record_decision(&reference)
            .map_err(|error| format!("cannot persist runtime-v3 decision reference: {error}"))?;
        if stored.completed {
            return Ok(DecisionAdmission::Reused(replay_stored_decision(&stored)?));
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
        super::super::try_lock(&self.store)?
            .reserve_provider(&reservation)
            .map_err(|error| format!("cannot reserve runtime-v3 provider usage: {error}"))?;
        Ok(DecisionAdmission::Fresh(ProviderReservationToken {
            reservation_id,
        }))
    }

    pub(in super::super::super) fn complete_decision(
        &self,
        token: &ProviderReservationToken,
        decision: &Decision,
    ) -> Result<(), String> {
        let (result_payload, result_digest) = decision_replay::encode(decision)?;
        let result_ref = format!("decision-result-{}", token.reservation_id);
        super::super::try_lock(&self.store)?
            .complete_provider_with_result(
                &token.reservation_id,
                &result_ref,
                &result_digest,
                &result_payload,
                PROVIDER_RESERVATION_UNITS,
            )
            .map(|_| ())
            .map_err(|error| format!("cannot persist runtime-v3 provider completion: {error}"))
    }

    pub(in super::super::super) fn fail_decision(
        &self,
        token: &ProviderReservationToken,
        failure: ProviderFailureClass,
    ) -> Result<(), String> {
        super::super::try_lock(&self.store)?
            .fail_provider(&token.reservation_id, failure, None)
            .map(|_| ())
            .map_err(|error| format!("cannot persist runtime-v3 provider failure: {error}"))
    }

    pub(in super::super::super) fn unknown_decision(
        &self,
        token: &ProviderReservationToken,
        failure: ProviderFailureClass,
    ) -> Result<(), String> {
        super::super::try_lock(&self.store)?
            .mark_provider_unknown(&token.reservation_id, failure, None)
            .map(|_| ())
            .map_err(|error| format!("cannot persist unknown runtime-v3 provider result: {error}"))
    }
}

fn replay_stored_decision(stored: &sts2_harness::StoredDecision) -> Result<Decision, String> {
    let payload = stored.result_payload.as_deref().ok_or_else(|| {
        String::from(
            "runtime-v3 completed provider result has no replayable payload; refusing provider call",
        )
    })?;
    let digest =
        stored.reference.result_digest.as_deref().ok_or_else(|| {
            String::from("runtime-v3 completed provider result has no replay digest")
        })?;
    decision_replay::decode(payload, digest)
}
