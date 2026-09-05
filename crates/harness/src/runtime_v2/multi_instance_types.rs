// SPDX-License-Identifier: MIT

use crate::identity::{ArtifactId, EpisodeId, RequestId, RunId, TraceId, TrajectoryId};

/// The maximum number of game-instance lanes the harness may coordinate.
pub const RUNTIME_V2_MAX_INSTANCES: usize = 4;

/// The maximum configured waiting capacity for one Runtime-v2 queue.
pub const RUNTIME_V2_MAX_QUEUE_CAPACITY: usize = 64;

/// The bounded number of terminal operation IDs remembered by the coordinator.
pub const RUNTIME_V2_MAX_RETAINED_OPERATIONS: usize = 256;

/// A bounded coordinator configuration. The harness owns these waiting queues;
/// gateway leases and game-process lifecycle remain outside this type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeV2CoordinatorConfig {
    max_instances: usize,
    global_queue_capacity: usize,
    per_instance_queue_capacity: usize,
}

impl RuntimeV2CoordinatorConfig {
    /// Creates a configuration for at most four isolated instance lanes.
    pub fn new(
        max_instances: usize,
        global_queue_capacity: usize,
        per_instance_queue_capacity: usize,
    ) -> Result<Self, RuntimeV2CoordinatorError> {
        if !(1..=RUNTIME_V2_MAX_INSTANCES).contains(&max_instances)
            || !(1..=RUNTIME_V2_MAX_QUEUE_CAPACITY).contains(&global_queue_capacity)
            || !(1..=RUNTIME_V2_MAX_QUEUE_CAPACITY).contains(&per_instance_queue_capacity)
        {
            return Err(RuntimeV2CoordinatorError::InvalidConfig);
        }
        Ok(Self {
            max_instances,
            global_queue_capacity,
            per_instance_queue_capacity,
        })
    }

    /// Returns the configured instance limit.
    #[must_use]
    pub const fn max_instances(self) -> usize {
        self.max_instances
    }

    /// Returns the global waiting capacity.
    #[must_use]
    pub const fn global_queue_capacity(self) -> usize {
        self.global_queue_capacity
    }

    /// Returns the per-instance waiting capacity.
    #[must_use]
    pub const fn per_instance_queue_capacity(self) -> usize {
        self.per_instance_queue_capacity
    }
}

/// Errors returned by the pure Runtime-v2 coordinator seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV2CoordinatorError {
    InvalidConfig,
    InvalidIdentity,
    InvalidProcessPort,
    AdmissionClosed,
    InstanceLimit,
    DuplicateInstance,
    NamespaceConflict,
    UnknownInstance,
    BindingMismatch,
    GlobalQueueFull,
    InstanceQueueFull,
    OperationInFlight,
    OperationRetained,
    UnknownOperation,
    InvalidCompletion,
}

impl std::fmt::Display for RuntimeV2CoordinatorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidConfig => "Runtime-v2 coordinator configuration is outside its bounds",
            Self::InvalidIdentity => "Runtime-v2 coordinator identity is invalid",
            Self::InvalidProcessPort => "Runtime-v2 process port must be nonzero",
            Self::AdmissionClosed => "Runtime-v2 coordinator admission is closed",
            Self::InstanceLimit => "Runtime-v2 coordinator instance limit was reached",
            Self::DuplicateInstance => "Runtime-v2 instance is already registered",
            Self::NamespaceConflict => "Runtime-v2 identity is already owned by another instance",
            Self::UnknownInstance => "Runtime-v2 instance is not registered",
            Self::BindingMismatch => "Runtime-v2 work binding does not match its registered lane",
            Self::GlobalQueueFull => "Runtime-v2 global waiting queue is full",
            Self::InstanceQueueFull => "Runtime-v2 instance waiting queue is full",
            Self::OperationInFlight => "Runtime-v2 operation is already queued or active",
            Self::OperationRetained => {
                "Runtime-v2 operation identity is retained and cannot be reused"
            }
            Self::UnknownOperation => "Runtime-v2 operation is not active",
            Self::InvalidCompletion => "Runtime-v2 completion must be a terminal outcome",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for RuntimeV2CoordinatorError {}

/// Harness-owned lineage for one allocated instance lane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV2InstanceBinding {
    context: RuntimeV2Context,
    mcp_session_id: String,
    caller_id: String,
    process_port: u16,
    run_id: RunId,
    episode_id: EpisodeId,
    trajectory_id: TrajectoryId,
    trace_id: TraceId,
    artifact_id: ArtifactId,
}

impl RuntimeV2InstanceBinding {
    /// Creates an explicit, independently named instance lineage.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        context: RuntimeV2Context,
        mcp_session_id: impl Into<String>,
        caller_id: impl Into<String>,
        process_port: u16,
        run_id: RunId,
        episode_id: EpisodeId,
        trajectory_id: TrajectoryId,
        trace_id: TraceId,
        artifact_id: ArtifactId,
    ) -> Result<Self, RuntimeV2CoordinatorError> {
        let binding = Self {
            context,
            mcp_session_id: mcp_session_id.into(),
            caller_id: caller_id.into(),
            process_port,
            run_id,
            episode_id,
            trajectory_id,
            trace_id,
            artifact_id,
        };
        binding.validate()?;
        Ok(binding)
    }

    fn validate(&self) -> Result<(), RuntimeV2CoordinatorError> {
        for value in [
            self.context.instance_id(),
            self.context.session_id(),
            self.context.lease_id(),
            self.mcp_session_id.as_str(),
            self.caller_id.as_str(),
        ] {
            if validate_identity(value).is_err() {
                return Err(RuntimeV2CoordinatorError::InvalidIdentity);
            }
        }
        if self.process_port == 0 {
            return Err(RuntimeV2CoordinatorError::InvalidProcessPort);
        }
        Ok(())
    }

    /// Returns the Runtime-v2 instance/lease fence.
    #[must_use]
    pub fn context(&self) -> &RuntimeV2Context {
        &self.context
    }

    /// Returns the gateway-routed instance identity.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        self.context.instance_id()
    }

    /// Returns the gateway session identity.
    #[must_use]
    pub fn gateway_session_id(&self) -> &str {
        self.context.session_id()
    }

    /// Returns the independently tracked MCP session identity.
    #[must_use]
    pub fn mcp_session_id(&self) -> &str {
        &self.mcp_session_id
    }

    /// Returns the harness caller identity.
    #[must_use]
    pub fn caller_id(&self) -> &str {
        &self.caller_id
    }

    /// Returns the allocated downstream process port reference.
    #[must_use]
    pub const fn process_port(&self) -> u16 {
        self.process_port
    }

    /// Returns the lease identity.
    #[must_use]
    pub fn lease_id(&self) -> &str {
        self.context.lease_id()
    }

    /// Returns the lease epoch.
    #[must_use]
    pub const fn lease_epoch(&self) -> u64 {
        self.context.lease_epoch()
    }

    /// Returns the run identity.
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    /// Returns the episode identity.
    #[must_use]
    pub const fn episode_id(&self) -> EpisodeId {
        self.episode_id
    }

    /// Returns the trajectory identity.
    #[must_use]
    pub const fn trajectory_id(&self) -> TrajectoryId {
        self.trajectory_id
    }

    /// Returns the trace identity.
    #[must_use]
    pub const fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    /// Returns the artifact identity.
    #[must_use]
    pub const fn artifact_id(&self) -> ArtifactId {
        self.artifact_id
    }
}

/// One action waiting for the coordinator to dispatch it through MCP.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV2WorkItem {
    binding: RuntimeV2InstanceBinding,
    request_id: RequestId,
    operation_id: RuntimeV2OperationId,
    action: RuntimeV2Action,
}

impl RuntimeV2WorkItem {
    /// Builds a bounded action work item from a registered lineage.
    pub fn new(
        binding: &RuntimeV2InstanceBinding,
        request_id: RequestId,
        operation_id: RuntimeV2OperationId,
        action: RuntimeV2Action,
    ) -> Result<Self, RuntimeV2CoordinatorError> {
        if action.validate().is_err() {
            return Err(RuntimeV2CoordinatorError::InvalidIdentity);
        }
        Ok(Self {
            binding: binding.clone(),
            request_id,
            operation_id,
            action,
        })
    }

    /// Returns the complete independently named binding.
    #[must_use]
    pub fn binding(&self) -> &RuntimeV2InstanceBinding {
        &self.binding
    }

    /// Returns the request identity.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Returns the operation identity.
    #[must_use]
    pub fn operation_id(&self) -> &RuntimeV2OperationId {
        &self.operation_id
    }

    /// Returns the action kind to pass to MCP.
    #[must_use]
    pub fn action(&self) -> &RuntimeV2Action {
        &self.action
    }
}
