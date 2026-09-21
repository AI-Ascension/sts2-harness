// SPDX-License-Identifier: MIT

use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use sts2_harness::management::{
    AuthContext, BoundaryCaptureSink, DurableProviderSessionPolicyCommandPort,
    EnvironmentAuthenticator, ExecutionMode, LiveProviderPolicyPort, LiveProviderSessionFactory,
    LiveRuntimeSessionFactory, LiveTargetCatalogPort, LiveWorkflowSessionFactory, ManagementError,
    ProductionLiveWorkflowSessionFactory, ProviderSessionPolicyCommandPort,
    ProviderSessionPolicyOwnerPort, RunRequest, RuntimeAuthorityBinding, TargetAvailability,
    TargetCatalogResponse, TargetDescriptor,
};
use sts2_harness::provider_session::{
    NativeCapabilities, ProviderSessionMetadataStore, ProviderSessionPolicyOwner, SessionScope,
};
use sts2_harness::workflow::WorkflowDefinition;
use zeroize::Zeroize;

use super::production_context_owner;
use super::{RuntimeConfig, runtime_v3, runtime_v3_admission, runtime_v3_settings};

pub(super) fn serve() -> Result<(), String> {
    let listen: std::net::SocketAddr = std::env::var("STS2_WORKFLOW_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8787".to_owned())
        .parse()
        .map_err(|_| String::from("STS2_WORKFLOW_LISTEN must be a socket address"))?;
    if !listen.ip().is_loopback() {
        return Err(String::from("STS2_WORKFLOW_LISTEN must be loopback"));
    }
    let store = required("STS2_WORKFLOW_STORE")?;
    let profile = required("STS2_WORKFLOW_AUTH_PROFILE")?;
    let authenticator =
        Arc::new(EnvironmentAuthenticator::from_profile(&profile).map_err(|e| e.to_string())?);
    let policy = ProviderPolicyConfiguration::from_environment()?;
    let context_owner = Arc::new(production_context_owner::Owner::open(
        production_context_owner::Configuration::from_environment()?,
    )?);
    let owner = Arc::new(policy.open_owner()?);
    let provider_policy: Arc<dyn LiveProviderPolicyPort> =
        Arc::new(ProviderSessionPolicyOwnerPort::new(Arc::clone(&owner)));
    let command_port: Arc<dyn ProviderSessionPolicyCommandPort> = Arc::new(
        DurableProviderSessionPolicyCommandPort::new(Arc::clone(&owner)),
    );
    // The served effective-limits record is built from the same descriptor the
    // session factory admits provider sessions against.
    let served_capabilities = policy.capabilities.clone();
    sts2_harness::management::serve_live_with_lifecycle(
        listen,
        &store,
        authenticator,
        factory(
            Arc::clone(&provider_policy),
            policy.scope,
            policy.capabilities,
            Arc::clone(&context_owner),
        )?,
        sts2_harness::management::ServedOwnerPorts {
            provider_policy,
            command_port,
            context_owner,
        },
        served_capabilities,
        lifecycle_owner::owner()?,
    )
    .map_err(|error| error.to_string())
}

#[path = "workflow_service_lifecycle.rs"]
mod lifecycle_owner;

fn factory(
    provider_policy: Arc<dyn LiveProviderPolicyPort>,
    policy_scope: SessionScope,
    provider_capabilities: NativeCapabilities,
    context_owner: Arc<production_context_owner::Owner>,
) -> Result<Arc<dyn LiveWorkflowSessionFactory>, String> {
    let observations: Arc<dyn sts2_harness::LiveContextObservationPort> = context_owner.clone();
    let render: Arc<dyn sts2_harness::LiveContextRenderPort> = context_owner.clone();
    Ok(Arc::new(ProductionLiveWorkflowSessionFactory::new(
        json!({"schema_version":"ascension.capabilities/v1","capabilities":["workflow.live","workflow.node.observe.v1","workflow.node.decide.v1","workflow.node.execute_action.v1","workflow.node.terminal.v1","workflow.execution.fence.mcp-observation.v1","observe.fair-play.v1","actions.catalog.v1","actions.settlement.v1","workflow.projection.fair-play.live.v1","workflow.provider.decision.live.v1","workflow.context.context.live.v1"]}),
        Arc::new(Catalog),
        Arc::new(Runtime {
            policy_scope,
            provider_capabilities: provider_capabilities.clone(),
        }),
        Arc::new(Provider),
        provider_policy,
        provider_capabilities.clone(),
    ).map_err(|error| error.to_string())?
        .with_context_observations(observations)
        .with_context_render_port(render)
        // The served boundary records through this sink: the approved material and the bytes the
        // boundary wrote are one value only if the sink records the release before the write, so the
        // served composition attaches a recording ring rather than the inert default.
        .with_capture_sink(
            BoundaryCaptureSink::memory_ring()
                .map_err(|error| format!("served boundary capture configuration: {error}"))?,
        )
        .with_inference_profile_catalog(Arc::new(
            inference_profiles::InferenceProfileCatalogProducer::new(provider_capabilities),
        ))))
}

#[path = "workflow_service_inference_profiles.rs"]
mod inference_profiles;
#[path = "workflow_service_policy.rs"]
mod policy;
use policy::ProviderPolicyConfiguration;

fn valid_environment_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            matches!(byte, b'A'..=b'Z' | b'0'..=b'9' | b'_')
                && (index != 0 || matches!(byte, b'A'..=b'Z' | b'_'))
        })
}

fn provider_policy_key(reference: &str) -> Result<[u8; 32], String> {
    let encoded = std::env::var(reference)
        .map_err(|_| format!("provider-policy key reference {reference} is unavailable"))?;
    let bytes = encoded.as_bytes();
    if bytes.len() != 64 {
        return Err(String::from(
            "provider-policy key must be exactly 64 hexadecimal characters",
        ));
    }
    let mut key = [0_u8; 32];
    for (index, slot) in key.iter_mut().enumerate() {
        let high = hex_nibble(bytes[index * 2])
            .ok_or_else(|| String::from("provider-policy key must be hexadecimal"))?;
        let low = hex_nibble(bytes[index * 2 + 1])
            .ok_or_else(|| String::from("provider-policy key must be hexadecimal"))?;
        *slot = (high << 4) | low;
    }
    if key.iter().all(|value| *value == 0) {
        return Err(String::from("provider-policy key must not be all zeroes"));
    }
    Ok(key)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

struct Catalog;
impl LiveTargetCatalogPort for Catalog {
    fn target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        if !actor.can("workflow:run") {
            return Err(ManagementError::forbidden(
                "target_scope_denied",
                "actor cannot discover live targets",
            ));
        }
        let config = RuntimeConfig::from_environment()
            .map_err(|e| ManagementError::unavailable("runtime_configuration", e))?;
        Ok(TargetCatalogResponse {
            schema_version: "ascension.workflow-targets/v1".to_owned(),
            catalog_revision: format!("runtime-v3:{}", config.runtime_profile),
            targets: vec![TargetDescriptor {
                instance_id: config.instance_id,
                execution_profiles: vec!["live.workflow.v1".to_owned()],
                execution_mode: ExecutionMode::Live,
                compatibility_revision: "runtime-v3.mcp.v1".to_owned(),
                capability_revision: "runtime-v3.mcp-observation-fence.v1".to_owned(),
                availability: TargetAvailability::Available,
                supported_operations: vec!["workflow:live".to_owned()],
                capabilities: vec![
                    "workflow.live".to_owned(),
                    "workflow.execution.fence.mcp-observation.v1".to_owned(),
                    "observe.fair-play.v1".to_owned(),
                    "actions.catalog.v1".to_owned(),
                    "actions.settlement.v1".to_owned(),
                    "workflow.projection.fair-play.live.v1".to_owned(),
                    "workflow.provider.decision.live.v1".to_owned(),
                    "workflow.context.context.live.v1".to_owned(),
                ],
                game_profiles: vec!["sts2-live-v1".to_owned()],
                save_profiles: Vec::new(),
                inference_profiles: vec!["exo.runtime-v3".to_owned()],
            }],
        })
    }
}
struct Runtime {
    policy_scope: SessionScope,
    provider_capabilities: NativeCapabilities,
}
impl LiveRuntimeSessionFactory for Runtime {
    fn open_runtime(
        &self,
        request: &RunRequest,
        _: &AuthContext,
        _: &WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn sts2_harness::EpisodeRuntimePort + Send>, ManagementError> {
        let config = RuntimeConfig::from_environment()
            .map_err(|e| ManagementError::unavailable("runtime_configuration", e))?;
        if config.instance_id != request.instance_id {
            return Err(ManagementError::conflict(
                "runtime_instance_mismatch",
                "configured runtime differs from target",
            ));
        }
        validate_runtime_lineage(&config, &self.policy_scope, request, definition_digest)?;
        runtime_v3::RuntimeV3SessionWorker::start(config)
            .map(|worker| Box::new(worker) as Box<dyn sts2_harness::EpisodeRuntimePort + Send>)
            .map_err(|e| ManagementError::unavailable("runtime_worker_start", e))
    }

    fn authority_binding(
        &self,
        request: &RunRequest,
        _: &AuthContext,
        _: &WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<RuntimeAuthorityBinding, ManagementError> {
        let config = RuntimeConfig::from_environment()
            .map_err(|error| ManagementError::unavailable("runtime_configuration", error))?;
        if config.instance_id != request.instance_id {
            return Err(ManagementError::conflict(
                "runtime_instance_mismatch",
                "configured runtime differs from target",
            ));
        }
        validate_runtime_lineage(&config, &self.policy_scope, request, definition_digest)?;
        let settings = runtime_v3_settings::RuntimeV3Settings::from_environment(&config)
            .map_err(|error| ManagementError::unavailable("runtime_configuration", error))?;
        let configuration_digest = runtime_v3::authority_configuration_digest(&config, &settings)
            .map_err(|error| {
            ManagementError::unavailable("runtime_configuration_digest", error)
        })?;
        Ok(RuntimeAuthorityBinding {
            instance_id: config.instance_id,
            session_id: config.session_id,
            lease_id: config.lease_id,
            lease_epoch: config.lease_epoch,
            run_id: config.run_id,
            episode_id: config.episode_id,
            trajectory_id: config.trajectory_id,
            trace_id: config.trace_id,
            artifact_id: config.artifact_id,
            agent_id: self.policy_scope.agent_id.clone(),
            adapter_revision: self.provider_capabilities.binding.adapter_revision.clone(),
            model_revision: self.provider_capabilities.binding.model_revision.clone(),
            configuration_digest,
            output_schema_digest: self.provider_capabilities.native_schema_sha256.clone(),
        })
    }
}

fn validate_runtime_lineage(
    config: &RuntimeConfig,
    policy_scope: &SessionScope,
    request: &RunRequest,
    definition_digest: &str,
) -> Result<(), ManagementError> {
    let workflow_run_id = sts2_harness::management::live_run_id(request, definition_digest)?;
    if config.run_id != workflow_run_id
        || policy_scope.run_id != workflow_run_id
        || policy_scope.episode_id != config.episode_id
    {
        return Err(ManagementError::conflict(
            "runtime_run_lineage_mismatch",
            "configured runtime and provider policy are not bound to the admitted workflow run",
        ));
    }
    Ok(())
}
struct Provider;
impl LiveProviderSessionFactory for Provider {
    fn open_provider(
        &self,
        _: &RunRequest,
        _: &AuthContext,
        _: &WorkflowDefinition,
        _: &str,
    ) -> Result<Box<dyn sts2_harness::DecisionSource + Send>, ManagementError> {
        let config = RuntimeConfig::from_environment()
            .map_err(|e| ManagementError::unavailable("runtime_configuration", e))?;
        let settings = runtime_v3_settings::RuntimeV3Settings::from_environment(&config)
            .map_err(|e| ManagementError::unavailable("provider_configuration", e))?;
        let transport = runtime_v3_admission::admit(&settings.admission, settings.process)
            .map_err(|e| ManagementError::unavailable("provider_admission", e))?;
        Ok(Box::new(sts2_harness::ExoDecisionSource::new(
            sts2_harness::ExoSession::new(sts2_harness::ExoProvider::new(transport, settings.exo)),
        )))
    }
}
fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}
