// SPDX-License-Identifier: MIT

use super::artifact::{
    CoopNativeAdmissionStatus, CoopNativeArtifactError, CoopNativeArtifactLineage,
    CoopNativeArtifactRecord, CoopNativeArtifactStatus, COOP_NATIVE_MAX_RECORDS,
};
use super::identity::{
    CoopNativeActionId, CoopNativeCorrelationId, CoopNativeInstanceId, CoopNativeLeaseId,
    CoopNativeLineage, CoopNativeOperationId,
};
use super::port::{CoopNativePort, CoopNativePortError};
use super::wire::{
    CoopNativeEnvelope, CoopNativeEnvelopeError, CoopNativeKind, CoopNativeRecoveryResponse,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoopNativeEventKind {
    Observation,
    LegalCatalogRequested,
    LegalCatalogObserved,
    LocalActionRequested,
    SharedVoteRequested,
    RejoinRequested,
    Accepted,
    Settled,
    Rejected,
    Unknown,
    ReconcileRequested,
    Reconciled,
    DuplicateReplay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoopNativeOperationState {
    Requested,
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Reconciled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CoopNativeReceipt {
    sequence: u16,
    event: CoopNativeEventKind,
    state: Option<CoopNativeOperationState>,
    operation_id: Option<CoopNativeOperationId>,
    artifact_status: CoopNativeArtifactStatus,
    admission: CoopNativeAdmissionStatus,
}

impl CoopNativeReceipt {
    #[must_use]
    pub const fn sequence(&self) -> u16 {
        self.sequence
    }

    #[must_use]
    pub const fn event(&self) -> CoopNativeEventKind {
        self.event
    }

    #[must_use]
    pub const fn state(&self) -> Option<CoopNativeOperationState> {
        self.state
    }

    #[must_use]
    pub fn operation_id(&self) -> Option<&CoopNativeOperationId> {
        self.operation_id.as_ref()
    }

    #[must_use]
    pub const fn artifact_status(&self) -> CoopNativeArtifactStatus {
        self.artifact_status
    }

    #[must_use]
    pub const fn admission(&self) -> CoopNativeAdmissionStatus {
        self.admission
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CoopNativeRecord {
    sequence: u16,
    event: CoopNativeEventKind,
    state: Option<CoopNativeOperationState>,
    correlation_id: CoopNativeCorrelationId,
    instance_id: CoopNativeInstanceId,
    session_id: super::identity::CoopNativeSessionId,
    lease_id: CoopNativeLeaseId,
    lease_epoch: u64,
    operation_id: Option<CoopNativeOperationId>,
    action_id: Option<CoopNativeActionId>,
    envelope_json: String,
    envelope_digest: String,
    lineage: CoopNativeLineage,
    artifact_status: CoopNativeArtifactStatus,
    admission: CoopNativeAdmissionStatus,
}

impl CoopNativeRecord {
    #[must_use]
    pub const fn sequence(&self) -> u16 {
        self.sequence
    }

    #[must_use]
    pub const fn event(&self) -> CoopNativeEventKind {
        self.event
    }

    #[must_use]
    pub const fn state(&self) -> Option<CoopNativeOperationState> {
        self.state
    }

    #[must_use]
    pub fn correlation_id(&self) -> &CoopNativeCorrelationId {
        &self.correlation_id
    }

    #[must_use]
    pub fn instance_id(&self) -> &CoopNativeInstanceId {
        &self.instance_id
    }

    #[must_use]
    pub fn session_id(&self) -> &super::identity::CoopNativeSessionId {
        &self.session_id
    }

    #[must_use]
    pub fn lease_id(&self) -> &CoopNativeLeaseId {
        &self.lease_id
    }

    #[must_use]
    pub const fn lease_epoch(&self) -> u64 {
        self.lease_epoch
    }

    #[must_use]
    pub fn operation_id(&self) -> Option<&CoopNativeOperationId> {
        self.operation_id.as_ref()
    }

    #[must_use]
    pub fn action_id(&self) -> Option<&CoopNativeActionId> {
        self.action_id.as_ref()
    }

    #[must_use]
    pub fn envelope_json(&self) -> &str {
        &self.envelope_json
    }

    #[must_use]
    pub fn envelope_digest(&self) -> &str {
        &self.envelope_digest
    }

    #[must_use]
    pub fn lineage(&self) -> &CoopNativeLineage {
        &self.lineage
    }

    #[must_use]
    pub const fn artifact_status(&self) -> CoopNativeArtifactStatus {
        self.artifact_status
    }

    #[must_use]
    pub const fn admission(&self) -> CoopNativeAdmissionStatus {
        self.admission
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoopNativeCoordinatorError {
    Artifact(CoopNativeArtifactError),
    Envelope(CoopNativeEnvelopeError),
    Port(CoopNativePortError),
    Unadmitted,
    TooManyRecords,
    MissingOperation,
    OperationConflict,
    ReconciliationMismatch,
    WrongReconciliation,
    Serialization,
    NoBlindRetry,
}

impl fmt::Display for CoopNativeCoordinatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Artifact(_) => "native co-op artifact verification failed",
            Self::Envelope(_) => "native co-op envelope validation failed",
            Self::Port(_) => "native co-op port consumption failed",
            Self::Unadmitted => "native co-op candidate is unadmitted",
            Self::TooManyRecords => "native co-op record bound was exceeded",
            Self::MissingOperation => "native co-op response has no recorded operation",
            Self::OperationConflict => "native co-op operation replay conflicts with its first input",
            Self::ReconciliationMismatch => "native co-op reconciliation does not match an unknown operation",
            Self::WrongReconciliation => "native co-op reconciliation requires a recovery response",
            Self::Serialization => "native co-op record serialization failed",
            Self::NoBlindRetry => "native co-op unknown mutation cannot be blindly retried",
        })
    }
}

impl std::error::Error for CoopNativeCoordinatorError {}

impl From<CoopNativeArtifactError> for CoopNativeCoordinatorError {
    fn from(error: CoopNativeArtifactError) -> Self {
        Self::Artifact(error)
    }
}

impl From<CoopNativeEnvelopeError> for CoopNativeCoordinatorError {
    fn from(error: CoopNativeEnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<CoopNativePortError> for CoopNativeCoordinatorError {
    fn from(error: CoopNativePortError) -> Self {
        Self::Port(error)
    }
}

trait CoopNativeStatusExt {
    fn event(self) -> CoopNativeEventKind;
    fn state(self) -> CoopNativeOperationState;
}

impl CoopNativeStatusExt for super::wire::CoopNativeStatus {
    fn event(self) -> CoopNativeEventKind {
        match self {
            Self::Accepted => CoopNativeEventKind::Accepted,
            Self::Settled => CoopNativeEventKind::Settled,
            Self::Rejected => CoopNativeEventKind::Rejected,
            Self::Unknown => CoopNativeEventKind::Unknown,
        }
    }

    fn state(self) -> CoopNativeOperationState {
        match self {
            Self::Accepted => CoopNativeOperationState::Accepted,
            Self::Settled => CoopNativeOperationState::Settled,
            Self::Rejected => CoopNativeOperationState::Rejected,
            Self::Unknown => CoopNativeOperationState::Unknown,
        }
    }
}

include!("coop_native_coordinator_impl.rs");
