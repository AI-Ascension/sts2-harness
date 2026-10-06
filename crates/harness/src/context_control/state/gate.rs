// SPDX-License-Identifier: MIT

//! Gate transitions for the control authority.
//!
//! Pause, commit, revision adoption, and resume are the transitions that move a run between running
//! and paused. They share one rule: every transition is guarded by the caller's expected control
//! version and reserves event capacity before it changes any state, so a rejected transition
//! leaves the authority exactly as it was.

use super::super::state::{ControlAuthority, ControlReceipt, GateStatus};
use super::super::types::{ContextBoundary, valid_id};

impl ControlAuthority {
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
        self.reserve_events(1)?;
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
        self.event("pause.accepted", Some(&receipt.command_id), None)?;
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
        self.reserve_events(2)?;
        self.state.active_revision_id = format!("revision-{}", self.state.plan_epoch + 1);
        self.state.plan_epoch = self.state.plan_epoch.saturating_add(1);
        self.bump_boundary();
        self.state.status = GateStatus::PausedCommitted;
        let receipt = self.receipt(idempotency_key, "revision_committed");
        self.persist_command(idempotency_key, "commit", &payload, &receipt);
        self.event("revision.committed", Some(&receipt.command_id), None)?;
        self.event(
            "plan.retired",
            Some(&receipt.command_id),
            Some("plan_epoch_advanced"),
        )?;
        Ok(receipt)
    }

    /// Atomically adopts one explicitly authorized source as the next active
    /// revision. Unlike an operator's content commit, this does not publish a
    /// caller-supplied manifest: the owner supplies the immutable source
    /// identity and verifies that identity in the same transaction that
    /// persists the journal, receipt, and active-source pointer.
    #[allow(clippy::too_many_arguments)]
    pub fn adopt_source_revision(
        &mut self,
        idempotency_key: &str,
        expected_control_version: u64,
        expected_revision_id: &str,
        expected_boundary: &ContextBoundary,
        source_id: &str,
        source_version: u64,
        source_digest: &str,
    ) -> Result<ControlReceipt, String> {
        let payload = source_adoption_payload(
            expected_control_version,
            expected_revision_id,
            expected_boundary,
            source_id,
            source_version,
            source_digest,
        );
        if let Some(receipt) = self.idempotent(idempotency_key, "source_adopt", &payload)? {
            return Ok(receipt);
        }
        self.guard(idempotency_key, expected_control_version)?;
        if self.state.active_revision_id != expected_revision_id {
            return Err("stale_revision".to_owned());
        }
        if self.state.boundary != *expected_boundary {
            return Err("preview_stale".to_owned());
        }
        if self.state.stop_latched || !self.state.unresolved_operations.is_empty() {
            return Err("source_adoption_not_ready".to_owned());
        }
        if source_id.is_empty()
            || !valid_id(source_id)
            || source_version == 0
            || source_digest.len() != 64
            || !source_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("invalid_source_identity".to_owned());
        }
        self.reserve_events(1)?;
        let next_plan_epoch = self
            .state
            .plan_epoch
            .checked_add(1)
            .ok_or_else(|| "plan_epoch_exhausted".to_owned())?;
        self.state
            .control_version
            .checked_add(1)
            .ok_or_else(|| "control_version_exhausted".to_owned())?;
        self.state.active_revision_id = format!("revision-{next_plan_epoch}");
        self.state.plan_epoch = next_plan_epoch;
        self.bump_boundary();
        let receipt = self.receipt(idempotency_key, "revision_committed");
        self.persist_command(idempotency_key, "source_adopt", &payload, &receipt);
        self.event("source.revision_adopted", Some(&receipt.command_id), None)?;
        Ok(receipt)
    }

    /// Looks up the historical source-aware receipt without changing authority state.
    ///
    /// Publication adoption also writes an outer owner-control receipt using the public Commit
    /// command shape. Callers use this read-only lookup to verify that an exact outer replay is
    /// backed by the inner source identity that originally authorized the transition.
    #[allow(clippy::too_many_arguments)]
    pub fn lookup_source_adoption_receipt(
        &self,
        idempotency_key: &str,
        expected_control_version: u64,
        expected_revision_id: &str,
        expected_boundary: &ContextBoundary,
        source_id: &str,
        source_version: u64,
        source_digest: &str,
    ) -> Result<Option<ControlReceipt>, String> {
        let payload = source_adoption_payload(
            expected_control_version,
            expected_revision_id,
            expected_boundary,
            source_id,
            source_version,
            source_digest,
        );
        self.idempotent(idempotency_key, "source_adopt", &payload)
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
        self.reserve_events(1)?;
        self.state.pause_latched = false;
        self.state.gate_epoch = self.state.gate_epoch.saturating_add(1);
        self.bump_boundary();
        self.state.status = GateStatus::Running;
        let receipt = self.receipt(idempotency_key, "resume_accepted");
        self.persist_command(idempotency_key, "resume", &payload, &receipt);
        self.event("resume.accepted", Some(&receipt.command_id), None)?;
        Ok(receipt)
    }
}

fn source_adoption_payload(
    expected_control_version: u64,
    expected_revision_id: &str,
    expected_boundary: &ContextBoundary,
    source_id: &str,
    source_version: u64,
    source_digest: &str,
) -> String {
    format!(
        "{expected_control_version}|{expected_revision_id}|{source_id}|{source_version}|{source_digest}|{}",
        serde_json::to_string(expected_boundary).unwrap_or_default()
    )
}
