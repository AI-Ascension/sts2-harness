// SPDX-License-Identifier: MIT

use super::PolicyClock;
use super::config::utc_timestamp;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use super::test_clock::TestPolicyClock;
#[cfg(test)]
use std::sync::Arc;

pub(super) struct RuntimePolicyClock;

impl PolicyClock for RuntimePolicyClock {
    fn now_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(u64::MAX, |duration| duration.as_secs())
    }

    fn now_timestamp(&self) -> String {
        utc_timestamp(self.now_seconds())
    }
}

pub(in crate::runtime_support) enum PolicyClockSource {
    System,
    #[cfg(test)]
    Test(Arc<TestPolicyClock>),
}
