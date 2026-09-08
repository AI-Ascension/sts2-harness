// SPDX-License-Identifier: MIT
//! Owned synthetic process-group shutdown and fallback cleanup.

use super::support::TestResult;
use std::process::Child;
use std::time::Duration;

pub(super) struct RuntimeChild {
    pub(super) child: Child,
    pub(super) reaped: bool,
}

impl RuntimeChild {
    pub(super) async fn finish(&mut self, success: bool) -> TestResult {
        rustix::process::kill_process(
            rustix::process::Pid::from_child(&self.child),
            rustix::process::Signal::TERM,
        )?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait()? {
                self.reaped = true;
                assert_eq!(
                    status.success(),
                    success,
                    "unexpected worker shutdown: {status}"
                );
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("worker did not shut down within five seconds".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub(super) fn cleanup(&mut self) {
        if !self.reaped {
            // The unreaped direct child pins this test-owned process-group ID.
            let _ = rustix::process::kill_process_group(
                rustix::process::Pid::from_child(&self.child),
                rustix::process::Signal::KILL,
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while std::time::Instant::now() < deadline {
                if matches!(self.child.try_wait(), Ok(Some(_))) {
                    self.reaped = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

impl Drop for RuntimeChild {
    fn drop(&mut self) {
        self.cleanup();
    }
}
