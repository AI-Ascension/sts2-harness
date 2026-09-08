// SPDX-License-Identifier: MIT

use sts2_harness::ResumeState;

use super::super::worker_store::{
    begin_quarantine, finish_quarantine, try_lock_quarantine, try_lock_recovery,
};
use super::DurableHandle;

impl DurableHandle {
    pub(in super::super) fn mark_interrupted_unknown(&self, reason: &str) -> Result<(), String> {
        if begin_quarantine(&self.store)? {
            let state = try_lock_recovery(&self.store)
                .map_err(|error| {
                    format!(
                        "cannot inspect runtime-v3 execution state while preserving interrupted-unknown quarantine: {error}"
                    )
                })?
                .resume_episode(&self.lineage.episode_id, &self.fingerprint)
                .map_err(|error| {
                    format!(
                        "cannot inspect runtime-v3 execution state while preserving interrupted-unknown quarantine: {error}"
                    )
                })?;
            if matches!(state, ResumeState::InterruptedUnknown { .. }) {
                return Ok(());
            }
        }
        let mut store = try_lock_quarantine(&self.store).map_err(|error| {
            format!(
                "cannot acquire runtime-v3 execution store for interrupted-unknown quarantine: {error}"
            )
        })?;
        store
            .mark_interrupted_unknown(&self.lineage.episode_id, reason)
            .map_err(|error| {
                format!("cannot persist runtime-v3 interrupted-unknown quarantine: {error}")
            })?;
        finish_quarantine(&self.store);
        Ok(())
    }

    pub(in super::super) fn quarantine_failure(&self, original: String, reason: &str) -> String {
        match self.mark_interrupted_unknown(reason) {
            Ok(()) => original,
            Err(error) => {
                format!("{original}; failed to persist interrupted-unknown quarantine: {error}")
            }
        }
    }
}
