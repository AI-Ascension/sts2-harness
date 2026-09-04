// SPDX-License-Identifier: MIT

use super::legal_actions::EpisodeLegalAction;
use super::observation::EpisodeObservation;

/// Lifecycle status of one mutating dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchStatus {
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Cancelled,
}

/// Transport result retained until the independent postcondition barrier settles it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionReceipt {
    operation_id: String,
    action: EpisodeLegalAction,
    status: DispatchStatus,
    after: Option<EpisodeObservation>,
    effect_kind: Option<String>,
    error_code: Option<String>,
}

impl TransitionReceipt {
    #[must_use]
    pub fn new(
        operation_id: impl Into<String>,
        action: EpisodeLegalAction,
        status: DispatchStatus,
        after: Option<EpisodeObservation>,
        effect_kind: Option<String>,
        error_code: Option<String>,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            action,
            status,
            after,
            effect_kind,
            error_code,
        }
    }

    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    #[must_use]
    pub fn action(&self) -> &EpisodeLegalAction {
        &self.action
    }

    #[must_use]
    pub const fn status(&self) -> DispatchStatus {
        self.status
    }

    #[must_use]
    pub fn after(&self) -> Option<&EpisodeObservation> {
        self.after.as_ref()
    }

    #[must_use]
    pub fn effect_kind(&self) -> Option<&str> {
        self.effect_kind.as_deref()
    }

    #[must_use]
    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }
}
