// SPDX-License-Identifier: MIT

use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionStoreError {
    InvalidConfiguration,
    InvalidIdentity,
    InvalidFingerprint,
    InvalidCheckpoint,
    InvalidOperation,
    InvalidDecision,
    InvalidProviderReservation,
    InvalidCompletion,
    InvalidJob,
    InvalidWorkflowIdentity,
    InvalidWorkflowRecord,
    InvalidWorkflowEvent,
    InvalidWorkflowProjection,
    Conflict,
    RevisionConflict,
    Missing,
    Busy,
    Corrupt,
    Incompatible,
    Capacity,
    StorageFull,
    Persistence(String),
}

impl fmt::Display for ExecutionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidConfiguration => "execution store configuration is invalid",
            Self::InvalidIdentity => "execution identity is invalid",
            Self::InvalidFingerprint => "execution fingerprint is invalid",
            Self::InvalidCheckpoint => "checkpoint is invalid",
            Self::InvalidOperation => "operation intent is invalid",
            Self::InvalidDecision => "decision reference is invalid",
            Self::InvalidProviderReservation => "provider reservation is invalid",
            Self::InvalidCompletion => "completion record is invalid",
            Self::InvalidJob => "job identity or digest is invalid",
            Self::InvalidWorkflowIdentity => "workflow identity is invalid",
            Self::InvalidWorkflowRecord => "workflow record is invalid",
            Self::InvalidWorkflowEvent => "workflow event is invalid",
            Self::InvalidWorkflowProjection => "workflow projection is invalid",
            Self::Conflict => "durable execution record conflicts with an existing record",
            Self::RevisionConflict => "workflow run revision does not match the expected revision",
            Self::Missing => "durable execution record is missing",
            Self::Busy => "execution store is busy",
            Self::Corrupt => "execution store integrity check failed",
            Self::Incompatible => "execution store or release is incompatible",
            Self::Capacity => "execution store capacity is exhausted",
            Self::StorageFull => "execution store reported a full filesystem or database",
            Self::Persistence(message) => message,
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ExecutionStoreError {}
