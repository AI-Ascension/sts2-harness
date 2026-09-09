// SPDX-License-Identifier: MIT

//! Test-only serialization for the process-wide verifier singleton.

use std::sync::{Condvar, Mutex};

static TEST_CONTROLLER_SERIAL: (Mutex<Option<std::thread::ThreadId>>, Condvar) =
    (Mutex::new(None), Condvar::new());

pub(super) struct TestControllerSerialGuard {
    owner: Option<std::thread::ThreadId>,
}

impl TestControllerSerialGuard {
    pub(super) fn acquire() -> Self {
        let thread = std::thread::current().id();
        let (owners, wake) = &TEST_CONTROLLER_SERIAL;
        let mut owner = owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if owner.as_ref() == Some(&thread) {
            // A same-thread constructor is allowed to reach the lifecycle
            // registry and receive its ordinary Occupied error. This avoids
            // deadlocking a test that probes the singleton while holding it.
            return Self { owner: None };
        }
        while owner.is_some() {
            owner = wake
                .wait(owner)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *owner = Some(thread);
        Self {
            owner: Some(thread),
        }
    }
}

impl Drop for TestControllerSerialGuard {
    fn drop(&mut self) {
        let Some(thread) = self.owner.take() else {
            return;
        };
        let (owners, wake) = &TEST_CONTROLLER_SERIAL;
        let mut owner = owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let reap_error = if owner.as_ref() == Some(&thread) {
            // Keep the serial owner while the exact retained child is reaped;
            // otherwise the next constructor could race the barrier and
            // observe the same uncertain registry owner.
            let reap_error = super::super::lifecycle::reap_poisoned_session_for_test().err();
            *owner = None;
            wake.notify_one();
            reap_error
        } else {
            None
        };
        drop(owner);
        assert!(
            reap_error.is_none(),
            "Linux verifier test session reap failed: {reap_error:?}"
        );
    }
}
