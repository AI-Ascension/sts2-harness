// SPDX-License-Identifier: MIT

use std::sync::Arc;

use serde_json::json;
use sts2_harness::management::{
    AuthContext, EnvironmentAuthenticator, ExecutionMode, LiveProviderSessionFactory,
    LiveRuntimeSessionFactory, LiveTargetCatalogPort, LiveWorkflowSessionFactory, ManagementError,
    ProductionLiveWorkflowSessionFactory, RunRequest, TargetAvailability, TargetCatalogResponse,
    TargetDescriptor,
};
use sts2_harness::workflow::WorkflowDefinition;

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
    sts2_harness::management::serve_live(listen, &store, authenticator, factory()?)
        .map_err(|error| error.to_string())
}

fn factory() -> Result<Arc<dyn LiveWorkflowSessionFactory>, String> {
    Ok(Arc::new(ProductionLiveWorkflowSessionFactory::new(
        json!({"schema_version":"ascension.capabilities/v1","capabilities":["workflow.live","workflow.node.observe.v1","workflow.node.decide.v1","workflow.node.execute_action.v1","workflow.node.terminal.v1","workflow.execution.fence.mcp-observation.v1"]}),
        Arc::new(Catalog),
        Arc::new(Runtime),
        Arc::new(Provider),
    ).map_err(|error| error.to_string())?))
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
