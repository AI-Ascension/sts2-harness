// SPDX-License-Identifier: MIT

use super::*;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sts2_harness::management::{ManagementServer, ManagementService, ServerConfig, ServerHandle};

pub(in crate::runtime_support::runtime_v3) struct MemoryPolicyPreflight {
    pub(in crate::runtime_support::runtime_v3) owner: Arc<RuntimeGameInformationOwner>,
    server: Option<ServerHandle>,
}

impl Drop for MemoryPolicyPreflight {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            let _ = server.shutdown();
        }
    }
}

pub(in crate::runtime_support::runtime_v3) fn begin_memory_policy_preflight(
    runtime_config: &super::super::RuntimeConfig,
) -> Result<Option<MemoryPolicyPreflight>, String> {
    if !runtime_config.lookup_binding_enabled()? {
        return Ok(None);
    }
    let (project_id, agent_id) = runtime_config.lookup_scope_identity()?;
    let expected_scope = MemoryScope::new(
        project_id,
        runtime_config.run_id.clone(),
        runtime_config.episode_id.clone(),
        agent_id,
    );
    let owner = RuntimeGameInformationOwner::from_environment(expected_scope)?;
    let server = start_management_server(&owner)?;
    if let Err(error) = wait_for_owner_ready(&owner, owner.preflight_timeout_seconds()) {
        let _ = server.shutdown();
        return Err(error);
    }
    Ok(Some(MemoryPolicyPreflight {
        owner,
        server: Some(server),
    }))
}

pub(in crate::runtime_support::runtime_v3) fn start_management_server(
    owner: &Arc<RuntimeGameInformationOwner>,
) -> Result<ServerHandle, String> {
    let service =
        ManagementService::in_memory().with_memory_policy_owner_management_port(owner.clone());
    let server_config = ServerConfig::new(owner.management_listen(), owner.authenticator.clone())
        .map_err(|_| {
        String::from("lookup owner management listener configuration is invalid")
    })?;
    let server = ManagementServer::start(server_config, Arc::new(service))
        .map_err(|_| String::from("lookup owner management listener could not start"))?;
    let endpoint = format!("http://{}/v1/memory-policy-owner", server.address());
    eprintln!(
        "runtime-v3 lookup-policy preflight waiting for explicit owner revalidation and adoption at {endpoint}"
    );
    Ok(server)
}

pub(in crate::runtime_support::runtime_v3) fn wait_for_owner_ready(
    owner: &RuntimeGameInformationOwner,
    timeout_seconds: u64,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    loop {
        match owner.lookup_snapshot(None) {
            Ok(_) => return Ok(()),
            Err(
                PolicyOwnerError::Missing
                | PolicyOwnerError::OwnerFenced
                | PolicyOwnerError::StaleReview,
            ) => {}
            Err(_) => {
                return Err(String::from(
                    "lookup-policy owner could not validate its current durable selection",
                ));
            }
        }
        if Instant::now() >= deadline {
            return Err(String::from(
                "lookup-policy owner preflight timed out before explicit adoption; no game or provider effects were started",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
