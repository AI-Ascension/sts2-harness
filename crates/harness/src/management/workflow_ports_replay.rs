// SPDX-License-Identifier: MIT

//! The synthetic replay port.
//!
//! Part of the `workflow_ports` split. Refs sts2-harness#570.

use super::contract::{RunEvent, RunSnapshot};
use super::service::{ManagementError, ReplayResult, WorkflowReplayPort};

impl WorkflowReplayPort for SyntheticReplayPort {
    fn replay(
        &self,
        _request: &super::contract::ReplayRequest,
        snapshot: &RunSnapshot,
        events: &[RunEvent],
    ) -> Result<ReplayResult, ManagementError> {
        let mut expected = 1_u64;
        for event in events {
            if event.sequence != expected
                || event.workflow_run_id != snapshot.workflow_run_id
                || event.definition_digest != snapshot.definition_digest
                || !event.integrity_valid()
            {
                return Ok(ReplayResult {
                    matched: false,
                    compared_events: expected.saturating_sub(1),
                    first_divergence: Some(super::contract::ReplayDivergence {
                        path: format!("/events/{expected}"),
                        code: if event.integrity_valid() {
                            "event_sequence_or_digest".to_owned()
                        } else {
                            "event_integrity".to_owned()
                        },
                    }),
                });
            }
            expected = expected.saturating_add(1);
        }
        Ok(ReplayResult {
            matched: !events.is_empty(),
            compared_events: events.len() as u64,
            first_divergence: None,
        })
    }
}

pub(super) struct SyntheticReplayPort;
