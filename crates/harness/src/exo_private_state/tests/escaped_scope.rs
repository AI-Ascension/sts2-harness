// SPDX-License-Identifier: MIT

use super::escaped_fixture::report_cleanup_failure;
use super::{BridgeChildScope, io_error};
use std::io;

/// Isolated supervisor helper for a target process that exits without running Rust Drop.
pub(super) struct ScopeDrainGuard {
    scope: BridgeChildScope,
    attempted: bool,
}

impl ScopeDrainGuard {
    pub(super) fn new(scope: BridgeChildScope) -> Self {
        Self {
            scope,
            attempted: false,
        }
    }

    pub(super) fn drain(&mut self) -> io::Result<bool> {
        if self.attempted {
            return Err(io::Error::other("supervisor drain was already attempted"));
        }
        self.attempted = true;
        self.scope.drain().map_err(io_error)
    }
}

impl Drop for ScopeDrainGuard {
    fn drop(&mut self) {
        if !self.attempted {
            self.attempted = true;
            if let Err(error) = self.scope.drain() {
                report_cleanup_failure("isolated supervisor child drain refused", error);
            }
        }
    }
}
