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
    pub unknown: u64,
    pub cancelled: u64,
    pub rejected: u64,
    pub service_time_samples: u64,
    pub service_time_total_millis: u64,
    pub service_time_max_millis: u64,
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
    pub unknown: u64,
    pub cancelled: u64,
    pub rejected: u64,
    pub service_time_samples: u64,
    pub service_time_total_millis: u64,
    pub service_time_max_millis: u64,
    pub retained: usize,
    pub instances: Vec<RuntimeV2InstanceSnapshot>,
}

impl RuntimeV2Coordinator {
    /// Records an outcome; `Unknown` retains the active lane until explicit reconciliation.
    pub fn complete(
        &mut self,
        operation_id: &RuntimeV2OperationId,
        status: RuntimeV2Status,
    ) -> Result<RuntimeV2WorkItem, RuntimeV2CoordinatorError> {
        self.complete_internal(operation_id, status, None)
    }

    /// Completes an active operation and records its bounded service time in milliseconds.
    ///
    /// The dispatcher owns the clock and supplies the elapsed duration only after the
    /// downstream outcome is known. A timeout or disconnect must therefore be completed
    /// as `Unknown`, never retried implicitly by this coordinator. Unknown outcomes retain
    /// the active operation and block the next mutation until a terminal outcome arrives.
    pub fn complete_with_service_time(
        &mut self,
        operation_id: &RuntimeV2OperationId,
        status: RuntimeV2Status,
        service_time_millis: u64,
    ) -> Result<RuntimeV2WorkItem, RuntimeV2CoordinatorError> {
        self.complete_internal(operation_id, status, Some(service_time_millis))
    }

    fn complete_internal(
        &mut self,
        operation_id: &RuntimeV2OperationId,
        status: RuntimeV2Status,
        service_time_millis: Option<u64>,
    ) -> Result<RuntimeV2WorkItem, RuntimeV2CoordinatorError> {
        if matches!(status, RuntimeV2Status::Accepted) {
            return Err(RuntimeV2CoordinatorError::InvalidCompletion);
        }
        if status == RuntimeV2Status::Unknown {
            let item = self
                .active_operations
                .get(operation_id)
                .ok_or(RuntimeV2CoordinatorError::UnknownOperation)?
                .clone();
            if self.uncertain_operations.insert(operation_id.clone()) {
                self.unknown = self.unknown.saturating_add(1);
                if let Some(lane) = self.lanes.get_mut(item.binding().instance_id()) {
                    lane.unknown = lane.unknown.saturating_add(1);
                }
            }
            return Ok(item);
        }
        let item = match self.active_operations.remove(operation_id) {
            Some(item) => item,
            None => return Err(RuntimeV2CoordinatorError::UnknownOperation),
        };
        self.uncertain_operations.remove(operation_id);
        let instance_id = item.binding().instance_id().to_owned();
        if let Some(lane) = self.lanes.get_mut(&instance_id) {
            lane.active = false;
            lane.completed = lane.completed.saturating_add(1);
            match status {
                RuntimeV2Status::Unknown => lane.unknown = lane.unknown.saturating_add(1),
                RuntimeV2Status::Cancelled => lane.cancelled = lane.cancelled.saturating_add(1),
                RuntimeV2Status::Rejected => lane.rejected = lane.rejected.saturating_add(1),
                RuntimeV2Status::Settled => {}
                RuntimeV2Status::Accepted => {}
            }
            if let Some(service_time_millis) = service_time_millis {
                lane.service_time_samples = lane.service_time_samples.saturating_add(1);
                lane.service_time_total_millis = lane
                    .service_time_total_millis
                    .saturating_add(service_time_millis);
                lane.service_time_max_millis =
                    lane.service_time_max_millis.max(service_time_millis);
            }
            if !lane.queue.is_empty() {
                self.enqueue_ready(instance_id);
            }
        }
        self.completed = self.completed.saturating_add(1);
        match status {
            RuntimeV2Status::Unknown => self.unknown = self.unknown.saturating_add(1),
            RuntimeV2Status::Cancelled => self.cancelled = self.cancelled.saturating_add(1),
            RuntimeV2Status::Rejected => self.rejected = self.rejected.saturating_add(1),
            RuntimeV2Status::Settled => {}
            RuntimeV2Status::Accepted => {}
        }
        if let Some(service_time_millis) = service_time_millis {
            self.service_time_samples = self.service_time_samples.saturating_add(1);
            self.service_time_total_millis = self
                .service_time_total_millis
                .saturating_add(service_time_millis);
            self.service_time_max_millis = self.service_time_max_millis.max(service_time_millis);
        }
        self.retain_operation(operation_id);
        Ok(item)
    }
}
