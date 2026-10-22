// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sts2_harness::management::{
    AuthContext, ContextOwnerControlLimits, LiveContextObservationPort, LiveProviderSessionFactory,
    LiveRuntimeSessionFactory, LiveTargetCatalogPort, LiveWorkflowSessionFactory, ManagementError,
    ProductionLiveWorkflowSessionFactory, RunRequest, RuntimeAuthorityBinding,
    TargetCatalogResponse, UnavailableLiveProviderPolicyPort, WorkflowStore, live_run_id,
};
use sts2_harness::provider_session::NativeCapabilities;
use sts2_harness::workflow::WorkflowDefinition;
use sts2_harness::{
    ActionIdentity, BarrierError, BarrierPort, DecisionSource, EpisodeLegalAction,
    EpisodeLegalActionSet, EpisodeObservation, EpisodeRuntimePort, PortError, RecoveryError,
    RecoveryPort, RuntimeLeaseBinding, ShutdownError, ShutdownPort, TransitionReceipt, WaitSample,
};

#[derive(Default)]
pub struct Counts {
    pub opens: AtomicUsize,
    pub launches: AtomicUsize,
    pub observations: AtomicUsize,
    pub providers: AtomicUsize,
    pub stops: AtomicUsize,
    pub releases: AtomicUsize,
}

struct Catalog;
impl LiveTargetCatalogPort for Catalog {
    fn target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        super::support::FakeFactory::new(false).target_catalog(actor)
    }
}

pub fn factory(
    store: Arc<dyn WorkflowStore>,
    counts: Arc<Counts>,
    retryable: bool,
    cleanup_failure: bool,
) -> Arc<dyn LiveWorkflowSessionFactory> {
    Arc::new(
        ProductionLiveWorkflowSessionFactory::new(
            super::support::capabilities(),
            Arc::new(Catalog),
            Arc::new(RuntimeFactory {
                store,
                counts: counts.clone(),
                retryable,
                cleanup_failure,
            }),
            Arc::new(Provider(counts)),
            Arc::new(UnavailableLiveProviderPolicyPort),
            NativeCapabilities::fixture(),
        )
        .expect("production session factory")
        .with_context_observations(Arc::new(ObservationOwner)),
    )
}

struct RuntimeFactory {
    store: Arc<dyn WorkflowStore>,
    counts: Arc<Counts>,
    retryable: bool,
    cleanup_failure: bool,
}

impl LiveRuntimeSessionFactory for RuntimeFactory {
    fn open_runtime(
        &self,
        request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn EpisodeRuntimePort + Send>, ManagementError> {
        self.counts.opens.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Runtime {
            store: self.store.clone(),
            counts: self.counts.clone(),
            retryable: self.retryable,
            cleanup_failure: self.cleanup_failure,
            lease: RuntimeLeaseBinding {
                instance_id: request.instance_id.clone(),
                session_id: "fence-session".into(),
                run_id: live_run_id(request, definition_digest)?,
                lease_id: "fence-lease".into(),
                lease_epoch: 1,
            },
        }))
    }

    fn authority_binding(
        &self,
        request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<RuntimeAuthorityBinding, ManagementError> {
        Ok(RuntimeAuthorityBinding {
            instance_id: request.instance_id.clone(),
            session_id: "fence-session".into(),
            lease_id: "fence-lease".into(),
            lease_epoch: 1,
            run_id: live_run_id(request, definition_digest)?,
            episode_id: "fence-episode".into(),
            trajectory_id: "fence-trajectory".into(),
            trace_id: "fence-trace".into(),
            artifact_id: "fence-artifact".into(),
            agent_id: "fence-agent".into(),
            adapter_revision: "fixture-adapter".into(),
            model_revision: "fixture-model".into(),
            configuration_digest: "a".repeat(64),
            output_schema_digest: "b".repeat(64),
        })
    }
}

struct Runtime {
    store: Arc<dyn WorkflowStore>,
    counts: Arc<Counts>,
    retryable: bool,
    cleanup_failure: bool,
    lease: RuntimeLeaseBinding,
}

impl EpisodeRuntimePort for Runtime {
    fn launch(&mut self) -> Result<(), PortError> {
        assert!(
            self.store
                .get_run(&self.lease.run_id)
                .expect("reservation lookup")
                .is_some(),
            "production launch must follow the durable run reservation"
        );
        self.counts.launches.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn current_lease_binding(&mut self) -> Result<RuntimeLeaseBinding, PortError> {
        Ok(self.lease.clone())
    }
    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        self.counts.observations.fetch_add(1, Ordering::SeqCst);
        Err(PortError::new(
            "fixture_observe",
            "injected launch observation failure",
            self.retryable,
        ))
    }
    fn legal_actions(
        &mut self,
        _state_id: &str,
        _generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError> {
        Err(PortError::new(
            "unexpected_catalog",
            "observe must refuse first",
            false,
        ))
    }
    fn dispatch_action(
        &mut self,
        _identity: &ActionIdentity,
        _action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError> {
        Err(PortError::new(
            "unexpected_action",
            "observe must refuse first",
            false,
        ))
    }
}
impl BarrierPort for Runtime {
    fn wait_for_transition(
        &mut self,
        _operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        Err(BarrierError::PortFailure)
    }
}
impl RecoveryPort for Runtime {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        Err(RecoveryError::Unsupported)
    }
    fn reconcile(&mut self, _operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        Err(RecoveryError::Unsupported)
    }
    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        self.counts.releases.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        self.counts.stops.fetch_add(1, Ordering::SeqCst);
        if self.cleanup_failure {
            Err(RecoveryError::PortFailure)
        } else {
            Ok(())
        }
    }
}
impl ShutdownPort for Runtime {
    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }
    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }
    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }
}

struct Provider(Arc<Counts>);
impl LiveProviderSessionFactory for Provider {
    fn open_provider(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        _definition_digest: &str,
    ) -> Result<Box<dyn DecisionSource + Send>, ManagementError> {
        self.0.providers.fetch_add(1, Ordering::SeqCst);
        Err(ManagementError::unavailable(
            "unexpected_provider",
            "provider must never be opened",
        ))
    }
}

struct ObservationOwner;
impl LiveContextObservationPort for ObservationOwner {
    fn record_observation(
        &self,
        _actor: &AuthContext,
        _request: &RunRequest,
        _definition_digest: &str,
        _binding: &RuntimeAuthorityBinding,
        _observation: &EpisodeObservation,
        _control_limits: &ContextOwnerControlLimits,
    ) -> Result<(), ManagementError> {
        Err(ManagementError::invalid(
            "unexpected_record",
            "observe must refuse first",
        ))
    }
    fn record_legal_actions(
        &self,
        _actor: &AuthContext,
        _request: &RunRequest,
        _definition_digest: &str,
        _binding: &RuntimeAuthorityBinding,
        _actions: &EpisodeLegalActionSet,
    ) -> Result<(), ManagementError> {
        Err(ManagementError::invalid(
            "unexpected_record",
            "observe must refuse first",
        ))
    }
    fn invalidate(&self, _actor: &AuthContext, _request: &RunRequest, _definition_digest: &str) {}
}
