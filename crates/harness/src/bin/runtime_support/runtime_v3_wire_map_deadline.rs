// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

const PROFILE_BUDGET: Duration = Duration::from_secs(10);
const CLOSE_RESERVE: Duration = Duration::from_millis(1_250);

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct MapProfileDeadline {
    deadline: Instant,
}

impl MapProfileDeadline {
    pub(in super::super) fn start() -> Option<Self> {
        Self::from_start(Instant::now())
    }

    pub(in super::super) fn from_start(start: Instant) -> Option<Self> {
        start
            .checked_add(PROFILE_BUDGET)
            .map(|deadline| Self { deadline })
    }

    pub(in super::super) const fn absolute(self) -> Instant {
        self.deadline
    }

    pub(in super::super) fn remaining(&self) -> Duration {
        self.remaining_at(Instant::now())
    }

    pub(in super::super) fn remaining_at(&self, now: Instant) -> Duration {
        self.deadline.saturating_duration_since(now)
    }

    pub(in super::super) fn rpc_timeout_at(
        &self,
        now: Instant,
        request_timeout: Duration,
    ) -> Option<Duration> {
        if request_timeout.is_zero() {
            return None;
        }
        let rpc_budget = self.remaining_at(now).checked_sub(CLOSE_RESERVE)?;
        let timeout = request_timeout.min(rpc_budget);
        (!timeout.is_zero()).then_some(timeout)
    }

    pub(in super::super) fn with_rpc_timeout<T>(
        &self,
        request_timeout: Duration,
        invoke: impl FnOnce(Duration) -> T,
    ) -> Option<T> {
        self.with_rpc_timeout_at(Instant::now(), request_timeout, invoke)
    }

    pub(in super::super) fn with_rpc_timeout_at<T>(
        &self,
        now: Instant,
        request_timeout: Duration,
        invoke: impl FnOnce(Duration) -> T,
    ) -> Option<T> {
        self.rpc_timeout_at(now, request_timeout).map(invoke)
    }
}
