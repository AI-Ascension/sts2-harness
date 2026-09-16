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
        let correlation = self.information_correlation()?;
        let id = self.next_rpc_id;
        self.next_rpc_id = id.checked_add(1).ok_or(LookupError::Bounds)?;
        let context = LookupMcpContext {
            instance_id: self.config.instance_id.clone(), mcp_session_id: self.config.mcp_session_id.clone(),
            lease_id: self.config.lease_id.clone(), lease_epoch: self.config.lease_epoch,
        };
        let bytes = call_capabilities_mcp(&context,id,|id,args| {
            wire::rpc_call_catalog_read(self.mcp.as_mut().ok_or(LookupError::Transport)?,
                id,"tools/call",args).map_err(|_|LookupError::Transport)
        })?;
        Ok((correlation,bytes))
    }
    fn call_information(&mut self, tool: &str, request: &Value) -> Result<Vec<u8>, LookupError> {
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
        call_lookup_mcp(&context, tool, request, |id, arguments| {
            wire::rpc_call_catalog_read(
                self.mcp.as_mut().ok_or(LookupError::Transport)?,
                id,
                "tools/call",
                arguments,
            )
            .map_err(|_| LookupError::Transport)
        })
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
        Ok(())
    }

    pub(super) fn initialize_game_information_binding(
        &mut self,
    ) -> Result<(), sts2_harness::PortError> {
        if !self.config.lookup_binding_enabled().map_err(|error| {
            wire::port_error("game_information_binding_configuration", error, false)
        })? {
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
}

#[cfg(test)]
mod lookup_binding_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    fn config(address: String) -> super::super::RuntimeConfig {
        super::super::RuntimeConfig {
            seed_transport: None,
            gateway_address: address,
            gateway_token: "synthetic-token".into(),
            mcp_binary: "unused-test-binary".into(),
            runtime_profile: "runtime-v3-gameplay".into(),
            instance_id: "instance-1".into(),
            caller_id: "harness".into(),
            session_id: "session-1".into(),
            lease_id: "lease-1".into(),
            lease_epoch: 1,
            mcp_session_id: "mcp-session-1".into(),
            run_id: "run-42".into(),
            episode_id: "episode-7".into(),
            trajectory_id: "trajectory-1".into(),
            trace_id: "trace-1".into(),
            artifact_id: "artifact-1".into(),
            wait_for_combat_seconds: 0,
            settlement_timeout_seconds: 30,
            map_context_enabled: false,
            recovery_environment: Vec::new(),
        }
    }

    #[test]
    fn episode_port_calls_the_fixed_gateway_lookup_binding_route() -> Result<(), Box<dyn std::error::Error>>
    {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        std::thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
            let gateway = scope.spawn(move || -> Result<(), String> {
                let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
                let mut bytes = Vec::new();
                let mut byte = [0_u8; 1];
                while !bytes.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).map_err(|error| error.to_string())?;
                    bytes.push(byte[0]);
                }
                let headers = String::from_utf8(bytes).map_err(|error| error.to_string())?;
                assert!(headers.starts_with(
                    "POST /v1/instances/instance-1/game-information/lookup-binding "
                ));
                assert!(headers.contains("x-mcp-session-id: mcp-session-1\r\n"));
                assert!(headers.contains("x-sts2-instance-id: instance-1\r\n"));
                assert!(headers.contains("x-sts2-session-id: session-1\r\n"));
                assert!(headers.contains("x-sts2-lease-id: lease-1\r\n"));
                assert!(headers.contains("x-sts2-lease-epoch: 1\r\n"));
                assert!(headers.contains(
                    "x-sts2-correlation-id: game-information-binding-discovery\r\n"
                ));
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .ok_or_else(|| "missing content length".to_owned())?
                    .parse::<usize>()
                    .map_err(|error| error.to_string())?;
                let mut body = vec![0; length];
                stream.read_exact(&mut body).map_err(|error| error.to_string())?;
                let body: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
                assert_eq!(
                    body,
                    json!({
                        "operation":"discovery",
                        "project_id":"proj-1",
                        "run_id":"run-42",
                        "episode_id":"episode-7",
                        "agent_id":"agent-3",
                        "authority_epoch":7
                    })
                );
                let response = String::from(
                    r#"{"correlation_id":"game-information-binding-discovery","kind":"lookup_binding_discovery_response","kind":"error_response"}"#,
                );
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{response}",
                    response.len()
                )
                .map_err(|error| error.to_string())
            });
            let mut port = RuntimeV3Port::new_with_telemetry(
                config(address),
                super::super::TelemetryHandle::disabled(),
            )?;
            let mut binding = LookupBindingSession::new(LookupBindingContext {
                instance_id: String::from("instance-1"),
                scope: LookupScope {
                    project_id: String::from("proj-1"),
                    run_id: String::from("run-42"),
                    episode_id: String::from("episode-7"),
                    agent_id: String::from("agent-3"),
                },
                authority_epoch: 7,
                supported_capabilities: vec![String::from(
                    sts2_harness::game_information_binding::LOOKUP_BINDING_PROFILE,
                )],
            });
            assert_eq!(
                binding.discover(&mut port),
                Err(LookupBindingError::Invalid)
            );
            gateway.join().map_err(|_| "gateway thread panicked")??;
            Ok(())
        })
    }
}
