// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum McpProcessErrorKind {
    Deadline,
    RequestWrite,
    RequestFlush,
    ResponseRead,
    ResponseEof,
    ProcessClosed,
    StdinClosed,
    StdoutClosed,
    SupervisorClosed,
    SupervisorUnavailable,
    SupervisorFailed,
    Protocol,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct McpProcessError {
    kind: McpProcessErrorKind,
    message: String,
}

impl McpProcessError {
    pub(super) fn new(kind: McpProcessErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(super) const fn is_transient_transport(&self) -> bool {
        matches!(
            self.kind,
            McpProcessErrorKind::Deadline
                | McpProcessErrorKind::RequestWrite
                | McpProcessErrorKind::RequestFlush
                | McpProcessErrorKind::ResponseRead
                | McpProcessErrorKind::ResponseEof
                | McpProcessErrorKind::ProcessClosed
                | McpProcessErrorKind::StdinClosed
                | McpProcessErrorKind::StdoutClosed
                | McpProcessErrorKind::SupervisorClosed
                | McpProcessErrorKind::SupervisorUnavailable
                | McpProcessErrorKind::SupervisorFailed
        )
    }

    pub(super) fn with_cleanup(mut self, cleanup: String) -> Self {
        self.message.push_str("; ");
        self.message.push_str(&cleanup);
        self
    }
}

impl std::fmt::Display for McpProcessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for McpProcessError {}
