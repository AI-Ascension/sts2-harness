// SPDX-License-Identifier: MIT

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use sts2_harness::EpisodeRuntimePort;

use super::*;

fn config(address: String) -> RuntimeConfig {
    RuntimeConfig {
        gateway_address: address,
        gateway_token: "synthetic-token".into(),
        mcp_binary: "unused-test-binary".into(),
        runtime_profile: "runtime-v3-gameplay".into(),
        instance_id: "instance-1".into(),
        caller_id: "harness".into(),
        session_id: "session-1".into(),
        mcp_session_id: "mcp-session-1".into(),
        lease_id: "lease-1".into(),
        lease_epoch: 1,
        run_id: "run-1".into(),
        episode_id: "episode-1".into(),
        trajectory_id: "trajectory-1".into(),
        trace_id: "trace-1".into(),
        artifact_id: "artifact-1".into(),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
    }
}

fn accept(listener: &TcpListener) -> Result<TcpStream, String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("fake gateway did not receive expected cleanup".into());
                }
                std::thread::yield_now();
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn request(stream: &mut TcpStream) -> Result<String, Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
        if bytes.len() > 8192 {
            return Err("oversized synthetic request".into());
        }
    }
    let headers = String::from_utf8(bytes)?;
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then_some(value.trim())
        })
        .ok_or("missing length")?
        .parse::<usize>()?;
    if length > 16384 {
        return Err("oversized synthetic body".into());
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok(headers)
}

#[test]
fn runtime_v3_lost_allocation_response_releases_the_configured_lease()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let mut port = RuntimeV3Port::new_with_telemetry(
        config(listener.local_addr()?.to_string()),
        TelemetryHandle::disabled(),
    )?;
    std::thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
        let gateway = scope.spawn(move || -> Result<(), String> {
            let mut allocation = accept(&listener)?;
            let headers = request(&mut allocation).map_err(|error| error.to_string())?;
            assert!(headers.starts_with("POST /v1/sessions/allocate "));
            assert!(headers.contains("x-mcp-session-id: mcp-session-1\r\n"));
            // Allocation has committed; its response is lost before the client reads it.
            drop(allocation);
            let mut release = accept(&listener)?;
            let headers = request(&mut release).map_err(|error| error.to_string())?;
            assert!(headers.starts_with("POST /v1/instances/instance-1/release "));
            for expected in [
                "x-sts2-lease-id: lease-1",
                "x-sts2-lease-epoch: 1",
                "x-sts2-session-id: session-1",
            ] {
                assert!(headers.contains(expected));
            }
            let body = r#"{"status":"released"}"#;
            write!(
                release,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .map_err(|error| error.to_string())?;
            Ok(())
        });
        assert!(port.launch().is_err());
        assert!(port.allocated && port.released);
        gateway.join().map_err(|_| "fake gateway panicked")??;
        Ok(())
    })
}

fn wrong_lease_gateway(listener: TcpListener, status: &str) -> Result<(), String> {
    let mut allocation = accept(&listener)?;
    let headers = request(&mut allocation).map_err(|error| error.to_string())?;
    assert!(headers.contains("x-mcp-session-id: mcp-session-explicit\r\n"));
    let body = json!({
        "status":"allocated", "instance_id":"instance-1", "caller_id":"harness",
        "session_id":"session-1", "lease_id":"returned-lease", "lease_epoch":9
    })
    .to_string();
    write!(
        allocation,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    drop(allocation);
    let mut release = accept(&listener)?;
    let headers = request(&mut release).map_err(|error| error.to_string())?;
    assert!(headers.starts_with("POST /v1/instances/instance-1/release "));
    assert!(headers.contains("x-sts2-lease-id: returned-lease\r\n"));
    assert!(headers.contains("x-sts2-lease-epoch: 9\r\n"));
    let body = json!({"status":status}).to_string();
    write!(
        release,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn runtime_v3_wrong_lease_uses_returned_fence_and_requires_release_confirmation()
-> Result<(), Box<dyn std::error::Error>> {
    for status in ["released", "rejected"] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let mut config = config(listener.local_addr()?.to_string());
        config.mcp_session_id = "mcp-session-explicit".into();
        let mut port = RuntimeV3Port::new_with_telemetry(config, TelemetryHandle::disabled())?;
        std::thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
            let gateway = scope.spawn(move || wrong_lease_gateway(listener, status));
            let error = port
                .launch()
                .err()
                .ok_or("wrong lease must reject launch")?;
            assert_eq!(error.code(), "gateway_allocate_invalid");
            assert!(port.allocated);
            assert_eq!(port.released, status == "released");
            assert!(port.mcp.is_none());
            if status == "rejected" {
                assert!(
                    error
                        .to_string()
                        .contains("cleanup did not confirm release")
                );
            }
            gateway.join().map_err(|_| "fake gateway panicked")??;
            Ok(())
        })?;
    }
    Ok(())
}

#[cfg(unix)]
mod reconnect {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::super::durable::DurableHandle;
    use sts2_harness::{
        ActionIdentity, ActionKind, Decision, DecisionInput, EpisodeLegalAction,
        ExecutionFingerprint, ExecutionLineage, ExecutionStore, ModelExecutionId, RecoveryPort,
    };

    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Result<Self, std::io::Error> {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "sts2-v3-reconnect-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path)?;
            Ok(Self(path))
        }

        fn script(&self, content: &str) -> Result<String, Box<dyn std::error::Error>> {
            let path = self.0.join("mcp");
            fs::write(&path, format!("#!/bin/sh\n{content}"))?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
            Ok(path.to_str().ok_or("non-UTF8 fixture path")?.to_owned())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _cleanup = fs::remove_dir_all(&self.0);
        }
    }

    fn reply(value: Value) -> String {
        format!(
            "IFS= read -r line || exit 1\nprintf '%s\\n' \"$line\" >> requests\nprintf '%s\\n' '{}'\n",
            value.to_string().replace('\'', "'\\''")
        )
    }

    fn recovery_script(fixture: &Fixture) -> Result<(), Box<dyn std::error::Error>> {
        let mut settled: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
        ))?;
        settled["kind"] = json!("recover_response");
        settled["correlation_id"] = json!("2");
        let tools: Vec<_> = [
            "sts2.observe",
            "sts2.legal_actions",
            "sts2.dispatch_action",
            "sts2.wait_for_transition",
            "sts2.reobserve",
            "sts2.recover",
        ]
        .into_iter()
        .map(|name| json!({"name":name}))
        .collect();
        let script = format!(
            "cd '{}' || exit 1\n{}{}{}",
            fixture.0.display(),
            reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
            reply(
                json!({"jsonrpc":"2.0","id":2,"result":{"revision":"runtime-v3-gameplay-mcp","tools":tools}})
            ),
            reply(
                json!({"jsonrpc":"2.0","id":2,"result":{"content":[{"text":settled.to_string()}]}})
            )
        );
        fixture.script(&script)?;
        Ok(())
    }

    fn dispatch_script(fixture: &Fixture) -> Result<String, Box<dyn std::error::Error>> {
        let mut settled: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
        ))?;
        settled["correlation_id"] = json!("1");
        settled["operation_id"] = json!("op-settled");
        let tools: Vec<_> = [
            "sts2.observe",
            "sts2.legal_actions",
            "sts2.dispatch_action",
            "sts2.wait_for_transition",
            "sts2.reobserve",
            "sts2.recover",
        ]
        .into_iter()
        .map(|name| json!({"name":name}))
        .collect();
        let script = format!(
            "cd '{}' || exit 1\n{}{}{}",
            fixture.0.display(),
            reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
            reply(
                json!({"jsonrpc":"2.0","id":2,"result":{"revision":"runtime-v3-gameplay-mcp","tools":tools}})
            ),
            reply(json!({
                "jsonrpc":"2.0",
                "id":1,
                "result":{"content":[{"text":settled.to_string()}]}
            }))
        );
        fixture.script(&script)
    }

    fn recovery_settled_script(
        fixture: &Fixture,
        correlation_id: &str,
        operation_id: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut settled: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
        ))?;
        settled["kind"] = json!("recover_response");
        settled["correlation_id"] = json!(correlation_id);
        settled["operation_id"] = json!(operation_id);
        let mut observed: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
        ))?;
        observed["correlation_id"] = json!("2");
        observed["generation"] = json!(1);
        observed["observation"]["generation"] = json!(1);
        observed["observation"]["state"]["turn_index"] = json!(2);
        let tools: Vec<_> = [
            "sts2.observe",
            "sts2.legal_actions",
            "sts2.dispatch_action",
            "sts2.wait_for_transition",
            "sts2.reobserve",
            "sts2.recover",
        ]
        .into_iter()
        .map(|name| json!({"name":name}))
        .collect();
        let script = format!(
            "cd '{}' || exit 1\n{}{}{}{}",
            fixture.0.display(),
            reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
            reply(
                json!({"jsonrpc":"2.0","id":2,"result":{"revision":"runtime-v3-gameplay-mcp","tools":tools}})
            ),
            reply(json!({
                "jsonrpc":"2.0",
                "id":1,
                "result":{"content":[{"text":settled.to_string()}]}
            })),
            reply(json!({
                "jsonrpc":"2.0",
                "id":2,
                "result":{"content":[{"text":observed.to_string()}]}
            }))
        );
        fixture.script(&script)
    }

    fn synthetic_observation(
        state_id: &str,
        generation: u64,
        stage: &str,
        legal_actions: Value,
    ) -> Result<sts2_harness::EpisodeObservation, Box<dyn std::error::Error>> {
        let state = if stage == "victory" {
            json!({"state": stage})
        } else {
            json!({"state": stage, "turn_index": 1, "enemies": []})
        };
        let observation = json!({
            "state_id": state_id,
            "generation": generation,
            "visible_seed": "synthetic-seed",
            "player": {"hp":50,"max_hp":50,"energy":3,"gold":99,"hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state": state,
            "legal_actions": legal_actions
        });
        Ok(sts2_harness::EpisodeObservation::new(
            state_id,
            generation,
            match stage {
                "combat" => sts2_harness::EpisodeStage::Combat,
                "victory" => sts2_harness::EpisodeStage::Victory,
                _ => return Err("unsupported synthetic stage".into()),
            },
            stage == "combat",
            stage != "combat",
            stage == "combat",
            observation,
        )?)
    }

    #[test]
    fn durable_runtime_lifecycle_checkpoints_accounts_provider_and_reconciles_after_restart()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = Fixture::new()?;
        let mut runtime_config = config("127.0.0.1:15525".into());
        runtime_config.mcp_binary = dispatch_script(&fixture)?;
        let lineage = ExecutionLineage::new(
            runtime_config.run_id.clone(),
            runtime_config.episode_id.clone(),
            "attempt-synthetic",
            runtime_config.trajectory_id.clone(),
        )?;
        let fingerprint = ExecutionFingerprint::new(
            "synthetic-seed",
            "synthetic-build",
            "synthetic-state",
            "synthetic-config",
            "synthetic-provider",
        )?;
        let mut store = ExecutionStore::open_in_memory()?;
        store.start_episode(&lineage, &fingerprint)?;
        let durable = DurableHandle::from_store_for_test(store, lineage.clone(), fingerprint)?;
        let mut port = RuntimeV3Port::new_with_store(
            runtime_config,
            TelemetryHandle::disabled(),
            durable.clone(),
        )?;
        port.allocated = true;
        let mut mcp = McpProcess::spawn(&port.config)?;
        wire::initialize_mcp(&mut mcp)?;
        port.mcp = Some(mcp);
        let mut state: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
        ))?;
        state["legal_actions"] = json!([
            {"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}
        ]);
        let parsed = parse::observation(&state, "state_response", &port.config)?;
        let action = parsed.actions.actions()[0].clone();
        let observation = port.install(parsed)?;
        durable.refresh_resume_boundary()?;
        let divergent = synthetic_observation(
            observation.state_id(),
            observation.generation(),
            "combat",
            json!([{"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}]),
        )?;
        assert!(durable.verify_resume_boundary(&divergent).is_err());
        durable.verify_resume_boundary(&observation)?;
        let identity = ActionIdentity::new(
            "op-settled",
            observation.state_id(),
            observation.generation(),
            action.action_id(),
        )?;
        let receipt = port.dispatch_action(&identity, &action)?;
        assert_eq!(receipt.status(), sts2_harness::DispatchStatus::Settled);
        assert_eq!(
            durable.operation_state("op-settled")?,
            sts2_harness::OperationState::Settled
        );

        let input = DecisionInput::new(
            ModelExecutionId::new(1).ok_or("execution identity")?,
            observation.clone(),
            port.current_actions.clone().ok_or("missing catalog")?,
            "synthetic lifecycle",
            Vec::new(),
        );
        let reservation = durable
            .decision_admission(&input)?
            .ok_or("new decision must reserve provider usage")?;
        let decision = Decision::Action {
            action_id: action.action_id().to_owned(),
            rationale: String::from("synthetic provider decision"),
            confidence: Some(90),
        };
        durable.complete_decision(&reservation, &decision)?;
        assert!(durable.decision_admission(&input)?.is_none());

        let pending_action =
            EpisodeLegalAction::new("combat.end-turn-pending", ActionKind::EndTurn)?;
        durable.operation_intent(
            "op-pending",
            observation.state_id(),
            observation.generation(),
            &pending_action,
            &json!({"kind":"end_turn"}),
            &json!({"state_id":observation.state_id(),"generation":observation.generation()}),
        )?;
        let pending_digest = durable.operation_payload_digest("op-pending")?;
        durable.operation_dispatched("op-pending", &pending_digest)?;
        drop(port);

        let mut resumed_config = config("127.0.0.1:15525".into());
        resumed_config.mcp_binary = recovery_settled_script(&fixture, "1", "op-pending")?;
        let mut resumed = RuntimeV3Port::new_with_store(
            resumed_config,
            TelemetryHandle::disabled(),
            durable.clone(),
        )?;
        resumed.allocated = true;
        let mut mcp = McpProcess::spawn(&resumed.config)?;
        wire::initialize_mcp(&mut mcp)?;
        resumed.mcp = Some(mcp);
        resumed.reconcile_pending_operations()?;
        assert_eq!(
            durable.operation_state("op-pending")?,
            sts2_harness::OperationState::Reconciled
        );
        let resumed_observation = resumed.observe()?;
        assert_eq!(resumed_observation.generation(), 1);

        let terminal = synthetic_observation("victory-2", 2, "victory", json!([]))?;
        durable.checkpoint(&terminal, &json!({}))?;
        resumed.complete_durable_observation(&terminal)?;
        assert!(durable.decision_admission(&input).is_err());
        resumed.mcp.as_mut().ok_or("missing MCP")?.close()?;
        Ok(())
    }

    #[test]
    fn runtime_v3_reconnect_reconciles_same_operation_without_redispatch()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = Fixture::new()?;
        let mut config = config("127.0.0.1:15525".into());
        config.mcp_binary = fixture.script("IFS= read -r line\nexit 0\n")?;
        let mut port = RuntimeV3Port::new_with_telemetry(config, TelemetryHandle::disabled())?;
        port.allocated = true;
        port.mcp = Some(McpProcess::spawn(&port.config)?);
        let mut state: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
        ))?;
        state["legal_actions"] = json!([
            {"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}
        ]);
        let parsed = parse::observation(&state, "state_response", &port.config)?;
        let action = parsed.actions.actions()[0].clone();
        let observation = port.install(parsed)?;
        let identity = ActionIdentity::new(
            "op-1",
            observation.state_id(),
            observation.generation(),
            action.action_id(),
        )?;
        assert!(port.dispatch_action(&identity, &action).is_err());
        assert!(port.mcp.as_ref().is_some_and(McpProcess::is_closed));
        recovery_script(&fixture)?;
        let receipt = port.reconcile("op-1")?;
        assert_eq!(receipt.operation_id(), "op-1");
        assert_eq!(receipt.status(), sts2_harness::DispatchStatus::Settled);
        assert_eq!(port.operations.len(), 1);
        assert_eq!(port.reconnect_attempts, 1);
        let requests = fs::read_to_string(fixture.0.join("requests"))?;
        let requests: Vec<Value> = requests
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2]["params"]["name"], "sts2.recover");
        assert_eq!(requests[2]["params"]["arguments"]["operation_id"], "op-1");
        assert!(
            !requests
                .iter()
                .any(|value| value["params"]["name"] == "sts2.dispatch_action")
        );
        port.mcp.as_mut().ok_or("missing MCP")?.close()?;
        port.reconnect_attempts = 2;
        assert!(port.reconcile("op-1").is_err());
        Ok(())
    }
}
