// SPDX-License-Identifier: MIT

//! Capacity-one verifier lease; cancellation poisons the owning session.

use std::sync::Arc;

use super::{SESSION_AVAILABLE, VerifierSession};

pub(super) struct VerifierLease {
    pub(super) session: Arc<VerifierSession>,
    pub(super) completed: bool,
}

impl VerifierLease {
    pub(super) fn complete(mut self) {
        self.completed = true;
        self.session
            .state
            .store(SESSION_AVAILABLE, std::sync::atomic::Ordering::Release);
    }

    pub(super) fn poison(mut self) {
        self.completed = true;
        self.session.poison();
    }
}

impl Drop for VerifierLease {
    fn drop(&mut self) {
        if !self.completed {
            self.session.poison();
        }
    }
}
