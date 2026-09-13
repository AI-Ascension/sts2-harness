// SPDX-License-Identifier: MIT

use uuid::Uuid;

use super::super::contract::PendingOperationState;
use super::execution::{LiveNodeState, PendingDispatch};
use super::node_projection::{catalog_digest, observation_value};
use crate::episode::{
    ActionIdentity, DecisionInput, DispatchStatus, TransitionReceipt, WaitOutcome, WaitSample,
    verify_settlement,
};
use crate::workflow::{
    ActionId, BoundedText, DecisionProposal, EdgeOutcome, Generation, NodeDefinition, NodeExecutor,
    NodeOutcome, ProviderExecutionId, RuntimeContext, RuntimeFault, TypedValue,
};

pub(super) struct LiveNodeExecutor<'a> {
    pub(super) state: &'a mut LiveNodeState,
}

impl NodeExecutor for LiveNodeExecutor<'_> {
    fn execute(
        &mut self,
        node: &NodeDefinition,
        context: &RuntimeContext,
    ) -> Result<NodeOutcome, RuntimeFault> {
        match node {
            NodeDefinition::Observe { .. } => self.observe(),
            NodeDefinition::Decide { .. } => self.decide(),
            NodeDefinition::ExecuteAction { config, .. } => self.execute_action(config, context),
            _ => Err(RuntimeFault::ExecutorUnavailable),
        }
    }
}

impl LiveNodeExecutor<'_> {
    fn observe(&mut self) -> Result<NodeOutcome, RuntimeFault> {
        let observation = self
            .state
            .session
            .observe()
            .map_err(|_| RuntimeFault::ExecutorUnavailable)?;
        let value = observation_value(&observation)?;
        self.state.observation = Some(observation);
        self.state.actions = None;
        Ok(NodeOutcome::new(
            EdgeOutcome::Ok,
            TypedValue::Observation(value),
        ))
    }

    fn decide(&mut self) -> Result<NodeOutcome, RuntimeFault> {
        let observation = self
            .state
            .observation
            .clone()
            .ok_or(RuntimeFault::InvalidState)?;
        if self.state.provider_calls >= self.state.max_provider_calls {
            return Err(RuntimeFault::BudgetExceeded);
        }
        let actions = self
            .state
            .session
            .legal_actions(observation.state_id(), observation.generation())
            .map_err(|_| RuntimeFault::ExecutorUnavailable)?;
        actions
            .assert_matches(observation.state_id(), observation.generation())
            .map_err(|_| RuntimeFault::ExecutorRejected)?;
        let execution_id = crate::ModelExecutionId::new(self.state.provider_calls + 1)
            .ok_or(RuntimeFault::InvalidState)?;
        let input = DecisionInput::new(
            execution_id,
            observation.clone(),
            actions.clone(),
            self.state.options.objective.clone(),
            self.state.options.hard_constraints.clone(),
        );
        let decision = self
            .state
            .session
            .decide(&input)
            .map_err(|_| RuntimeFault::ExecutorUnavailable)?;
        self.state.provider_calls = self.state.provider_calls.saturating_add(1);
        self.state.actions = Some(actions.clone());
        let crate::Decision::Action {
            action_id,
            rationale,
            ..
        } = decision
        else {
            return Ok(NodeOutcome::new(
                EdgeOutcome::Unavailable,
                TypedValue::Unavailable,
            ));
        };
        let action = actions
            .find(&action_id)
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
        Ok(NodeOutcome::new(
            EdgeOutcome::Ok,
            TypedValue::DecisionProposal(Box::new(proposal)),
        ))
    }

    fn execute_action(
        &mut self,
        config: &crate::workflow::ExecuteActionConfig,
        context: &RuntimeContext,
    ) -> Result<NodeOutcome, RuntimeFault> {
        if self.state.pending.is_some() {
            let receipt = self
                .state
                .pending
                .as_mut()
                .and_then(|pending| pending.resolved.take());
            if let Some(receipt) = receipt {
                self.state.pending = None;
                return self.finish_receipt(receipt);
            }
            return Err(RuntimeFault::UnknownEffect);
        }
        let key = format!(
            "{}/{}",
            config.proposal_from.node_id, config.proposal_from.output
        );
        let TypedValue::DecisionProposal(proposal) = context
            .outputs
            .get(&key)
            .ok_or(RuntimeFault::InvalidState)?
        else {
            return Err(RuntimeFault::TypeMismatch);
        };
        let observation = self
            .state
            .observation
            .clone()
            .ok_or(RuntimeFault::InvalidState)?;
        let actions = self
            .state
            .actions
            .clone()
            .ok_or(RuntimeFault::InvalidState)?;
        if proposal.state_id.as_str() != observation.state_id()
            || proposal.generation.get() != observation.generation()
            || proposal.catalog_digest != catalog_digest(&actions)?
        {
            return Err(RuntimeFault::ExecutorRejected);
        }
        let action = actions
            .find(proposal.action_id.as_str())
            .cloned()
            .ok_or(RuntimeFault::ExecutorRejected)?;
        let operation_id = Uuid::new_v4().to_string();
        let identity = ActionIdentity::new_v4(
            operation_id,
            observation.state_id().to_owned(),
            observation.generation(),
            action.action_id().to_owned(),
        )
        .map_err(|_| RuntimeFault::InvalidState)?;
        let receipt = self
            .state
            .session
            .dispatch_action(&identity, &action)
            .map_err(|_| RuntimeFault::UnknownEffect)?;
        match receipt.status() {
            DispatchStatus::Rejected | DispatchStatus::Cancelled => {
                self.state.session.action_completed(false);
                Ok(NodeOutcome::new(EdgeOutcome::Error, TypedValue::Null))
            }
            DispatchStatus::Unknown => {
                self.state.pending = Some(PendingDispatch {
                    identity,
                    action,
                    state: PendingOperationState::Unknown,
                    resolved: None,
                });
                Err(RuntimeFault::UnknownEffect)
            }
            DispatchStatus::Accepted => {
                self.state.pending = Some(PendingDispatch {
                    identity,
                    action,
                    state: PendingOperationState::Accepted,
                    resolved: None,
                });
                let pending = self
                    .state
                    .pending
                    .as_ref()
                    .ok_or(RuntimeFault::InvalidState)?;
                let sample = self
                    .state
                    .session
                    .wait_for_transition(
                        pending.identity.operation_id.as_str(),
                        self.state.options.transition_wait_millis,
                    )
                    .map_err(|_| RuntimeFault::UnknownEffect)?;
                let settled = settled_receipt(pending, sample)?;
                self.state.pending = None;
                self.finish_receipt(settled)
            }
            DispatchStatus::Settled => self.finish_receipt(receipt),
        }
    }

    fn finish_receipt(&mut self, receipt: TransitionReceipt) -> Result<NodeOutcome, RuntimeFault> {
        match receipt.status() {
            DispatchStatus::Rejected | DispatchStatus::Cancelled => {
                self.state.session.action_completed(false);
                return Ok(NodeOutcome::new(EdgeOutcome::Error, TypedValue::Null));
            }
            DispatchStatus::Settled => {}
            DispatchStatus::Accepted | DispatchStatus::Unknown => {
                return Err(RuntimeFault::UnknownEffect);
            }
        }
        let before = self
            .state
            .observation
            .as_ref()
            .ok_or(RuntimeFault::InvalidState)?;
        verify_settlement(before, &receipt).map_err(|_| RuntimeFault::ExecutorRejected)?;
        if let Some(after) = receipt.after().cloned() {
            self.state.observation = Some(after);
        }
        self.state.session.action_completed(true);
        Ok(NodeOutcome::new(EdgeOutcome::Ok, TypedValue::Null))
    }
}

pub(super) fn reconcile_pending(
    state: &mut LiveNodeState,
) -> Result<(), super::super::service::ManagementError> {
    let Some(pending) = state.pending.as_mut() else {
        return Ok(());
    };
    let receipt = state
        .session
        .reconcile(pending.identity.operation_id.as_str())?;
    if receipt.operation_id() != pending.identity.operation_id
        || receipt.action() != &pending.action
    {
        return Err(super::super::service::ManagementError::conflict(
            "live_reconcile_identity",
            "reconciliation returned a different operation or action",
        ));
    }
    match receipt.status() {
        DispatchStatus::Settled | DispatchStatus::Rejected | DispatchStatus::Cancelled => {
            pending.resolved = Some(receipt);
            Ok(())
        }
        DispatchStatus::Accepted | DispatchStatus::Unknown => {
            pending.state = PendingOperationState::Unknown;
            Err(super::super::service::ManagementError::unresolved(
                "live_operation_unknown",
                "accepted mutation remains unresolved; no replacement action is permitted",
            ))
        }
    }
}

fn settled_receipt(
    pending: &PendingDispatch,
    sample: WaitSample,
) -> Result<TransitionReceipt, RuntimeFault> {
    if !matches!(
        sample.outcome(),
        WaitOutcome::Successor | WaitOutcome::SameStateMutation
    ) {
        return Err(RuntimeFault::UnknownEffect);
    }
    let after = sample
        .observation()
        .cloned()
        .ok_or(RuntimeFault::UnknownEffect)?;
    let effect = sample.effect_kind().ok_or(RuntimeFault::UnknownEffect)?;
    Ok(TransitionReceipt::new(
        pending.identity.operation_id.clone(),
        pending.action.clone(),
        DispatchStatus::Settled,
        Some(after),
        Some(effect.to_owned()),
        None,
    ))
}
