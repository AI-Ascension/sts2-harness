// SPDX-License-Identifier: MIT

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub(crate) struct ActionReadGate {
    entered: Mutex<bool>,
    entered_cv: Condvar,
    released: Mutex<bool>,
    released_cv: Condvar,
    settle: AtomicBool,
}
impl ActionReadGate {
    pub(crate) fn new() -> Self {
        Self {
            entered: Mutex::new(false),
            entered_cv: Condvar::new(),
            released: Mutex::new(false),
            released_cv: Condvar::new(),
            settle: AtomicBool::new(false),
        }
    }
    pub(crate) fn wait_entered(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut entered = self.entered.lock().unwrap_or_else(|e| e.into_inner());
        while !*entered {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let Ok((next, result)) = self.entered_cv.wait_timeout(entered, remaining) else {
                return false;
            };
            entered = next;
            if result.timed_out() && !*entered {
                return false;
            }
        }
        true
    }
    pub(crate) fn release(&self) {
        if let Ok(mut released) = self.released.lock() {
            *released = true;
            self.released_cv.notify_all();
        }
    }
    pub(crate) fn settle(&self) {
        self.settle.store(true, Ordering::Release)
    }
    pub(super) fn unsettled(&self) -> bool {
        !self.settle.load(Ordering::Acquire)
    }
    pub(super) fn block(&self) {
        if let Ok(mut entered) = self.entered.lock() {
            *entered = true;
            self.entered_cv.notify_all();
        }
        let mut released = self.released.lock().unwrap_or_else(|e| e.into_inner());
        while !*released {
            released = self
                .released_cv
                .wait(released)
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}
