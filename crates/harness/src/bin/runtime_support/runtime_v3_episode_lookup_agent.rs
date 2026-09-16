// SPDX-License-Identifier: MIT

use sts2_harness::context_memory::policy_owner::ActivePolicyBinding;
use sts2_harness::game_information::{LookupAgentInput, LookupAgentPort, LookupError, LookupTurn};

pub(super) struct FencedLookupAgent<'a> {
    pub(super) owner:
        std::sync::Arc<super::super::game_information_owner::RuntimeGameInformationOwner>,
    pub(super) expected: ActivePolicyBinding,
    pub(super) agent: &'a mut dyn LookupAgentPort,
}

impl LookupAgentPort for FencedLookupAgent<'_> {
    fn next_turn(&mut self, input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError> {
        self.owner
            .lookup_snapshot(Some(&self.expected))
            .map_err(|_| LookupError::Scope)?;
        let turn = self.agent.next_turn(input)?;
        self.owner
            .lookup_snapshot(Some(&self.expected))
            .map_err(|_| LookupError::Scope)?;
        Ok(turn)
    }
}
