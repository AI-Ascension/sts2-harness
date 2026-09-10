// SPDX-License-Identifier: MIT

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Monotonic, owner-local cancellation signal for an execution's asynchronous I/O.
///
/// Cancellation never proves non-execution, refunds usage, or grants new authority.
/// Clones share the signal; a cancelled signal cannot be reset for another attempt.
#[derive(Clone, Debug, Default)]
pub struct ExecutionCancellation(Arc<AtomicBool>);

impl ExecutionCancellation {
    /// Request cancellation of current and future exchanges using this signal.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Wait with bounded polling, without a blocking thread or an unbounded queue.
    /// Scheduling delays remain subject to the owning runtime and OS.
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
