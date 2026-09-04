// SPDX-License-Identifier: MIT

const MAX_ACTIONS: usize = 256;

/// Semantic action kinds accepted by the episode coordinator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKind {
    StartRun,
    SelectMapNode,
    PlayCard,
    EndTurn,
    ChooseReward,
    SkipReward,
    ShopPurchase,
    ShopRemove,
    Rest,
    Smith,
    EventChoice,
    SelectCard,
    ConfirmVictory,
    SaveQuit,
}

/// One host-generated legal action reference. The action payload remains owned by the host/MCP
/// boundary; the model can select only this stable ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpisodeLegalAction {
    action_id: String,
    kind: ActionKind,
}

impl EpisodeLegalAction {
    pub fn new(action_id: impl Into<String>, kind: ActionKind) -> Result<Self, ActionSetError> {
        let action_id = action_id.into();
        if action_id.is_empty()
            || action_id.len() > 512
            || !action_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
        {
            return Err(ActionSetError::InvalidIdentity);
        }
        Ok(Self { action_id, kind })
    }

    #[must_use]
    pub fn action_id(&self) -> &str {
        &self.action_id
    }

    #[must_use]
    pub const fn kind(&self) -> ActionKind {
        self.kind
    }
}

/// Complete host-generated legal-action set tied to an observation generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpisodeLegalActionSet {
    state_id: String,
    generation: u64,
    actions: Vec<EpisodeLegalAction>,
}

impl EpisodeLegalActionSet {
    pub fn new(
        state_id: impl Into<String>,
        generation: u64,
        actions: Vec<EpisodeLegalAction>,
    ) -> Result<Self, ActionSetError> {
        let state_id = state_id.into();
        if !valid_identity(&state_id) || generation > 9_007_199_254_740_991 {
            return Err(ActionSetError::InvalidIdentity);
        }
        if actions.len() > MAX_ACTIONS {
            return Err(ActionSetError::TooManyActions);
        }
        for (index, action) in actions.iter().enumerate() {
            if actions[..index]
                .iter()
                .any(|previous| previous.action_id == action.action_id)
            {
                return Err(ActionSetError::DuplicateAction);
            }
        }
        Ok(Self {
            state_id,
            generation,
            actions,
        })
    }

    #[must_use]
    pub fn state_id(&self) -> &str {
        &self.state_id
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn actions(&self) -> &[EpisodeLegalAction] {
        &self.actions
    }

    #[must_use]
    pub fn find(&self, action_id: &str) -> Option<&EpisodeLegalAction> {
        self.actions.iter().find(|action| action.action_id == action_id)
    }

    pub fn assert_matches(
        &self,
        state_id: &str,
        generation: u64,
    ) -> Result<(), ActionSetError> {
        if self.state_id != state_id || self.generation != generation {
            return Err(ActionSetError::StaleObservation);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionSetError {
    InvalidIdentity,
    TooManyActions,
    DuplicateAction,
    StaleObservation,
}

impl std::fmt::Display for ActionSetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "legal-action identity or generation is invalid",
            Self::TooManyActions => "legal-action set exceeds its bound",
            Self::DuplicateAction => "legal-action IDs must be unique",
            Self::StaleObservation => "legal-action set is stale for the observation",
        })
    }
}

impl std::error::Error for ActionSetError {}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}
