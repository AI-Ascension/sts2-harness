// SPDX-License-Identifier: MIT

use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use sts2_harness::management::{
    AuthContext, EnvironmentAuthenticator, ExecutionMode, LiveProviderPolicyPort,
    LiveProviderSessionFactory, LiveRuntimeSessionFactory, LiveTargetCatalogPort,
    LiveWorkflowSessionFactory, ManagementError, ProductionLiveWorkflowSessionFactory,
    ProviderSessionPolicyOwnerPort, RunRequest, TargetAvailability, TargetCatalogResponse,
    TargetDescriptor,
};
use sts2_harness::provider_session::{
    NativeCapabilities, ProviderSessionMetadataStore, ProviderSessionPolicyOwner, SessionScope,
};
use sts2_harness::workflow::WorkflowDefinition;
use zeroize::Zeroize;

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
    let owner = Arc::new(policy.open_owner()?);
    let provider_policy: Arc<dyn LiveProviderPolicyPort> =
        Arc::new(ProviderSessionPolicyOwnerPort::new(Arc::clone(&owner)));
    sts2_harness::management::serve_live_with_provider_policy(
        listen,
        &store,
        authenticator,
        factory(Arc::clone(&provider_policy), policy.capabilities)?,
        provider_policy,
    )
    .map_err(|error| error.to_string())
}

fn factory(
    provider_policy: Arc<dyn LiveProviderPolicyPort>,
    provider_capabilities: NativeCapabilities,
) -> Result<Arc<dyn LiveWorkflowSessionFactory>, String> {
    Ok(Arc::new(ProductionLiveWorkflowSessionFactory::new(
        json!({"schema_version":"ascension.capabilities/v1","capabilities":["workflow.live","workflow.node.observe.v1","workflow.node.decide.v1","workflow.node.execute_action.v1","workflow.node.terminal.v1","workflow.execution.fence.mcp-observation.v1","observe.fair-play.v1","actions.catalog.v1","actions.settlement.v1","workflow.projection.fair-play.live.v1","workflow.provider.decision.live.v1","workflow.context.context.live.v1"]}),
        Arc::new(Catalog),
        Arc::new(Runtime),
        Arc::new(Provider),
        provider_policy,
        provider_capabilities,
    ).map_err(|error| error.to_string())?))
}

const PROVIDER_POLICY_CONFIGURATION_SCHEMA: &str = "ascension.workflow-provider-policy-config.v1";
const MAX_PROVIDER_POLICY_CONFIGURATION_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderPolicyConfiguration {
    schema_version: String,
    store_path: std::path::PathBuf,
    key_reference: String,
    scope: SessionScope,
    capabilities: NativeCapabilities,
    selected_profile: String,
}

impl ProviderPolicyConfiguration {
    fn from_environment() -> Result<Self, String> {
        let bytes = required("STS2_WORKFLOW_PROVIDER_POLICY_CONFIG")?;
        if bytes.len() > MAX_PROVIDER_POLICY_CONFIGURATION_BYTES {
            return Err(String::from(
                "STS2_WORKFLOW_PROVIDER_POLICY_CONFIG exceeds its byte bound",
            ));
        }
        let configuration: Self = serde_json::from_str(&bytes).map_err(|_| {
            String::from(
                "STS2_WORKFLOW_PROVIDER_POLICY_CONFIG must be a closed provider-policy configuration",
            )
        })?;
        if configuration.schema_version != PROVIDER_POLICY_CONFIGURATION_SCHEMA
            || !configuration.scope.valid()
            || !valid_environment_name(&configuration.key_reference)
            || configuration.selected_profile != configuration.capabilities.profile_id
        {
            return Err(String::from(
                "STS2_WORKFLOW_PROVIDER_POLICY_CONFIG contains an invalid provider-policy binding",
            ));
        }
        configuration.capabilities.validate().map_err(|_| {
            String::from(
                "STS2_WORKFLOW_PROVIDER_POLICY_CONFIG contains invalid native capabilities",
            )
        })?;
        Ok(configuration)
    }

    fn open_owner(&self) -> Result<ProviderSessionPolicyOwner, String> {
        let mut key = provider_policy_key(&self.key_reference)?;
        let store =
            ProviderSessionMetadataStore::encrypted(&self.store_path, key, self.scope.clone());
        key.zeroize();
        let store = store
            .map_err(|_| String::from("provider-policy metadata store configuration is invalid"))?;
        ProviderSessionPolicyOwner::open(store, self.scope.clone(), self.capabilities.clone())
            .map_err(|_| String::from("provider-policy owner could not be opened"))
    }
}

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
struct Runtime;
impl LiveRuntimeSessionFactory for Runtime {
    fn open_runtime(
        &self,
        request: &RunRequest,
        _: &AuthContext,
        _: &WorkflowDefinition,
        _: &str,
    ) -> Result<Box<dyn sts2_harness::EpisodeRuntimePort + Send>, ManagementError> {
        let config = RuntimeConfig::from_environment()
            .map_err(|e| ManagementError::unavailable("runtime_configuration", e))?;
        if config.instance_id != request.instance_id {
            return Err(ManagementError::conflict(
                "runtime_instance_mismatch",
                "configured runtime differs from target",
            ));
        }
        runtime_v3::RuntimeV3SessionWorker::start(config)
            .map(|worker| Box::new(worker) as Box<dyn sts2_harness::EpisodeRuntimePort + Send>)
            .map_err(|e| ManagementError::unavailable("runtime_worker_start", e))
    }
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
