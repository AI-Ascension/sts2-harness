// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_harness::{
    Decision, DecisionInput, DecisionSource, EpisodeRunReport, PolicyError, TransitionReceipt,
    WaitOutcome, WaitSample,
};

fn enabled() -> bool {
    std::env::var("STS2_LIVE_EPISODE").as_deref() == Ok("true")
}

pub(super) struct DecisionRecorder<'a, S>(pub(super) &'a mut S);

impl<S: DecisionSource> DecisionSource for DecisionRecorder<'_, S> {
    fn model_execution_id(&self) -> Option<sts2_harness::ModelExecutionId> {
        self.0.model_execution_id()
    }
    fn action_completed(&mut self, settled: bool) {
        self.0.action_completed(settled);
    }

    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
        let decision = self.0.decide(input)?;
        let execution_id = self.0.model_execution_id().unwrap_or(input.execution_id);
        if enabled()
            && let Decision::Action {
                action_id,
                rationale,
                ..
            } = &decision
        {
            println!(
                "{}",
                json!({"event":"model_decision",
                    "model_execution_id":execution_id.get(), "action_id":action_id,
                    "reused_model_execution":execution_id != input.execution_id,
                    "rationale":rationale, "observation":input.observation.fair_play().as_value()})
            );
        }
        Ok(decision)
    }
}

pub(super) fn receipt(receipt: &TransitionReceipt) {
    if enabled() {
        println!(
            "{}",
            json!({"event":"action_receipt", "operation_id":receipt.operation_id(),
            "action_id":receipt.action().action_id(), "status":format!("{:?}",receipt.status()),
            "effect":receipt.effect_kind(),
            "observation":receipt.after().map(|after| after.fair_play().as_value())})
        );
    }
}

pub(super) fn wait(operation_id: &str, sample: &WaitSample) {
    if enabled()
        && matches!(
            sample.outcome(),
            WaitOutcome::Successor | WaitOutcome::SameStateMutation
        )
    {
        println!(
            "{}",
            json!({"event":"operation_wait_completed", "operation_id":operation_id,
            "effect":sample.effect_kind(),
            "observation":sample.observation().map(|after| after.fair_play().as_value())})
        );
    }
}

pub(super) fn complete(report: &EpisodeRunReport) {
    if enabled() {
        println!(
            "{}",
            json!({"event":"episode_complete", "steps":report.steps(),
            "transitions":report.transitions(), "recoveries":report.recoveries(),
            "observation":report.final_observation().fair_play().as_value()})
        );
    }
}
