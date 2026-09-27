// SPDX-License-Identifier: MIT

//! Idempotency, receipt, and event-ledger bookkeeping for the control authority.
//!
//! These are the private helpers every mutating command shares: the guard that refuses a stale
//! control version, the receipt that mints a command id, the command digest that makes a replay
//! safe, and the bounded event ledger that refuses to record past the selected transition bound.
//! They are split from the transitions themselves so the rules that protect the journal can be
//! read, and changed, as one unit.

use super::super::state::{ControlAuthority, ControlEvent, ControlReceipt};
use super::digest::command_digest;

impl ControlAuthority {
    pub(super) fn idempotent(
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

    pub(super) fn guard(&self, key: &str, expected_control_version: u64) -> Result<(), String> {
        if key.is_empty() || key.len() > 128 {
            return Err("invalid_command".to_owned());
        }
        if self.state.control_version != expected_control_version {
            return Err("stale_control".to_owned());
        }
        Ok(())
    }

    pub(super) fn receipt(&mut self, key: &str, effect: &str) -> ControlReceipt {
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

    pub(super) fn persist_command(
        &mut self,
        key: &str,
        kind: &str,
        payload: &str,
        receipt: &ControlReceipt,
    ) {
        self.commands.insert(
            key.to_owned(),
            (command_digest(key, kind, payload), receipt.clone()),
        );
    }

    /// Reserves room for `count` recorded transitions before any state changes.
    ///
    /// Each mutator that records a transition reserves its capacity first, so an authority that has
    /// reached its bound refuses without advancing the plan, the boundary or the operation ledger.
    /// [`Self::event`] keeps its own guard as a defensive backstop rather than the primary check.
    pub(super) fn reserve_events(&self, count: u64) -> Result<(), String> {
        if (self.events.len() as u64).saturating_add(count) > self.max_control_events {
            return Err("context_control_events_exhausted".to_owned());
        }
        Ok(())
    }

    pub(super) fn event(
        &mut self,
        event_type: &str,
        command_id: Option<&str>,
        reason: Option<&str>,
    ) -> Result<(), String> {
        if self.events.len() as u64 >= self.max_control_events {
            return Err("context_control_events_exhausted".to_owned());
        }
        self.events.push(ControlEvent {
            sequence: self.events.len() as u64 + 1,
            event_type: event_type.to_owned(),
            command_id: command_id.map(str::to_owned),
            reason: reason.map(str::to_owned),
        });
        Ok(())
    }

    pub(super) fn bump_boundary(&mut self) {
        self.state.control_version = self.state.control_version.saturating_add(1);
        self.state.boundary.control_version = self.state.control_version;
        self.state.boundary.gate_epoch = self.state.gate_epoch;
    }
}
