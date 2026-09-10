// SPDX-License-Identifier: MIT

impl RuntimeV3Port {
    pub(super) fn is_expert_profile(&self) -> bool {
        matches!(self.config.runtime_profile.as_str(), PROFILE | REST_PROFILE)
    }

    pub(super) fn is_rest_profile(&self) -> bool {
        self.config.runtime_profile == REST_PROFILE
    }

    pub(super) fn expert_mcp_profile(&self) -> &'static str {
        if self.is_rest_profile() {
            REST_PROFILE
        } else {
            PROFILE
        }
    }

    pub(super) fn uses_expert_transport(
        &self,
        action: &EpisodeLegalAction,
        payload: &Value,
    ) -> bool {
        if action.kind() == ActionKind::UsePotion {
            return self.is_expert_profile() && !self.is_rest_profile();
        }
        self.is_rest_profile()
            && matches!(
                action.kind(),
                ActionKind::RestOption
                    | ActionKind::SelectCard
                    | ActionKind::SelectPlayer
                    | ActionKind::ConfirmSelection
                    | ActionKind::CancelSelection
            )
            && payload
                .get("rest_option_id")
                .and_then(Value::as_str)
                .is_some()
    }
}
