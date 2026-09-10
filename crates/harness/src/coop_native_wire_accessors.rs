// SPDX-License-Identifier: MIT

impl CoopNativeKind {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "observation" => Self::Observation,
            "legal_catalog_request" => Self::LegalCatalogRequest,
            "legal_catalog_response" => Self::LegalCatalogResponse,
            "local_action_request" => Self::LocalActionRequest,
            "shared_vote_request" => Self::SharedVoteRequest,
            "rejoin_request" => Self::RejoinRequest,
            "effect_response" => Self::EffectResponse,
            "recovery_response" => Self::RecoveryResponse,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::LegalCatalogRequest => "legal_catalog_request",
            Self::LegalCatalogResponse => "legal_catalog_response",
            Self::LocalActionRequest => "local_action_request",
            Self::SharedVoteRequest => "shared_vote_request",
            Self::RejoinRequest => "rejoin_request",
            Self::EffectResponse => "effect_response",
            Self::RecoveryResponse => "recovery_response",
        }
    }

    #[must_use]
    pub const fn is_request(self) -> bool {
        matches!(
            self,
            Self::LegalCatalogRequest
                | Self::LocalActionRequest
                | Self::SharedVoteRequest
                | Self::RejoinRequest
        )
    }
}

impl CoopNativeStatus {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "accepted" => Self::Accepted,
            "settled" => Self::Settled,
            "rejected" => Self::Rejected,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Settled => "settled",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
        }
    }
}

impl CoopNativeActionKind {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "play_card" => Self::PlayCard,
            "end_turn" => Self::EndTurn,
            "select_card" => Self::SelectCard,
            "choose_reward" => Self::ChooseReward,
            "confirm_selection" => Self::ConfirmSelection,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlayCard => "play_card",
            Self::EndTurn => "end_turn",
            Self::SelectCard => "select_card",
            Self::ChooseReward => "choose_reward",
            Self::ConfirmSelection => "confirm_selection",
        }
    }
}

impl CoopNativePeerRole {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "local" => Self::Local,
            "ally" => Self::Ally,
            _ => return None,
        })
    }
}

impl CoopNativeChecksumStatus {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "available" => Self::Available,
            "unavailable" => Self::Unavailable,
            "unknown" => Self::Unknown,
            "disabled" => Self::Disabled,
            "enabled_unread" => Self::EnabledUnread,
            "enabled" => Self::Enabled,
            "divergent" => Self::Divergent,
            "matched" => Self::Matched,
            _ => return None,
        })
    }
}

impl CoopNativeEffectKind {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "turn_ended" => Self::TurnEnded,
            "shared_event_vote" => Self::SharedEventVote,
            "native_end_turn_settled" => Self::NativeEndTurnSettled,
            "native_play_card_settled" => Self::NativePlayCardSettled,
            "native_shared_event_vote_settled" => Self::NativeSharedEventVoteSettled,
            "native_treasure_relic_vote_settled" => Self::NativeTreasureRelicVoteSettled,
            _ => return None,
        })
    }
}

impl CoopNativeRecoveryKind {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "reconcile" => Self::Reconcile,
            "rejoin" => Self::Rejoin,
            _ => return None,
        })
    }
}

impl CoopNativeHeader {
    #[must_use]
    pub fn correlation_id(&self) -> &CoopNativeCorrelationId {
        &self.correlation_id
    }

    #[must_use]
    pub fn instance_id(&self) -> &CoopNativeInstanceId {
        &self.instance_id
    }

    #[must_use]
    pub fn session_id(&self) -> &CoopNativeSessionId {
        &self.session_id
    }

    #[must_use]
    pub fn lease_id(&self) -> &CoopNativeLeaseId {
        &self.lease_id
    }

    #[must_use]
    pub const fn lease_epoch(&self) -> u64 {
        self.lease_epoch
    }
}

impl CoopNativeAction {
    #[must_use]
    pub const fn kind(&self) -> CoopNativeActionKind {
        self.kind
    }

    #[must_use]
    pub fn action_id(&self) -> &CoopNativeActionId {
        &self.action_id
    }

    #[must_use]
    pub fn target_peer(&self) -> Option<&CoopNativePeerId> {
        self.target_peer.as_ref()
    }
}

impl CoopNativeLocalActionRequest {
    #[must_use]
    pub fn operation_id(&self) -> &CoopNativeOperationId {
        &self.operation_id
    }

    #[must_use]
    pub fn actor_peer(&self) -> &CoopNativePeerId {
        &self.actor_peer
    }

    #[must_use]
    pub const fn expected_host_generation(&self) -> u64 {
        self.expected_host_generation
    }

    #[must_use]
    pub fn action(&self) -> &CoopNativeAction {
        &self.action
    }
}

impl CoopNativeLegalCatalogRequest {
    #[must_use]
    pub fn actor_peer(&self) -> &CoopNativePeerId {
        &self.actor_peer
    }

    #[must_use]
    pub const fn expected_host_generation(&self) -> u64 {
        self.expected_host_generation
    }
}

impl CoopNativeLegalCatalogResponse {
    #[must_use]
    pub fn actor_peer(&self) -> &CoopNativePeerId {
        &self.actor_peer
    }

    #[must_use]
    pub const fn expected_host_generation(&self) -> u64 {
        self.expected_host_generation
    }

    #[must_use]
    pub fn observation(&self) -> &CoopNativeObservation {
        &self.observation
    }

    #[must_use]
    pub fn catalog(&self) -> &CoopNativeCatalog {
        &self.catalog
    }
}

impl CoopNativeSharedVoteRequest {
    #[must_use]
    pub fn operation_id(&self) -> &CoopNativeOperationId {
        &self.operation_id
    }

    #[must_use]
    pub fn actor_peer(&self) -> &CoopNativePeerId {
        &self.actor_peer
    }

    #[must_use]
    pub const fn expected_host_generation(&self) -> u64 {
        self.expected_host_generation
    }

    #[must_use]
    pub fn vote(&self) -> &CoopNativeVote {
        &self.vote
    }
}

impl CoopNativeRejoinRequest {
    #[must_use]
    pub fn operation_id(&self) -> &CoopNativeOperationId {
        &self.operation_id
    }

    #[must_use]
    pub fn actor_peer(&self) -> &CoopNativePeerId {
        &self.actor_peer
    }

    #[must_use]
    pub const fn expected_host_generation(&self) -> u64 {
        self.expected_host_generation
    }

    #[must_use]
    pub fn recovery(&self) -> &CoopNativeRecovery {
        &self.recovery
    }
}

impl CoopNativeVote {
    #[must_use]
    pub fn proposal_id(&self) -> &CoopNativeProposalId {
        &self.proposal_id
    }

    #[must_use]
    pub fn voter_peer(&self) -> &CoopNativePeerId {
        &self.voter_peer
    }

    #[must_use]
    pub fn choice(&self) -> &str {
        &self.choice
    }
}

include!("coop_native_wire_accessors_extra.rs");
