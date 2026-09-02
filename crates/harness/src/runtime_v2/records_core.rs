// SPDX-License-Identifier: MIT

/// The action lifecycle outcomes remain distinct in records.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeV2EventKind {
    Observation,
    Requested,
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Reconciled,
    DuplicateReplay,
}

/// The v1-compatible record semantic attached to a Runtime-v2 event.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeV2RecordKind {
    Observation,
    ActionRequested,
    ActionAccepted,
    ActionCompleted,
    Marker,
}

/// Evidence that the harness did not blind-retry an admitted operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuntimeV2NoRetryEvidence {
    no_blind_retry_after_disconnect: bool,
    retry_attempts: u8,
    mutation_attempts: u8,
    disconnect_after_write: bool,
    reconcile_operation_id: Option<RuntimeV2OperationId>,
}

impl RuntimeV2NoRetryEvidence {
    /// Constructs bounded retry evidence for one trajectory record.
    #[must_use]
    pub fn new(
        no_blind_retry_after_disconnect: bool,
        retry_attempts: u8,
        mutation_attempts: u8,
        disconnect_after_write: bool,
        reconcile_operation_id: Option<RuntimeV2OperationId>,
    ) -> Self {
        Self {
            no_blind_retry_after_disconnect,
            retry_attempts,
            mutation_attempts,
            disconnect_after_write,
            reconcile_operation_id,
        }
    }

    /// Returns whether the no-blind-retry rule was followed.
    #[must_use]
    pub const fn no_blind_retry_after_disconnect(&self) -> bool {
        self.no_blind_retry_after_disconnect
    }

    /// Returns the number of retry submissions.
    #[must_use]
    pub const fn retry_attempts(&self) -> u8 {
        self.retry_attempts
    }

    /// Returns the number of fake mutations attempted for the operation.
    #[must_use]
    pub const fn mutation_attempts(&self) -> u8 {
        self.mutation_attempts
    }

    /// Returns whether this record represents the simulated post-write disconnect.
    #[must_use]
    pub const fn disconnect_after_write(&self) -> bool {
        self.disconnect_after_write
    }

    /// Returns the operation reconciled by this evidence, when present.
    #[must_use]
    pub fn reconcile_operation_id(&self) -> Option<&RuntimeV2OperationId> {
        self.reconcile_operation_id.as_ref()
    }
}

/// A bounded, flattened Runtime-v2 trajectory record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuntimeV2Record {
    sequence: u16,
    event_kind: RuntimeV2EventKind,
    record_kind: RuntimeV2RecordKind,
    #[serde(flatten)]
    message: RuntimeV2Message,
    no_retry: RuntimeV2NoRetryEvidence,
}

impl RuntimeV2Record {
    /// Creates a record while enforcing message and sequence bounds.
    pub fn new(
        sequence: u16,
        event_kind: RuntimeV2EventKind,
        record_kind: RuntimeV2RecordKind,
        message: RuntimeV2Message,
        no_retry: RuntimeV2NoRetryEvidence,
    ) -> Result<Self, RuntimeV2Error> {
        if usize::from(sequence) >= RUNTIME_V2_MAX_RECORDS {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 trajectory record exceeds its bound",
            ));
        }
        message.validate()?;
        Ok(Self {
            sequence,
            event_kind,
            record_kind,
            message,
            no_retry,
        })
    }

    /// Returns the bounded record sequence.
    #[must_use]
    pub const fn sequence(&self) -> u16 {
        self.sequence
    }

    /// Returns the lifecycle event kind.
    #[must_use]
    pub const fn event_kind(&self) -> RuntimeV2EventKind {
        self.event_kind
    }

    /// Returns the v1-compatible record semantic.
    #[must_use]
    pub const fn record_kind(&self) -> RuntimeV2RecordKind {
        self.record_kind
    }

    /// Returns the flattened wire message.
    #[must_use]
    pub fn message(&self) -> &RuntimeV2Message {
        &self.message
    }

    /// Returns no-retry evidence for the event.
    #[must_use]
    pub fn no_retry(&self) -> &RuntimeV2NoRetryEvidence {
        &self.no_retry
    }
}

/// Protocol/artifact lineage attached to the trajectory and artifact record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuntimeV2ArtifactLineage {
    artifact: String,
    protocol_version: String,
    schema_digest: String,
    provenance: RuntimeV2Provenance,
    source_trajectory_id: String,
}

impl RuntimeV2ArtifactLineage {
    fn new(source_trajectory_id: &str) -> Result<Self, RuntimeV2Error> {
        validate_identity(source_trajectory_id)?;
        Ok(Self {
            artifact: RUNTIME_V2_ARTIFACT.to_owned(),
            protocol_version: RUNTIME_V2_PROTOCOL_VERSION.to_owned(),
            schema_digest: RUNTIME_V2_SCHEMA_DIGEST.to_owned(),
            provenance: RuntimeV2Provenance::default(),
            source_trajectory_id: source_trajectory_id.to_owned(),
        })
    }

    /// Returns the release-like artifact identity.
    #[must_use]
    pub fn artifact(&self) -> &str {
        &self.artifact
    }

    /// Returns the copied schema digest.
    #[must_use]
    pub fn schema_digest(&self) -> &str {
        &self.schema_digest
    }

    /// Returns the source trajectory identity.
    #[must_use]
    pub fn source_trajectory_id(&self) -> &str {
        &self.source_trajectory_id
    }
}

/// A published-looking artifact record for the deterministic trajectory bytes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuntimeV2ArtifactRecord {
    artifact_id: String,
    kind: &'static str,
    content_digest: String,
    byte_length: u64,
    schema_bytes_verified: bool,
    schema_bytes_digest: String,
    lineage: RuntimeV2ArtifactLineage,
}

impl RuntimeV2ArtifactRecord {
    fn new(
        artifact_id: &str,
        content_digest: String,
        byte_length: u64,
        lineage: RuntimeV2ArtifactLineage,
    ) -> Result<Self, RuntimeV2Error> {
        validate_identity(artifact_id)?;
        Ok(Self {
            artifact_id: artifact_id.to_owned(),
            kind: "trajectory",
            content_digest,
            byte_length,
            schema_bytes_verified: true,
            schema_bytes_digest: RUNTIME_V2_SCHEMA_DIGEST.to_owned(),
            lineage,
        })
    }

    /// Returns the artifact identity.
    #[must_use]
    pub fn artifact_id(&self) -> &str {
        &self.artifact_id
    }

    /// Returns the trajectory content digest.
    #[must_use]
    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }

    /// Returns the copied schema digest bound to this artifact.
    #[must_use]
    pub fn schema_bytes_digest(&self) -> &str {
        &self.schema_bytes_digest
    }

    /// Returns whether the copied schema bytes passed verification.
    #[must_use]
    pub const fn schema_bytes_verified(&self) -> bool {
        self.schema_bytes_verified
    }

    /// Returns artifact lineage.
    #[must_use]
    pub fn lineage(&self) -> &RuntimeV2ArtifactLineage {
        &self.lineage
    }
}
