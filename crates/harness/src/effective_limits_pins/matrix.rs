// SPDX-License-Identifier: MIT

use super::types::{
    Adoption, ArtifactMode, ConsumerPin, EFFECTIVE_LIMIT_PIN_MATRIX_SCHEMA, PinMatrix,
    ProducerArtifact, ProducerSurface, matrix_json, producer_artifact_bytes, valid_date,
    valid_revision, valid_sha256, workflow_pin_matches,
};
use crate::effective_limits::{EffectiveLimitRecord, UnavailableReason};
use std::collections::BTreeSet;

/// Fail-closed conformance error for the producer/consumer pin matrix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PinMatrixError {
    InvalidMatrix,
    DuplicateConsumer,
    MissingProducerArtifact,
    ProducerArtifactDrift,
    ConsumerRevisionMalformed,
    ConsumerPinDrift,
    ConsumerAlignmentMismatch,
    AdoptionLabelStale,
    IncompleteProducerInventory,
}

impl PinMatrix {
    /// Load the repository-owned matrix committed with the harness.
    ///
    /// # Errors
    ///
    /// Returns a JSON error when the committed matrix is not readable as [`PinMatrix`].
    pub fn repository() -> Result<Self, serde_json::Error> {
        serde_json::from_str(matrix_json())
    }

    /// Validate producer digests, consumer pins, and adoption labels.
    ///
    /// # Errors
    ///
    /// Returns [`PinMatrixError`] on drift, tampering, staleness, or malformed entries.
    pub fn validate(&self) -> Result<(), PinMatrixError> {
        if self.schema != EFFECTIVE_LIMIT_PIN_MATRIX_SCHEMA
            || self.producer.repository != "AI-Ascension/sts2-harness"
            || self.producer.owner != "sts2-harness"
            || !valid_date(&self.recorded_on)
            || self.producer.surfaces.is_empty()
        {
            return Err(PinMatrixError::InvalidMatrix);
        }
        for surface in &self.producer.surfaces {
            validate_producer_surface(surface)?;
        }
        let mut repositories = BTreeSet::new();
        for consumer in &self.consumers {
            if !repositories.insert(consumer.repository.as_str()) {
                return Err(PinMatrixError::DuplicateConsumer);
            }
            self.validate_consumer(consumer)?;
        }
        Ok(())
    }

    fn validate_consumer(&self, consumer: &ConsumerPin) -> Result<(), PinMatrixError> {
        if consumer.role.trim().is_empty() || !valid_revision(&consumer.revision) {
            return Err(PinMatrixError::ConsumerRevisionMalformed);
        }
        let mut surfaces = BTreeSet::new();
        for surface in &consumer.surfaces {
            if !surfaces.insert(surface.surface.as_str()) {
                return Err(PinMatrixError::InvalidMatrix);
            }
        }
        if let Some(pin) = &consumer.harness_ci_pin {
            if !valid_revision(&pin.revision) {
                return Err(PinMatrixError::ConsumerRevisionMalformed);
            }
            if !workflow_pin_matches(&pin.workflow, &pin.revision) {
                return Err(PinMatrixError::ConsumerPinDrift);
            }
        }
        match (consumer.adoption, self.alignment_holds(consumer)) {
            (Adoption::Aligned, true) | (Adoption::Pending, false) => Ok(()),
            (Adoption::Aligned, false) => Err(PinMatrixError::ConsumerAlignmentMismatch),
            (Adoption::Pending, true) => Err(PinMatrixError::AdoptionLabelStale),
        }
    }

    /// True when every producer surface is declared with the current capability schema and
    /// effective-limit disclosure, and every consumer artifact digest matches its producer digest.
    #[must_use]
    pub fn alignment_holds(&self, consumer: &ConsumerPin) -> bool {
        if consumer.surfaces.len() != self.producer.surfaces.len() || consumer.artifacts.is_empty()
        {
            return false;
        }
        let surfaces_match = self.producer.surfaces.iter().all(|producer| {
            consumer
                .surfaces
                .iter()
                .find(|entry| entry.surface == producer.surface)
                .is_some_and(|entry| {
                    entry.effective_limits_advertised
                        && entry.advertised_capability_schema.as_deref()
                            == Some(producer.capability_schema.as_str())
                })
        });
        let artifacts_match = match consumer.artifact_mode {
            ArtifactMode::CopiedContracts => {
                let producer_paths = self
                    .producer
                    .surfaces
                    .iter()
                    .flat_map(|surface| &surface.artifacts)
                    .map(|artifact| artifact.path.as_str())
                    .collect::<BTreeSet<_>>();
                let consumer_paths = consumer
                    .artifacts
                    .iter()
                    .map(|artifact| artifact.path.as_str())
                    .collect::<BTreeSet<_>>();
                producer_paths == consumer_paths
                    && consumer.artifacts.iter().all(|artifact| {
                        self.producer_artifact(&artifact.path)
                            .is_some_and(|producer| producer.sha256 == artifact.sha256)
                    })
            }
            ArtifactMode::SchemaAdapter => true,
        };
        surfaces_match && artifacts_match
    }

    fn producer_artifact(&self, path: &str) -> Option<&ProducerArtifact> {
        self.producer
            .surfaces
            .iter()
            .flat_map(|surface| &surface.artifacts)
            .find(|artifact| artifact.path == path)
    }

    #[must_use]
    pub fn producer_surface(&self, surface: &str) -> Option<&ProducerSurface> {
        self.producer
            .surfaces
            .iter()
            .find(|entry| entry.surface == surface)
    }

    /// Verify a published record is bound to the matrix's trusted owner pins.
    ///
    /// # Errors
    ///
    /// Returns [`UnavailableReason::ProfileMismatch`] for another surface or capability schema, so
    /// a descriptor for a different revision cannot authorize a larger limit.
    pub fn validate_record(&self, record: &EffectiveLimitRecord) -> Result<(), UnavailableReason> {
        let producer = self
            .producer_surface(&record.surface)
            .ok_or(UnavailableReason::ProfileMismatch)?;
        if record.owner != self.producer.owner
            || record.capability_schema != producer.capability_schema
            || record.owner_revision != producer.capability_revision
        {
            return Err(UnavailableReason::ProfileMismatch);
        }
        Ok(())
    }

    /// Consumer-side admission: a value may be presented only when the pinned consumer can
    /// discover the effective limit and the record admits it. An absent surface is never unlimited.
    ///
    /// # Errors
    ///
    /// Returns [`UnavailableReason`] for an unknown consumer or surface, a pending adoption, or a
    /// value above the executable ceiling.
    pub fn admit_consumer(
        &self,
        repository: &str,
        surface: &str,
        record: &EffectiveLimitRecord,
        trusted: &EffectiveLimitRecord,
        field: &str,
        requested: u64,
    ) -> Result<(), UnavailableReason> {
        self.validate()
            .map_err(|_| UnavailableReason::ConsumerPinNotAdopted)?;
        let consumer = self
            .consumers
            .iter()
            .find(|entry| entry.repository == repository)
            .ok_or(UnavailableReason::ConsumerNotRecorded)?;
        let entry = consumer
            .surfaces
            .iter()
            .find(|entry| entry.surface == surface)
            .ok_or(UnavailableReason::FieldNotAdvertised)?;
        if consumer.adoption == Adoption::Pending {
            return Err(UnavailableReason::ConsumerPinNotAdopted);
        }
        let schema_matches = entry.advertised_capability_schema.as_deref()
            == Some(record.capability_schema.as_str());
        if !entry.effective_limits_advertised || !schema_matches {
            return Err(UnavailableReason::FieldNotAdvertised);
        }
        self.validate_record(record)?;
        self.validate_record(trusted)?;
        record.authenticate(trusted)?;
        trusted.admit(field, requested)
    }
}

fn validate_producer_surface(surface: &ProducerSurface) -> Result<(), PinMatrixError> {
    if surface.surface.is_empty() || surface.capability_schema.is_empty() {
        return Err(PinMatrixError::InvalidMatrix);
    }
    let mut paths = BTreeSet::new();
    for artifact in &surface.artifacts {
        let Some(bytes) = producer_artifact_bytes(&artifact.path) else {
            return Err(PinMatrixError::MissingProducerArtifact);
        };
        if !valid_sha256(&artifact.sha256) || crate::sha256_hex(bytes) != artifact.sha256 {
            return Err(PinMatrixError::ProducerArtifactDrift);
        }
        let expected_suffix = match artifact.kind.as_str() {
            "policy_schema" => "policy.schema.json",
            "capabilities_schema" => "capabilities.schema.json",
            _ => return Err(PinMatrixError::InvalidMatrix),
        };
        if !artifact.path.ends_with(expected_suffix) || !paths.insert(artifact.path.as_str()) {
            return Err(PinMatrixError::InvalidMatrix);
        }
    }
    let policy = surface
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "policy_schema")
        .count();
    let capabilities = surface
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "capabilities_schema")
        .count();
    if policy != 1 || capabilities != 1 || surface.artifacts.len() != 2 {
        return Err(PinMatrixError::IncompleteProducerInventory);
    }
    Ok(())
}
