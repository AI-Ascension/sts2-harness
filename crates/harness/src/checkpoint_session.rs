// SPDX-License-Identifier: MIT

//! Session admission that requires both a verified restore and independent verification.
//!
//! A model or player session is admitted only when a destination actually recaptured the expected
//! exact state and the stored artifacts independently verify against the expected compatibility and
//! coverage contracts. Integrity alone, or a receipt alone, is never enough: the two facts are
//! checked separately and both must hold before any decision is accepted.

use std::fmt;

use crate::checkpoint_capability::{CapabilityError, CheckpointEvidence};
use crate::checkpoint_verify::{VerificationFailure, VerificationOutcome, verify_checkpoint};
use crate::execution::{ExactArtifactStore, ExactCheckpointId, ExactCheckpointReference};
use crate::restore_gate::{GateError, RestoreEvidence, RestoreGate, RestoreReceipt};

/// One admitted session scope bound to a verified restore and verified artifacts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionAdmission {
    /// Checkpoint the session may start from.
    pub checkpoint_id: ExactCheckpointId,
    /// Execution epoch the session decisions are bound to.
    pub execution_epoch: u64,
    /// Destination that produced the verified restore.
    pub destination: String,
    /// Combined evidence, never stronger than what both checks proved.
    pub evidence: CheckpointEvidence,
}

/// Rejection reasons for session admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    /// The restore gate refused the receipt.
    Gate(GateError),
    /// Independent artifact verification failed.
    Verification(VerificationFailure),
    /// The combined evidence chain was inconsistent.
    Evidence(CapabilityError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gate(error) => write!(formatter, "restore gate refused: {error}"),
            Self::Verification(failure) => {
                write!(
                    formatter,
                    "artifact verification failed: {}",
                    failure.as_str()
                )
            }
            Self::Evidence(error) => write!(formatter, "session evidence is inconsistent: {error}"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<GateError> for SessionError {
    fn from(error: GateError) -> Self {
        Self::Gate(error)
    }
}

/// Admits a session only after a verified restore and independent artifact verification.
pub fn admit_session(
    store: &ExactArtifactStore,
    gate: &mut RestoreGate,
    reference: &ExactCheckpointReference,
    receipt: &RestoreReceipt,
    expected_compatibility: &str,
    expected_coverage_contract: &str,
) -> Result<SessionAdmission, SessionError> {
    let verification = verify_checkpoint(
        store,
        reference,
        expected_compatibility,
        expected_coverage_contract,
    );
    let verified_evidence = match verification {
        VerificationOutcome::Verified(evidence) => evidence,
        VerificationOutcome::Rejected(failure) => {
            return Err(SessionError::Verification(failure));
        }
    };
    let admission = gate.admit(reference, receipt)?;
    let evidence = CheckpointEvidence {
        captured: verified_evidence.captured,
        durable: verified_evidence.durable,
        integrity_verified: verified_evidence.integrity_verified,
        restore_verified: receipt.evidence.is_restore_verified(),
        continuation_certified: receipt.evidence == RestoreEvidence::ContinuationCertified,
        producer: "harness:session".to_owned(),
    };
    evidence.validate().map_err(SessionError::Evidence)?;
    Ok(SessionAdmission {
        checkpoint_id: admission.checkpoint_id,
        execution_epoch: admission.execution_epoch,
        destination: admission.destination,
        evidence,
    })
}

impl SessionAdmission {
    /// Reports whether controlled continuations were certified for this session.
    #[must_use]
    pub const fn continuation_certified(&self) -> bool {
        self.evidence.continuation_certified
    }
}
