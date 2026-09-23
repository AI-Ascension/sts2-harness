// SPDX-License-Identifier: MIT

//! Private, immutable benchmark declarations and effect-free receipt association.
//!
//! A parsed manifest declares inputs; it neither attests native completeness nor
//! authorizes mutation. Only [`Manifest::public_projection`] is a public/model payload.

mod contract;
mod gate;
mod receipt;
mod validation;

use std::fmt;

use serde::Serialize;

use contract::Document;
pub use gate::{
    RerunAdmission, RerunAllocationSeam, RerunGateError, RerunRefusal, admit_and_allocate,
};
pub use receipt::{PlannedTrial, TrialStatus};

/// Only supported private manifest version. Unknown semantics require a new version.
pub const VERSION: &str = "ascension.benchmark-manifest.v1";
/// Whole input bound, checked before allocating a JSON tree.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// Bounded diagnostics contain no supplied field values, seeds, paths or digests.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestError {
    TooLarge,
    InvalidDocument,
    UnsupportedVersion,
    InvalidSeed,
    InvalidProfile,
    InvalidCompatibility,
    InvalidContextDigest,
    InvalidExperiment,
    InvalidBudget,
    InvalidOccurrence,
    InvalidReceipt,
    ReceiptIdentityMismatch,
    ReceiptSeedMismatch,
    ReceiptContextMismatch,
    ReceiptProtocolMismatch,
    ReceiptConflict,
    InvalidProjectionKey,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "benchmark manifest error: {self:?}")
    }
}
impl std::error::Error for ManifestError {}

/// Exact comparison categories; no private values are reflected.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mismatch {
    Seed,
    SelectedContext,
    Assemblies,
    GameVersion,
    Components,
    Protocol,
    Coverage,
    Platform,
    ProfileArtifact,
    UnlockProgress,
    GameplaySettings,
    Experiment,
}

/// Validated immutable private artifact. No unchecked deserializer or mutable accessor.
#[derive(Clone)]
pub struct Manifest {
    document: Document,
    canonical: Vec<u8>,
    configuration_digest: String,
    experiment_digest: String,
    artifact_digest: String,
}

impl fmt::Debug for Manifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Manifest(<private>)")
    }
}

impl Manifest {
    /// Validates closed v1 input and captures its immutable canonical representation.
    /// Unknown/null required gameplay facts and unsupported versions fail closed.
    pub fn parse_private(bytes: &[u8]) -> Result<Self, ManifestError> {
        let document: Document = validation::parse(bytes)?;
        validation::document(&document)?;
        let canonical = encode(&document)?;
        let configuration_digest = commitment("configuration", &encode(&document.gameplay)?);
        let experiment_digest = commitment("experiment", &canonical);
        let artifact_digest = commitment("artifact", &canonical);
        Ok(Self {
            document,
            canonical,
            configuration_digest,
            experiment_digest,
            artifact_digest,
        })
    }

    /// Privileged canonical artifact bytes. Store only through an authorized private adapter.
    pub fn export_private(&self) -> &[u8] {
        &self.canonical
    }

    /// Privileged controlled-gameplay identity; excludes experiments and occurrences.
    pub fn configuration_digest_private(&self) -> &str {
        &self.configuration_digest
    }

    /// Privileged gameplay-plus-experiment identity; excludes all occurrence fields.
    pub fn experiment_digest_private(&self) -> &str {
        &self.experiment_digest
    }

    /// Privileged complete versioned artifact identity, distinct from public handles.
    pub fn artifact_digest_private(&self) -> &str {
        &self.artifact_digest
    }

    /// Compares every required input exactly. Empty means equal declarations only,
    /// never native compatibility or authorization to allocate. No relaxations exist.
    pub fn compare(&self, other: &Self) -> Vec<Mismatch> {
        let a = &self.document.gameplay;
        let b = &other.document.gameplay;
        let checks = [
            (
                a.requested_seed != b.requested_seed
                    || a.effective_seed != b.effective_seed
                    || a.seed_contract != b.seed_contract,
                Mismatch::Seed,
            ),
            (
                a.selected_context != b.selected_context,
                Mismatch::SelectedContext,
            ),
            (a.assembly_hashes != b.assembly_hashes, Mismatch::Assemblies),
            (a.game_version != b.game_version, Mismatch::GameVersion),
            (a.components != b.components, Mismatch::Components),
            (
                a.protocol_version != b.protocol_version || a.protocol_digest != b.protocol_digest,
                Mismatch::Protocol,
            ),
            (
                a.coverage_version != b.coverage_version || a.coverage_digest != b.coverage_digest,
                Mismatch::Coverage,
            ),
            (a.platform != b.platform, Mismatch::Platform),
            (
                a.profile_artifact != b.profile_artifact,
                Mismatch::ProfileArtifact,
            ),
            (
                a.unlock_progress_digest != b.unlock_progress_digest,
                Mismatch::UnlockProgress,
            ),
            (
                a.gameplay_settings_digest != b.gameplay_settings_digest,
                Mismatch::GameplaySettings,
            ),
            (
                self.document.experiment != other.document.experiment,
                Mismatch::Experiment,
            ),
        ];
        checks
            .into_iter()
            .filter_map(|(different, reason)| different.then_some(reason))
            .collect()
    }

    /// Captures a validated original occurrence without changing manifest identity.
    /// This does not persist a seed or create/allocate a process, run or lease.
    pub fn plan_trial(&self, occurrence_json: &[u8]) -> Result<PlannedTrial, ManifestError> {
        let occurrence = validation::parse(occurrence_json)?;
        validation::occurrence(&occurrence)?;
        Ok(PlannedTrial::new(self.clone(), occurrence))
    }

    /// Derives a public opaque handle with a trusted secret key (32..=64 bytes).
    /// Key provisioning/storage belongs to the caller. No private settings are projected.
    pub fn public_projection(&self, key: &[u8]) -> Result<PublicManifest, ManifestError> {
        if !(32..=64).contains(&key.len()) {
            return Err(ManifestError::InvalidProjectionKey);
        }
        let handle = crate::checkpoint_projection::hmac_sha256(
            key,
            b"AI-ASCENSION/PUBLIC-BENCHMARK-MANIFEST/v1\0",
            self.artifact_digest.as_bytes(),
        );
        Ok(PublicManifest {
            version: "ascension.benchmark-public.v1",
            reference: format!("benchmark-h1:{}", crate::hex_bytes(handle)),
            evidence: "declared_inputs_only",
        })
    }
}

/// Deliberately minimal public/model projection: no seed, profile, input or exact digest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PublicManifest {
    version: &'static str,
    reference: String,
    evidence: &'static str,
}

pub(super) fn encode(value: &impl Serialize) -> Result<Vec<u8>, ManifestError> {
    serde_json::to_vec(value).map_err(|_| ManifestError::InvalidDocument)
}

pub(super) fn commitment(kind: &str, bytes: &[u8]) -> String {
    let mut preimage = format!("AI-ASCENSION/BENCHMARK/{kind}/v1\0").into_bytes();
    preimage.extend_from_slice(bytes);
    crate::sha256_hex(preimage)
}
