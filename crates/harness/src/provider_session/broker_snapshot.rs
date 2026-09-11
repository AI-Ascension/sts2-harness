// SPDX-License-Identifier: MIT

use super::super::protocol::parse_strict_json_bounded;
use super::super::types::*;
use super::{BROKER_SNAPSHOT_SCHEMA, BrokerSnapshot, ProviderSessionBroker};

impl ProviderSessionBroker {
    #[must_use]
    pub fn snapshot(&self) -> BrokerSnapshot {
        BrokerSnapshot {
            schema: BROKER_SNAPSHOT_SCHEMA.to_owned(),
            scope: self.scope.clone(),
            owner_epoch: self.owner_epoch,
            revocation_epoch: self.revocation_epoch,
            policy: self.policy.clone(),
            capabilities: self.capabilities.clone(),
            bindings: self.bindings.values().cloned().collect(),
            operations: self
                .operations
                .values()
                .filter(|operation| {
                    !matches!(
                        operation.kind,
                        NativeOperationKind::Turn | NativeOperationKind::Interrupt
                    )
                })
                .cloned()
                .collect(),
            events: self.events.clone(),
            histories: self.histories.clone(),
            compaction_jobs: self.compaction_jobs.values().cloned().collect(),
            fork_plans: self.fork_plans.values().cloned().collect(),
            retirements: self.retirements.values().cloned().collect(),
        }
    }

    pub fn snapshot_json(&self) -> Result<Vec<u8>, SessionError> {
        let bytes = serde_json::to_vec(&self.snapshot()).map_err(|_| SessionError::Protocol)?;
        if bytes.len() > MAX_HISTORY_BYTES {
            return Err(SessionError::Capacity);
        }
        Ok(bytes)
    }

    /// Restore the metadata journal after a process restart.  The caller must provide a fresh
    /// owner token; serialized state never contains that secret or prepared bytes.  Any in-flight
    /// turn therefore remains unrecoverable until an explicit reconciliation, and the restored
    /// broker cannot auto-resume gameplay.
    pub fn from_snapshot_json(
        bytes: &[u8],
        owner_token: impl Into<String>,
    ) -> Result<Self, SessionError> {
        let snapshot = parse_snapshot(bytes)?;
        let owner_token = owner_token.into();
        let mut broker = Self::new(
            snapshot.scope.clone(),
            snapshot.policy.clone(),
            snapshot.capabilities.clone(),
            owner_token,
        )?;
        broker.owner_epoch = snapshot.owner_epoch;
        broker.revocation_epoch = snapshot.revocation_epoch;
        broker.restore_bindings(snapshot.bindings)?;
        broker.restore_operations(snapshot.operations)?;
        broker.restore_events(snapshot.events)?;
        broker.restore_histories(snapshot.histories)?;
        broker.restore_maintenance(
            snapshot.compaction_jobs,
            snapshot.fork_plans,
            snapshot.retirements,
        )?;
        broker.next_id = next_id_after_restore(&broker)?;
        broker.next_sequence = broker
            .events
            .iter()
            .map(|event| event.local_ingest_sequence)
            .max()
            .unwrap_or_default()
            .saturating_add(1);
        Ok(broker)
    }
}

impl ProviderSessionBroker {
    fn restore_bindings(&mut self, bindings: Vec<SessionBinding>) -> Result<(), SessionError> {
        for binding in bindings {
            if binding.scope != self.scope
                || self
                    .bindings
                    .insert(binding.binding_id.clone(), binding)
                    .is_some()
            {
                return Err(SessionError::InvalidBinding);
            }
        }
        for binding in self.bindings.values() {
            binding.validate()?;
        }
        Ok(())
    }

    fn restore_operations(&mut self, operations: Vec<NativeOperation>) -> Result<(), SessionError> {
        for operation in operations {
            if operation.scope != self.scope
                || matches!(
                    operation.kind,
                    NativeOperationKind::Turn | NativeOperationKind::Interrupt
                )
                || !self.bindings.contains_key(&operation.binding_id)
                || self
                    .operations
                    .insert(operation.operation_id.clone(), operation.clone())
                    .is_some()
            {
                return Err(SessionError::InvalidOperation);
            }
            operation.validate()?;
            if self
                .idempotency
                .insert(
                    operation.idempotency_key.clone(),
                    (
                        operation.request_sha256.clone(),
                        operation.operation_id.clone(),
                    ),
                )
                .is_some()
            {
                return Err(SessionError::Conflict);
            }
        }
        Ok(())
    }

    fn restore_events(&mut self, events: Vec<SessionEvent>) -> Result<(), SessionError> {
        for event in events {
            if event.schema != SESSION_EVENT_SCHEMA
                || event.scope != self.scope
                || !self.bindings.contains_key(&event.binding_id)
                || self
                    .events
                    .iter()
                    .any(|existing| existing.event_id == event.event_id)
            {
                return Err(SessionError::InvalidRequest);
            }
            self.events.push(event);
        }
        Ok(())
    }

    fn restore_histories(
        &mut self,
        histories: std::collections::BTreeMap<String, Vec<HistoryItem>>,
    ) -> Result<(), SessionError> {
        for (binding_id, items) in histories {
            if !self.bindings.contains_key(&binding_id) || items.len() > MAX_SESSION_ITEMS {
                return Err(SessionError::Capacity);
            }
            for item in &items {
                item.validate()?;
            }
            self.histories.insert(binding_id, items);
        }
        Ok(())
    }

    fn restore_maintenance(
        &mut self,
        compaction_jobs: Vec<CompactionJob>,
        fork_plans: Vec<ForkPlan>,
        retirements: Vec<Retirement>,
    ) -> Result<(), SessionError> {
        for job in compaction_jobs {
            if job.scope != self.scope || !self.bindings.contains_key(&job.binding_id) {
                return Err(SessionError::InvalidRequest);
            }
            self.compaction_jobs.insert(job.job_id.clone(), job);
        }
        for plan in fork_plans {
            if plan.scope != self.scope
                || !self.bindings.contains_key(&plan.source_binding_id)
                || !self.bindings.contains_key(&plan.target_binding_id)
            {
                return Err(SessionError::InvalidRequest);
            }
            self.fork_plans.insert(plan.fork_plan_id.clone(), plan);
        }
        for retirement in retirements {
            if retirement.scope != self.scope
                || retirement
                    .binding_ids
                    .iter()
                    .any(|id| !self.bindings.contains_key(id))
            {
                return Err(SessionError::InvalidRequest);
            }
            self.retirements
                .insert(retirement.retirement_id.clone(), retirement);
        }
        Ok(())
    }
}

fn parse_snapshot(bytes: &[u8]) -> Result<BrokerSnapshot, SessionError> {
    if bytes.is_empty() || bytes.len() > MAX_HISTORY_BYTES {
        return Err(SessionError::Capacity);
    }
    let snapshot: BrokerSnapshot =
        parse_strict_json_bounded(bytes, MAX_HISTORY_BYTES, MAX_JSON_DEPTH)?;
    if snapshot.schema != BROKER_SNAPSHOT_SCHEMA
        || snapshot.owner_epoch == 0
        || !snapshot.scope.valid()
        || snapshot.bindings.len() > MAX_CANDIDATES.saturating_mul(32)
        || snapshot.operations.len() > MAX_OPERATIONS
        || snapshot.events.len() > MAX_EVENTS
        || snapshot.histories.len() > MAX_CANDIDATES.saturating_mul(32)
    {
        return Err(SessionError::InvalidRequest);
    }
    Ok(snapshot)
}

fn next_id_after_restore(broker: &ProviderSessionBroker) -> Result<u64, SessionError> {
    let mut highest = 0_u64;
    let mut observe = |value: &str| {
        if let Some(suffix) = value.rsplit('-').next()
            && let Ok(number) = suffix.parse::<u64>()
        {
            highest = highest.max(number);
        }
    };

    for binding in broker.bindings.values() {
        observe(&binding.binding_id);
    }
    for operation in broker.operations.values() {
        observe(&operation.operation_id);
    }
    for event in &broker.events {
        observe(&event.event_id);
    }
    for job in broker.compaction_jobs.values() {
        observe(&job.job_id);
    }
    for plan in broker.fork_plans.values() {
        observe(&plan.fork_plan_id);
    }
    for retirement in broker.retirements.values() {
        observe(&retirement.retirement_id);
    }

    highest.checked_add(1).ok_or(SessionError::Capacity)
}
