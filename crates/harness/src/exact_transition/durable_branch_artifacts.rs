// SPDX-License-Identifier: MIT

//! Artifact vocabulary retained by durable branches, and whether the owning store can still serve it.
//!
//! The durable branch graph records only opaque artifact identities and their roles. Whether a
//! retained reference is still readable is owned by the artifact store, so continuation asks a
//! resolver rather than assuming presence. An artifact that is absent, or whose stored bytes no
//! longer match their identity, is reported explicitly; it is never recreated, re-derived, or
//! silently replaced by a fresh start.
//!
//! Availability is necessary but not sufficient for continuation readiness. This module answers only
//! whether the owning store can still serve the recorded references. Strategy evidence
//! (`exact_restore_receipt` versus `prefix_replay_boundary`) stays a separate durable lifecycle gate,
//! and scope, destination, and lease ownership stay with their own owners.
//!
//! The exact store has no metadata-only probe, so resolving a reference reads and verifies its bytes.
//! One branch retains at most `MAX_BRANCH_ARTIFACTS` references and one exact artifact is bounded by
//! the artifact store's own size limit, so one resolution is bounded work; resolution never writes.

use std::fmt;

use rusqlite::params;

use crate::execution::{BlobDigest, ExactArtifactStore, ExactCheckpointError, ExactCheckpointId};

use super::validation::validate_label;
use super::{BranchStoreError, DurableBranch, MAX_TRANSITION_LABEL_BYTES, SqliteBranchStore};

/// Role of an artifact reference retained by a branch.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BranchArtifactRole {
    /// Exact checkpoint manifest or payload.
    Checkpoint,
    /// Additional exact restore-closure artifact.
    RestoreClosure,
    /// Public trajectory prefix used by a replay strategy.
    ReplayPrefix,
    /// Context or configuration snapshot needed by an active attempt.
    ContextSnapshot,
}

impl BranchArtifactRole {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Checkpoint => "checkpoint",
            Self::RestoreClosure => "restore_closure",
            Self::ReplayPrefix => "replay_prefix",
            Self::ContextSnapshot => "context_snapshot",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, BranchStoreError> {
        match value {
            "checkpoint" => Ok(Self::Checkpoint),
            "restore_closure" => Ok(Self::RestoreClosure),
            "replay_prefix" => Ok(Self::ReplayPrefix),
            "context_snapshot" => Ok(Self::ContextSnapshot),
            _ => Err(BranchStoreError::Corrupt),
        }
    }
}

/// One artifact retained for a branch or an in-flight continuation.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BranchArtifactReference {
    /// Opaque artifact identity owned by the artifact store.
    pub artifact_id: String,
    /// Why this artifact is reachable from the branch.
    pub role: BranchArtifactRole,
}

/// Resolution state of one opaque branch artifact identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchArtifactState {
    /// The owning store read the artifact and its bytes matched the recorded identity.
    Available,
    /// The owning store no longer serves the artifact; a continuation must not recreate it.
    Missing,
    /// The owning store could not vouch for the identity.
    ///
    /// This covers an identity outside the verified namespaces, stored bytes that no longer match
    /// their identity, and a store failure. Those cases are deliberately reported as one state: each
    /// of them blocks a continuation, and none of them authorizes recreating the artifact.
    Unverifiable,
}

/// Resolves whether an opaque branch artifact reference is still readable.
///
/// Implementations are owned by the artifact store; the branch graph never guesses presence.
pub trait BranchArtifactResolver {
    /// Resolves one artifact identity owned by the artifact store.
    fn resolve(&self, artifact_id: &str) -> BranchArtifactState;
}

/// Resolves branch artifact identities against the harness exact artifact store.
#[derive(Clone, Copy, Debug)]
pub struct ExactArtifactStoreResolver<'a> {
    store: &'a ExactArtifactStore,
}

impl<'a> ExactArtifactStoreResolver<'a> {
    /// Wraps an exact artifact store as a branch artifact resolver.
    #[must_use]
    pub const fn new(store: &'a ExactArtifactStore) -> Self {
        Self { store }
    }
}

impl BranchArtifactResolver for ExactArtifactStoreResolver<'_> {
    fn resolve(&self, artifact_id: &str) -> BranchArtifactState {
        if let Ok(identifier) = ExactCheckpointId::parse(artifact_id) {
            return classify(self.store.read_manifest(&identifier));
        }
        if let Ok(digest) = BlobDigest::parse(artifact_id) {
            return classify(self.store.read_blob(&digest));
        }
        BranchArtifactState::Unverifiable
    }
}

fn classify(result: Result<Vec<u8>, ExactCheckpointError>) -> BranchArtifactState {
    match result {
        Ok(_) => BranchArtifactState::Available,
        Err(ExactCheckpointError::Missing) => BranchArtifactState::Missing,
        Err(_) => BranchArtifactState::Unverifiable,
    }
}

/// One retained reference paired with the state reported by the resolver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchArtifactResolution {
    /// Retained artifact reference recorded by the durable branch graph.
    pub reference: BranchArtifactReference,
    /// Resolution state reported by the resolver.
    pub state: BranchArtifactState,
}

impl BranchArtifactResolution {
    /// Returns whether the owning store can still serve this reference.
    #[must_use]
    pub const fn is_available(&self) -> bool {
        matches!(self.state, BranchArtifactState::Available)
    }
}

/// Availability of the artifact references retained by one branch, in recorded order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchArtifactAvailability {
    resolutions: Vec<BranchArtifactResolution>,
}

impl BranchArtifactAvailability {
    /// Resolves each reference against the owning artifact store, preserving input order.
    #[must_use]
    pub fn resolve(
        references: &[BranchArtifactReference],
        resolver: &dyn BranchArtifactResolver,
    ) -> Self {
        let resolutions = references
            .iter()
            .map(|reference| BranchArtifactResolution {
                reference: reference.clone(),
                state: resolver.resolve(&reference.artifact_id),
            })
            .collect();
        Self { resolutions }
    }

    /// Returns every resolution, in recorded order.
    #[must_use]
    pub fn resolutions(&self) -> &[BranchArtifactResolution] {
        &self.resolutions
    }

    /// Returns the references the owning store cannot serve, in recorded order.
    #[must_use]
    pub fn unavailable(&self) -> Vec<&BranchArtifactResolution> {
        self.resolutions
            .iter()
            .filter(|resolution| !resolution.is_available())
            .collect()
    }

    /// Returns whether the owning store can still serve every retained reference.
    ///
    /// A record that retains no artifacts is vacuously available: this reports readability only. It
    /// does not assert that a strategy retained the artifacts it needs, nor that strategy evidence,
    /// scope, or a destination is sufficient for readiness.
    #[must_use]
    pub fn all_available(&self) -> bool {
        self.resolutions
            .iter()
            .all(BranchArtifactResolution::is_available)
    }

    /// Refuses a continuation while any retained reference cannot be served.
    ///
    /// # Errors
    ///
    /// Returns [`BranchArtifactUnavailable`] naming the references the store could not serve.
    pub fn require_all_available(&self) -> Result<(), BranchArtifactUnavailable> {
        if self.all_available() {
            return Ok(());
        }
        Err(BranchArtifactUnavailable {
            references: self.unavailable().into_iter().cloned().collect(),
        })
    }
}

/// Refusal recorded when an artifact retained by a branch can no longer be served.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchArtifactUnavailable {
    references: Vec<BranchArtifactResolution>,
}

impl BranchArtifactUnavailable {
    /// Returns the references the owning store could not serve, in recorded order.
    #[must_use]
    pub fn references(&self) -> &[BranchArtifactResolution] {
        &self.references
    }
}

impl fmt::Display for BranchArtifactUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.references.first() {
            Some(first) => write!(
                formatter,
                "durable branch artifact {} is {}; {} retained reference(s) unavailable",
                first.reference.artifact_id,
                state_label(first.state),
                self.references.len()
            ),
            None => formatter.write_str("durable branch retained artifacts are unavailable"),
        }
    }
}

impl std::error::Error for BranchArtifactUnavailable {}

const fn state_label(state: BranchArtifactState) -> &'static str {
    match state {
        BranchArtifactState::Available => "available",
        BranchArtifactState::Missing => "missing",
        BranchArtifactState::Unverifiable => "unverifiable",
    }
}

impl DurableBranch {
    /// Resolves every retained artifact reference against the owning artifact store.
    #[must_use]
    pub fn artifact_availability(
        &self,
        resolver: &dyn BranchArtifactResolver,
    ) -> BranchArtifactAvailability {
        BranchArtifactAvailability::resolve(&self.artifacts, resolver)
    }
}

impl SqliteBranchStore {
    /// Resolves the artifacts retained by one open branch against the owning artifact store.
    ///
    /// # Errors
    ///
    /// Returns [`BranchStoreError::UnknownBranch`] when the experiment has no such branch, and
    /// [`BranchStoreError::ArtifactUnavailable`] when the branch row is tombstoned. An explicit
    /// prune collects the retained edges of a tombstoned branch, so its recorded references are no
    /// longer readable *as a set*: reporting that empty remainder as available would let a
    /// collected dependency look like a fresh start.
    pub fn artifact_availability(
        &self,
        experiment_id: &str,
        branch_id: &str,
        resolver: &dyn BranchArtifactResolver,
    ) -> Result<BranchArtifactAvailability, BranchStoreError> {
        validate_label(experiment_id, MAX_TRANSITION_LABEL_BYTES)?;
        validate_label(branch_id, MAX_TRANSITION_LABEL_BYTES)?;
        if self.branch_is_tombstoned(experiment_id, branch_id)? {
            return Err(BranchStoreError::ArtifactUnavailable);
        }
        let branch = self
            .get(experiment_id, branch_id)?
            .ok_or(BranchStoreError::UnknownBranch)?;
        Ok(branch.artifact_availability(resolver))
    }

    fn branch_is_tombstoned(
        &self,
        experiment_id: &str,
        branch_id: &str,
    ) -> Result<bool, BranchStoreError> {
        let connection = self.lock()?;
        let tombstoned: bool = connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM branch_tombstones
                    WHERE experiment_id = ?1 AND branch_id = ?2
                )",
                params![experiment_id, branch_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(BranchStoreError::persistence)?
            != 0;
        Ok(tombstoned)
    }
}
