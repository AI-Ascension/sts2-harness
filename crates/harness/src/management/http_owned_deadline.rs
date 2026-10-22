// SPDX-License-Identifier: MIT

//! One absolute Linux monotonic deadline survives every private IPC hop.

use std::time::Duration;

use rustix::time::{ClockId, clock_gettime};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Deadline(u64);

impl Deadline {
    pub(super) fn after(budget: Duration) -> Option<Self> {
        let nanos = u64::try_from(budget.as_nanos()).ok()?;
        now()?.checked_add(nanos).map(Self)
    }

    pub(super) fn from_wire(nanos: u64, maximum: Duration) -> Option<Self> {
        let current = now()?;
        let maximum = u64::try_from(maximum.as_nanos()).ok()?;
        if nanos == 0 || nanos.saturating_sub(current) > maximum {
            return None;
        }
        Some(Self(nanos))
    }

    pub(super) fn wire(self) -> u64 {
        self.0
    }

    pub(super) fn remaining(self) -> Duration {
        now()
            .map(|current| Duration::from_nanos(self.0.saturating_sub(current)))
            .unwrap_or(Duration::ZERO)
    }

    pub(super) fn expired(self) -> bool {
        self.remaining().is_zero()
    }
}

fn now() -> Option<u64> {
    let time = clock_gettime(ClockId::Monotonic);
    let seconds = u64::try_from(time.tv_sec).ok()?;
    let nanos = u64::try_from(time.tv_nsec).ok()?;
    if nanos >= 1_000_000_000 {
        return None;
    }
    seconds.checked_mul(1_000_000_000)?.checked_add(nanos)
}
