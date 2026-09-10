// SPDX-License-Identifier: MIT

use super::types::{CONTROL_JOURNAL_SCHEMA, ContextBoundary};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum GateStatus {
    Running,
    PauseRequested,
    PausedReady,
    PausedStale,
    PausedCommitted,
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlState {
    pub status: GateStatus,
    pub control_version: u64,
    pub gate_epoch: u64,
    pub plan_epoch: u64,
    pub active_revision_id: String,
    pub pause_latched: bool,
    pub stop_latched: bool,
    pub unresolved_operations: Vec<String>,
    pub boundary: ContextBoundary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlReceipt {
    pub command_id: String,
    pub idempotency_key: String,
    pub effect: String,
    pub control_version: u64,
    pub plan_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlEvent {
    pub sequence: u64,
    pub event_type: String,
    pub command_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Journal {
    schema: String,
    owner_epoch: u64,
    state: ControlState,
    commands: BTreeMap<String, (String, ControlReceipt)>,
    events: Vec<ControlEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlAuthority {
    owner_epoch: u64,
    state: ControlState,
    commands: BTreeMap<String, (String, ControlReceipt)>,
    events: Vec<ControlEvent>,
    next_id: u64,
}

impl ControlAuthority {
    pub fn new(boundary: ContextBoundary, active_revision_id: impl Into<String>) -> Self {
        let gate_epoch = boundary.gate_epoch;
        let control_version = boundary.control_version;
        Self {
            owner_epoch: boundary.controller_epoch,
            state: ControlState {
                status: GateStatus::Running,
                control_version,
                gate_epoch,
                plan_epoch: 1,
                active_revision_id: active_revision_id.into(),
                pause_latched: false,
                stop_latched: false,
                unresolved_operations: Vec::new(),
                boundary,
            },
            commands: BTreeMap::new(),
            events: Vec::new(),
            next_id: 1,
        }
    }

    pub fn state(&self) -> &ControlState {
        &self.state
    }

    pub fn events(&self) -> &[ControlEvent] {
        &self.events
    }

    /// Admit one provider or game operation under the current plan epoch.
    ///
    /// Admission and the pause latch share this state transition boundary: a request that wins
    /// the boundary before pause is tracked as unresolved, while a request that arrives after
    /// pause is rejected as an obsolete plan. The control authority never performs external I/O.
    pub fn admit_operation(&mut self, operation_id: &str, plan_epoch: u64) -> Result<(), String> {
        validate_operation_id(operation_id)?;
        self.admit_plan(plan_epoch)?;
        if self
            .state
            .unresolved_operations
            .iter()
            .any(|existing| existing == operation_id)
        {
            return Err("operation_in_flight".to_owned());
        }
        self.state
            .unresolved_operations
            .push(operation_id.to_owned());
        self.event("operation.admitted", None, Some("awaiting_settlement"));
        Ok(())
    }

    /// Settle one admitted operation and, when pause is latched, publish the safe boundary only
    /// after every unresolved operation has been reconciled.
    pub fn settle_operation(&mut self, operation_id: &str) -> Result<(), String> {
        validate_operation_id(operation_id)?;
        let Some(index) = self
            .state
            .unresolved_operations
            .iter()
            .position(|existing| existing == operation_id)
        else {
            return Err("unknown_operation".to_owned());
        };
        self.state.unresolved_operations.remove(index);
        self.event("operation.settled", None, None);
        if self.state.pause_latched
            && self.state.unresolved_operations.is_empty()
            && self.state.status == GateStatus::PauseRequested
        {
            self.state.status = GateStatus::PausedReady;
            self.event("pause.ready", None, Some("all_operations_settled"));
        }
        Ok(())
    }

    /// Retain an unknown operation as an unresolved ledger entry. Recovery must reconcile the
    /// original identity before readiness can be reported or resume can be accepted.
    pub fn retain_unknown_operation(&mut self, operation_id: &str) -> Result<(), String> {
        validate_operation_id(operation_id)?;
        if !self
            .state
            .unresolved_operations
            .iter()
            .any(|existing| existing == operation_id)
        {
            return Err("unknown_operation".to_owned());
        }
        self.event("operation.unknown", None, Some("reconciliation_required"));
        Ok(())
    }

    /// Accept a provider completion only for the current plan epoch. A late completion from an
    /// older plan is discarded before it can acquire authority or mutate the ledger.
    pub fn complete_provider_attempt(
        &mut self,
        attempt_id: &str,
        plan_epoch: u64,
    ) -> Result<(), String> {
        validate_operation_id(attempt_id)?;
        if plan_epoch != self.state.plan_epoch {
            return Err("obsolete_plan".to_owned());
        }
        self.settle_operation(attempt_id)
    }

    pub fn request_pause(
        &mut self,
        idempotency_key: &str,
        expected_control_version: u64,
    ) -> Result<ControlReceipt, String> {
        let payload = expected_control_version.to_string();
        if let Some(receipt) = self.idempotent(idempotency_key, "pause", &payload)? {
            return Ok(receipt);
        }
        self.guard(idempotency_key, expected_control_version)?;
        if self.state.stop_latched || self.state.pause_latched {
            return Err("already_paused".to_owned());
        }
        self.state.pause_latched = true;
        self.state.gate_epoch = self.state.gate_epoch.saturating_add(1);
        self.bump_boundary();
        self.state.status = if self.state.unresolved_operations.is_empty() {
            GateStatus::PausedReady
        } else {
            GateStatus::PauseRequested
        };
        let receipt = self.receipt(idempotency_key, "pause_requested");
        self.persist_command(idempotency_key, "pause", &payload, &receipt);
        self.event("pause.accepted", Some(&receipt.command_id), None);
        Ok(receipt)
    }

    pub fn commit(
        &mut self,
        idempotency_key: &str,
        expected_control_version: u64,
        expected_revision_id: &str,
        expected_boundary: &ContextBoundary,
        preview_manifest_sha256: &str,
        approved_manifest_sha256: &str,
    ) -> Result<ControlReceipt, String> {
        let payload = format!(
            "{expected_control_version}|{expected_revision_id}|{preview_manifest_sha256}|{approved_manifest_sha256}|{}",
            serde_json::to_string(expected_boundary).unwrap_or_default()
        );
        if let Some(receipt) = self.idempotent(idempotency_key, "commit", &payload)? {
            return Ok(receipt);
        }
        self.guard(idempotency_key, expected_control_version)?;
        if !self.state.pause_latched || self.state.status != GateStatus::PausedReady {
            return Err("run_not_ready".to_owned());
        }
        if self.state.active_revision_id != expected_revision_id {
            return Err("stale_revision".to_owned());
        }
        if !self.state.boundary.external_eq(expected_boundary)
            || preview_manifest_sha256 != approved_manifest_sha256
        {
            return Err("preview_stale".to_owned());
        }
        self.state.active_revision_id = format!("revision-{}", self.state.plan_epoch + 1);
        self.state.plan_epoch = self.state.plan_epoch.saturating_add(1);
        self.bump_boundary();
        self.state.status = GateStatus::PausedCommitted;
        let receipt = self.receipt(idempotency_key, "revision_committed");
        self.persist_command(idempotency_key, "commit", &payload, &receipt);
        self.event("revision.committed", Some(&receipt.command_id), None);
        self.event(
            "plan.retired",
            Some(&receipt.command_id),
            Some("plan_epoch_advanced"),
        );
        Ok(receipt)
    }

    pub fn resume(
        &mut self,
        idempotency_key: &str,
        expected_control_version: u64,
        expected_boundary: &ContextBoundary,
    ) -> Result<ControlReceipt, String> {
        let payload = format!(
            "{expected_control_version}|{}",
            serde_json::to_string(expected_boundary).unwrap_or_default()
        );
        if let Some(receipt) = self.idempotent(idempotency_key, "resume", &payload)? {
            return Ok(receipt);
        }
        self.guard(idempotency_key, expected_control_version)?;
        if self.state.stop_latched {
            return Err("stopped".to_owned());
        }
        if !self.state.pause_latched || !self.state.unresolved_operations.is_empty() {
            return Err("not_ready".to_owned());
        }
        if !self.state.boundary.external_eq(expected_boundary) {
            return Err("preview_stale".to_owned());
        }
        self.state.pause_latched = false;
        self.state.gate_epoch = self.state.gate_epoch.saturating_add(1);
        self.bump_boundary();
        self.state.status = GateStatus::Running;
        let receipt = self.receipt(idempotency_key, "resume_accepted");
        self.persist_command(idempotency_key, "resume", &payload, &receipt);
        self.event("resume.accepted", Some(&receipt.command_id), None);
        Ok(receipt)
    }

    pub fn stop(&mut self) {
        self.state.stop_latched = true;
        self.state.pause_latched = true;
        self.state.status = GateStatus::Stopped;
        self.state.gate_epoch = self.state.gate_epoch.saturating_add(1);
        self.bump_boundary();
        self.event("stop.latched", None, Some("stop_dominates_resume"));
    }

    pub fn advance_boundary(&mut self) {
        self.state.boundary.generation = self.state.boundary.generation.saturating_add(1);
        self.state.boundary.observation_sha256 =
            digest(format!("observation-{}", self.state.boundary.generation).as_bytes());
        if self.state.pause_latched {
            self.state.status = GateStatus::PausedStale;
        }
        self.event("boundary.changed", None, Some("requires_reobserve"));
    }

    pub fn admit_plan(&self, plan_epoch: u64) -> Result<(), String> {
        if self.state.pause_latched
            || self.state.stop_latched
            || plan_epoch != self.state.plan_epoch
        {
            return Err("obsolete_plan".to_owned());
        }
        Ok(())
    }

    pub fn export_journal(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&Journal {
            schema: CONTROL_JOURNAL_SCHEMA.to_owned(),
            owner_epoch: self.owner_epoch,
            state: self.state.clone(),
            commands: self.commands.clone(),
            events: self.events.clone(),
        })
        .map_err(|_| "journal_encode".to_owned())
    }

    pub fn recover(journal: &[u8]) -> Result<Self, String> {
        let journal: Journal =
            serde_json::from_slice(journal).map_err(|_| "journal_decode".to_owned())?;
        if journal.schema != CONTROL_JOURNAL_SCHEMA || journal.events.len() > 4096 {
            return Err("journal_invalid".to_owned());
        }
        if journal.owner_epoch != journal.state.boundary.controller_epoch {
            return Err("journal_owner_epoch_mismatch".to_owned());
        }
        let next_owner_epoch = journal
            .owner_epoch
            .checked_add(1)
            .ok_or_else(|| "journal_owner_epoch_exhausted".to_owned())?;
        let next_id = journal
            .commands
            .values()
            .filter_map(|(_, receipt)| receipt.command_id.strip_prefix("control-command-"))
            .filter_map(|value| value.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| "journal_command_id_exhausted".to_owned())?;
        let mut state = journal.state;
        state.boundary.controller_epoch = next_owner_epoch;
        Ok(Self {
            owner_epoch: next_owner_epoch,
            state,
            commands: journal.commands,
            events: journal.events,
            next_id,
        })
    }

    fn idempotent(
        &self,
        key: &str,
        kind: &str,
        payload: &str,
    ) -> Result<Option<ControlReceipt>, String> {
        let Some((stored_digest, receipt)) = self.commands.get(key) else {
            return Ok(None);
        };
        let current = command_digest(key, kind, payload);
        if stored_digest == &current {
            return Ok(Some(receipt.clone()));
        }
        Err("idempotency_conflict".to_owned())
    }

    fn guard(&self, key: &str, expected_control_version: u64) -> Result<(), String> {
        if key.is_empty() || key.len() > 128 {
            return Err("invalid_command".to_owned());
        }
        if self.state.control_version != expected_control_version {
            return Err("stale_control".to_owned());
        }
        Ok(())
    }

    fn receipt(&mut self, key: &str, effect: &str) -> ControlReceipt {
        let command_id = format!("control-command-{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        ControlReceipt {
            command_id,
            idempotency_key: key.to_owned(),
            effect: effect.to_owned(),
            control_version: self.state.control_version,
            plan_epoch: self.state.plan_epoch,
        }
    }

    fn persist_command(&mut self, key: &str, kind: &str, payload: &str, receipt: &ControlReceipt) {
        self.commands.insert(
            key.to_owned(),
            (command_digest(key, kind, payload), receipt.clone()),
        );
    }

    fn event(&mut self, event_type: &str, command_id: Option<&str>, reason: Option<&str>) {
        if self.events.len() >= 4096 {
            return;
        }
        self.events.push(ControlEvent {
            sequence: self.events.len() as u64 + 1,
            event_type: event_type.to_owned(),
            command_id: command_id.map(str::to_owned),
            reason: reason.map(str::to_owned),
        });
    }

    fn bump_boundary(&mut self) {
        self.state.control_version = self.state.control_version.saturating_add(1);
        self.state.boundary.control_version = self.state.control_version;
        self.state.boundary.gate_epoch = self.state.gate_epoch;
    }
}

fn command_digest(key: &str, kind: &str, payload: &str) -> String {
    Sha256::digest(format!("{kind}\0{key}\0{payload}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_operation_id(operation_id: &str) -> Result<(), String> {
    if operation_id.is_empty() || operation_id.len() > 128 {
        return Err("invalid_operation".to_owned());
    }
    if operation_id
        .bytes()
        .any(|byte| byte <= 0x20 || byte == b'/' || byte == b'\\')
    {
        return Err("invalid_operation".to_owned());
    }
    Ok(())
}
