// SPDX-License-Identifier: MIT

//! Harness-owned decoder for the frozen watchdog handoff contract.
//! Valid framing does not authenticate a caller or authorize execution.

#[path = "worker_command.rs"]
mod command;
pub(crate) mod json;
mod request;
mod response;
mod terminal;

pub use command::{
    ApprovedWorkerExecution, AuthenticatedWorkerRequest, WorkerCapability, WorkerCommandAdmission,
    WorkerCommandConfig, WorkerCommandError, WorkerCommandResult, WorkerDispatchPreparation,
    WorkerExecutionReservation,
};
pub use request::{WorkerCommand, WorkerRequest};
pub use response::{AcknowledgmentStatus, DispatchReply, LookupReply, ProbeReply, WorkerReply};
pub use terminal::{TerminalCompletion, TerminalRecord, TerminalStatus};

/// Exact artifact identity; mixed schemas fail closed.
pub const SCHEMA_DIGEST: &str = "bb13d15f6c0e4b8d0f58f7391fe4ba319ebc57a0a09effc06d73ea718bbff4cf";
/// Empty approved episode parameter object's SHA-256.
pub const PAYLOAD_DIGEST: &str = "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";
/// Maximum encoded frame size, before allocating a transport body.
pub const MAX_FRAME_BYTES: usize = 65_536;
const MAX_INTEGER: u64 = 9_007_199_254_740_991;

/// Deliberately redacted: malformed frames must not echo private input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandoffError;

impl std::fmt::Display for HandoffError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("invalid worker handoff frame")
    }
}

impl std::error::Error for HandoffError {}
