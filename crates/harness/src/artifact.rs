// SPDX-License-Identifier: MIT

use crate::error::PortError;
use crate::identity::{ArtifactId, Digest, RunId, SchemaVersion, TrajectoryId};
use sha2::{Digest as _, Sha256};

const MAX_PRODUCER_BYTES: usize = 128;
const MAX_PARENT_ARTIFACTS: usize = 32;
const MAX_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactKind {
    Trajectory,
    ReplayReport,
    Score,
    Dataset,
    ModelEvaluation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactLineage {
    source_run: RunId,
    source_trajectory: Option<TrajectoryId>,
    parent_artifacts: Vec<ArtifactId>,
}

impl ArtifactLineage {
    pub fn new(
        source_run: RunId,
        source_trajectory: Option<TrajectoryId>,
        parent_artifacts: Vec<ArtifactId>,
    ) -> Result<Self, PortError> {
        if parent_artifacts.len() > MAX_PARENT_ARTIFACTS {
            return Err(PortError::new(
                "artifact_lineage_too_large",
                "artifact parent list exceeds its bound",
                false,
            ));
        }
        Ok(Self {
            source_run,
            source_trajectory,
            parent_artifacts,
        })
    }

    #[must_use]
    pub const fn source_run(&self) -> RunId {
        self.source_run
    }

    #[must_use]
    pub const fn source_trajectory(&self) -> Option<TrajectoryId> {
        self.source_trajectory
    }

    #[must_use]
    pub fn parent_artifacts(&self) -> &[ArtifactId] {
        &self.parent_artifacts
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactMetadataInput {
    pub artifact_id: ArtifactId,
    pub owner_run: RunId,
    pub kind: ArtifactKind,
    pub schema_version: SchemaVersion,
    pub content_digest: Digest,
    pub producer: String,
    pub lineage: ArtifactLineage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactMetadata {
    artifact_id: ArtifactId,
    owner_run: RunId,
    kind: ArtifactKind,
    schema_version: SchemaVersion,
    content_digest: Digest,
    byte_length: u64,
    producer: String,
    lineage: ArtifactLineage,
}

impl ArtifactMetadata {
    pub fn from_input(input: ArtifactMetadataInput, byte_length: u64) -> Result<Self, PortError> {
        if input.producer.is_empty() || input.producer.len() > MAX_PRODUCER_BYTES {
            return Err(PortError::new(
                "invalid_artifact_producer",
                "artifact producer must be nonempty and bounded",
                false,
            ));
        }
        if input.lineage.source_run() != input.owner_run {
            return Err(PortError::new(
                "artifact_lineage_mismatch",
                "lineage source run must match artifact owner run",
                false,
            ));
        }
        Ok(Self {
            artifact_id: input.artifact_id,
            owner_run: input.owner_run,
            kind: input.kind,
            schema_version: input.schema_version,
            content_digest: input.content_digest,
            byte_length,
            producer: input.producer,
            lineage: input.lineage,
        })
    }

    #[must_use]
    pub const fn artifact_id(&self) -> ArtifactId {
        self.artifact_id
    }

    #[must_use]
    pub const fn owner_run(&self) -> RunId {
        self.owner_run
    }

    #[must_use]
    pub const fn kind(&self) -> ArtifactKind {
        self.kind
    }

    #[must_use]
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    #[must_use]
    pub fn content_digest(&self) -> &Digest {
        &self.content_digest
    }

    #[must_use]
    pub const fn byte_length(&self) -> u64 {
        self.byte_length
    }

    #[must_use]
    pub fn producer(&self) -> &str {
        &self.producer
    }

    #[must_use]
    pub fn lineage(&self) -> &ArtifactLineage {
        &self.lineage
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDraft {
    metadata: ArtifactMetadata,
    bytes: Vec<u8>,
}

impl ArtifactDraft {
    pub fn new(metadata: ArtifactMetadata, bytes: Vec<u8>) -> Result<Self, PortError> {
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(PortError::new(
                "artifact_too_large",
                "artifact content exceeds its bound",
                false,
            ));
        }
        if metadata.byte_length() != bytes.len() as u64 {
            return Err(PortError::new(
                "artifact_length_mismatch",
                "artifact metadata length does not match content",
                false,
            ));
        }
        if metadata.content_digest().as_str() != format!("{:x}", Sha256::digest(&bytes)) {
            return Err(PortError::new(
                "artifact_digest_mismatch",
                "artifact digest does not match content",
                false,
            ));
        }
        Ok(Self { metadata, bytes })
    }

    #[must_use]
    pub fn metadata(&self) -> &ArtifactMetadata {
        &self.metadata
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactReceipt {
    metadata: ArtifactMetadata,
    was_published: bool,
}

impl ArtifactReceipt {
    #[must_use]
    pub const fn new(metadata: ArtifactMetadata, was_published: bool) -> Self {
        Self {
            metadata,
            was_published,
        }
    }

    #[must_use]
    pub fn metadata(&self) -> &ArtifactMetadata {
        &self.metadata
    }

    #[must_use]
    pub const fn was_published(&self) -> bool {
        self.was_published
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactPublicationRequest {
    kind: ArtifactKind,
    schema_version: SchemaVersion,
    content_digest: Digest,
    producer: String,
    bytes: Vec<u8>,
    lineage: ArtifactLineage,
}

impl ArtifactPublicationRequest {
    #[must_use]
    pub fn new(
        kind: ArtifactKind,
        schema_version: SchemaVersion,
        content_digest: Digest,
        producer: impl Into<String>,
        bytes: Vec<u8>,
        lineage: ArtifactLineage,
    ) -> Self {
        Self {
            kind,
            schema_version,
            content_digest,
            producer: producer.into(),
            bytes,
            lineage,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> ArtifactKind {
        self.kind
    }

    #[must_use]
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    #[must_use]
    pub fn content_digest(&self) -> &Digest {
        &self.content_digest
    }

    #[must_use]
    pub fn producer(&self) -> &str {
        &self.producer
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn lineage(&self) -> &ArtifactLineage {
        &self.lineage
    }
}

pub trait ArtifactPort {
    fn publish(&mut self, draft: ArtifactDraft) -> Result<ArtifactReceipt, PortError>;

    fn close(&mut self) -> Result<(), PortError>;
}
