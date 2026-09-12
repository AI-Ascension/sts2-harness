// SPDX-License-Identifier: MIT

//! Restore-before-model gate for exact checkpoints.
//!
//! A policy decision is admitted only after a destination actually recaptured the expected exact
//! state under a compatible profile. Integrity checking alone is not a restore. Each admitted
//! restore starts a new execution epoch, so decisions computed against an earlier epoch stay
//! rejected even when the exact state digest is identical.

use std::fmt;

use crate::execution::{BlobDigest, ExactCheckpointId, ExactCheckpointReference, ExactStateDigest};

/// Evidence a restore receipt actually established.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreEvidence {
    /// Artifacts and hashes verified, but nothing was restored.
    IntegrityVerified,
    /// A destination was restored and recaptured the expected exact state.
    RestoreVerified,
    /// Controlled continuations matched after a verified restore.
    ContinuationCertified,
}

impl RestoreEvidence {
    /// Returns the stable evidence label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IntegrityVerified => "integrity_verified",
            Self::RestoreVerified => "restore_verified",
            Self::ContinuationCertified => "continuation_certified",
        }
    }

    /// Reports whether the evidence proves a destination recaptured the state.
    #[must_use]
    pub const fn is_restore_verified(self) -> bool {
        matches!(self, Self::RestoreVerified | Self::ContinuationCertified)
    }
}

/// One restore attempt's observed facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreReceipt {
    /// Immutable checkpoint that was restored.
    pub checkpoint_id: ExactCheckpointId,
    /// Exact state identity recorded in the source checkpoint.
    pub source_state_digest: ExactStateDigest,
    /// Exact state identity recaptured at the destination.
    pub observed_state_digest: ExactStateDigest,
    /// Compatibility digest the destination ran under.
    pub compatibility_digest: String,
    /// Coverage-contract digest the destination was validated against.
    pub coverage_contract_digest: String,
    /// Epoch this restore established.
    pub execution_epoch: u64,
    /// Opaque destination identity.
    pub destination: String,
    /// Evidence level the receipt establishes.
    pub evidence: RestoreEvidence,
}

/// One admitted model decision scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateAdmission {
    /// Checkpoint the decision may start from.
    pub checkpoint_id: ExactCheckpointId,
    /// Epoch the decision is bound to.
    pub execution_epoch: u64,
    /// Destination that produced the verified restore.
    pub destination: String,
}

/// Rejection reasons for the restore gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateError {
    /// The receipt does not describe a completed, verified restore.
    NotRestoreVerified,
    /// The receipt belongs to a different checkpoint.
    CheckpointMismatch,
    /// The recaptured state differs from the checkpoint's recorded state.
    DigestMismatch,
    /// Compatibility or coverage differs from the admitted profile.
    Incompatible,
    /// The receipt's epoch does not advance the current epoch.
    StaleEpoch,
    /// A destination identifier is empty, too long, or contains a NUL separator.
    InvalidDestination,
    /// A digest field is not the expected namespace.
    InvalidDigest,
    /// No verified restore has been admitted yet.
    NotAdmitted,
}

impl fmt::Display for GateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for GateError {}

impl GateError {
    /// Returns the stable machine-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotRestoreVerified => "not_restore_verified",
            Self::CheckpointMismatch => "checkpoint_mismatch",
            Self::DigestMismatch => "digest_mismatch",
            Self::Incompatible => "incompatible",
            Self::StaleEpoch => "stale_epoch",
            Self::InvalidDestination => "invalid_destination",
            Self::InvalidDigest => "invalid_digest",
            Self::NotAdmitted => "not_admitted",
        }
    }
}

/// Gate that admits one verified restore and fences decisions by execution epoch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreGate {
    compatibility_digest: String,
    coverage_contract_digest: String,
    current_epoch: u64,
    admission: Option<GateAdmission>,
}

impl RestoreGate {
    /// Creates a gate bound to one compatibility and coverage contract.
    pub fn new(
        compatibility_digest: &str,
        coverage_contract_digest: &str,
    ) -> Result<Self, GateError> {
        validate_digest(compatibility_digest)?;
        validate_digest(coverage_contract_digest)?;
        Ok(Self {
            compatibility_digest: compatibility_digest.to_owned(),
            coverage_contract_digest: coverage_contract_digest.to_owned(),
            current_epoch: 0,
            admission: None,
        })
    }

    /// Admits a verified restore of `reference`, opening a new execution epoch.
    pub fn admit(
        &mut self,
        reference: &ExactCheckpointReference,
        receipt: &RestoreReceipt,
    ) -> Result<GateAdmission, GateError> {
        reference.validate().map_err(|_| GateError::InvalidDigest)?;
        if !receipt.evidence.is_restore_verified() {
            return Err(GateError::NotRestoreVerified);
        }
        if !valid_label(&receipt.destination) {
            return Err(GateError::InvalidDestination);
        }
        if receipt.checkpoint_id != reference.exact_checkpoint_id {
            return Err(GateError::CheckpointMismatch);
        }
        if receipt.source_state_digest != reference.exact_state_digest {
            return Err(GateError::CheckpointMismatch);
        }
        if receipt.observed_state_digest != reference.exact_state_digest {
            return Err(GateError::DigestMismatch);
        }
        if receipt.compatibility_digest != self.compatibility_digest
            || receipt.coverage_contract_digest != self.coverage_contract_digest
        {
            return Err(GateError::Incompatible);
        }
        if receipt.execution_epoch <= self.current_epoch {
            return Err(GateError::StaleEpoch);
        }
        validate_digest(&receipt.compatibility_digest)?;
        validate_digest(&receipt.coverage_contract_digest)?;
        let admission = GateAdmission {
            checkpoint_id: receipt.checkpoint_id.clone(),
            execution_epoch: receipt.execution_epoch,
            destination: receipt.destination.clone(),
        };
        self.current_epoch = receipt.execution_epoch;
        self.admission = Some(admission.clone());
        Ok(admission)
    }

    /// Returns the current execution epoch.
    #[must_use]
    pub const fn current_epoch(&self) -> u64 {
        self.current_epoch
    }

    /// Returns the admitted decision scope, if a verified restore happened.
    #[must_use]
    pub fn admission(&self) -> Option<&GateAdmission> {
        self.admission.as_ref()
    }

    /// Admits a model decision only while its epoch is the current one.
    pub fn admit_decision(&self, execution_epoch: u64) -> Result<&GateAdmission, GateError> {
        let admission = self.admission.as_ref().ok_or(GateError::NotAdmitted)?;
        if execution_epoch != self.current_epoch || admission.execution_epoch != execution_epoch {
            return Err(GateError::StaleEpoch);
        }
        Ok(admission)
    }
}

fn validate_digest(value: &str) -> Result<(), GateError> {
    BlobDigest::parse(value).map_err(|_| GateError::InvalidDigest)?;
    Ok(())
}

fn valid_label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.contains('\0')
}
