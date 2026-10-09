// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::super::super::runtime_v3_admission::RuntimeV3AdmissionMode;
use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use sts2_harness::EpisodeRuntimePort;
use sts2_harness::management::{
    AuthContext, ExecutionMode, InferenceProfileCatalog, InferenceProfileState,
    LiveInferenceProfileCatalogPort, LiveProviderPolicyPort, LiveProviderSessionFactory,
    LiveRuntimeSessionFactory, LiveWorkflowSessionFactory, ManagementError,
    ProductionLiveWorkflowSessionFactory, RunRequest, RunTargetConfiguration,
    RuntimeAuthorityBinding, TargetAdmissionBinding, UnavailableLiveProviderPolicyPort,
};
use sts2_harness::provider_session::NativeCapabilities;
use sts2_harness::workflow::WorkflowDefinition;

#[derive(Clone)]
struct StaticCatalog(InferenceProfileCatalog);

impl LiveInferenceProfileCatalogPort for StaticCatalog {
    fn inference_profile_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<InferenceProfileCatalog, ManagementError> {
        Ok(self.0.clone())
    }
}

struct RuntimeEffects {
    authority: AtomicUsize,
    opens: AtomicUsize,
}

struct Runtime(Arc<RuntimeEffects>);

impl LiveRuntimeSessionFactory for Runtime {
    fn open_runtime(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        _definition_digest: &str,
    ) -> Result<Box<dyn EpisodeRuntimePort + Send>, ManagementError> {
        self.0.opens.fetch_add(1, Ordering::SeqCst);
        Err(ManagementError::unavailable(
            "test_runtime_open_stopped",
            "test stops after the served factory admits the legacy profile reference",
        ))
    }

    fn authority_binding(
        &self,
        request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<RuntimeAuthorityBinding, ManagementError> {
        self.0.authority.fetch_add(1, Ordering::SeqCst);
        Ok(RuntimeAuthorityBinding {
            instance_id: request.instance_id.clone(),
            session_id: "session-1".to_owned(),
            lease_id: "lease-1".to_owned(),
            lease_epoch: 1,
            run_id: sts2_harness::management::live_run_id(request, definition_digest)?,
            episode_id: "episode-1".to_owned(),
            trajectory_id: "trajectory-1".to_owned(),
            trace_id: "trace-1".to_owned(),
            artifact_id: "artifact-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            adapter_revision: "exo.runtime-v3".to_owned(),
            model_revision: "model-reviewed-1".to_owned(),
            configuration_digest: "a".repeat(64),
            output_schema_digest: "b".repeat(64),
        })
    }
}

struct CountingProvider {
    inner: Provider,
    opens: Arc<AtomicUsize>,
    preflights: Arc<AtomicUsize>,
}

impl LiveProviderSessionFactory for CountingProvider {
    fn open_provider(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn sts2_harness::DecisionSource + Send>, ManagementError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        self.inner
            .open_provider(request, actor, definition, definition_digest)
    }

    fn prepare_profiled_provider(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &WorkflowDefinition,
        definition_digest: &str,
        authority_binding: &RuntimeAuthorityBinding,
        profiles: &sts2_harness::management::AdmittedInferenceProfileDispatch,
    ) -> Result<Box<dyn sts2_harness::management::LiveProviderSessionAdmission>, ManagementError>
    {
        self.preflights.fetch_add(1, Ordering::SeqCst);
        self.inner.prepare_profiled_provider(
            request,
            actor,
            definition,
            definition_digest,
            authority_binding,
            profiles,
        )
    }
}

fn unsupported_catalog() -> InferenceProfileCatalog {
    let mut catalog =
        sts2_harness::management::synthetic_inference_profile_catalog().expect("synthetic catalog");
    for descriptor in &mut catalog.descriptors {
        if descriptor.profile_id == "decision.synthetic.v1" {
            let mut unsupported = descriptor.clone();
            unsupported.state = InferenceProfileState::Unsupported;
            *descriptor = unsupported.seal().expect("seal unsupported descriptor");
        }
    }
    let catalog = catalog.seal().expect("seal unsupported catalog");
    catalog.validate().expect("validate unsupported catalog");
    catalog
}

fn served_factory(
    mode: RuntimeV3AdmissionMode,
    catalog: Arc<dyn LiveInferenceProfileCatalogPort>,
) -> (
    ProductionLiveWorkflowSessionFactory,
    Arc<RuntimeEffects>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    let capabilities = NativeCapabilities::fixture();
    let runtime_effects = Arc::new(RuntimeEffects {
        authority: AtomicUsize::new(0),
        opens: AtomicUsize::new(0),
    });
    let provider_opens = Arc::new(AtomicUsize::new(0));
    let provider_preflights = Arc::new(AtomicUsize::new(0));
    let provider = CountingProvider {
        inner: Provider {
            admission_mode: mode,
            provider_capabilities: capabilities.clone(),
        },
        opens: Arc::clone(&provider_opens),
        preflights: Arc::clone(&provider_preflights),
    };
    let factory = ProductionLiveWorkflowSessionFactory::new(
        serde_json::json!({
            "schema_version":"ascension.capabilities/v1",
            "capabilities":[
                "workflow.live", "workflow.node.observe.v1", "workflow.node.decide.v1",
                "workflow.node.execute_action.v1", "workflow.node.terminal.v1",
                "workflow.execution.fence.mcp-observation.v1", "observe.fair-play.v1",
                "actions.catalog.v1", "actions.settlement.v1",
                "workflow.projection.fair-play.live.v1", "workflow.provider.decision.live.v1",
                "workflow.context.context.live.v1"
            ]
        }),
        Arc::new(super::super::Catalog),
        Arc::new(Runtime(Arc::clone(&runtime_effects))),
        Arc::new(provider),
        Arc::new(UnavailableLiveProviderPolicyPort) as Arc<dyn LiveProviderPolicyPort>,
        capabilities,
    )
    .expect("valid production factory");
    let factory = super::super::inference_profiles::attach_profile_catalog(factory, mode, catalog);
    (
        factory,
        runtime_effects,
        provider_opens,
        provider_preflights,
    )
}

fn request_and_definition() -> (RunRequest, WorkflowDefinition, String) {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("valid strict workflow");
    let digest = sts2_harness::management::digest_value(&value).expect("definition digest");
    let definition = serde_json::from_value(value.clone()).expect("workflow definition");
    let request = RunRequest {
        schema_version: "ascension.workflow-run/v1".to_owned(),
        request_id: "legacy-profile-request".to_owned(),
        definition: Some(value),
        artifact_id: None,
        instance_id: "instance-1".to_owned(),
        profile: "live.workflow.v1".to_owned(),
        admission: Some(TargetAdmissionBinding {
            schema_version: "ascension.workflow-admission/v1".to_owned(),
            request_id: "legacy-profile-request".to_owned(),
            workflow_definition_digest: digest.clone(),
            target: RunTargetConfiguration {
                instance_id: "instance-1".to_owned(),
                execution_profile: "live.workflow.v1".to_owned(),
                execution_mode: ExecutionMode::Live,
                workflow_revision: digest.clone(),
                compatibility_revision: "runtime-v3.mcp.v1".to_owned(),
                capability_revision: "runtime-v3.mcp-observation-fence.v1".to_owned(),
                game_profile: "sts2-live-v1".to_owned(),
                save_profile: None,
                inference_profile: None,
                context_capability: None,
                provider_capability: None,
            },
            descriptor_digest: "c".repeat(64),
            catalog_revision: "runtime-v3-test".to_owned(),
        }),
    };
    (request, definition, digest)
}

fn actor() -> AuthContext {
    AuthContext::new("served-profile-test", ["workflow:*".to_owned()]).expect("actor")
}

#[test]
fn explicit_legacy_served_factory_keeps_no_catalog_and_admits_the_existing_profile_reference() {
    let catalog = Arc::new(StaticCatalog(unsupported_catalog()));
    let (factory, effects, provider_opens, provider_preflights) =
        served_factory(RuntimeV3AdmissionMode::Legacy, catalog);
    let actor = actor();
    assert_eq!(
        factory
            .inference_profile_catalog(&actor)
            .expect("legacy has no served catalog"),
        None
    );

    let (request, definition, digest) = request_and_definition();
    let error = factory
        .open(&request, &actor, &definition, &digest)
        .err()
        .expect("test stops after legacy admission reaches runtime open");
    assert_eq!(error.code, "test_runtime_open_stopped");
    assert_eq!(effects.authority.load(Ordering::SeqCst), 1);
    assert_eq!(effects.opens.load(Ordering::SeqCst), 1);
    assert_eq!(provider_opens.load(Ordering::SeqCst), 0);
    assert_eq!(provider_preflights.load(Ordering::SeqCst), 0);
}

#[test]
fn enveloped_served_factory_refuses_unsupported_profile_before_runtime_or_provider_effects() {
    let catalog = Arc::new(StaticCatalog(unsupported_catalog()));
    let (factory, effects, provider_opens, provider_preflights) =
        served_factory(RuntimeV3AdmissionMode::Enveloped, catalog);
    let actor = actor();
    let served = factory
        .inference_profile_catalog(&actor)
        .expect("enveloped factory serves catalog")
        .expect("enveloped factory attaches catalog");
    assert!(
        served
            .descriptors
            .iter()
            .any(|descriptor| descriptor.state == InferenceProfileState::Unsupported)
    );

    let (request, definition, digest) = request_and_definition();
    let error = factory
        .open(&request, &actor, &definition, &digest)
        .err()
        .expect("unsupported strict binding must refuse admission");
    assert_eq!(error.code, "inference_profile_unsupported");
    assert_eq!(effects.authority.load(Ordering::SeqCst), 0);
    assert_eq!(effects.opens.load(Ordering::SeqCst), 0);
    assert_eq!(provider_opens.load(Ordering::SeqCst), 0);
    assert_eq!(provider_preflights.load(Ordering::SeqCst), 0);
}

#[test]
fn the_actual_legacy_provider_builder_accepts_only_legacy_settings() {
    let settings = super::tests::runtime_settings(super::tests::exo_config());
    let source = provider::legacy_decision_source(settings).expect("legacy source adapter builds");
    assert!(source.managed_render_config().is_some());
}

#[test]
fn frozen_mode_rejects_environment_or_settings_mode_changes_before_fallback() {
    assert!(
        provider::validate_admission_mode(
            RuntimeV3AdmissionMode::Legacy,
            RuntimeV3AdmissionMode::Legacy,
            Some(RuntimeV3AdmissionMode::Legacy),
            RuntimeV3AdmissionMode::Legacy,
        )
        .is_ok()
    );
    assert!(
        provider::validate_admission_mode(
            RuntimeV3AdmissionMode::Legacy,
            RuntimeV3AdmissionMode::Enveloped,
            Some(RuntimeV3AdmissionMode::Enveloped),
            RuntimeV3AdmissionMode::Legacy,
        )
        .is_err()
    );
    assert!(
        provider::validate_admission_mode(
            RuntimeV3AdmissionMode::Enveloped,
            RuntimeV3AdmissionMode::Legacy,
            Some(RuntimeV3AdmissionMode::Legacy),
            RuntimeV3AdmissionMode::Enveloped,
        )
        .is_err()
    );
    assert!(
        provider::validate_admission_mode(
            RuntimeV3AdmissionMode::Enveloped,
            RuntimeV3AdmissionMode::Enveloped,
            Some(RuntimeV3AdmissionMode::Legacy),
            RuntimeV3AdmissionMode::Enveloped,
        )
        .is_err()
    );
}
