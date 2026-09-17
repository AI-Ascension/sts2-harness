// SPDX-License-Identifier: MIT

use super::*;

/// Wraps the recording fake so the authored observe node can be held inside a
/// first command while another command reaches the execution boundary.
pub(super) struct GatedFactory {
    pub(super) inner: Arc<FakeFactory>,
    pub(super) gate: Arc<SessionGate>,
    pub(super) timeout_after_wait_gate: bool,
}

impl LiveWorkflowSessionFactory for GatedFactory {
    fn capabilities(&self) -> Value {
        self.inner.capabilities()
    }

    fn target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        self.inner.target_catalog(actor)
    }

    fn open(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        let inner = self
            .inner
            .open(request, actor, definition, definition_digest)?;
        Ok(Box::new(GatedSession {
            inner,
            gate: Arc::clone(&self.gate),
            timeout_after_wait_gate: self.timeout_after_wait_gate,
        }))
    }

    fn open_admitted(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        definition_digest: &str,
        control_limits: Option<&sts2_harness::management::ContextOwnerControlLimits>,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        let inner = self.inner.open_admitted(
            request,
            actor,
            definition,
            definition_digest,
            control_limits,
        )?;
        Ok(Box::new(GatedSession {
            inner,
            gate: Arc::clone(&self.gate),
            timeout_after_wait_gate: self.timeout_after_wait_gate,
        }))
    }
}
