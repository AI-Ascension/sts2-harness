// SPDX-License-Identifier: MIT

use std::collections::VecDeque;

use crate::decision_records::{DecisionRecord, DecisionRecordKind};
use crate::identity::RecordId;

const MAX_CAPACITY: usize = 65_536;

/// Outcome of appending one typed decision record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryAppend {
    Inserted,
    Duplicate,
}

/// Bounded in-memory retention for sanitized decision evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionMemory {
    capacity: usize,
    records: VecDeque<DecisionRecord>,
}

impl DecisionMemory {
    pub fn new(capacity: usize) -> Result<Self, MemoryError> {
        if capacity == 0 || capacity > MAX_CAPACITY {
            return Err(MemoryError::InvalidCapacity);
        }
        Ok(Self {
            capacity,
            records: VecDeque::with_capacity(capacity.min(1024)),
        })
    }

    pub fn append(&mut self, record: DecisionRecord) -> Result<MemoryAppend, MemoryError> {
        if self
            .records
            .iter()
            .find(|existing| existing.record_id() == record.record_id())
            .is_some_and(|existing| existing != &record)
        {
            return Err(MemoryError::RecordConflict);
        }
        if self
            .records
            .iter()
            .any(|existing| existing.record_id() == record.record_id())
        {
            return Ok(MemoryAppend::Duplicate);
        }
        if self.records.len() >= self.capacity {
            return Err(MemoryError::Full);
        }
        self.records.push_back(record);
        Ok(MemoryAppend::Inserted)
    }

    #[must_use]
    pub fn records(&self) -> &VecDeque<DecisionRecord> { &self.records }

    #[must_use]
    pub fn len(&self) -> usize { self.records.len() }

    #[must_use]
    pub fn is_empty(&self) -> bool { self.records.is_empty() }

    #[must_use]
    pub fn by_id(&self, record_id: RecordId) -> Option<&DecisionRecord> {
        self.records.iter().find(|record| record.record_id() == record_id)
    }

    #[must_use]
    pub fn count_kind(&self, kind: DecisionRecordKind) -> usize {
        self.records.iter().filter(|record| record.kind() == kind).count()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryError {
    InvalidCapacity,
    Full,
    RecordConflict,
}

impl std::fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidCapacity => "decision memory capacity is invalid",
            Self::Full => "decision memory is full",
            Self::RecordConflict => "decision record identity conflicts with retained evidence",
        })
    }
}

impl std::error::Error for MemoryError {}
