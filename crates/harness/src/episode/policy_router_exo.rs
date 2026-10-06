// SPDX-License-Identifier: MIT

use super::*;

impl<T> ExoDecisionSource<T> {
    #[must_use]
    pub fn new(session: ExoSession<T>) -> Self {
        Self {
            session,
            plan: None,
            execution_id: None,
        }
    }

    pub fn close(&mut self) -> Result<(), ExoError>
    where
        T: crate::exo::ExoTransport,
    {
        self.plan = None;
        self.session.close()
    }
}

impl<T: crate::exo::ExoTransport> DecisionSource for ExoDecisionSource<T> {
    fn close(&mut self) -> Result<(), PolicyError> {
        ExoDecisionSource::close(self).map_err(map_exo_error)
    }

    fn action_completed(&mut self, settled: bool) {
        if !settled {
            self.plan = None;
            return;
        }
        if let Some(plan) = &mut self.plan {
            plan.action_completed(settled);
        }
    }

    fn model_execution_id(&self) -> Option<ModelExecutionId> {
        self.execution_id
    }

    fn decide_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
    ) -> Result<Decision, PolicyError> {
        if decision_profile_ref.is_empty() || context_ref.is_empty() {
            return Err(PolicyError::InputBlocked);
        }
        self.decide(input)
    }

    fn prepare_managed_context(
        &mut self,
        input: &DecisionInput,
        source: &ContextRenderSource,
    ) -> Result<PreparedContext, PolicyError> {
        self.prepare_managed_context_impl(input, source)
    }

    fn managed_render_config(&self) -> Option<crate::exo::ExoConfig> {
        Some(self.session.config().clone())
    }

    fn decide_prepared_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
        prepared: &PreparedContext,
    ) -> Result<Decision, PolicyError> {
        self.decide_prepared_for_impl(input, decision_profile_ref, context_ref, prepared)
    }

    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
        input
            .observation
            .assert_actionable()
            .map_err(|_| PolicyError::InputBlocked)?;
        input
            .legal_actions
            .assert_matches(input.observation.state_id(), input.observation.generation())
            .map_err(|_| PolicyError::StaleCatalog)?;
        if self
            .plan
            .as_ref()
            .is_some_and(ActionPlan::awaiting_settlement)
        {
            return Err(PolicyError::InputBlocked);
        }
        if let Some(plan) = &mut self.plan
            && let Some(decision) = plan.next(input, false)
        {
            self.execution_id = Some(plan.execution_id);
            return Ok(decision);
        }
        self.plan = None;
        self.execution_id = Some(input.execution_id);
        let legal_action_ids = input
            .legal_actions
            .actions()
            .iter()
            .map(|action| action.action_id().to_owned())
            .collect();
        let decision = if let Some(map_context) = input.map_context() {
            self.session.decide_with_map(
                input.execution_id,
                input.observation.state_id(),
                input.observation.generation(),
                input.observation.fair_play().clone(),
                legal_action_ids,
                input.objective.clone(),
                input.hard_constraints.clone(),
                map_context.clone(),
            )
        } else {
            self.session.decide(
                input.execution_id,
                input.observation.state_id(),
                input.observation.generation(),
                input.observation.fair_play().clone(),
                legal_action_ids,
                input.objective.clone(),
                input.hard_constraints.clone(),
            )
        }
        .map_err(map_exo_error)?;
        if let Decision::Plan {
            action_ids,
            rationale,
        } = decision
        {
            let mut plan = ActionPlan::new(input, &action_ids, rationale)?;
            let action = plan.next(input, true).ok_or(PolicyError::IllegalAction)?;
            self.plan = Some(plan);
            Ok(action)
        } else {
            Ok(decision)
        }
    }
}
