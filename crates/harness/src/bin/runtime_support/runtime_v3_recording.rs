// SPDX-License-Identifier: MIT

use sts2_harness::{
    Decision, DecisionInput, DecisionSource, DispatchStatus, EpisodeRunReport, PolicyError,
    TransitionReceipt, WaitOutcome, WaitSample,
};

use super::super::runtime_v3_telemetry::{
    DecisionKind, FailureCode, GameOutcome, ObservationSource, TelemetryHandle,
};

pub(super) struct DecisionRecorder<'a, S> {
    source: &'a mut S,
    telemetry: TelemetryHandle,
}

impl<'a, S> DecisionRecorder<'a, S> {
    pub(super) fn new(source: &'a mut S, telemetry: TelemetryHandle) -> Self {
        Self { source, telemetry }
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
        let decision = match self.source.decide(input) {
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
        };
        let execution_id = self
            .source
            .model_execution_id()
            .unwrap_or(input.execution_id);
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

pub(super) fn receipt(receipt: &TransitionReceipt, generation: u64, telemetry: &TelemetryHandle) {
    let failure_code = match receipt.status() {
        DispatchStatus::Rejected | DispatchStatus::Cancelled => Some(FailureCode::Rejected),
        DispatchStatus::Unknown => Some(FailureCode::UnknownOutcome),
        DispatchStatus::Accepted | DispatchStatus::Settled => None,
    };
    let _ = telemetry.action_dispatch(
        receipt.operation_id(),
        receipt.action().action_id(),
        receipt.action().kind(),
        generation,
        receipt.status(),
        failure_code,
    );
    if receipt.status() == DispatchStatus::Settled
        && receipt
            .after()
            .is_some_and(|after| after.generation() > generation)
        && receipt.effect_kind().is_some()
        && let Some(after) = receipt.after()
        && let Some(effect_kind) = receipt.effect_kind()
    {
        let _ = telemetry.settlement(
            receipt.operation_id(),
            receipt.action().action_id(),
            generation,
            after,
            effect_kind,
            ObservationSource::Transition,
        );
    }
}

pub(super) fn wait(
    operation_id: &str,
    action_id: &str,
    generation: u64,
    sample: &WaitSample,
    telemetry: &TelemetryHandle,
) {
    let source = match sample.outcome() {
        WaitOutcome::Successor | WaitOutcome::SameStateMutation => ObservationSource::Transition,
        WaitOutcome::Timeout => return,
        WaitOutcome::RecoveryRequired => ObservationSource::Recovery,
    };
    if let Some(after) = sample.observation()
        && after.generation() > generation
        && let Some(effect_kind) = sample.effect_kind()
    {
        let _ = telemetry.settlement(
            operation_id,
            action_id,
            generation,
            after,
            effect_kind,
            source,
        );
    }
}

pub(super) fn complete(report: &EpisodeRunReport, telemetry: &TelemetryHandle) {
    let outcome = match report.terminal_stage() {
        sts2_harness::EpisodeStage::Victory => GameOutcome::Success,
        sts2_harness::EpisodeStage::Defeat => GameOutcome::Failure,
        _ => GameOutcome::Unavailable,
    };
    let _ = telemetry.terminal(report.final_observation(), outcome);
}
