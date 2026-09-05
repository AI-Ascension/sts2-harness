// SPDX-License-Identifier: MIT

use sts2_harness::{ActionIdentity, EpisodeLegalAction};

#[derive(Clone, Debug)]
pub(super) struct OperationRecord {
    pub(super) state_id: String,
    pub(super) generation: u64,
    pub(super) action: EpisodeLegalAction,
}

impl OperationRecord {
    pub(super) fn new(identity: &ActionIdentity, action: &EpisodeLegalAction) -> Self {
        Self {
            state_id: identity.state_id.clone(),
            generation: identity.generation,
            action: action.clone(),
        }
    }
}
