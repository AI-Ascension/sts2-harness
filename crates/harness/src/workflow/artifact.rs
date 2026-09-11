// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::canonical::CanonicalError;
use super::decoder::{DecodeError, decode_strict};
use super::definition::{WORKFLOW_SCHEMA_VERSION, WorkflowDefinition, WorkflowSchemaVersion};
use super::ids::{CompilerId, Digest, ProducerId, SemanticVersion, WorkflowId};
use super::validation::{ValidationError, validate_definition};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactError {
    Decode(DecodeError),
    Validation(ValidationError),
    Canonical(CanonicalError),
    Serialization,
    DigestMismatch,
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Decode(_) => "workflow artifact decoding failed",
            Self::Validation(_) => "workflow artifact validation failed",
            Self::Canonical(_) => "workflow artifact canonicalization failed",
            Self::Serialization => "workflow artifact serialization failed",
            Self::DigestMismatch => "workflow artifact digest mismatch",
        })
    }
}

impl std::error::Error for ArtifactError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactManifest {
    pub artifact: WorkflowSchemaVersion,
    pub workflow_id: WorkflowId,
    pub version: SemanticVersion,
    pub source_digest: Digest,
    pub semantic_digest: Digest,
    pub producer: ProducerId,
    pub compiler: CompilerId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowArtifact {
    definition: WorkflowDefinition,
    source_bytes: Vec<u8>,
    source_digest: Digest,
    semantic_digest: Digest,
    producer: ProducerId,
    compiler: CompilerId,
}

impl WorkflowArtifact {
    pub fn from_json(
        source_bytes: &[u8],
        producer: ProducerId,
        compiler: CompilerId,
    ) -> Result<Self, ArtifactError> {
        let definition: WorkflowDefinition =
            decode_strict(source_bytes).map_err(ArtifactError::Decode)?;
        validate_definition(&definition).map_err(ArtifactError::Validation)?;
        let semantic_digest = definition
            .semantic_digest()
            .map_err(ArtifactError::Canonical)?;
        Ok(Self {
            definition,
            source_bytes: source_bytes.to_vec(),
            source_digest: Digest::sha256(source_bytes),
            semantic_digest,
            producer,
            compiler,
        })
    }

    pub fn from_definition(
        definition: WorkflowDefinition,
        producer: ProducerId,
        compiler: CompilerId,
    ) -> Result<Self, ArtifactError> {
        validate_definition(&definition).map_err(ArtifactError::Validation)?;
        let source_bytes =
            serde_json::to_vec(&definition).map_err(|_| ArtifactError::Serialization)?;
        let semantic_digest = definition
            .semantic_digest()
            .map_err(ArtifactError::Canonical)?;
        Ok(Self {
            definition,
            source_digest: Digest::sha256(&source_bytes),
            source_bytes,
            semantic_digest,
            producer,
            compiler,
        })
    }

    pub fn consume(&self, expected_digest: &Digest) -> Result<&WorkflowDefinition, ArtifactError> {
        if &self.semantic_digest != expected_digest {
            return Err(ArtifactError::DigestMismatch);
        }
        Ok(&self.definition)
    }

    pub fn manifest(&self) -> ArtifactManifest {
        ArtifactManifest {
            artifact: WorkflowSchemaVersion::V1,
            workflow_id: self.definition.workflow_id.clone(),
            version: self.definition.version.clone(),
            source_digest: self.source_digest.clone(),
            semantic_digest: self.semantic_digest.clone(),
            producer: self.producer.clone(),
            compiler: self.compiler.clone(),
        }
    }

    pub fn definition(&self) -> &WorkflowDefinition {
        &self.definition
    }

    pub fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }

    pub fn source_digest(&self) -> &Digest {
        &self.source_digest
    }

    pub fn semantic_digest(&self) -> &Digest {
        &self.semantic_digest
    }

    pub fn producer(&self) -> &ProducerId {
        &self.producer
    }

    pub fn compiler(&self) -> &CompilerId {
        &self.compiler
    }

    pub fn contract_version(&self) -> &'static str {
        WORKFLOW_SCHEMA_VERSION
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogInsert {
    Inserted,
    AlreadyPresent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    ImmutableConflict,
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("workflow identity/version already has different semantic bytes")
    }
}

impl std::error::Error for CatalogError {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkflowCatalog {
    artifacts: BTreeMap<(WorkflowId, SemanticVersion), WorkflowArtifact>,
}

impl WorkflowCatalog {
    pub fn publish(&mut self, artifact: WorkflowArtifact) -> Result<CatalogInsert, CatalogError> {
        let key = (
            artifact.definition.workflow_id.clone(),
            artifact.definition.version.clone(),
        );
        if let Some(existing) = self.artifacts.get(&key) {
            if existing.semantic_digest() == artifact.semantic_digest() {
                return Ok(CatalogInsert::AlreadyPresent);
            }
            return Err(CatalogError::ImmutableConflict);
        }
        self.artifacts.insert(key, artifact);
        Ok(CatalogInsert::Inserted)
    }

    pub fn get(
        &self,
        workflow_id: &WorkflowId,
        version: &SemanticVersion,
    ) -> Option<&WorkflowArtifact> {
        self.artifacts.get(&(workflow_id.clone(), version.clone()))
    }

    pub fn len(&self) -> usize {
        self.artifacts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }
}
