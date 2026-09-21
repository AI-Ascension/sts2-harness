// SPDX-License-Identifier: MIT

//! Held provider-decision attempts for the authored-workflow `Decide` node.
//!
//! A live `Decide` node pays a model runtime for exactly one exchange. The attempt identity is
//! therefore installed *before* the exchange and released only once a usable decision for that
//! exact admitted request exists, so a lost reply, an operator re-issued step or a restart cannot
//! become a second paid exchange. The hold is also projected into the run snapshot, so an
//! outstanding attempt is published as `pending_operation` with `authority.recovery =
//! pending_effect_visible` and admitted as `Reconciling`, instead of leaving the run
//! indistinguishable from a state with nothing outstanding.
//!
//! Only a refusal the provider owner reported before it could have written releases the hold.
//! Every other outcome keeps the attempt identity held -- an unresolved transport outcome, a
//! provider that was reached and did not answer, a retry whose admitted request no longer matches
//! the one that was paid for, and even a fault while consuming an accepted decision -- because the
//! harness cannot prove no inference happened, and it may not replace an unproven exchange with a
//! fresh one.

use serde_json::json;
use uuid::Uuid;

use super::super::contract::{ErrorClass, PendingOperation, PendingOperationState};
use super::super::service::ManagementError;
use super::execution_state::PendingDecision;
use super::node::LiveNodeExecutor;
use super::node_projection::catalog_digest;
use crate::episode::{DecisionInput, EpisodeLegalActionSet, EpisodeObservation};
use crate::workflow::{
    ActionId, BoundedText, DecisionProposal, EdgeOutcome, Generation, NodeOutcome,
    ProviderExecutionId, RuntimeFault, TypedValue,
};

/// Classify a refused provider exchange into the runtime fault the node must report.
///
/// The session reports how far the call got, and the held attempt must survive exactly the cases
/// where the harness cannot rule out a completed write. `Unresolved` is that report: the boundary
/// owner cannot prove the provider was never reached, so the attempt stays held and the run needs
/// reconciliation.
///
/// Any other class is the boundary owner's statement that it refused before it could write, so the
/// attempt is released and the refusal stays retryable by policy. The decision is deliberately not
/// made from the error *code*: the served composition raises the pre-exchange refusal
/// `context_render_source_unavailable` with the same code that its post-exchange re-assertion can
/// raise, and releasing on that code alone would authorize a second paid exchange.
pub(super) fn classify_decision_refusal(error: &ManagementError) -> RuntimeFault {
    match error.class {
        ErrorClass::Unresolved => RuntimeFault::UnknownEffect,
        _ => RuntimeFault::ExecutorUnavailable,
    }
}

/// Canonical digest of the exact admitted request one held attempt was paid for.
///
/// The legal-action catalog is folded in through the caller-supplied set, so a retry whose
/// observation, objective, constraints or catalog changed can never reuse the attempt.
pub(super) fn decision_input_digest(
    input: &DecisionInput,
    actions: &EpisodeLegalActionSet,
) -> Result<String, RuntimeFault> {
    let catalog = actions
        .actions()
        .iter()
        .map(|action| json!({"action_id": action.action_id(), "kind": format!("{:?}", action.kind())}))
        .collect::<Vec<_>>();
    let envelope = json!({
        "execution_id": input.execution_id.get(),
        "state_id": input.observation.state_id(),
        "generation": input.observation.generation(),
        "catalog": catalog,
        "objective": input.objective,
        "hard_constraints": input.hard_constraints,
    });
    let bytes = serde_json::to_vec(&envelope).map_err(|_| RuntimeFault::InvalidState)?;
    Ok(crate::sha256_hex(bytes))
}

/// The durable intent projection of one held decision attempt.
pub(super) fn decision_intent(held: &PendingDecision, instance_id: &str) -> PendingOperation {
    PendingOperation {
        operation_id: held.operation_id.clone(),
        classification: held.state.clone(),
        instance_id: instance_id.to_owned(),
        original_generation: held.generation,
        payload_digest: held.input_digest.clone(),
    }
}

impl LiveNodeExecutor<'_, '_> {
    /// Execute one `Decide` node under the held-attempt discipline.
    pub(super) fn decide_with_held_attempt(
        &mut self,
        decision_profile_ref: &str,
        context_ref: &str,
    ) -> Result<NodeOutcome, RuntimeFault> {
        if self.state.pending_decision.is_some() {
            return self.retry_held_decision();
        }
        let observation = self.admitted_observation()?;
        if self.state.provider_calls >= self.state.max_provider_calls {
            return Err(RuntimeFault::BudgetExceeded);
        }
        let actions = self.admitted_actions(&observation)?;
        let execution_id = crate::ModelExecutionId::new(self.state.provider_calls + 1)
            .ok_or(RuntimeFault::InvalidState)?;
        let input = self.decision_input(execution_id, &observation, &actions);
        self.hold_decision_attempt(&input, &actions)?;
        let decision =
            match self
                .state
                .session
                .decide_for(&input, decision_profile_ref, context_ref)
            {
                Ok(decision) => decision,
                Err(error) => return Err(self.fail_decision_attempt(&error)),
            };
        self.state.provider_calls = self.state.provider_calls.saturating_add(1);
        self.state.actions = Some(actions);
        if let Some(held) = self.state.pending_decision.as_mut() {
            // Marked accepted before consumption: if turning the decision into a node outcome
            // faults, the hold survives carrying the accepted attempt rather than silently
            // authorizing a replacement exchange.
            held.state = PendingOperationState::Accepted;
            held.resolved = Some(Box::new(decision.clone()));
        }
        self.finish_decision(&observation, execution_id, &decision)
    }

    /// Re-enter a held attempt without paying for a second exchange.
    fn retry_held_decision(&mut self) -> Result<NodeOutcome, RuntimeFault> {
        let (held_execution_id, held_digest) = {
            let held = self
                .state
                .pending_decision
                .as_ref()
                .ok_or(RuntimeFault::InvalidState)?;
            (held.execution_id, held.input_digest.clone())
        };
        let observation = self.admitted_observation()?;
        let actions = self.admitted_actions(&observation)?;
        let execution_id =
            crate::ModelExecutionId::new(held_execution_id).ok_or(RuntimeFault::InvalidState)?;
        let input = self.decision_input(execution_id, &observation, &actions);
        // The admitted request is the identity, not only the execution number: the same number
        // against a different observation, objective or catalog is a request the attempt was never
        // paid for. No served command reaches this fence today -- a fault raised after the decision
        // was accepted fails the runtime, and only a `NeedsOperator` runtime is re-opened -- so it
        // is held for any future caller that can re-step a node already carrying an accepted
        // attempt, and it is deliberately asserted here rather than assumed from the call graph.
        if decision_input_digest(&input, &actions)? != held_digest {
            // The admitted request is no longer the one the attempt was paid for. The held
            // identity may not be retargeted, and a second identity may not be minted while this
            // one is unresolved, so the attempt stays held and the run blocks for reconciliation.
            return Err(RuntimeFault::UnknownEffect);
        }
        let held = self
            .state
            .pending_decision
            .take()
            .ok_or(RuntimeFault::InvalidState)?;
        self.state.actions = Some(actions);
        match held.resolved {
            Some(decision) => self.finish_decision(&observation, execution_id, &decision),
            None => {
                // A possible provider write stands unresolved: no other exchange is permitted.
                self.state.pending_decision = Some(held);
                Err(RuntimeFault::UnknownEffect)
            }
        }
    }

    fn admitted_observation(&self) -> Result<EpisodeObservation, RuntimeFault> {
        self.state
            .observation
            .clone()
            .ok_or(RuntimeFault::InvalidState)
    }

    fn admitted_actions(
        &mut self,
        observation: &EpisodeObservation,
    ) -> Result<EpisodeLegalActionSet, RuntimeFault> {
        let actions = self
            .state
            .session
            .legal_actions(observation.state_id(), observation.generation())
            .map_err(|_| RuntimeFault::ExecutorUnavailable)?;
        actions
            .assert_matches(observation.state_id(), observation.generation())
            .map_err(|_| RuntimeFault::ExecutorRejected)?;
        Ok(actions)
    }

    fn decision_input(
        &self,
        execution_id: crate::ModelExecutionId,
        observation: &EpisodeObservation,
        actions: &EpisodeLegalActionSet,
    ) -> DecisionInput {
        DecisionInput::new(
            execution_id,
            observation.clone(),
            actions.clone(),
            self.state.options.objective.clone(),
            self.state.options.hard_constraints.clone(),
        )
    }

    /// Install and durably record the attempt before crossing the provider boundary.
    fn hold_decision_attempt(
        &mut self,
        input: &DecisionInput,
        actions: &EpisodeLegalActionSet,
    ) -> Result<(), RuntimeFault> {
        let held = PendingDecision {
            operation_id: Uuid::new_v4().to_string(),
            execution_id: input.execution_id.get(),
            generation: input.observation.generation(),
            input_digest: decision_input_digest(input, actions)?,
            state: PendingOperationState::Intent,
            resolved: None,
        };
        if let Some(recorder) = self.intent_recorder {
            let pending = decision_intent(&held, self.state.instance_id.as_str());
            if recorder(pending).is_err() {
                return Err(RuntimeFault::ExecutorUnavailable);
            }
        }
        self.state.pending_decision = Some(held);
        Ok(())
    }

    fn fail_decision_attempt(&mut self, error: &ManagementError) -> RuntimeFault {
        let fault = classify_decision_refusal(error);
        if fault != RuntimeFault::UnknownEffect {
            // The refusal was reported before the adapter could write, so nothing is in doubt.
            self.state.pending_decision = None;
        }
        fault
    }

    /// Turn one usable decision into the node outcome and release the attempt.
    fn finish_decision(
        &mut self,
        observation: &EpisodeObservation,
        execution_id: crate::ModelExecutionId,
        decision: &crate::Decision,
    ) -> Result<NodeOutcome, RuntimeFault> {
        let crate::Decision::Action {
            action_id,
            rationale,
            ..
        } = decision
        else {
            self.state.pending_decision = None;
            return Ok(NodeOutcome::new(
                EdgeOutcome::Unavailable,
                TypedValue::Unavailable,
            ));
        };
        let actions = self
            .state
            .actions
            .clone()
            .ok_or(RuntimeFault::InvalidState)?;
        let action = actions
            .find(action_id)
            .ok_or(RuntimeFault::ExecutorRejected)?;
        let model_id = self
            .state
            .session
            .model_execution_id()
            .unwrap_or(execution_id);
        let provider_execution_id = ProviderExecutionId::new(model_id.to_string())
            .map_err(|_| RuntimeFault::InvalidState)?;
        let proposal = DecisionProposal {
            state_id: BoundedText::new(observation.state_id())
                .map_err(|_| RuntimeFault::InvalidState)?,
            generation: Generation::new(observation.generation())
                .map_err(|_| RuntimeFault::InvalidState)?,
            catalog_digest: catalog_digest(&actions)?,
            action_id: ActionId::new(action.action_id()).map_err(|_| RuntimeFault::InvalidState)?,
            provider_execution_id,
            reason_code: BoundedText::new(rationale).ok(),
        };
        self.state.pending_decision = None;
        Ok(NodeOutcome::new(
            EdgeOutcome::Ok,
            TypedValue::DecisionProposal(Box::new(proposal)),
        ))
    }
}
