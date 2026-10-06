// SPDX-License-Identifier: MIT

use super::*;

/// Starts a served-live management endpoint from a binary that owns concrete
/// runtime and provider adapters. The ordinary CLI retains synthetic mode.
pub fn serve_live(
    listen: std::net::SocketAddr,
    store_path: &str,
    authenticator: std::sync::Arc<dyn Authenticator>,
    factory: std::sync::Arc<dyn LiveWorkflowSessionFactory>,
) -> Result<(), ManagementError> {
    let store = SqliteWorkflowStore::open(store_path)
        .map_err(|error| ManagementError::store("workflow_store_open", error.to_string()))?;
    let store: std::sync::Arc<dyn WorkflowStore> = std::sync::Arc::new(store);
    let service = std::sync::Arc::new(live_store(store, factory, LiveWorkflowOptions::default())?);
    serve_live_service(listen, authenticator, service)
}

/// Starts a served-live management endpoint with the durable provider-policy
/// owner attached to management and the production session factory.
pub fn serve_live_with_provider_policy(
    listen: std::net::SocketAddr,
    store_path: &str,
    authenticator: std::sync::Arc<dyn Authenticator>,
    factory: std::sync::Arc<dyn LiveWorkflowSessionFactory>,
    provider_policy: std::sync::Arc<dyn LiveProviderPolicyPort>,
) -> Result<(), ManagementError> {
    let store = SqliteWorkflowStore::open(store_path)
        .map_err(|error| ManagementError::store("workflow_store_open", error.to_string()))?;
    let store: std::sync::Arc<dyn WorkflowStore> = std::sync::Arc::new(store);
    let service = std::sync::Arc::new(live_store_with_provider_policy(
        store,
        factory,
        LiveWorkflowOptions::default(),
        provider_policy,
    )?);
    serve_live_service(listen, authenticator, service)
}

pub fn serve_live_with_provider_policy_and_context_owner(
    listen: std::net::SocketAddr,
    store_path: &str,
    authenticator: std::sync::Arc<dyn Authenticator>,
    factory: std::sync::Arc<dyn LiveWorkflowSessionFactory>,
    provider_policy: std::sync::Arc<dyn LiveProviderPolicyPort>,
    context_owner: std::sync::Arc<dyn ContextOwnerPort>,
) -> Result<(), ManagementError> {
    let store = SqliteWorkflowStore::open(store_path)
        .map_err(|error| ManagementError::store("workflow_store_open", error.to_string()))?;
    let store: std::sync::Arc<dyn WorkflowStore> = std::sync::Arc::new(store);
    let service = std::sync::Arc::new(
        live_store_with_provider_policy(
            store,
            factory,
            LiveWorkflowOptions::default(),
            provider_policy,
        )?
        .with_context_owner_port(context_owner)
        .with_context_binding_history()?,
    );
    serve_live_service(listen, authenticator, service)
}

/// The owner ports a served management composition attaches.
///
/// Bundling them keeps the served-live constructors within the argument bound
/// as the required owner set grows, and keeps the three ports of one owner
/// reviewable as a single value rather than as positional arguments whose
/// order a caller must remember.
pub struct ServedOwnerPorts {
    pub provider_policy: std::sync::Arc<dyn LiveProviderPolicyPort>,
    pub command_port: std::sync::Arc<dyn ProviderSessionPolicyCommandPort>,
    pub context_owner: std::sync::Arc<dyn ContextOwnerPort>,
}

/// Optional long-lived management-only services attached by the binary that
/// owns their protected configuration.
pub struct ServedOwnerServices {
    ports: ServedOwnerPorts,
    seed_derivation_keys: Option<std::sync::Arc<dyn SeedDerivationKeyAuthority>>,
}

impl ServedOwnerServices {
    pub fn new(ports: ServedOwnerPorts) -> Self {
        Self {
            ports,
            seed_derivation_keys: None,
        }
    }

    pub fn with_seed_derivation_keys(
        mut self,
        authority: std::sync::Arc<dyn SeedDerivationKeyAuthority>,
    ) -> Self {
        self.seed_derivation_keys = Some(authority);
        self
    }
}

/// Starts served-live management with shared durable provider-policy owner
/// ports, explicit saved-policy commands, and an attached context owner.
pub fn serve_live_with_provider_policy_commands_and_context_owner(
    listen: std::net::SocketAddr,
    store_path: &str,
    authenticator: std::sync::Arc<dyn Authenticator>,
    factory: std::sync::Arc<dyn LiveWorkflowSessionFactory>,
    owner: ServedOwnerPorts,
    provider_session_capabilities: crate::provider_session::NativeCapabilities,
) -> Result<(), ManagementError> {
    serve_live_with_lifecycle(
        listen,
        store_path,
        authenticator,
        factory,
        owner,
        provider_session_capabilities,
        None,
    )
}

/// Starts served-live management with the optional gateway process-lifecycle
/// owner attached.
///
/// The lifecycle port and its durable intent directory are supplied together by
/// the binary that owns the gateway credential. Passing neither leaves the
/// surface composed but unavailable, which refuses every lifecycle command
/// instead of inventing an effect.
pub fn serve_live_with_lifecycle(
    listen: std::net::SocketAddr,
    store_path: &str,
    authenticator: std::sync::Arc<dyn Authenticator>,
    factory: std::sync::Arc<dyn LiveWorkflowSessionFactory>,
    owner: ServedOwnerPorts,
    provider_session_capabilities: crate::provider_session::NativeCapabilities,
    lifecycle: Option<ProcessLifecycleOwner>,
) -> Result<(), ManagementError> {
    serve_live_with_lifecycle_and_owner_services(
        listen,
        store_path,
        authenticator,
        factory,
        ServedOwnerServices::new(owner),
        provider_session_capabilities,
        lifecycle,
    )
}

/// Starts served-live management with explicitly composed owner-local
/// services, including the optional immutable seed-key authority.
pub fn serve_live_with_lifecycle_and_owner_services(
    listen: std::net::SocketAddr,
    store_path: &str,
    authenticator: std::sync::Arc<dyn Authenticator>,
    factory: std::sync::Arc<dyn LiveWorkflowSessionFactory>,
    owner: ServedOwnerServices,
    provider_session_capabilities: crate::provider_session::NativeCapabilities,
    lifecycle: Option<ProcessLifecycleOwner>,
) -> Result<(), ManagementError> {
    let store = std::sync::Arc::new(
        SqliteWorkflowStore::open(store_path)
            .map_err(|error| ManagementError::store("workflow_store_open", error.to_string()))?,
    );
    let journal: std::sync::Arc<dyn InferenceProfileRevisionJournal> = std::sync::Arc::new(
        SqliteInferenceProfileRevisionJournal::new(std::sync::Arc::clone(&store)),
    );
    let store: std::sync::Arc<dyn WorkflowStore> = store;
    let seed_derivation_keys = owner.seed_derivation_keys;
    let service = live_store_with_provider_policy_and_command_port(
        store,
        factory,
        LiveWorkflowOptions::default(),
        owner.ports.provider_policy,
        owner.ports.command_port,
    )?
    .with_inference_profile_revision_journal(journal)
    .with_context_owner_port(owner.ports.context_owner)
    .with_context_binding_history()?
    .with_provider_session_capabilities(provider_session_capabilities)?;
    let service = match seed_derivation_keys {
        Some(authority) => service.with_seed_derivation_key_authority(authority),
        None => service,
    };
    let service = match lifecycle {
        Some((port, intents)) => service.with_process_lifecycle(port, intents),
        None => service,
    };
    let service = std::sync::Arc::new(service);
    serve_live_service(listen, authenticator, service)
}

fn serve_live_service(
    listen: std::net::SocketAddr,
    authenticator: std::sync::Arc<dyn Authenticator>,
    service: std::sync::Arc<ManagementService>,
) -> Result<(), ManagementError> {
    let config = ServerConfig::new(listen, authenticator)
        .map_err(|error| ManagementError::invalid("workflow_server_config", error.to_string()))?;
    let server = ManagementServer::start(config, service).map_err(|error| {
        ManagementError::unavailable("workflow_server_start", error.to_string())
    })?;
    server
        .wait()
        .map_err(|error| ManagementError::unavailable("workflow_server_wait", error.to_string()))
}
