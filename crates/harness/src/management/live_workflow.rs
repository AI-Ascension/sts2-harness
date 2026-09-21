// SPDX-License-Identifier: MIT

//! Explicit live workflow composition. Synthetic execution remains a separate
//! adapter and is never used as a live fallback.

#[path = "execution.rs"]
mod execution;
#[path = "execution_commands.rs"]
mod execution_commands;
#[path = "execution_context.rs"]
mod execution_context;
#[path = "execution_records.rs"]
mod execution_records;
#[path = "execution_state.rs"]
mod execution_state;
#[path = "node.rs"]
mod node;
#[path = "node_decision.rs"]
mod node_decision;
#[path = "node_projection.rs"]
mod node_projection;
#[path = "node_recovery.rs"]
mod node_recovery;
#[path = "production.rs"]
mod production;
#[path = "session.rs"]
mod session;
#[path = "validation.rs"]
mod validation;

pub use execution::LiveWorkflowExecutionPort;
pub use execution_records::live_run_id;
pub use production::{
    BoundaryCaptureSink, LiveContextObservationPort, LiveContextRenderPort,
    LiveProviderSessionFactory, LiveRuntimeSessionFactory, LiveTargetCatalogPort,
    ProductionLiveWorkflowSessionFactory, RuntimeAuthorityBinding,
};
pub use session::{
    EpisodeRuntimeSession, LiveWorkflowFactory, LiveWorkflowOptions, LiveWorkflowSession,
    LiveWorkflowSessionFactory,
};
pub use validation::{LIVE_WORKFLOW_CAPABILITY, LIVE_WORKFLOW_PROFILE};

use std::sync::Arc;

use super::service::{ManagementError, ManagementService};
use super::store::WorkflowStore;

/// Compose an authenticated management service around an authoritative,
/// caller-supplied gateway/MCP and provider session factory.
pub fn live_store(
    store: Arc<dyn WorkflowStore>,
    factory: Arc<dyn LiveWorkflowSessionFactory>,
    options: LiveWorkflowOptions,
) -> Result<ManagementService, ManagementError> {
    live_store_with_provider_policy(
        store,
        factory,
        options,
        Arc::new(super::UnavailableLiveProviderPolicyPort),
    )
}

/// Compose a served-live management service with the durable provider-policy
/// owner that also gates provider construction in the session factory.
pub fn live_store_with_provider_policy(
    store: Arc<dyn WorkflowStore>,
    factory: Arc<dyn LiveWorkflowSessionFactory>,
    options: LiveWorkflowOptions,
    provider_policy: Arc<dyn super::LiveProviderPolicyPort>,
) -> Result<ManagementService, ManagementError> {
    live_store_with_provider_policy_and_command_port(
        store,
        factory,
        options,
        provider_policy,
        Arc::new(super::UnavailableProviderSessionPolicyCommandPort),
    )
}

/// Compose a served-live management service with both provider-policy ports.
/// The command port owns authenticated reads and explicit durable mutations;
/// the live provider port gates provider construction in each session.
pub fn live_store_with_provider_policy_and_command_port(
    store: Arc<dyn WorkflowStore>,
    factory: Arc<dyn LiveWorkflowSessionFactory>,
    options: LiveWorkflowOptions,
    provider_policy: Arc<dyn super::LiveProviderPolicyPort>,
    command_port: Arc<dyn super::ProviderSessionPolicyCommandPort>,
) -> Result<ManagementService, ManagementError> {
    let capabilities = factory.capabilities();
    let definitions = validation::LiveDefinitionPort::new(capabilities.clone())?;
    let execution = LiveWorkflowExecutionPort::new(Arc::clone(&factory), options)?;
    Ok(ManagementService::new(store)
        .with_definition_port(Arc::new(definitions))
        .with_execution_port(Arc::new(execution))
        .with_live_provider_policy_port(provider_policy)
        .with_provider_session_policy_command_port(command_port)
        .with_capability_port(Arc::new(validation::LiveCapabilityPort {
            capabilities,
            factory,
        })))
}
