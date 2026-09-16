// SPDX-License-Identifier: MIT

use super::*;

pub(super) struct ReservationObservingFactory {
    pub(super) inner: support::FakeFactory,
    store: Arc<dyn WorkflowStore>,
    pub(super) opens: Arc<AtomicUsize>,
}

impl ReservationObservingFactory {
    pub(super) fn new(store: Arc<dyn WorkflowStore>) -> (Self, Arc<AtomicUsize>) {
        let opens = Arc::new(AtomicUsize::new(0));
        (
            Self {
                inner: support::FakeFactory::new(false),
                store,
                opens: Arc::clone(&opens),
            },
            opens,
        )
    }

    fn open_reserved(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        definition_digest: &str,
        control_limits: Option<&sts2_harness::management::ContextOwnerControlLimits>,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        let run_identity = json!({
            "request_id": request.request_id,
            "instance_id": request.instance_id,
            "definition_digest": definition_digest,
        });
        let run_digest = digest_value(&run_identity).map_err(ManagementError::from)?;
        let run_id = format!("run.live.{}", &run_digest[..32]);
        match self.store.get_run(&run_id).map_err(ManagementError::from)? {
            Some(snapshot)
                if snapshot.admission == request.admission
                    && snapshot.status == WorkflowRunStatus::Created => {}
            _ => {
                return Err(ManagementError::conflict(
                    "reservation_not_persisted",
                    "live factory opened before the created reservation was persisted",
                ));
            }
        }
        let events = self
            .store
            .events(&run_id, 0, 128)
            .map_err(ManagementError::from)?
            .events;
        if !events.first().is_some_and(|event| {
            event.event_type == EventType::RunStarted
                && event.sequence == 1
                && event.run_revision == 1
        }) {
            return Err(ManagementError::conflict(
                "reservation_receipt_not_persisted",
                "live factory opened before the RunStarted receipt was persisted",
            ));
        }
        self.inner.open_admitted(
            request,
            actor,
            definition,
            definition_digest,
            control_limits,
        )
    }
}

impl LiveWorkflowSessionFactory for ReservationObservingFactory {
    fn capabilities(&self) -> serde_json::Value {
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
        self.open_reserved(request, actor, definition, definition_digest, None)
    }

    fn open_admitted(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        definition_digest: &str,
        control_limits: Option<&sts2_harness::management::ContextOwnerControlLimits>,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        self.open_reserved(
            request,
            actor,
            definition,
            definition_digest,
            control_limits,
        )
    }
}
