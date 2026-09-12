// SPDX-License-Identifier: MIT

//! Bounded verification of a stored exact checkpoint without a live restore.
//!
//! Verification resolves the immutable manifest, checks its recorded exact-state identity, compares
//! compatibility and coverage against the expected contracts, and re-hashes every referenced blob.
//! Success establishes integrity only: `restore_verified` and `continuation_certified` stay false
//! because nothing was restored into a destination.

use std::fmt;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use crate::checkpoint_capability::CheckpointEvidence;
use crate::execution::{
    BlobDigest, ExactArtifactStore, ExactCheckpointError, ExactCheckpointReference,
};

#[path = "checkpoint_payload.rs"]
mod payload;

/// Why a checkpoint failed independent verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationFailure {
    /// The reference itself is not well formed.
    InvalidReference,
    /// The manifest is absent.
    ManifestMissing,
    /// The manifest is not readable JSON or lacks required members.
    MalformedManifest,
    /// A digest check failed.
    IntegrityFailure,
    /// Compatibility differs from the expected contract.
    IncompatibleProfile,
    /// Coverage evidence differs from the expected contract.
    CoverageIncomplete,
    /// A referenced dependency is absent.
    MissingDependency,
    /// The evidence chain could not be constructed.
    EvidenceIncomplete,
}

impl fmt::Display for VerificationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::error::Error for VerificationFailure {}

impl VerificationFailure {
    /// Returns the stable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidReference => "invalid_reference",
            Self::ManifestMissing => "manifest_missing",
            Self::MalformedManifest => "malformed_manifest",
            Self::IntegrityFailure => "integrity_failure",
            Self::IncompatibleProfile => "incompatible_profile",
            Self::CoverageIncomplete => "coverage_incomplete",
            Self::MissingDependency => "missing_dependency",
            Self::EvidenceIncomplete => "evidence_incomplete",
        }
    }
}

/// Result of independent verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerificationOutcome {
    /// Artifacts and contracts checked out; nothing was restored.
    Verified(CheckpointEvidence),
    /// Verification failed for the recorded reason.
    Rejected(VerificationFailure),
}

/// Verifies a stored checkpoint against expected compatibility and coverage contracts.
pub fn verify_checkpoint(
    store: &ExactArtifactStore,
    reference: &ExactCheckpointReference,
    expected_compatibility: &str,
    expected_coverage_contract: &str,
) -> VerificationOutcome {
    if reference.validate().is_err() {
        return VerificationOutcome::Rejected(VerificationFailure::InvalidReference);
    }
    let bytes = match store.read_manifest(&reference.exact_checkpoint_id) {
        Ok(bytes) => bytes,
        Err(ExactCheckpointError::Missing) => {
            return VerificationOutcome::Rejected(VerificationFailure::ManifestMissing);
        }
        Err(_) => return VerificationOutcome::Rejected(VerificationFailure::MalformedManifest),
    };
    let manifest: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(manifest) => manifest,
        Err(_) => return VerificationOutcome::Rejected(VerificationFailure::MalformedManifest),
    };
    if !valid_manifest(&manifest) || !payload::is_canonical(&manifest, &bytes) {
        return VerificationOutcome::Rejected(VerificationFailure::MalformedManifest);
    }
    if manifest
        .get("exact_state_digest")
        .and_then(serde_json::Value::as_str)
        != Some(reference.exact_state_digest.as_str())
    {
        return VerificationOutcome::Rejected(VerificationFailure::IntegrityFailure);
    }
    if manifest["boundary"]["kind"].as_str() != Some(reference.boundary_kind.as_str())
        || manifest["boundary"]["phase"].as_str() != Some(reference.boundary_phase.as_str())
    {
        return VerificationOutcome::Rejected(VerificationFailure::IntegrityFailure);
    }
    let compatibility = manifest
        .get("compatibility_digest")
        .and_then(serde_json::Value::as_str);
    if compatibility != Some(expected_compatibility) {
        return VerificationOutcome::Rejected(VerificationFailure::IncompatibleProfile);
    }
    let coverage = manifest
        .get("coverage_contract_digest")
        .and_then(serde_json::Value::as_str);
    if coverage != Some(expected_coverage_contract) {
        return VerificationOutcome::Rejected(VerificationFailure::CoverageIncomplete);
    }
    match verify_dependencies(store, &manifest) {
        Ok(()) => {}
        Err(failure) => return VerificationOutcome::Rejected(failure),
    }
    let evidence = CheckpointEvidence {
        captured: true,
        durable: true,
        integrity_verified: true,
        restore_verified: false,
        continuation_certified: false,
        producer: "harness:verify".to_owned(),
    };
    match evidence.validate() {
        Ok(()) => VerificationOutcome::Verified(evidence),
        Err(_) => VerificationOutcome::Rejected(VerificationFailure::EvidenceIncomplete),
    }
}

fn verify_dependencies(
    store: &ExactArtifactStore,
    manifest: &serde_json::Value,
) -> Result<(), VerificationFailure> {
    let payload = &manifest["canonical_payload"];
    if payload["codec"] != "asc-jcs-state-v1" {
        return Err(VerificationFailure::MalformedManifest);
    }
    let entries = std::iter::once(payload).chain(
        manifest["restore_artifacts"]
            .as_array()
            .ok_or(VerificationFailure::MalformedManifest)?
            .iter(),
    );
    for entry in entries {
        let digest = parse_digest(
            entry["digest"]
                .as_str()
                .ok_or(VerificationFailure::MalformedManifest)?,
        )?;
        let bytes = match store.read_blob(&digest) {
            Ok(bytes) => bytes,
            Err(ExactCheckpointError::Missing) => {
                return Err(VerificationFailure::MissingDependency);
            }
            Err(_) => return Err(VerificationFailure::IntegrityFailure),
        };
        if entry["size_bytes"].as_u64() != Some(bytes.len() as u64) {
            return Err(VerificationFailure::IntegrityFailure);
        }
        if std::ptr::eq(entry, payload) {
            self::payload::verify(&bytes, manifest)?;
            let mut hash = Sha256::new();
            hash.update(b"AI-ASCENSION/EXACT-STATE/v1\0");
            hash.update(&bytes);
            let hex: String = hash
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let identity = format!("asc-state:v1:sha256:{hex}");
            if manifest["exact_state_digest"].as_str() != Some(identity.as_str()) {
                return Err(VerificationFailure::IntegrityFailure);
            }
        }
    }
    Ok(())
}

fn valid_manifest(manifest: &serde_json::Value) -> bool {
    static VALIDATOR: OnceLock<Result<jsonschema::Validator, String>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let schema: serde_json::Value = serde_json::from_str(include_str!(
                "../../../protocol-artifact/exact-state-v1/checkpoint-manifest.schema.json"
            ))
            .map_err(|error| error.to_string())?;
            jsonschema::validator_for(&schema).map_err(|error| error.to_string())
        })
        .as_ref()
        .is_ok_and(|validator| validator.is_valid(manifest))
}

fn parse_digest(value: &str) -> Result<BlobDigest, VerificationFailure> {
    BlobDigest::parse(value).map_err(|_| VerificationFailure::MalformedManifest)
}
