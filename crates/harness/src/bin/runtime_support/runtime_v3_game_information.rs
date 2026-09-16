// SPDX-License-Identifier: MIT
use super::*;
use sts2_harness::game_information::{
    LookupError, LookupMcpContext, LookupMcpPort, call_lookup_mcp, call_capabilities_mcp,
};
use sts2_harness::game_information_binding::{
    LookupBindingContext, LookupBindingError, LookupBindingPort, LookupBindingRequest,
    LookupBindingSession, LookupScope,
};
use serde_json::json;

impl LookupMcpPort for RuntimeV3Port {
    fn information_correlation(&self) -> Result<String, LookupError> {
        if self.mcp.is_none() {
            return Err(LookupError::Transport);
        }
        Ok(self.next_rpc_id.to_string())
    }
    fn information_capabilities(&mut self) -> Result<(String, Vec<u8>), LookupError> {
        self.validate_lookup_owner_now()?;
        let correlation = self.information_correlation()?;
        let id = self.next_rpc_id;
        self.next_rpc_id = id.checked_add(1).ok_or(LookupError::Bounds)?;
        let context = LookupMcpContext {
            instance_id: self.config.instance_id.clone(),
            mcp_session_id: self.config.mcp_session_id.clone(),
            lease_id: self.config.lease_id.clone(),
            lease_epoch: self.config.lease_epoch,
        };
        let bytes = call_capabilities_mcp(&context, id, |id, args| {
            wire::rpc_call_catalog_read(
                self.mcp.as_mut().ok_or(LookupError::Transport)?,
                id,
                "tools/call",
                args,
            )
            .map_err(|_| LookupError::Transport)
        })?;
        self.validate_lookup_owner_now()?;
        Ok((correlation, bytes))
    }
    fn call_information(&mut self, tool: &str, request: &Value) -> Result<Vec<u8>, LookupError> {
        self.validate_lookup_owner_now()?;
        let context = LookupMcpContext {
            instance_id: self.config.instance_id.clone(),
            mcp_session_id: self.config.mcp_session_id.clone(),
            lease_id: self.config.lease_id.clone(),
            lease_epoch: self.config.lease_epoch,
        };
        let expected_correlation = self.next_rpc_id.to_string();
        if request["correlation_id"].as_str() != Some(expected_correlation.as_str()) {
            return Err(LookupError::Scope);
        }
        self.next_rpc_id = self.next_rpc_id.checked_add(1).ok_or(LookupError::Bounds)?;
        let response = call_lookup_mcp(&context, tool, request, |id, arguments| {
            wire::rpc_call_catalog_read(
                self.mcp.as_mut().ok_or(LookupError::Transport)?,
                id,
                "tools/call",
                arguments,
            )
            .map_err(|_| LookupError::Transport)
        })?;
        self.validate_lookup_owner_now()?;
        Ok(response)
    }
}

impl LookupBindingPort for RuntimeV3Port {
    fn lookup_binding(
        &mut self,
        request: &LookupBindingRequest,
    ) -> Result<Vec<u8>, LookupBindingError> {
        let operation = match request.operation {
            sts2_harness::game_information_binding::LookupBindingOperation::Discovery => {
                "discovery"
            }
            sts2_harness::game_information_binding::LookupBindingOperation::Observe => "observe",
        };
        self.gateway
            .request_bytes(
                "POST",
                &format!(
                    "/v1/instances/{}/game-information/lookup-binding",
                    self.config.instance_id
                ),
                &json!({
                    "operation": operation,
                    "project_id": request.scope.project_id,
                    "run_id": request.scope.run_id,
                    "episode_id": request.scope.episode_id,
                    "agent_id": request.scope.agent_id,
                    "authority_epoch": request.authority_epoch,
                }),
                super::identity_headers(&self.config, &request.correlation_id),
            )
            .map_err(|_| LookupBindingError::NativeUnavailable)
    }
}

impl RuntimeV3Port {
    pub(super) fn validate_lookup_owner_now(&self) -> Result<(), LookupError> {
        if !self.lookup_binding_required {
            return Ok(());
        }
        let owner = self
            .lookup_policy_owner
            .as_ref()
            .ok_or(LookupError::Scope)?;
        let expected = self
            .lookup_policy_binding
            .as_ref()
            .ok_or(LookupError::Scope)?;
        owner
            .lookup_snapshot(Some(expected))
            .map(|_| ())
            .map_err(|_| LookupError::Scope)
    }

    pub(super) fn discover_game_information_binding(&mut self) -> Result<(), String> {
        let (project_id, agent_id, authority_epoch) = self.config.lookup_scope()?;
        let context = LookupBindingContext {
            instance_id: self.config.instance_id.clone(),
            scope: LookupScope {
                project_id,
                run_id: self.config.run_id.clone(),
                episode_id: self.config.episode_id.clone(),
                agent_id,
            },
            authority_epoch,
            supported_capabilities: vec![String::from(
                sts2_harness::game_information_binding::LOOKUP_BINDING_PROFILE,
            )],
        };
        let mut binding = LookupBindingSession::new(context);
        binding
            .discover(self)
            .map_err(|error| format!("lookup-binding discovery failed: {error}"))?;
        binding
            .observe(self)
            .map_err(|error| format!("lookup-binding observation failed: {error}"))?;
        if let Some(owner) = self.lookup_policy_owner.as_ref() {
            let discovered = binding
                .binding()
                .cloned()
                .ok_or_else(|| String::from("lookup binding discovery was not retained"))?;
            let selected = owner
                .lookup_snapshot(None)
                .map_err(|_| String::from("selected memory policy is no longer current"))?;
            self.lookup_policy_binding = Some(selected.binding.clone());
            let game_binding = sts2_harness::game_information::LookupBinding {
                scope: owner.scope.clone(),
                game_profile: discovered.game_profile,
                content_manifest_id: discovered.content_manifest_id,
                locale: discovered.locale,
                authority_epoch,
                // No native entity reference is available at this boundary. Static lookup remains
                // available; live lookup stays refused until a host observation supplies one.
                snapshot: None,
            };
            let now = game_information_owner::RuntimeGameInformationOwner::policy_now_timestamp();
            let expires_at = game_information_owner::RuntimeGameInformationOwner::policy_timestamp_after(
                owner.archive_retention_seconds(),
            );
            if owner.replay_archive() {
                let (lookup_session, lookup_corpus) = owner
                    .restore_lookup_archive(&selected.binding, game_binding, &now, &expires_at)?
                    .ok_or_else(|| String::from("no matching persisted lookup archive exists"))?;
                self.lookup_replay_mode = true;
                self.lookup_corpus = Some(lookup_corpus);
                self.lookup_session = Some(lookup_session);
            } else {
                let mut lookup_session = sts2_harness::game_information::LookupSession::new(
                    game_binding,
                    selected.policy,
                    &selected.corpus,
                    &now,
                    &expires_at,
                )
                .map_err(|_| String::from("selected lookup policy cannot open a session"))?;
                lookup_session.negotiate_port(self).map_err(|_| {
                    String::from("game-information MCP capabilities are unavailable")
                })?;
                self.lookup_corpus = Some(selected.corpus);
                self.lookup_session = Some(lookup_session);
            }
        }
        self.lookup_binding = Some(binding);
        Ok(())
    }

    pub(super) fn initialize_game_information_binding(
        &mut self,
    ) -> Result<(), sts2_harness::PortError> {
        let enabled = self.config.lookup_binding_enabled().map_err(|error| {
            wire::port_error("game_information_binding_configuration", error, false)
        })?;
        self.lookup_binding_required = enabled;
        if !enabled {
            return Ok(());
        }
        if let Err(error) = self.discover_game_information_binding() {
            let release = self.release_lease_inner();
            return Err(wire::port_error(
                "game_information_binding_unavailable",
                wire::combine_cleanup(error, Ok(()), release),
                false,
            ));
        }
        Ok(())
    }

    pub(super) fn refresh_game_information_binding(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<(), sts2_harness::PortError> {
        if !self.lookup_binding_required {
            return Ok(());
        }
        let mut binding = self.lookup_binding.take().ok_or_else(|| {
            wire::port_error(
                "game_information_binding_unavailable",
                "enabled lookup binding was not retained after discovery",
                false,
            )
        })?;
        let observation = binding
            .observe(self)
            .map(|observation| observation.state_generation);
        self.lookup_binding = Some(binding);
        let observed_generation = observation.map_err(|error| {
            wire::port_error(
                "game_information_binding_unavailable",
                format!("lookup-binding re-observation failed: {error}"),
                false,
            )
        })?;
        if observed_generation != generation {
            return Err(wire::port_error(
                "catalog_reobserve",
                format!(
                    "lookup-binding generation {observed_generation} does not match episode state {state_id} generation {generation}"
                ),
                true,
            ));
        }
        if let (Some(owner), Some(expected)) = (
            self.lookup_policy_owner.as_ref(),
            self.lookup_policy_binding.as_ref(),
        ) {
            owner.lookup_snapshot(Some(expected)).map_err(|_| {
                wire::port_error(
                    "memory_policy_revalidation",
                    "selected memory policy or trusted owner fence changed before decision",
                    false,
                )
            })?;
        }
        Ok(())
    }
}

include!("runtime_v3_game_information_tests.rs");
