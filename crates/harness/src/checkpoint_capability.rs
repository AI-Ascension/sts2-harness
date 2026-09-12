// SPDX-License-Identifier: MIT

//! Typed checkpoint capability, evidence, and failure reporting.
//!
//! Capture completeness, durability, integrity, restore verification, and continuation
//! certification are separate facts, so a single success boolean is not enough. An adapter reports
//! what it supports and what it explicitly does not, and each evidence level implies the levels
//! below it. Exact-state claims are refused when the reported mode cannot back them.

use std::fmt;

use serde::Serialize;

/// Highest checkpoint mode an adapter reports.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CheckpointMode {
    /// Allowed public observation only; no exact-state claim is possible.
    ObservationOnly,
    /// Exact state can be captured but no restore path is admitted.
    CaptureOnly,
    /// A restore path exists but no destination recaptured the state yet.
    RestoreSupported,
    /// A destination recaptured the expected exact state.
    RestoreVerified,
}

impl CheckpointMode {
    /// Returns the stable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ObservationOnly => "observation_only",
            Self::CaptureOnly => "capture_only",
            Self::RestoreSupported => "restore_supported",
            Self::RestoreVerified => "restore_verified",
        }
    }

    /// Reports whether this mode permits any exact-state claim.
    #[must_use]
    pub const fn allows_exact_claims(self) -> bool {
        !matches!(self, Self::ObservationOnly)
    }
}

/// A phase the adapter explicitly does not support.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsupportedPhase {
    /// Phase name.
    pub phase: String,
    /// Why the phase is unsupported.
    pub reason: String,
}

/// What an adapter reports it can do, before any capture is attempted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityReport {
    /// Adapter identifier.
    pub adapter_id: String,
    /// Adapter version.
    pub adapter_version: String,
    /// Canonical profile the adapter encodes.
    pub canonical_profile: String,
    /// Phases the adapter supports.
    pub supported_phases: Vec<String>,
    /// Phases the adapter explicitly rejects, with reasons.
    pub unsupported_phases: Vec<UnsupportedPhase>,
    /// Highest mode the adapter implements.
    pub capture_mode: CheckpointMode,
    /// Coverage-contract digest; required once exact capture is claimed.
    pub coverage_contract_digest: Option<String>,
    /// Whether a destination actually recaptured the expected state.
    pub restore_verified_evidence: bool,
}

impl CapabilityReport {
    /// Validates the report against its own claims.
    pub fn validate(&self) -> Result<(), CapabilityError> {
        for label in [
            &self.adapter_id,
            &self.adapter_version,
            &self.canonical_profile,
        ] {
            if !valid_label(label) {
                return Err(CapabilityError::InvalidLabel);
            }
        }
        for phase in self.supported_phases.iter().chain(
            self.unsupported_phases
                .iter()
                .flat_map(|entry| [&entry.phase, &entry.reason]),
        ) {
            if !valid_label(phase) {
                return Err(CapabilityError::InvalidLabel);
            }
        }
        if self.capture_mode.allows_exact_claims() && self.coverage_contract_digest.is_none() {
            return Err(CapabilityError::MissingCoverage);
        }
        if self.capture_mode == CheckpointMode::RestoreVerified && !self.restore_verified_evidence {
            return Err(CapabilityError::UnsupportedRestoreClaim);
        }
        if self.restore_verified_evidence && self.capture_mode < CheckpointMode::RestoreVerified {
            return Err(CapabilityError::InconsistentMode);
        }
        Ok(())
    }

    /// Reports whether a phase is advertised as supported.
    #[must_use]
    pub fn supports(&self, phase: &str) -> bool {
        self.supported_phases.iter().any(|entry| entry == phase)
    }

    /// Returns the reason a phase is unsupported, if declared.
    #[must_use]
    pub fn unsupported_reason(&self, phase: &str) -> Option<&str> {
        self.unsupported_phases
            .iter()
            .find(|entry| entry.phase == phase)
            .map(|entry| entry.reason.as_str())
    }
}

/// Typed capture failures; none of these may be reported as a successful capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureFailure {
    /// The instant is mid-effect or otherwise incoherent.
    UnsafeBoundary,
    /// The profile or boundary is not supported.
    UnsupportedProfile,
    /// Some future-affecting field is unknown or unsupported.
    IncompleteCoverage,
    /// The snapshot was torn or impure.
    InconsistentCapture,
    /// The runtime instance or lease was lost.
    AuthorityLost,
    /// The bounded wait elapsed.
    Timeout,
    /// The request was cancelled without a partial artifact.
    Cancelled,
    /// Durability failed after encoding.
    StorageFailure,
    /// The game-side producer failed.
    ProducerFailure,
}

impl CaptureFailure {
    /// Returns the stable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsafeBoundary => "unsafe_boundary",
            Self::UnsupportedProfile => "unsupported_profile",
            Self::IncompleteCoverage => "incomplete_coverage",
            Self::InconsistentCapture => "inconsistent_capture",
            Self::AuthorityLost => "authority_lost",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::StorageFailure => "storage_failure",
            Self::ProducerFailure => "producer_failure",
        }
    }
}

/// Typed restore failures; none of these may be reported as a verified restore.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreFailure {
    /// The manifest or a dependency is absent.
    ManifestMissing,
    /// A digest or codec check failed.
    IntegrityFailure,
    /// Content, mods, or profile are incompatible.
    IncompatibleProfile,
    /// Coverage evidence is incomplete.
    CoverageIncomplete,
    /// Destination lifecycle authority was not acquired.
    AuthorityLost,
    /// A prior action or restore is still in flight.
    DestinationBusy,
    /// The destination recaptured different state.
    RecaptureMismatch,
    /// Storage or rollback failed.
    StorageFailure,
}

impl RestoreFailure {
    /// Returns the stable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ManifestMissing => "manifest_missing",
            Self::IntegrityFailure => "integrity_failure",
            Self::IncompatibleProfile => "incompatible_profile",
            Self::CoverageIncomplete => "coverage_incomplete",
            Self::AuthorityLost => "authority_lost",
            Self::DestinationBusy => "destination_busy",
            Self::RecaptureMismatch => "recapture_mismatch",
            Self::StorageFailure => "storage_failure",
        }
    }
}

/// Ordered facts proven for one checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointEvidence {
    /// A coherent complete state was encoded.
    pub captured: bool,
    /// Referenced bytes and manifest were committed durably.
    pub durable: bool,
    /// Hashes and codecs were checked.
    pub integrity_verified: bool,
    /// A destination recaptured the expected exact state.
    pub restore_verified: bool,
    /// Controlled continuations matched for the declared scope.
    pub continuation_certified: bool,
    /// Producer that recorded this evidence.
    pub producer: String,
}

impl CheckpointEvidence {
    /// Validates that higher levels imply the levels below them.
    pub fn validate(&self) -> Result<(), CapabilityError> {
        if !valid_label(&self.producer) {
            return Err(CapabilityError::InvalidLabel);
        }
        let chain = [
            self.durable,
            self.integrity_verified,
            self.restore_verified,
            self.continuation_certified,
        ];
        let mut previous = self.captured;
        for level in chain {
            if level && !previous {
                return Err(CapabilityError::BrokenEvidenceChain);
            }
            previous = level;
        }
        Ok(())
    }

    /// Returns the highest level proven.
    #[must_use]
    pub const fn highest_level(&self) -> &'static str {
        if self.continuation_certified {
            "continuation_certified"
        } else if self.restore_verified {
            "restore_verified"
        } else if self.integrity_verified {
            "integrity_verified"
        } else if self.durable {
            "durable"
        } else if self.captured {
            "captured"
        } else {
            "none"
        }
    }
}

/// Public-safe evidence summary without digests or hidden values.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PublicEvidenceSummary {
    /// Highest level proven.
    pub level: String,
    /// Whether a restore was recaptured.
    pub restore_verified: bool,
    /// Whether continuations matched.
    pub continuation_certified: bool,
}

impl CheckpointEvidence {
    /// Projects the evidence to a summary safe for ordinary consumers.
    pub fn public_summary(&self) -> Result<PublicEvidenceSummary, CapabilityError> {
        self.validate()?;
        Ok(PublicEvidenceSummary {
            level: self.highest_level().to_owned(),
            restore_verified: self.restore_verified,
            continuation_certified: self.continuation_certified,
        })
    }
}

/// Rejection reasons for capability and evidence records.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityError {
    /// A label is empty, too long, or contains a NUL separator.
    InvalidLabel,
    /// Exact capture was claimed without a coverage contract.
    MissingCoverage,
    /// A restore-verified mode was claimed without restore evidence.
    UnsupportedRestoreClaim,
    /// The mode and the evidence contradict each other.
    InconsistentMode,
    /// A higher evidence level was claimed without the lower one.
    BrokenEvidenceChain,
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLabel => "capability label is invalid",
            Self::MissingCoverage => "exact capture claimed without a coverage contract",
            Self::UnsupportedRestoreClaim => "restore-verified claim lacks restore evidence",
            Self::InconsistentMode => "capture mode contradicts recorded evidence",
            Self::BrokenEvidenceChain => "evidence level claimed without the level below it",
        })
    }
}

impl std::error::Error for CapabilityError {}

fn valid_label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.contains('\0')
}
