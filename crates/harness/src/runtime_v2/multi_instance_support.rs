// SPDX-License-Identifier: MIT

impl RuntimeV2Coordinator {
    fn retain_operation(&mut self, operation_id: &RuntimeV2OperationId) {
        if self.retained_operations.len() >= RUNTIME_V2_MAX_RETAINED_OPERATIONS {
            let _ = self.retained_operations.pop_front();
        }
        self.retained_operations.push_back(operation_id.clone());
    }

    fn namespace_conflicts(&self, binding: &RuntimeV2InstanceBinding) -> bool {
        self.lanes.values().any(|lane| {
            let existing = &lane.binding;
            existing.gateway_session_id() == binding.gateway_session_id()
                || existing.mcp_session_id() == binding.mcp_session_id()
                || existing.lease_id() == binding.lease_id()
                || existing.process_port() == binding.process_port()
                || existing.run_id() == binding.run_id()
                || existing.episode_id() == binding.episode_id()
                || existing.trajectory_id() == binding.trajectory_id()
                || existing.trace_id() == binding.trace_id()
                || existing.artifact_id() == binding.artifact_id()
        })
    }

    fn enqueue_ready(&mut self, instance_id: String) {
        if !self.ready.contains(&instance_id) {
            self.ready.push_back(instance_id);
        }
    }

    fn record_rejection(&mut self, instance_id: Option<&str>) {
        self.rejected = self.rejected.saturating_add(1);
        if let Some(instance_id) = instance_id
            && let Some(lane) = self.lanes.get_mut(instance_id)
        {
            lane.rejected = lane.rejected.saturating_add(1);
        }
    }
}

// SPDX-License-Identifier: MIT

/// The result of shutdown: queued work is explicitly cancelled; active work remains uncertain
/// until its downstream boundary reports a terminal result or reconciliation outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV2ShutdownReport {
    cancelled: Vec<RuntimeV2WorkItem>,
    active_operations: Vec<RuntimeV2OperationId>,
}

impl RuntimeV2ShutdownReport {
    /// Returns the queued work resolved as cancelled during shutdown.
    #[must_use]
    pub fn cancelled(&self) -> &[RuntimeV2WorkItem] {
        &self.cancelled
    }

    /// Returns active operation IDs that require downstream settlement/reconciliation.
    #[must_use]
    pub fn active_operations(&self) -> &[RuntimeV2OperationId] {
        &self.active_operations
    }
}

/// A sanitized per-instance queue/lifecycle snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV2InstanceSnapshot {
    pub instance_id: String,
    pub process_port: u16,
    pub queued: usize,
    pub active: bool,
    pub admitted: u64,
    pub completed: u64,
    pub cancelled: u64,
    pub rejected: u64,
}

/// A sanitized global and per-instance coordinator snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV2CoordinatorSnapshot {
    pub max_instances: usize,
    pub global_queue_capacity: usize,
    pub per_instance_queue_capacity: usize,
    pub admission_open: bool,
    pub queued: usize,
    pub active: usize,
    pub admitted: u64,
    pub completed: u64,
    pub cancelled: u64,
    pub rejected: u64,
    pub retained: usize,
    pub instances: Vec<RuntimeV2InstanceSnapshot>,
}
