// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_harness::{
    Decision, DecisionInput, DecisionSource, DispatchStatus, EpisodeObservation, EpisodeRunReport,
    EpisodeRunnerError, PolicyError, TransitionReceipt, WaitOutcome, WaitSample,
};

use super::super::runtime_v3_telemetry::{
    DecisionKind, FailureCode, GameOutcome, ObservationSource, TelemetryHandle,
};
use super::decision_admission::DecisionAdmission;
use super::durable::{DurableHandle, ProviderReservationToken};

include!("runtime_v3_recording_stream.rs");

pub(super) struct DecisionRecorder<'a, S> {
    source: &'a mut S,
    telemetry: TelemetryHandle,
    durable: Option<DurableHandle>,
}

impl<'a, S> DecisionRecorder<'a, S> {
    #[cfg(test)]
    pub(super) fn new(source: &'a mut S, telemetry: TelemetryHandle) -> Self {
        Self {
            source,
            telemetry,
            durable: None,
        }
    }

    pub(super) fn with_durable(
        source: &'a mut S,
        telemetry: TelemetryHandle,
        durable: DurableHandle,
    ) -> Self {
        Self {
            source,
            telemetry,
            durable: Some(durable),
        }
    }
}

impl<S: DecisionSource> DecisionSource for DecisionRecorder<'_, S> {
    fn model_execution_id(&self) -> Option<sts2_harness::ModelExecutionId> {
        self.source.model_execution_id()
    }

    fn action_completed(&mut self, settled: bool) {
        self.source.action_completed(settled);
    }

    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
        let mut reservation: Option<ProviderReservationToken> = None;
        let decision = if let Some(durable) = &self.durable {
            let admission = match durable.decision_admission_with_reuse(input) {
                Ok(admission) => admission,
                Err(_) => return Err(self.durable_decision_failure()),
            };
            match admission {
                DecisionAdmission::Reused(decision) => decision,
                DecisionAdmission::Fresh(token) => {
                    reservation = Some(token);
                    match self.source.decide(input) {
                        Ok(decision) => decision,
                        Err(error) => {
                            if let Some(token) = reservation.as_ref() {
                                let failure = provider_failure(&error);
                                let result = if matches!(
                                    error,
                                    PolicyError::ProviderUnavailable | PolicyError::ProviderClosed
                                ) {
                                    durable.unknown_decision(token, failure)
                                } else {
                                    durable.fail_decision(token, failure)
                                };
                                if result.is_err() {
                                    return Err(self.durable_decision_failure());
                                }
                            }
                            let execution_id = self
                                .source
                                .model_execution_id()
                                .unwrap_or(input.execution_id);
                            let _ = self
                                .telemetry
                                .model_failure(execution_id.get(), FailureCode::from(&error));
                            return Err(error);
                        }
                    }
                }
            }
        } else {
            match self.source.decide(input) {
                Ok(decision) => decision,
                Err(error) => {
                    let execution_id = self
                        .source
                        .model_execution_id()
                        .unwrap_or(input.execution_id);
                    let _ = self
                        .telemetry
                        .model_failure(execution_id.get(), FailureCode::from(&error));
                    return Err(error);
                }
            }
        };
        if let Some(token) = reservation
            && let Some(durable) = &self.durable
            && durable.complete_decision(&token, &decision).is_err()
        {
            return Err(self.durable_decision_failure());
        }
        let execution_id = self
            .source
            .model_execution_id()
            .unwrap_or(input.execution_id);
        // ExoDecisionSource expands a supported provider Plan into one Action at a time while
        // retaining its originating execution ID. Recording the post-expansion choice here
        // gives replay one correspondence row per dispatch, including reused plan executions.
        if let Decision::Action { action_id, .. } = &decision {
            emit_replay_event(json!({
                "event": "model_decision",
                "model_execution_id": execution_id.get(),
                "reused_model_execution": execution_id != input.execution_id,
                "action_id": action_id,
                "observation": input.observation.fair_play().as_value()
            }));
        }
        let (kind, action_id, operation_id, confidence) = match &decision {
            Decision::Plan { action_ids, .. } => (
                DecisionKind::Plan,
                action_ids.first().map(String::as_str),
                None,
                None,
            ),
            Decision::Action {
                action_id,
                confidence,
                ..
            } => (
                DecisionKind::Action,
                Some(action_id.as_str()),
                None,
                *confidence,
            ),
            Decision::Wait { .. } => (DecisionKind::Wait, None, None, None),
            Decision::Reobserve { .. } => (DecisionKind::Reobserve, None, None, None),
            Decision::Recovery { operation_id, .. } => {
                (DecisionKind::Recovery, None, operation_id.as_deref(), None)
            }
        };
        let _ = self.telemetry.model_decision(
            execution_id.get(),
            kind,
            action_id,
            operation_id,
            confidence,
        );
        Ok(decision)
    }
}

impl<S> DecisionRecorder<'_, S> {
    /// Durable admission and persistence failures must stop the episode without reclassifying a
    /// local store boundary as malformed provider output. `InputBlocked` is the existing public
    /// fail-closed policy outcome for a decision that cannot safely proceed.
    fn durable_decision_failure(&self) -> PolicyError {
        let _ = self
            .telemetry
            .failure("durable_decision", FailureCode::Other, false, None);
        PolicyError::InputBlocked
    }
}

fn provider_failure(error: &PolicyError) -> sts2_harness::ProviderFailureClass {
    match error {
        PolicyError::ProviderUnavailable => sts2_harness::ProviderFailureClass::Outage,
        PolicyError::ProviderClosed => sts2_harness::ProviderFailureClass::Cancelled,
        PolicyError::ProviderMalformed | PolicyError::MalformedDecision => {
            sts2_harness::ProviderFailureClass::IncompatibleOutput
        }
        _ => sts2_harness::ProviderFailureClass::IncompatibleOutput,
    }
}

include!("runtime_v3_recording_events.rs");

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    include!("runtime_v3_recording_tests.rs");
}
