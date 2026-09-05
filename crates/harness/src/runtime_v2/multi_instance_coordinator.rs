// SPDX-License-Identifier: MIT

use std::collections::{BTreeSet, VecDeque};

struct RuntimeV2Lane {
    binding: RuntimeV2InstanceBinding,
    queue: VecDeque<RuntimeV2WorkItem>,
    active: bool,
    admitted: u64,
    completed: u64,
    unknown: u64,
    cancelled: u64,
    rejected: u64,
    service_time_samples: u64,
    service_time_total_millis: u64,
    service_time_max_millis: u64,
}

/// A bounded scheduler with one serial action slot per game instance.
pub struct RuntimeV2Coordinator {
    config: RuntimeV2CoordinatorConfig,
    lanes: BTreeMap<String, RuntimeV2Lane>,
    ready: VecDeque<String>,
    queued_operations: BTreeSet<RuntimeV2OperationId>,
    retained_operations: VecDeque<RuntimeV2OperationId>,
    active_operations: BTreeMap<RuntimeV2OperationId, RuntimeV2WorkItem>,
    uncertain_operations: BTreeSet<RuntimeV2OperationId>,
    admission_open: bool,
    admitted: u64,
    completed: u64,
    unknown: u64,
    cancelled: u64,
    rejected: u64,
    service_time_samples: u64,
    service_time_total_millis: u64,
    service_time_max_millis: u64,
}

impl RuntimeV2Coordinator {
    /// Creates an empty coordinator with no network or process authority.
    pub fn new(config: RuntimeV2CoordinatorConfig) -> Self {
        Self {
            config,
            lanes: BTreeMap::new(),
            ready: VecDeque::new(),
            queued_operations: BTreeSet::new(),
            retained_operations: VecDeque::new(),
            active_operations: BTreeMap::new(),
            uncertain_operations: BTreeSet::new(),
            admission_open: true,
            admitted: 0,
            completed: 0,
            unknown: 0,
            cancelled: 0,
            rejected: 0,
            service_time_samples: 0,
            service_time_total_millis: 0,
            service_time_max_millis: 0,
        }
    }

    /// Registers one instance and rejects reused cross-instance lineage.
    pub fn register_instance(
        &mut self,
        binding: RuntimeV2InstanceBinding,
    ) -> Result<(), RuntimeV2CoordinatorError> {
        if !self.admission_open {
            return Err(RuntimeV2CoordinatorError::AdmissionClosed);
        }
        if self.lanes.len() >= self.config.max_instances {
            return Err(RuntimeV2CoordinatorError::InstanceLimit);
        }
        if self.lanes.contains_key(binding.instance_id()) {
            return Err(RuntimeV2CoordinatorError::DuplicateInstance);
        }
        if self.namespace_conflicts(&binding) {
            return Err(RuntimeV2CoordinatorError::NamespaceConflict);
        }
        let instance_id = binding.instance_id().to_owned();
        self.lanes.insert(
            instance_id,
            RuntimeV2Lane {
                binding,
                queue: VecDeque::new(),
                active: false,
                admitted: 0,
                completed: 0,
                unknown: 0,
                cancelled: 0,
                rejected: 0,
                service_time_samples: 0,
                service_time_total_millis: 0,
                service_time_max_millis: 0,
            },
        );
        Ok(())
    }

    /// Admits work without dispatching it. Capacity failure is explicit and never forwarded.
    pub fn admit(&mut self, item: RuntimeV2WorkItem) -> Result<usize, RuntimeV2CoordinatorError> {
        if !self.admission_open {
            self.record_rejection(None);
            return Err(RuntimeV2CoordinatorError::AdmissionClosed);
        }
        let instance_id = item.binding().instance_id().to_owned();
        let lane_binding_matches = match self.lanes.get(&instance_id) {
            Some(lane) => lane.binding == *item.binding(),
            None => {
                self.record_rejection(Some(&instance_id));
                return Err(RuntimeV2CoordinatorError::UnknownInstance);
            }
        };
        if !lane_binding_matches {
            self.record_rejection(Some(&instance_id));
            return Err(RuntimeV2CoordinatorError::BindingMismatch);
        }
        if self.queued_operations.contains(item.operation_id())
            || self.active_operations.contains_key(item.operation_id())
        {
            self.record_rejection(Some(&instance_id));
            return Err(RuntimeV2CoordinatorError::OperationInFlight);
        }
        if self.retained_operations.contains(item.operation_id()) {
            self.record_rejection(Some(&instance_id));
            return Err(RuntimeV2CoordinatorError::OperationRetained);
        }
        if self.queued_len() >= self.config.global_queue_capacity {
            self.record_rejection(Some(&instance_id));
            return Err(RuntimeV2CoordinatorError::GlobalQueueFull);
        }
        let instance_queue_len = self
            .lanes
            .get(&instance_id)
            .map_or(0, |lane| lane.queue.len());
        if instance_queue_len >= self.config.per_instance_queue_capacity {
            self.record_rejection(Some(&instance_id));
            return Err(RuntimeV2CoordinatorError::InstanceQueueFull);
        }

        let was_empty = instance_queue_len == 0;
        if let Some(lane) = self.lanes.get_mut(&instance_id) {
            lane.queue.push_back(item.clone());
            lane.admitted = lane.admitted.saturating_add(1);
        } else {
            self.record_rejection(Some(&instance_id));
            return Err(RuntimeV2CoordinatorError::UnknownInstance);
        }
        self.queued_operations.insert(item.operation_id().clone());
        self.admitted = self.admitted.saturating_add(1);
        let lane_is_active = self.lanes.get(&instance_id).is_none_or(|lane| lane.active);
        if was_empty && !lane_is_active {
            self.enqueue_ready(instance_id);
        }
        Ok(self.queued_len())
    }

    /// Pops the next action using fair round-robin ordering across idle instances.
    pub fn next_work(&mut self) -> Option<RuntimeV2WorkItem> {
        while let Some(instance_id) = self.ready.pop_front() {
            let lane = match self.lanes.get_mut(&instance_id) {
                Some(lane) => lane,
                None => continue,
            };
            let item = match lane.queue.pop_front() {
                Some(item) => item,
                None => continue,
            };
            lane.active = true;
            self.queued_operations.remove(item.operation_id());
            self.active_operations
                .insert(item.operation_id().clone(), item.clone());
            return Some(item);
        }
        None
    }

    /// Cancels a queued operation before it is dispatched. Active work is not silently cancelled.
    pub fn cancel_queued(
        &mut self,
        operation_id: &RuntimeV2OperationId,
    ) -> Result<RuntimeV2WorkItem, RuntimeV2CoordinatorError> {
        let instance_id = self
            .lanes
            .iter()
            .find_map(|(instance_id, lane)| {
                lane.queue
                    .iter()
                    .any(|item| item.operation_id() == operation_id)
                    .then(|| instance_id.clone())
            })
            .ok_or(RuntimeV2CoordinatorError::UnknownOperation)?;
        let item = {
            let lane = match self.lanes.get_mut(&instance_id) {
                Some(lane) => lane,
                None => return Err(RuntimeV2CoordinatorError::UnknownInstance),
            };
            let mut retained = VecDeque::with_capacity(lane.queue.len());
            let mut cancelled = None;
            while let Some(item) = lane.queue.pop_front() {
                if item.operation_id() == operation_id {
                    cancelled = Some(item);
                } else {
                    retained.push_back(item);
                }
            }
            lane.queue = retained;
            let item = match cancelled {
                Some(item) => item,
                None => return Err(RuntimeV2CoordinatorError::UnknownOperation),
            };
            lane.cancelled = lane.cancelled.saturating_add(1);
            item
        };
        self.queued_operations.remove(operation_id);
        self.cancelled = self.cancelled.saturating_add(1);
        self.retain_operation(operation_id);
        Ok(item)
    }

    /// Closes admission, cancels queued work, and leaves active work for explicit settlement.
    pub fn shutdown(&mut self) -> RuntimeV2ShutdownReport {
        self.admission_open = false;
        self.ready.clear();
        let mut cancelled = Vec::new();
        let mut cancelled_operation_ids = Vec::new();
        for lane in self.lanes.values_mut() {
            while let Some(item) = lane.queue.pop_front() {
                self.queued_operations.remove(item.operation_id());
                lane.cancelled = lane.cancelled.saturating_add(1);
                self.cancelled = self.cancelled.saturating_add(1);
                cancelled_operation_ids.push(item.operation_id().clone());
                cancelled.push(item);
            }
        }
        for operation_id in cancelled_operation_ids {
            self.retain_operation(&operation_id);
        }
        RuntimeV2ShutdownReport {
            cancelled,
            active_operations: self.active_operations.keys().cloned().collect(),
        }
    }

    /// Returns whether new instances and work may be admitted.
    #[must_use]
    pub const fn admission_open(&self) -> bool {
        self.admission_open
    }

    /// Returns the number of queued but not dispatched operations.
    #[must_use]
    pub fn queued_len(&self) -> usize {
        self.lanes.values().map(|lane| lane.queue.len()).sum()
    }

    /// Returns the number of active per-instance dispatch slots.
    #[must_use]
    pub fn active_len(&self) -> usize {
        self.active_operations.len()
    }

    /// Returns a sanitized coordinator snapshot for evidence and metrics.
    #[must_use]
    pub fn snapshot(&self) -> RuntimeV2CoordinatorSnapshot {
        RuntimeV2CoordinatorSnapshot {
            max_instances: self.config.max_instances,
            global_queue_capacity: self.config.global_queue_capacity,
            per_instance_queue_capacity: self.config.per_instance_queue_capacity,
            admission_open: self.admission_open,
            queued: self.queued_len(),
            active: self.active_len(),
            admitted: self.admitted,
            completed: self.completed,
            unknown: self.unknown,
            cancelled: self.cancelled,
            rejected: self.rejected,
            service_time_samples: self.service_time_samples,
            service_time_total_millis: self.service_time_total_millis,
            service_time_max_millis: self.service_time_max_millis,
            retained: self.retained_operations.len(),
            instances: self
                .lanes
                .values()
                .map(|lane| RuntimeV2InstanceSnapshot {
                    instance_id: lane.binding.instance_id().to_owned(),
                    process_port: lane.binding.process_port,
                    queued: lane.queue.len(),
                    active: lane.active,
                    admitted: lane.admitted,
                    completed: lane.completed,
                    unknown: lane.unknown,
                    cancelled: lane.cancelled,
                    rejected: lane.rejected,
                    service_time_samples: lane.service_time_samples,
                    service_time_total_millis: lane.service_time_total_millis,
                    service_time_max_millis: lane.service_time_max_millis,
                })
                .collect(),
        }
    }
}
