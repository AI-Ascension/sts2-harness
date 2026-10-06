// SPDX-License-Identifier: MIT

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub(in crate::runtime_support) struct TestPolicyClock {
    seconds: AtomicU64,
    rollback_after_adoption: bool,
    saw_grant_revoked_preflight: AtomicBool,
}

impl TestPolicyClock {
    pub(super) fn fixed(seconds: u64) -> Arc<Self> {
        Arc::new(Self {
            seconds: AtomicU64::new(seconds),
            rollback_after_adoption: false,
            saw_grant_revoked_preflight: AtomicBool::new(false),
        })
    }

    pub(super) fn rollback_after_adoption() -> Arc<Self> {
        Arc::new(Self {
            seconds: AtomicU64::new(100),
            rollback_after_adoption: true,
            saw_grant_revoked_preflight: AtomicBool::new(false),
        })
    }

    pub(super) fn current_seconds(&self) -> u64 {
        self.seconds.load(Ordering::Acquire)
    }

    pub(super) fn record_preflight_refusal(&self, error: &PolicyOwnerError) {
        if matches!(error, PolicyOwnerError::GrantRevoked) {
            self.saw_grant_revoked_preflight
                .store(true, Ordering::Release);
        }
    }

    pub(super) fn after_adoption(&self) {
        if self.rollback_after_adoption {
            self.seconds.store(99, Ordering::Release);
        }
    }

    pub(super) fn observed_grant_revoked_preflight(&self) -> bool {
        self.saw_grant_revoked_preflight.load(Ordering::Acquire)
    }
}

impl PolicyClock for TestPolicyClock {
    fn now_seconds(&self) -> u64 {
        self.current_seconds()
    }

    fn now_timestamp(&self) -> String {
        utc_timestamp(self.now_seconds())
    }
}
