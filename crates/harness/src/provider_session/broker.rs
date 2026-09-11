// SPDX-License-Identifier: MIT

use super::types::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[path = "broker_candidates.rs"]
mod candidates;
#[path = "broker_compaction.rs"]
mod compaction;
#[path = "broker_history.rs"]
mod history;
#[path = "broker_interrupt.rs"]
mod interrupts;
#[path = "broker_maintenance.rs"]
mod maintenance;
#[path = "broker_retirement.rs"]
mod retirement;
#[path = "broker_snapshot.rs"]
mod snapshot;
#[path = "broker_turn_lifecycle.rs"]
mod turn_lifecycle;
#[path = "broker_turns.rs"]
mod turns;

/// A bounded, serializable view of the broker.  Exact prepared bytes are intentionally retained
/// only by the broker and are never included in this metadata journal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerSnapshot {
    pub schema: String,
    pub scope: SessionScope,
    pub owner_epoch: u64,
    pub revocation_epoch: u64,
    pub policy: ProviderSessionPolicy,
    pub capabilities: NativeCapabilities,
    pub bindings: Vec<SessionBinding>,
    pub operations: Vec<NativeOperation>,
    pub events: Vec<SessionEvent>,
    pub histories: BTreeMap<String, Vec<HistoryItem>>,
    pub compaction_jobs: Vec<CompactionJob>,
    pub fork_plans: Vec<ForkPlan>,
    pub retirements: Vec<Retirement>,
}

const BROKER_SNAPSHOT_SCHEMA: &str = "ascension.provider-session.broker-snapshot.v1";

/// Harness-owned session registry and lifecycle fence.  All mutating operations are serialized by
/// the caller's owner token; this object deliberately has no public raw native RPC method.
#[derive(Clone, Debug)]
pub struct ProviderSessionBroker {
    scope: SessionScope,
    policy: ProviderSessionPolicy,
    capabilities: NativeCapabilities,
    owner_token: String,
    owner_epoch: u64,
    revocation_epoch: u64,
    next_id: u64,
    next_sequence: u64,
    bindings: BTreeMap<String, SessionBinding>,
    operations: BTreeMap<String, NativeOperation>,
    idempotency: BTreeMap<String, (String, String)>,
    prepared: BTreeMap<String, PreparedSessionTurn>,
    histories: BTreeMap<String, Vec<HistoryItem>>,
    compaction_jobs: BTreeMap<String, CompactionJob>,
    fork_plans: BTreeMap<String, ForkPlan>,
    retirements: BTreeMap<String, Retirement>,
    events: Vec<SessionEvent>,
    inflight_turn: Option<String>,
}

impl ProviderSessionBroker {
    pub fn new(
        scope: SessionScope,
        policy: ProviderSessionPolicy,
        capabilities: NativeCapabilities,
        owner_token: impl Into<String>,
    ) -> Result<Self, SessionError> {
        policy.validate()?;
        capabilities.validate()?;
        if !matches!(policy.mode, ProviderSessionMode::Disabled)
            && policy.profile_sha256 != capabilities.profile_sha256
        {
            return Err(SessionError::InvalidPolicy);
        }
        if matches!(policy.mode, ProviderSessionMode::Enabled) && !capabilities.strict_executable {
            return Err(SessionError::Unsupported);
        }
        let owner_token = owner_token.into();
        if policy.scope != scope || owner_token.is_empty() {
            return Err(SessionError::InvalidScope);
        }
        Ok(Self {
            scope,
            policy,
            capabilities,
            owner_token,
            owner_epoch: 1,
            revocation_epoch: 0,
            next_id: 1,
            next_sequence: 1,
            bindings: BTreeMap::new(),
            operations: BTreeMap::new(),
            idempotency: BTreeMap::new(),
            prepared: BTreeMap::new(),
            histories: BTreeMap::new(),
            compaction_jobs: BTreeMap::new(),
            fork_plans: BTreeMap::new(),
            retirements: BTreeMap::new(),
            events: Vec::new(),
            inflight_turn: None,
        })
    }

    #[must_use]
    pub fn scope(&self) -> &SessionScope {
        &self.scope
    }

    #[must_use]
    pub fn policy(&self) -> &ProviderSessionPolicy {
        &self.policy
    }

    #[must_use]
    pub fn capabilities(&self) -> &NativeCapabilities {
        &self.capabilities
    }

    #[must_use]
    pub fn owner_epoch(&self) -> u64 {
        self.owner_epoch
    }

    #[must_use]
    pub fn revocation_epoch(&self) -> u64 {
        self.revocation_epoch
    }

    pub fn bindings(&self) -> impl Iterator<Item = &SessionBinding> {
        self.bindings.values()
    }

    #[must_use]
    pub fn events(&self) -> &[SessionEvent] {
        &self.events
    }

    pub fn binding(&self, binding_id: &str) -> Result<&SessionBinding, SessionError> {
        self.bindings.get(binding_id).ok_or(SessionError::NotFound)
    }

    pub fn operation(&self, operation_id: &str) -> Result<&NativeOperation, SessionError> {
        self.operations
            .get(operation_id)
            .ok_or(SessionError::NotFound)
    }

    /// Revalidates the caller-owned serialization token.  Native IDs are never accepted here.
    pub fn authorize_owner(&self, owner_token: &str) -> Result<(), SessionError> {
        if owner_token == self.owner_token {
            Ok(())
        } else {
            Err(SessionError::Unauthorized)
        }
    }
    fn existing_idempotent(
        &self,
        key: &str,
        request: &serde_json::Value,
    ) -> Result<Option<NativeOperation>, SessionError> {
        let Some((digest, operation_id)) = self.idempotency.get(key) else {
            return Ok(None);
        };
        let current = digest_json(request);
        if digest != &current {
            return Err(SessionError::Conflict);
        }
        Ok(self.operations.get(operation_id).cloned())
    }

    fn new_operation(
        &mut self,
        binding_id: &str,
        kind: NativeOperationKind,
        key: &str,
        request: &serde_json::Value,
        generation_class: bool,
    ) -> Result<NativeOperation, SessionError> {
        if !valid_id(binding_id) || !valid_id(key) || self.operations.len() >= MAX_OPERATIONS {
            return Err(SessionError::Capacity);
        }
        let operation = NativeOperation {
            schema: SESSION_OPERATION_SCHEMA.to_owned(),
            operation_id: self.allocate_id("operation"),
            scope: self.scope.clone(),
            binding_id: binding_id.to_owned(),
            kind,
            idempotency_key: key.to_owned(),
            request_sha256: digest_json(request),
            state: NativeOperationState::IntentPersisted,
            owner_epoch: self.owner_epoch,
            session_epoch: self.bindings.get(binding_id).map_or(1, |b| b.session_epoch),
            generation_permission: generation_class,
            generation_class,
            automatic_retry: false,
            auto_resume: false,
            game_effects: 0,
            terminal_evidence_ref: None,
        };
        operation.validate()?;
        self.operations
            .insert(operation.operation_id.clone(), operation.clone());
        Ok(operation)
    }

    fn allocate_id(&mut self, prefix: &str) -> String {
        let value = format!("{prefix}-{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        value
    }

    fn emit(
        &mut self,
        binding_id: &str,
        operation_id: Option<&str>,
        kind: SessionEventKind,
        status: SessionEventStatus,
        count: usize,
    ) {
        if self.events.len() >= MAX_EVENTS {
            self.events.remove(0);
        }
        let event = SessionEvent {
            schema: SESSION_EVENT_SCHEMA.to_owned(),
            event_id: self.allocate_id("event"),
            scope: self.scope.clone(),
            binding_id: binding_id.to_owned(),
            operation_id: operation_id.map(str::to_owned),
            local_ingest_sequence: self.next_sequence,
            sequence_origin: "local_broker".to_owned(),
            owner_epoch: self.owner_epoch,
            session_epoch: self.bindings.get(binding_id).map_or(1, |b| b.session_epoch),
            kind,
            metadata: EventMetadata { status, count },
            starts_inference: false,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.events.push(event);
    }
}

fn digest_json(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    digest(bytes)
}
