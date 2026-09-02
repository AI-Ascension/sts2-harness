// SPDX-License-Identifier: MIT

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuntimeV2Evidence {
    operation_id: RuntimeV2OperationId,
    initial_generation: u64,
    settled_generation: u64,
    mutation_count: u16,
    duplicate_replay_without_second_application: bool,
    stale_epoch_rejected: bool,
    no_blind_retry_after_disconnect: bool,
    live_host_settlement: &'static str,
    provider_model_lane: &'static str,
}

impl RuntimeV2Evidence {
    /// Returns the stable operation identity.
    #[must_use]
    pub fn operation_id(&self) -> &RuntimeV2OperationId {
        &self.operation_id
    }

    /// Returns the state generation before submission.
    #[must_use]
    pub const fn initial_generation(&self) -> u64 {
        self.initial_generation
    }

    /// Returns the fresh settled generation.
    #[must_use]
    pub const fn settled_generation(&self) -> u64 {
        self.settled_generation
    }

    /// Returns the number of fake mutations.
    #[must_use]
    pub const fn mutation_count(&self) -> u16 {
        self.mutation_count
    }

    /// Returns whether duplicate replay avoided a second mutation.
    #[must_use]
    pub const fn duplicate_replay_without_second_application(&self) -> bool {
        self.duplicate_replay_without_second_application
    }

    /// Returns whether the stale epoch was rejected before mutation.
    #[must_use]
    pub const fn stale_epoch_rejected(&self) -> bool {
        self.stale_epoch_rejected
    }

    /// Returns whether no blind retry followed the disconnect.
    #[must_use]
    pub const fn no_blind_retry_after_disconnect(&self) -> bool {
        self.no_blind_retry_after_disconnect
    }

    /// Returns the live host evidence state.
    #[must_use]
    pub const fn live_host_settlement(&self) -> &str {
        self.live_host_settlement
    }

    /// Returns the provider/model evidence state.
    #[must_use]
    pub const fn provider_model_lane(&self) -> &str {
        self.provider_model_lane
    }
}

/// The bounded trajectory emitted by one fake runtime instance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuntimeV2Trajectory {
    trajectory_id: String,
    instance_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
    artifact_lineage: RuntimeV2ArtifactLineage,
    records: Vec<RuntimeV2Record>,
}

impl RuntimeV2Trajectory {
    fn new(
        trajectory_id: &str,
        context: &RuntimeV2Context,
        artifact_lineage: RuntimeV2ArtifactLineage,
        records: Vec<RuntimeV2Record>,
    ) -> Result<Self, RuntimeV2Error> {
        validate_identity(trajectory_id)?;
        context.validate()?;
        if records.is_empty() || records.len() > RUNTIME_V2_MAX_RECORDS {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 trajectory record count is outside its bound",
            ));
        }
        for (index, record) in records.iter().enumerate() {
            if usize::from(record.sequence()) != index {
                return Err(RuntimeV2Error::Invalid(
                    "Runtime-v2 trajectory sequence is not contiguous",
                ));
            }
            if record.message().instance_id() != context.instance_id()
                || record.message().session_id() != context.session_id()
                || record.message().lease_id() != context.lease_id()
            {
                return Err(RuntimeV2Error::Invalid(
                    "Runtime-v2 trajectory crosses an identity boundary",
                ));
            }
        }
        Ok(Self {
            trajectory_id: trajectory_id.to_owned(),
            instance_id: context.instance_id.clone(),
            session_id: context.session_id.clone(),
            lease_id: context.lease_id.clone(),
            lease_epoch: context.lease_epoch,
            artifact_lineage,
            records,
        })
    }

    /// Returns the trajectory identity.
    #[must_use]
    pub fn trajectory_id(&self) -> &str {
        &self.trajectory_id
    }

    /// Returns the bound instance identity.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Returns the bound session identity.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Returns the bound lease identity.
    #[must_use]
    pub fn lease_id(&self) -> &str {
        &self.lease_id
    }

    /// Returns the current lease epoch.
    #[must_use]
    pub const fn lease_epoch(&self) -> u64 {
        self.lease_epoch
    }

    /// Returns protocol/artifact lineage.
    #[must_use]
    pub fn artifact_lineage(&self) -> &RuntimeV2ArtifactLineage {
        &self.artifact_lineage
    }

    /// Returns the bounded records.
    #[must_use]
    pub fn records(&self) -> &[RuntimeV2Record] {
        &self.records
    }
}

/// The complete result of the deterministic Runtime-v2 fake lifecycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV2Report {
    trace_bytes: String,
    trajectory: RuntimeV2Trajectory,
    artifact: RuntimeV2ArtifactRecord,
    evidence: RuntimeV2Evidence,
}

impl RuntimeV2Report {
    /// Returns the canonical JSON trace emitted by the fake binary.
    #[must_use]
    pub fn trace_bytes(&self) -> &str {
        &self.trace_bytes
    }

    /// Returns the bounded trajectory.
    #[must_use]
    pub fn trajectory(&self) -> &RuntimeV2Trajectory {
        &self.trajectory
    }

    /// Returns the artifact record bound to the trajectory bytes.
    #[must_use]
    pub fn artifact(&self) -> &RuntimeV2ArtifactRecord {
        &self.artifact
    }

    /// Returns the deterministic evidence summary.
    #[must_use]
    pub fn evidence(&self) -> &RuntimeV2Evidence {
        &self.evidence
    }
}
