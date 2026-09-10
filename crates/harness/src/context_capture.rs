// SPDX-License-Identifier: MIT

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

//! Harness-owned, fail-soft capture primitives.  The port receives the already encoded bytes at
//! a named application boundary; it never edits provider payloads or owns provider/game access.

#[path = "context_capture_input.rs"]
mod input;
pub use input::{PreparedAstraInput, PreparedOllamaInput};

pub const MAX_CAPTURE_RECORDS: usize = 128;
pub const MAX_CAPTURE_BYTES: usize = 1_048_576;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureMode {
    Off,
    Metadata,
    Memory,
    Private,
}

impl CaptureMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Metadata => "metadata",
            Self::Memory => "memory",
            Self::Private => "private",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureBoundary {
    HarnessRequest,
    ExoSessionRequest,
    ProviderRequest,
}

/// The allowlisted kinds that may be attached to a prepared bridge input.  These labels describe
/// application-controlled bytes only; they never imply that a provider received the bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureComponentKind {
    Stdin,
    OutputSchema,
    Configuration,
    SystemMessage,
    UserMessage,
    Attachment,
    Opaque,
}

impl CaptureComponentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stdin => "stdin",
            Self::OutputSchema => "output_schema",
            Self::Configuration => "configuration",
            Self::SystemMessage => "system_message",
            Self::UserMessage => "user_message",
            Self::Attachment => "attachment",
            Self::Opaque => "opaque",
        }
    }
}

impl CaptureBoundary {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HarnessRequest => "harness.request",
            Self::ExoSessionRequest => "adapter.cli_input",
            Self::ProviderRequest => "provider.request",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportState {
    Prepared,
    WriteCompleted,
    WriteFailed,
    ReceiptReported,
    Unknown,
}

impl TransportState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::WriteCompleted => "input.write_completed",
            Self::WriteFailed => "input.write_failed",
            Self::ReceiptReported => "provider.receipt_reported",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureInput<'a> {
    pub execution_id: &'a str,
    pub attempt_id: Option<&'a str>,
    pub boundary: CaptureBoundary,
    pub bytes: &'a [u8],
}

/// One exact byte component at a provider boundary.  A bridge creates these from the same bytes
/// that it dispatches, so a sideband sink cannot accidentally become a second serializer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureComponent<'a> {
    pub kind: CaptureComponentKind,
    pub ordinal: u16,
    pub media_type: &'a str,
    pub bytes: &'a [u8],
}

/// A complete prepared input, used by the real Astra and Ollama bridge paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreparedInput<'a> {
    pub execution_id: &'a str,
    pub attempt_id: Option<&'a str>,
    pub boundary: CaptureBoundary,
    pub components: &'a [CaptureComponent<'a>],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureRecord {
    pub snapshot_id: String,
    pub execution_id: String,
    pub attempt_id: Option<String>,
    pub boundary: CaptureBoundary,
    pub state: TransportState,
    pub observed_bytes: usize,
    pub sha256: Option<[u8; 32]>,
    pub content: Option<Vec<u8>>,
    pub component_kind: Option<CaptureComponentKind>,
    pub ordinal: Option<u16>,
    pub media_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureError {
    Disabled,
    InvalidConfiguration,
    TooLarge,
    InvalidIdentity,
    QueueFull,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Disabled => "capture is disabled",
            Self::InvalidConfiguration => "capture configuration is invalid",
            Self::TooLarge => "captured bytes exceed their bound",
            Self::InvalidIdentity => "capture identity is invalid",
            Self::QueueFull => "capture queue is full",
        })
    }
}

impl std::error::Error for CaptureError {}

pub trait CapturePort: std::fmt::Debug + Send {
    fn enabled(&self) -> bool {
        false
    }

    fn prepared(&mut self, input: CaptureInput<'_>) -> Result<(), CaptureError>;

    /// Records one component while preserving compatibility with the original single-byte port.
    /// Implementations that retain structured manifests override this method.
    fn prepared_component(
        &mut self,
        input: CaptureInput<'_>,
        _kind: CaptureComponentKind,
        _ordinal: u16,
        _media_type: &str,
    ) -> Result<(), CaptureError> {
        self.prepared(input)
    }

    /// Records all components of one prepared input.  A sink failure is intentionally local to
    /// the sideband; bridge dispatch code may continue with the original bytes.
    fn prepared_input(&mut self, input: PreparedInput<'_>) -> Result<(), CaptureError> {
        for component in input.components {
            self.prepared_component(
                CaptureInput {
                    execution_id: input.execution_id,
                    attempt_id: input.attempt_id,
                    boundary: input.boundary,
                    bytes: component.bytes,
                },
                component.kind,
                component.ordinal,
                component.media_type,
            )?;
        }
        Ok(())
    }

    fn write_completed(&mut self, execution_id: &str) -> Result<(), CaptureError>;
    fn write_failed(&mut self, execution_id: &str, code: &str) -> Result<(), CaptureError>;

    fn write_completed_with_attempt(
        &mut self,
        execution_id: &str,
        attempt_id: Option<&str>,
    ) -> Result<(), CaptureError> {
        let _ = attempt_id;
        self.write_completed(execution_id)
    }

    fn write_failed_with_attempt(
        &mut self,
        execution_id: &str,
        attempt_id: Option<&str>,
        code: &str,
    ) -> Result<(), CaptureError> {
        let _ = attempt_id;
        self.write_failed(execution_id, code)
    }
}

#[derive(Debug, Default)]
pub struct NoopCapture;

impl CapturePort for NoopCapture {
    fn prepared(&mut self, _input: CaptureInput<'_>) -> Result<(), CaptureError> {
        Ok(())
    }

    fn write_completed(&mut self, _execution_id: &str) -> Result<(), CaptureError> {
        Ok(())
    }

    fn write_failed(&mut self, _execution_id: &str, _code: &str) -> Result<(), CaptureError> {
        Ok(())
    }
}

#[path = "context_capture_memory.rs"]
mod memory;
pub use memory::MemoryCapture;

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().enumerate().all(|(index, character)| {
            character.is_ascii_alphanumeric() || (index > 0 && "._:-".contains(character))
        })
}

fn snapshot_id(execution_id: &str, attempt_id: Option<&str>) -> String {
    match attempt_id {
        Some(attempt_id) => format!("snapshot-{execution_id}-{attempt_id}"),
        None => format!("snapshot-{execution_id}"),
    }
}

#[cfg(test)]
#[path = "context_capture_tests.rs"]
mod tests;
