// SPDX-License-Identifier: MIT

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

//! Harness-owned, fail-soft capture primitives.  The port receives the already encoded bytes at
//! a named application boundary; it never edits provider payloads or owns provider/game access.

use sha2::{Digest, Sha256};
use std::collections::VecDeque;

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

/// Final application-controlled bytes assembled by the Astra bridge.  The bridge passes
/// `stdin` and `output_schema` to the child exactly as provided here; `configuration` is a
/// bounded description of the argv/cwd settings the child sees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreparedAstraInput<'a> {
    pub stdin: &'a [u8],
    pub output_schema: &'a [u8],
    pub configuration: &'a [u8],
}

impl<'a> PreparedAstraInput<'a> {
    pub const fn new(stdin: &'a [u8], output_schema: &'a [u8], configuration: &'a [u8]) -> Self {
        Self {
            stdin,
            output_schema,
            configuration,
        }
    }

    pub fn capture(
        self,
        capture: &mut dyn CapturePort,
        execution_id: &'a str,
        attempt_id: Option<&'a str>,
    ) {
        if !capture.enabled() {
            return;
        }
        let components = [
            CaptureComponent {
                kind: CaptureComponentKind::Stdin,
                ordinal: 0,
                media_type: "text/plain; charset=utf-8",
                bytes: self.stdin,
            },
            CaptureComponent {
                kind: CaptureComponentKind::OutputSchema,
                ordinal: 1,
                media_type: "application/schema+json",
                bytes: self.output_schema,
            },
            CaptureComponent {
                kind: CaptureComponentKind::Configuration,
                ordinal: 2,
                media_type: "application/json",
                bytes: self.configuration,
            },
        ];
        let _ = capture.prepared_input(PreparedInput {
            execution_id,
            attempt_id,
            boundary: CaptureBoundary::ProviderRequest,
            components: &components,
        });
    }
}

/// Final serialized Ollama body.  Capturing this one opaque component preserves exact JSON byte
/// ordering while the bridge keeps its existing request construction and response parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreparedOllamaInput<'a> {
    pub body: &'a [u8],
}

impl<'a> PreparedOllamaInput<'a> {
    pub const fn new(body: &'a [u8]) -> Self {
        Self { body }
    }

    pub fn capture(
        self,
        capture: &mut dyn CapturePort,
        execution_id: &'a str,
        attempt_id: Option<&'a str>,
    ) {
        if !capture.enabled() {
            return;
        }
        let component = CaptureComponent {
            kind: CaptureComponentKind::Opaque,
            ordinal: 0,
            media_type: "application/json",
            bytes: self.body,
        };
        let _ = capture.prepared_input(PreparedInput {
            execution_id,
            attempt_id,
            boundary: CaptureBoundary::ProviderRequest,
            components: std::slice::from_ref(&component),
        });
    }
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

#[derive(Debug)]
pub struct MemoryCapture {
    mode: CaptureMode,
    max_records: usize,
    max_content_bytes: usize,
    records: VecDeque<CaptureRecord>,
    dropped_entries: u64,
}

impl MemoryCapture {
    pub fn new(
        mode: CaptureMode,
        max_records: usize,
        max_content_bytes: usize,
    ) -> Result<Self, CaptureError> {
        if mode == CaptureMode::Off
            || max_records == 0
            || max_records > MAX_CAPTURE_RECORDS
            || max_content_bytes > MAX_CAPTURE_BYTES
        {
            return Err(CaptureError::InvalidConfiguration);
        }
        Ok(Self {
            mode,
            max_records,
            max_content_bytes,
            records: VecDeque::new(),
            dropped_entries: 0,
        })
    }

    pub fn records(&self) -> impl Iterator<Item = &CaptureRecord> {
        self.records.iter()
    }

    pub const fn dropped_entries(&self) -> u64 {
        self.dropped_entries
    }

    fn push(&mut self, record: CaptureRecord) {
        if self.records.len() >= self.max_records {
            self.records.pop_front();
            self.dropped_entries = self.dropped_entries.saturating_add(1);
        }
        self.records.push_back(record);
    }
}

impl CapturePort for MemoryCapture {
    fn enabled(&self) -> bool {
        true
    }

    fn prepared(&mut self, input: CaptureInput<'_>) -> Result<(), CaptureError> {
        if input.bytes.len() > MAX_CAPTURE_BYTES || input.bytes.len() > self.max_content_bytes {
            return Err(CaptureError::TooLarge);
        }
        if !valid_identity(input.execution_id)
            || input
                .attempt_id
                .is_some_and(|attempt_id| !valid_identity(attempt_id))
        {
            return Err(CaptureError::InvalidIdentity);
        }
        let snapshot_id = snapshot_id(input.execution_id, input.attempt_id);
        let content = (self.mode == CaptureMode::Memory).then(|| input.bytes.to_vec());
        let sha256 =
            (self.mode != CaptureMode::Metadata).then(|| Sha256::digest(input.bytes).into());
        self.push(CaptureRecord {
            snapshot_id,
            execution_id: input.execution_id.to_owned(),
            attempt_id: input.attempt_id.map(str::to_owned),
            boundary: input.boundary,
            state: TransportState::Prepared,
            observed_bytes: input.bytes.len(),
            sha256,
            content,
            component_kind: None,
            ordinal: None,
            media_type: None,
        });
        Ok(())
    }

    fn prepared_component(
        &mut self,
        input: CaptureInput<'_>,
        kind: CaptureComponentKind,
        ordinal: u16,
        media_type: &str,
    ) -> Result<(), CaptureError> {
        if input.bytes.len() > MAX_CAPTURE_BYTES || input.bytes.len() > self.max_content_bytes {
            return Err(CaptureError::TooLarge);
        }
        if !valid_identity(input.execution_id)
            || input
                .attempt_id
                .is_some_and(|attempt_id| !valid_identity(attempt_id))
        {
            return Err(CaptureError::InvalidIdentity);
        }
        let snapshot_id = snapshot_id(input.execution_id, input.attempt_id);
        let content = (self.mode == CaptureMode::Memory).then(|| input.bytes.to_vec());
        let sha256 =
            (self.mode != CaptureMode::Metadata).then(|| Sha256::digest(input.bytes).into());
        self.push(CaptureRecord {
            snapshot_id,
            execution_id: input.execution_id.to_owned(),
            attempt_id: input.attempt_id.map(str::to_owned),
            boundary: input.boundary,
            state: TransportState::Prepared,
            observed_bytes: input.bytes.len(),
            sha256,
            content,
            component_kind: Some(kind),
            ordinal: Some(ordinal),
            media_type: Some(media_type.to_owned()),
        });
        Ok(())
    }

    fn write_completed(&mut self, execution_id: &str) -> Result<(), CaptureError> {
        self.write_completed_with_attempt(execution_id, None)
    }

    fn write_completed_with_attempt(
        &mut self,
        execution_id: &str,
        attempt_id: Option<&str>,
    ) -> Result<(), CaptureError> {
        if !valid_identity(execution_id)
            || attempt_id.is_some_and(|attempt_id| !valid_identity(attempt_id))
        {
            return Err(CaptureError::InvalidIdentity);
        }
        self.push(CaptureRecord {
            snapshot_id: snapshot_id(execution_id, attempt_id),
            execution_id: execution_id.to_owned(),
            attempt_id: attempt_id.map(str::to_owned),
            boundary: CaptureBoundary::ProviderRequest,
            state: TransportState::WriteCompleted,
            observed_bytes: 0,
            sha256: None,
            content: None,
            component_kind: None,
            ordinal: None,
            media_type: None,
        });
        Ok(())
    }

    fn write_failed(&mut self, execution_id: &str, _code: &str) -> Result<(), CaptureError> {
        self.write_failed_with_attempt(execution_id, None, _code)
    }

    fn write_failed_with_attempt(
        &mut self,
        execution_id: &str,
        attempt_id: Option<&str>,
        _code: &str,
    ) -> Result<(), CaptureError> {
        if !valid_identity(execution_id)
            || attempt_id.is_some_and(|attempt_id| !valid_identity(attempt_id))
        {
            return Err(CaptureError::InvalidIdentity);
        }
        self.push(CaptureRecord {
            snapshot_id: snapshot_id(execution_id, attempt_id),
            execution_id: execution_id.to_owned(),
            attempt_id: attempt_id.map(str::to_owned),
            boundary: CaptureBoundary::ProviderRequest,
            state: TransportState::WriteFailed,
            observed_bytes: 0,
            sha256: None,
            content: None,
            component_kind: None,
            ordinal: None,
            media_type: None,
        });
        Ok(())
    }
}

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
mod tests {
    use super::*;

    #[test]
    fn metadata_mode_does_not_retain_content() {
        let mut capture = MemoryCapture::new(CaptureMode::Metadata, 4, 128).expect("config");
        capture
            .prepared(CaptureInput {
                execution_id: "model-execution-7",
                attempt_id: Some("attempt-1"),
                boundary: CaptureBoundary::HarnessRequest,
                bytes: b"synthetic content",
            })
            .expect("record");
        let record = capture.records().next().expect("record");
        assert!(record.content.is_none());
        assert!(record.sha256.is_none());
    }

    #[test]
    fn queue_overflow_is_a_gap_and_states_do_not_fabricate_receipt() {
        let mut capture = MemoryCapture::new(CaptureMode::Memory, 1, 128).expect("config");
        for _ in 0..2 {
            capture
                .prepared(CaptureInput {
                    execution_id: "model-execution-7",
                    attempt_id: None,
                    boundary: CaptureBoundary::ExoSessionRequest,
                    bytes: b"input",
                })
                .expect("record");
        }
        assert_eq!(capture.dropped_entries(), 1);
        assert_ne!(
            TransportState::WriteCompleted,
            TransportState::ReceiptReported
        );
    }

    #[test]
    fn repeated_input_with_distinct_attempts_keeps_distinct_snapshot_identity() {
        let mut capture = MemoryCapture::new(CaptureMode::Metadata, 8, 128).expect("config");
        for attempt_id in ["attempt-a", "attempt-b"] {
            capture
                .prepared(CaptureInput {
                    execution_id: "model-execution-7",
                    attempt_id: Some(attempt_id),
                    boundary: CaptureBoundary::ProviderRequest,
                    bytes: b"same input",
                })
                .expect("record");
        }
        let records = capture.records().collect::<Vec<_>>();
        assert_ne!(records[0].snapshot_id, records[1].snapshot_id);
        assert_ne!(records[0].attempt_id, records[1].attempt_id);
    }
}
