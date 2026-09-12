// SPDX-License-Identifier: MIT

//! Durable content-addressed storage for exact checkpoint artifacts.
//!
//! The harness stores exact-state payloads, immutable manifests, and restore blobs by verified
//! content digest under a configured root. It never reaches a game process: the game-mod owns
//! capture, restore, and host-thread authority. Exact references stay a separate type from the
//! legacy public `Checkpoint` so a public-only record cannot masquerade as an exact one.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

/// Serialized prefix of an exact-state content digest.
pub const EXACT_STATE_DIGEST_PREFIX: &str = "asc-state:v1:sha256:";
/// Serialized prefix of an immutable checkpoint identifier.
pub const EXACT_CHECKPOINT_ID_PREFIX: &str = "asc-checkpoint:v1:sha256:";
/// Serialized prefix of a raw artifact blob digest.
pub const BLOB_DIGEST_PREFIX: &str = "sha256:";
/// Domain separator for the immutable checkpoint-manifest identity.
pub const CHECKPOINT_MANIFEST_DOMAIN: &[u8] = b"AI-ASCENSION/CHECKPOINT/v1\0";
/// Maximum accepted size of one exact artifact blob or manifest.
pub const MAX_EXACT_BLOB_BYTES: usize = 16 * 1024 * 1024;
/// Maximum accepted length of a boundary kind or phase label.
pub const MAX_BOUNDARY_LABEL_BYTES: usize = 128;

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// Rejection reasons for exact artifact storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactCheckpointError {
    /// A serialized identifier is not the expected namespace and lowercase hex body.
    InvalidDigest,
    /// An artifact exceeds [`MAX_EXACT_BLOB_BYTES`].
    Oversized,
    /// The referenced artifact is absent.
    Missing,
    /// Stored bytes do not match their content identity.
    DigestMismatch,
    /// A manifest is empty, malformed, or lacks the referenced exact-state digest.
    InvalidManifest,
    /// A boundary label is empty or too long.
    InvalidBoundary,
    /// The backing store failed.
    Persistence(String),
}

impl fmt::Display for ExactCheckpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidDigest => "exact identity is not the expected namespace",
            Self::Oversized => "exact artifact exceeds the configured bound",
            Self::Missing => "exact artifact is missing",
            Self::DigestMismatch => "exact artifact content does not match its identity",
            Self::InvalidManifest => "exact checkpoint manifest is invalid",
            Self::InvalidBoundary => "exact checkpoint boundary label is invalid",
            Self::Persistence(_) => "exact artifact store failed",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ExactCheckpointError {}

/// A domain-separated exact-state content identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExactStateDigest(String);

impl ExactStateDigest {
    /// Parses a serialized identifier, rejecting other identity namespaces.
    pub fn parse(value: &str) -> Result<Self, ExactCheckpointError> {
        split_hex(value, EXACT_STATE_DIGEST_PREFIX)?;
        Ok(Self(value.to_owned()))
    }

    /// Returns the serialized identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A domain-separated immutable checkpoint identifier.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExactCheckpointId(String);

impl ExactCheckpointId {
    /// Parses a serialized identifier, rejecting other identity namespaces.
    pub fn parse(value: &str) -> Result<Self, ExactCheckpointError> {
        split_hex(value, EXACT_CHECKPOINT_ID_PREFIX)?;
        Ok(Self(value.to_owned()))
    }

    /// Returns the serialized identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A digest of exact artifact bytes under the blob namespace.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BlobDigest(String);

impl BlobDigest {
    /// Parses a serialized blob digest.
    pub fn parse(value: &str) -> Result<Self, ExactCheckpointError> {
        split_hex(value, BLOB_DIGEST_PREFIX)?;
        Ok(Self(value.to_owned()))
    }

    /// Returns the serialized digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Distinct assurance levels for a checkpoint reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactAssurance {
    /// Public observation only; no exact-state claim may be made.
    PublicObservationOnly,
    /// An exact snapshot was encoded, but no restore was attempted.
    CaptureOnly,
    /// A restore path exists and was admitted, but no destination recaptured it.
    RestoreSupported,
    /// A destination recaptured the expected exact state.
    RestoreVerified,
    /// Controlled continuations matched for the declared scope.
    ContinuationCertified,
}

impl ExactAssurance {
    /// Returns the stable label recorded in evidence.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PublicObservationOnly => "public_observation_only",
            Self::CaptureOnly => "capture_only",
            Self::RestoreSupported => "restore_supported",
            Self::RestoreVerified => "restore_verified",
            Self::ContinuationCertified => "continuation_certified",
        }
    }

    /// Reports whether this level makes an exact-state claim at all.
    #[must_use]
    pub const fn is_exact(self) -> bool {
        !matches!(self, Self::PublicObservationOnly)
    }
}

/// An additive reference to one stored exact checkpoint occurrence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactCheckpointReference {
    /// Content identity of the captured exact state.
    pub exact_state_digest: ExactStateDigest,
    /// Identity of the immutable manifest binding that state to its artifacts.
    pub exact_checkpoint_id: ExactCheckpointId,
    /// Boundary kind at which capture occurred.
    pub boundary_kind: String,
    /// Game phase at which capture occurred.
    pub boundary_phase: String,
    /// Evidence level proven for this reference.
    pub assurance: ExactAssurance,
}

impl ExactCheckpointReference {
    /// Validates the reference before storage or comparison.
    pub fn validate(&self) -> Result<(), ExactCheckpointError> {
        for label in [&self.boundary_kind, &self.boundary_phase] {
            if label.is_empty() || label.len() > MAX_BOUNDARY_LABEL_BYTES {
                return Err(ExactCheckpointError::InvalidBoundary);
            }
        }
        Ok(())
    }
}

/// Content-addressed store for exact payloads, manifests, and restore blobs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactArtifactStore {
    pub(crate) root: PathBuf,
}

impl ExactArtifactStore {
    /// Creates a store rooted at `root`; directories are created on first write.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Stores blob bytes by their own digest and returns that digest.
    pub fn stage_blob(&self, bytes: &[u8]) -> Result<BlobDigest, ExactCheckpointError> {
        if bytes.len() > MAX_EXACT_BLOB_BYTES {
            return Err(ExactCheckpointError::Oversized);
        }
        let hex = blob_hex(bytes);
        self.write_atomic(&self.blob_path(&hex), bytes)?;
        Ok(BlobDigest(format!("{BLOB_DIGEST_PREFIX}{hex}")))
    }

    /// Reads a stored blob, rejecting absent or tampered bytes.
    pub fn read_blob(&self, digest: &BlobDigest) -> Result<Vec<u8>, ExactCheckpointError> {
        let hex = split_hex(digest.as_str(), BLOB_DIGEST_PREFIX)?;
        let bytes = self.read_verified(&self.blob_path(hex))?;
        if blob_hex(&bytes) != hex {
            return Err(ExactCheckpointError::DigestMismatch);
        }
        Ok(bytes)
    }

    /// Publishes canonical manifest bytes and returns their immutable identifier.
    pub fn publish_manifest(
        &self,
        canonical_manifest: &[u8],
    ) -> Result<ExactCheckpointId, ExactCheckpointError> {
        if canonical_manifest.is_empty() {
            return Err(ExactCheckpointError::InvalidManifest);
        }
        let hex = manifest_hex(canonical_manifest)?;
        self.write_atomic(&self.manifest_path(&hex), canonical_manifest)?;
        Ok(ExactCheckpointId(format!(
            "{EXACT_CHECKPOINT_ID_PREFIX}{hex}"
        )))
    }

    /// Reads a stored manifest, rejecting absent or tampered bytes.
    pub fn read_manifest(
        &self,
        identifier: &ExactCheckpointId,
    ) -> Result<Vec<u8>, ExactCheckpointError> {
        let hex = split_hex(identifier.as_str(), EXACT_CHECKPOINT_ID_PREFIX)?;
        let bytes = self.read_verified(&self.manifest_path(hex))?;
        if manifest_hex(&bytes)? != hex {
            return Err(ExactCheckpointError::DigestMismatch);
        }
        Ok(bytes)
    }

    /// Verifies that a reference resolves to a stored manifest binding the same state.
    pub fn verify_reference(
        &self,
        reference: &ExactCheckpointReference,
    ) -> Result<(), ExactCheckpointError> {
        reference.validate()?;
        let bytes = self.read_manifest(&reference.exact_checkpoint_id)?;
        let manifest: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| ExactCheckpointError::InvalidManifest)?;
        let recorded = manifest
            .get("exact_state_digest")
            .and_then(serde_json::Value::as_str)
            .ok_or(ExactCheckpointError::InvalidManifest)?;
        if recorded != reference.exact_state_digest.as_str() {
            return Err(ExactCheckpointError::DigestMismatch);
        }
        Ok(())
    }

    fn read_verified(&self, path: &Path) -> Result<Vec<u8>, ExactCheckpointError> {
        if !path.is_file() {
            return Err(ExactCheckpointError::Missing);
        }
        fs::read(path).map_err(persistence)
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<(), ExactCheckpointError> {
        if path.is_file() {
            return if fs::read(path).map_err(persistence)? == bytes {
                Ok(())
            } else {
                Err(ExactCheckpointError::DigestMismatch)
            };
        }
        let directory = path.parent().ok_or(ExactCheckpointError::InvalidDigest)?;
        fs::create_dir_all(directory).map_err(persistence)?;
        let nonce = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(".tmp-{}-{nonce}", std::process::id()));
        fs::write(&temporary, bytes).map_err(persistence)?;
        fs::rename(&temporary, path).map_err(persistence)
    }

    fn blob_path(&self, hex: &str) -> PathBuf {
        content_path(&self.root, "blobs", hex)
    }

    fn manifest_path(&self, hex: &str) -> PathBuf {
        content_path(&self.root, "manifests", hex)
    }
}

fn content_path(root: &Path, kind: &str, hex: &str) -> PathBuf {
    let (prefix, _) = hex.split_at(2);
    root.join("exact").join(kind).join(prefix).join(hex)
}

fn blob_hex(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes))
}

fn manifest_hex(bytes: &[u8]) -> Result<String, ExactCheckpointError> {
    if bytes.len() > MAX_EXACT_BLOB_BYTES {
        return Err(ExactCheckpointError::Oversized);
    }
    let mut hasher = Sha256::new();
    hasher.update(CHECKPOINT_MANIFEST_DOMAIN);
    hasher.update(bytes);
    Ok(to_hex(&hasher.finalize()))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn split_hex<'a>(value: &'a str, prefix: &str) -> Result<&'a str, ExactCheckpointError> {
    let hex = value
        .strip_prefix(prefix)
        .ok_or(ExactCheckpointError::InvalidDigest)?;
    let lowercase = hex
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if hex.len() != 64 || !lowercase {
        return Err(ExactCheckpointError::InvalidDigest);
    }
    Ok(hex)
}

fn persistence(error: io::Error) -> ExactCheckpointError {
    ExactCheckpointError::Persistence(error.to_string())
}
