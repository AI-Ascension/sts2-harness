// SPDX-License-Identifier: MIT

use super::{
    CaptureBoundary, CaptureComponentKind, CaptureError, CaptureInput, CaptureMode, CapturePort,
    CaptureRecord, MAX_CAPTURE_BYTES, MAX_CAPTURE_RECORDS, TransportState, lifecycle_id,
    snapshot_id, valid_identity,
};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;

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
        if mode == CaptureMode::Private {
            return Err(CaptureError::PrivateRequiresVault);
        }
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
        let snapshot_id = snapshot_id(input.execution_id, input.attempt_id, input.boundary);
        let content = (self.mode == CaptureMode::Memory).then(|| input.bytes.to_vec());
        let sha256 =
            (self.mode != CaptureMode::Metadata).then(|| Sha256::digest(input.bytes).into());
        self.push(CaptureRecord {
            snapshot_id,
            parent_snapshot_id: None,
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
        let snapshot_id = snapshot_id(input.execution_id, input.attempt_id, input.boundary);
        let content = (self.mode == CaptureMode::Memory).then(|| input.bytes.to_vec());
        let sha256 =
            (self.mode != CaptureMode::Metadata).then(|| Sha256::digest(input.bytes).into());
        self.push(CaptureRecord {
            snapshot_id,
            parent_snapshot_id: None,
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
        self.write_completed_at(execution_id, attempt_id, CaptureBoundary::HttpBody)
    }

    fn write_completed_at(
        &mut self,
        execution_id: &str,
        attempt_id: Option<&str>,
        boundary: CaptureBoundary,
    ) -> Result<(), CaptureError> {
        if !valid_identity(execution_id)
            || attempt_id.is_some_and(|attempt_id| !valid_identity(attempt_id))
        {
            return Err(CaptureError::InvalidIdentity);
        }
        let parent_snapshot_id = snapshot_id(execution_id, attempt_id, boundary);
        self.push(CaptureRecord {
            // Lifecycle records have their own immutable event identity and point back to the
            // prepared snapshot through parent_snapshot_id.
            snapshot_id: lifecycle_id(
                execution_id,
                attempt_id,
                boundary,
                TransportState::WriteCompleted,
                None,
            ),
            parent_snapshot_id: Some(parent_snapshot_id),
            execution_id: execution_id.to_owned(),
            attempt_id: attempt_id.map(str::to_owned),
            boundary,
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

    fn write_failed(&mut self, execution_id: &str, code: &str) -> Result<(), CaptureError> {
        self.write_failed_with_attempt(execution_id, None, code)
    }

    fn write_failed_with_attempt(
        &mut self,
        execution_id: &str,
        attempt_id: Option<&str>,
        code: &str,
    ) -> Result<(), CaptureError> {
        if !valid_identity(execution_id)
            || attempt_id.is_some_and(|attempt_id| !valid_identity(attempt_id))
        {
            return Err(CaptureError::InvalidIdentity);
        }
        self.write_unknown(execution_id, attempt_id, code, CaptureBoundary::HttpBody)
    }

    fn write_unknown(
        &mut self,
        execution_id: &str,
        attempt_id: Option<&str>,
        code: &str,
        boundary: CaptureBoundary,
    ) -> Result<(), CaptureError> {
        if !valid_identity(execution_id)
            || attempt_id.is_some_and(|attempt_id| !valid_identity(attempt_id))
        {
            return Err(CaptureError::InvalidIdentity);
        }
        let parent_snapshot_id = snapshot_id(execution_id, attempt_id, boundary);
        self.push(CaptureRecord {
            // Keep a separate lifecycle identity so repeated state transitions cannot masquerade
            // as the prepared snapshot itself.
            snapshot_id: lifecycle_id(
                execution_id,
                attempt_id,
                boundary,
                TransportState::Unknown,
                Some(code),
            ),
            parent_snapshot_id: Some(parent_snapshot_id),
            execution_id: execution_id.to_owned(),
            attempt_id: attempt_id.map(str::to_owned),
            boundary,
            state: TransportState::Unknown,
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
