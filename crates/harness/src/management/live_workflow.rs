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
#[path = "node.rs"]
mod node;
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
pub use production::{
    LiveProviderSessionFactory, LiveRuntimeSessionFactory, LiveTargetCatalogPort,
    ProductionLiveWorkflowSessionFactory,
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
    let capabilities = factory.capabilities();
    let definitions = validation::LiveDefinitionPort::new(capabilities.clone())?;
    let execution = LiveWorkflowExecutionPort::new(Arc::clone(&factory), options)?;
    Ok(ManagementService::new(store)
        .with_definition_port(Arc::new(definitions))
        .with_execution_port(Arc::new(execution))
        .with_live_provider_policy_port(provider_policy)
        .with_capability_port(Arc::new(validation::LiveCapabilityPort {
            capabilities,
            factory,
        })))
}
