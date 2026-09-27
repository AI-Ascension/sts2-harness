// SPDX-License-Identifier: MIT

//! Durable journal boundary for the control authority.
//!
//! Recovery is the journal boundary. A journal is refused if it carries the wrong schema, retains
//! more transitions than the harness ceiling, or claims an owner epoch that disagrees with the
//! boundary it was written against. A successful recovery advances the controller epoch, so a
//! journal can never be replayed by a second live owner.

use super::super::state::{
    ControlAuthority, ControlEvent, ControlReceipt, ControlState, MAX_CONTROL_EVENTS,
};
use super::super::types::CONTROL_JOURNAL_SCHEMA;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Journal {
    schema: String,
    owner_epoch: u64,
    state: ControlState,
    commands: BTreeMap<String, (String, ControlReceipt)>,
    events: Vec<ControlEvent>,
}

impl ControlAuthority {
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
        if journal.schema != CONTROL_JOURNAL_SCHEMA
            || journal.events.len() as u64 > MAX_CONTROL_EVENTS
        {
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
            max_control_events: MAX_CONTROL_EVENTS,
        })
    }

    /// Recovers the authority under the selected control-transition bound.
    ///
    /// Recovery is the journal boundary: a journal that already retains more transitions than the
    /// selected owner/profile accepts is refused with the precise `context_control_events_exhausted`
    /// reason instead of being loaded and silently saturated later.
    pub fn recover_bounded(journal: &[u8], max_control_events: u64) -> Result<Self, String> {
        Self::recover(journal)?.with_max_control_events(max_control_events)
    }
}
