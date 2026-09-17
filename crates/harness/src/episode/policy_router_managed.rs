// SPDX-License-Identifier: MIT

use super::*;

impl<T: crate::exo::ExoTransport> ExoDecisionSource<T> {
    pub(super) fn prepare_managed_context_impl(
        &mut self,
        input: &DecisionInput,
        source: &ContextRenderSource,
    ) -> Result<PreparedContext, PolicyError> {
        // A managed decision is authorized by this exact prepared request. A plan tail
        // would reuse an earlier provider response without consuming the newly rendered bytes.
        self.plan = None;
        input
            .observation
            .assert_actionable()
            .map_err(|_| PolicyError::InputBlocked)?;
        input
            .legal_actions
            .assert_matches(input.observation.state_id(), input.observation.generation())
            .map_err(|_| PolicyError::StaleCatalog)?;
        let input_for_render = ManagedRenderInput {
            execution_id: input.execution_id.to_string(),
            state_id: input.observation.state_id().to_owned(),
            generation: input.observation.generation(),
            observation: input.observation.fair_play().as_value().clone(),
            legal_action_ids: input
                .legal_actions
                .actions()
                .iter()
                .map(|action| action.action_id().to_owned())
                .collect(),
            objective: input.objective.clone(),
            hard_constraints: input.hard_constraints.clone(),
            map_context: input
                .map_context()
                .map(crate::episode::map::MapDecisionContext::to_wire),
        };
        Self::render_managed_context(source, input_for_render, &self.session)
    }

    /// Renders one managed request through the membership boundary.
    ///
    /// The invocation's selector is bound to this exact invocation identity and draft revision, so
    /// the effective set is gated before any provider bytes exist. Effective absence is refused for
    /// every continuity until the render path can actually omit the observation from those bytes;
    /// admitting it earlier would publish a hidden observation. Failures before dispatch surface as
    /// precise policy errors.
    fn render_managed_context(
        source: &ContextRenderSource,
        input_for_render: ManagedRenderInput,
        session: &ExoSession<T>,
    ) -> Result<PreparedContext, PolicyError> {
        let policy = source.membership.as_ref().map(|selector| {
            selector.bind(
                &source.identity.invocation_id,
                &source.document.draft.base_revision_id,
            )
        });
        crate::context_control::render_with_membership(
            crate::context_control::MembershipRenderRequest {
                boundary: &source.boundary,
                request: input_for_render,
                document: &source.document,
                config: session.config(),
                now: source.now,
                limits: &source.limits,
                policy: policy.as_ref(),
                continuity: source.continuity,
            },
        )
        .map(|(prepared, _)| prepared)
        .map_err(|error| match error {
            crate::context_control::MembershipRenderError::Render(
                crate::context_control::ContextRenderError::ExceedsSelectedLimit(limit),
            ) => PolicyError::SelectedContextLimit(limit),
            crate::context_control::MembershipRenderError::Membership(membership) => {
                PolicyError::MembershipRefused(membership.reason_code())
            }
            _ => PolicyError::ProviderMalformed,
        })
    }

    pub(super) fn decide_prepared_for_impl(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
        prepared: &PreparedContext,
    ) -> Result<Decision, PolicyError> {
        if decision_profile_ref.is_empty()
            || context_ref.is_empty()
            || self
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
        let expected = ManagedRenderInput {
            execution_id: input.execution_id.to_string(),
            state_id: input.observation.state_id().to_owned(),
            generation: input.observation.generation(),
            observation: input.observation.fair_play().as_value().clone(),
            legal_action_ids: input
                .legal_actions
                .actions()
                .iter()
                .map(|action| action.action_id().to_owned())
                .collect(),
            objective: input.objective.clone(),
            hard_constraints: input.hard_constraints.clone(),
            map_context: input
                .map_context()
                .map(crate::episode::map::MapDecisionContext::to_wire),
        };
        if !prepared.matches_input(&expected) {
            return Err(PolicyError::InputBlocked);
        }
        let decision = self
            .session
            .decide_prepared(input.execution_id, prepared)
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
