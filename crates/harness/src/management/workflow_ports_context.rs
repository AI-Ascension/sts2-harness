// SPDX-License-Identifier: MIT

//! The synthetic context-inspection port.
//!
//! Part of the `workflow_ports` split. Refs sts2-harness#570.

use super::auth::AuthContext;
use super::contract::{
    ContextAssociationContext, ContextAvailability, ContextCaptureEvidence, ContextCaptureMode,
    ContextCaptureState, ContextInspectionCapabilities, RunSnapshot,
};
use super::service::{ContextInspectionPort, ContextInspectionResult, ManagementError};

pub(super) struct SyntheticContextInspectionPort;

impl ContextInspectionPort for SyntheticContextInspectionPort {
    fn inspect(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
    ) -> Result<ContextInspectionResult, ManagementError> {
        Ok(ContextInspectionResult {
            context: ContextAssociationContext {
                availability: ContextAvailability::Unavailable,
                context_ref: None,
                run_id: None,
                episode_id: None,
                agent_id: None,
                snapshot_id: None,
                approved_revision_id: None,
                plan_epoch: None,
                reason_code: Some("synthetic_context_adapter_unavailable".to_owned()),
            },
            capture: ContextCaptureEvidence {
                mode: ContextCaptureMode::Unavailable,
                state: ContextCaptureState::Unavailable,
                attempt_id: None,
                reason_code: Some("synthetic_context_adapter_unavailable".to_owned()),
            },
            capabilities: ContextInspectionCapabilities {
                inspect_metadata: true,
                ..ContextInspectionCapabilities::default()
            },
        })
    }
}
