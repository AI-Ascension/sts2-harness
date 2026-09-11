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
            operations: self.operations.values().cloned().collect(),
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
        Self::restore_snapshot(snapshot, owner_token)
    }

    /// Restores only when the caller supplies the currently approved scope, policy and native
    /// capability profile. This prevents a snapshot from silently selecting a different binary,
    /// schema or credential realm during an upgrade.
    pub fn from_snapshot_json_checked(
        bytes: &[u8],
        owner_token: impl Into<String>,
        expected_scope: &SessionScope,
        expected_policy: &ProviderSessionPolicy,
        expected_capabilities: &NativeCapabilities,
    ) -> Result<Self, SessionError> {
        expected_policy.validate()?;
        expected_capabilities.validate()?;
        if expected_policy.scope != *expected_scope
            || expected_policy.profile_sha256 != expected_capabilities.profile_sha256
        {
            return Err(SessionError::InvalidPolicy);
        }
        let snapshot = parse_snapshot(bytes)?;
        if snapshot.scope != *expected_scope
            || snapshot.policy != *expected_policy
            || snapshot.capabilities != *expected_capabilities
        {
            return Err(SessionError::Unsupported);
        }
        Self::restore_snapshot(snapshot, owner_token)
    }

    fn restore_snapshot(
        snapshot: BrokerSnapshot,
        owner_token: impl Into<String>,
    ) -> Result<Self, SessionError> {
        let owner_token = owner_token.into();
        let mut broker = Self::new(
            snapshot.scope.clone(),
            snapshot.policy.clone(),
            snapshot.capabilities.clone(),
            owner_token,
        )?;
        broker.owner_epoch = snapshot
            .owner_epoch
            .checked_add(1)
            .ok_or(SessionError::Capacity)?;
        broker.revocation_epoch = snapshot.revocation_epoch;
        broker.restore_bindings(snapshot.bindings)?;
        broker.restore_retirements(snapshot.retirements)?;
        broker.restore_operations(snapshot.operations)?;
        broker.restore_events(snapshot.events)?;
        broker.restore_histories(snapshot.histories)?;
        broker.restore_maintenance(snapshot.compaction_jobs, snapshot.fork_plans)?;
        broker.fence_pending_maintenance();
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
        let bounded_candidates = bindings
            .iter()
            .filter(|binding| {
                matches!(binding.state, BindingState::Candidate)
                    || binding.purpose == SessionPurpose::Evaluation
            })
            .count();
        let active_executable = bindings
            .iter()
            .filter(|binding| binding.executable())
            .count();
        if bounded_candidates > MAX_CANDIDATES || active_executable > 1 {
            return Err(SessionError::Capacity);
        }
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
            self.ensure_not_expired(&binding.expires_at)?;
        }
        for binding in self.bindings.values_mut() {
            binding.owner_epoch = self.owner_epoch;
            if binding.state == BindingState::Active {
                binding.state = BindingState::Recovering;
            }
            binding.game_dispatch_capability = false;
        }
        Ok(())
    }

    fn restore_operations(&mut self, operations: Vec<NativeOperation>) -> Result<(), SessionError> {
        for operation in operations {
            if operation.scope != self.scope || !self.bindings.contains_key(&operation.binding_id) {
                return Err(SessionError::InvalidOperation);
            }
            let binding_retired = self
                .bindings
                .get(&operation.binding_id)
                .is_some_and(|binding| {
                    matches!(binding.state, BindingState::Retired | BindingState::Closed)
                });
            let mut restored = operation;
            let nonterminal = !matches!(
                restored.state,
                NativeOperationState::Completed
                    | NativeOperationState::Rejected
                    | NativeOperationState::Cancelled
                    | NativeOperationState::Quarantined
            );
            if binding_retired && nonterminal {
                restored.state = NativeOperationState::Quarantined;
            } else if matches!(
                restored.kind,
                NativeOperationKind::Turn | NativeOperationKind::Interrupt
            ) && nonterminal
            {
                restored.state = NativeOperationState::Unknown;
                if let Some(binding) = self.bindings.get_mut(&restored.binding_id) {
                    binding.state = BindingState::Recovering;
                    binding.game_dispatch_capability = false;
                }
            }
            if self
                .operations
                .insert(restored.operation_id.clone(), restored.clone())
                .is_some()
            {
                return Err(SessionError::InvalidOperation);
            }
            restored.validate()?;
            if self
                .idempotency
                .insert(
                    restored.idempotency_key.clone(),
                    (
                        restored.request_sha256.clone(),
                        restored.operation_id.clone(),
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
            if items
                .iter()
                .filter(|item| item.kind == HistoryItemKind::ValidatedDecision)
                .count()
                > self.policy.max_completed_turns
            {
                return Err(SessionError::Capacity);
            }
            self.projected_history_bytes(&binding_id, &items)?;
            self.histories.insert(binding_id, items);
        }
        Ok(())
    }

    fn restore_maintenance(
        &mut self,
        compaction_jobs: Vec<CompactionJob>,
        fork_plans: Vec<ForkPlan>,
    ) -> Result<(), SessionError> {
        if compaction_jobs
            .len()
            .checked_add(fork_plans.len())
            .is_none_or(|count| count > MAX_MAINTENANCE_JOBS)
        {
            return Err(SessionError::Capacity);
        }
        for job in compaction_jobs {
            if job.scope != self.scope || !self.bindings.contains_key(&job.binding_id) {
                return Err(SessionError::InvalidRequest);
            }
            job.validate()?;
            if self
                .compaction_jobs
                .insert(job.job_id.clone(), job)
                .is_some()
            {
                return Err(SessionError::Conflict);
            }
        }
        for plan in fork_plans {
            if plan.scope != self.scope
                || !self.bindings.contains_key(&plan.source_binding_id)
                || !self.bindings.contains_key(&plan.target_binding_id)
            {
                return Err(SessionError::InvalidRequest);
            }
            plan.validate()?;
            if self
                .fork_plans
                .insert(plan.fork_plan_id.clone(), plan)
                .is_some()
            {
                return Err(SessionError::Conflict);
            }
        }
        Ok(())
    }

    fn restore_retirements(&mut self, retirements: Vec<Retirement>) -> Result<(), SessionError> {
        for retirement in retirements {
            if retirement.scope != self.scope
                || retirement
                    .binding_ids
                    .iter()
                    .any(|id| !self.bindings.contains_key(id))
            {
                return Err(SessionError::InvalidRequest);
            }
            retirement.validate()?;
            if self
                .retirements
                .insert(retirement.retirement_id.clone(), retirement)
                .is_some()
            {
                return Err(SessionError::Conflict);
            }
        }
        let tombstoned: Vec<String> = self
            .retirements
            .values()
            .flat_map(|retirement| retirement.binding_ids.iter().cloned())
            .collect();
        for binding_id in tombstoned {
            if let Some(binding) = self.bindings.get_mut(&binding_id) {
                binding.state = BindingState::Retired;
                binding.game_dispatch_capability = false;
            }
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
