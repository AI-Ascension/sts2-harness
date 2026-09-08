// SPDX-License-Identifier: MIT

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RpcFailureKind {
    TransientTransport,
    TerminalProtocol,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RpcFailure {
    kind: RpcFailureKind,
    message: String,
}

impl RpcFailure {
    fn transient(message: impl Into<String>) -> Self {
        Self {
            kind: RpcFailureKind::TransientTransport,
            message: message.into(),
        }
    }

    fn terminal(message: impl Into<String>) -> Self {
        Self {
            kind: RpcFailureKind::TerminalProtocol,
            message: message.into(),
        }
    }

    fn from_mcp(error: McpProcessError) -> Self {
        if error.is_transient_transport() {
            Self::transient(error.to_string())
        } else {
            Self::terminal(error.to_string())
        }
    }

    pub(super) const fn is_transient(&self) -> bool {
        matches!(self.kind, RpcFailureKind::TransientTransport)
    }
}

impl fmt::Display for RpcFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
