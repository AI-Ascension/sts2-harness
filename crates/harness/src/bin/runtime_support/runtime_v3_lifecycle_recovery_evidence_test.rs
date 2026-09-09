// SPDX-License-Identifier: MIT

use std::io::Write;
use std::net::TcpListener;

use sts2_harness::{
    ActionKind, Decision, EpisodeLegalAction, EpisodeRunner, EpisodeRunnerConfig,
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, OperationState, RecoveryController,
    StabilityBarrier,
};

use super::super::durable::DurableHandle;
use super::reconnect_support::*;
use super::*;

fn authority_value(authority: &super::allocation_context::RecoveryAuthority) -> Value {
    json!({
        "contract": "watchdog-runtime-allocation-v1",
        "schema_digest": super::allocation_context::ALLOCATION_SCHEMA_DIGEST,
        "context": {
            "deployment_id": authority.deployment_id,
            "instance_id": authority.instance_id,
            "instance_incarnation": authority.instance_incarnation,
            "boot_id": authority.boot_id,
            "authority_generation": authority.authority_generation,
            "lease_id": authority.lease_id,
            "lease_epoch": authority.lease_epoch
        },
        "current_fence": authority.current_fence
    })
}

fn runner_gateway(
    listener: TcpListener,
    authority: &super::allocation_context::RecoveryAuthority,
) -> Result<(), String> {
    let mut allocation = super::accept(&listener)?;
    let headers = super::request(&mut allocation).map_err(|error| error.to_string())?;
    if !headers.starts_with("POST /v1/sessions/allocate ") {
        return Err(String::from("runner did not allocate through the gateway"));
    }
    let body = json!({
        "status": "allocated",
        "instance_id": authority.instance_id,
        "caller_id": "harness",
        "session_id": "session-1",
        "lease_id": authority.lease_id,
        "lease_epoch": authority.lease_epoch,
        "recovery_authority": authority_value(authority)
    })
    .to_string();
    write!(
        allocation,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    drop(allocation);

    let mut release = super::accept(&listener)?;
    let headers = super::request(&mut release).map_err(|error| error.to_string())?;
    if !headers.starts_with("POST /v1/instances/") {
        return Err(String::from("runner did not release through the gateway"));
    }
    let body = r#"{"status":"released"}"#;
    write!(
        release,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn replacement_authority() -> super::allocation_context::RecoveryAuthority {
    let mut authority = recovery_authority();
    authority.instance_incarnation = String::from("dddddddd-dddd-4ddd-8ddd-dddddddddddd");
    authority.boot_id = String::from("eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee");
    authority.current_fence["instance_incarnation"] = json!(authority.instance_incarnation.clone());
    authority.current_fence["boot_id"] = json!(authority.boot_id.clone());
    authority
}

struct RecoveryCase {
    fixture: Fixture,
    durable: DurableHandle,
    lookup: Value,
    reconcile: Value,
}

impl RecoveryCase {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let fixture = Fixture::new()?;
        let lineage = ExecutionLineage::new("run-1", "episode-1", "attempt-1", "trajectory-1")?;
        let fingerprint =
            ExecutionFingerprint::new("seed", "build", "state", "config", "provider")?;
        let mut store = ExecutionStore::open_in_memory()?;
        store.start_episode(&lineage, &fingerprint)?;
        let durable = DurableHandle::from_store_for_test(store, lineage, fingerprint)?;
        let action = EpisodeLegalAction::new("combat.end-turn-pending", ActionKind::EndTurn)?;
        durable.operation_intent(
            PENDING_OPERATION_ID,
            PENDING_STATE_ID,
            0,
            &action,
            &json!({"kind":"end_turn"}),
            &json!({"state_id":PENDING_STATE_ID,"generation":0,"legal_actions":[
                {"action_id":action.action_id(),"action":{"kind":"end_turn"}}
            ]}),
        )?;
        let digest = durable.operation_payload_digest(PENDING_OPERATION_ID)?;
        durable.operation_dispatched(PENDING_OPERATION_ID, &digest)?;
        let operation = durable
            .pending_operations()?
            .pop()
            .ok_or("missing pending operation")?;
        let encoded = encode_base64(
            operation
                .intent
                .action_payload
                .as_deref()
                .ok_or("missing action bytes")?,
        );
        assert!(
            encoded.ends_with('='),
            "fixture must exercise unpadded gateway bytes"
        );
        let (lookup, reconcile) = settled_frames(
            PENDING_OPERATION_ID,
            PENDING_STATE_ID,
            0,
            &digest,
            operation
                .intent
                .catalog_digest
                .as_deref()
                .ok_or("missing catalog")?,
            encoded.trim_end_matches('='),
        );
        Ok(Self {
            fixture,
            durable,
            lookup,
            reconcile,
        })
    }

    fn run(&self) -> Result<(), String> {
        let mut runtime_config = config("127.0.0.1:15525".into());
        runtime_config.recovery_environment = RecoveryEnvironment::new().0;
        runtime_config.mcp_binary = response_script(&self.fixture, &self.lookup, &self.reconcile)
            .map_err(|error| error.to_string())?;
        let mut port = RuntimeV3Port::new_with_store(
            runtime_config,
            TelemetryHandle::disabled(),
            self.durable.clone(),
        )?;
        // A terminal sideband result is followed by a normal gameplay witness read.  Model the
        // live allocation boundary so the recovery reconnect guard can admit that read.
        port.allocated = true;
        port.recovery_authority = Some(recovery_authority());
        port.reconcile_pending_operations()
    }

    fn run_through_episode_runner(
        &self,
        authority: super::allocation_context::RecoveryAuthority,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.run_through_episode_runner_with_wait(authority, false)
    }

    fn run_through_episode_runner_with_wait(
        &self,
        authority: super::allocation_context::RecoveryAuthority,
        unresolved_wait: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let mut runtime_config = config(listener.local_addr()?.to_string());
        runtime_config.instance_id = authority.instance_id.clone();
        runtime_config.lease_id = authority.lease_id.clone();
        runtime_config.lease_epoch = authority.lease_epoch;
        runtime_config.recovery_environment = RecoveryEnvironment::new().0;
        if unresolved_wait {
            runtime_config.gateway_token = String::from("fixture-unresolved-witness");
        }
        runtime_config.mcp_binary = response_script(&self.fixture, &self.lookup, &self.reconcile)?;
        let mut port = RuntimeV3Port::new_with_store(
            runtime_config,
            TelemetryHandle::disabled(),
            self.durable.clone(),
        )?;
        let gateway_authority = authority.clone();
        std::thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
            let gateway = scope.spawn(move || runner_gateway(listener, &gateway_authority));
            let runner = EpisodeRunner::new(EpisodeRunnerConfig::new(
                1,
                StabilityBarrier::new(1, 1)?,
                RecoveryController::new(1)?,
                "recovery boundary test",
                Vec::new(),
            )?);
            let mut source = CountingSource {
                calls: 0,
                decision: Decision::Wait {
                    rationale: String::from("launch must fail before policy"),
                },
            };
            let result = runner.run(&mut port, &mut source);
            assert!(result.is_err(), "invalid recovery evidence was accepted");
            assert_eq!(source.calls, 0, "policy ran across an unresolved boundary");
            assert!(port.released, "failed launch did not release its lease");
            gateway.join().map_err(|_| "fake gateway panicked")??;
            Ok(())
        })?;
        Ok(())
    }

    fn assert_requests(
        &self,
        expected_reconcile: bool,
        expected_gameplay_witness: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let requests = std::fs::read_to_string(self.fixture.0.join("requests"))?;
        let calls: Vec<Value> = requests
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        let names: Vec<&str> = calls
            .iter()
            .filter_map(|value| value["params"]["name"].as_str())
            .collect();
        let expected = if expected_gameplay_witness {
            vec![
                "watchdog.operation_lookup",
                "watchdog.operation_reconcile",
                "sts2.wait_for_transition",
            ]
        } else if expected_reconcile {
            vec!["watchdog.operation_lookup", "watchdog.operation_reconcile"]
        } else {
            vec!["watchdog.operation_lookup"]
        };
        assert_eq!(
            names, expected,
            "recovery may fetch gameplay only after terminal sideband evidence and must never redispatch"
        );
        if expected_gameplay_witness {
            let wait = calls
                .iter()
                .find(|call| call["params"]["name"] == "sts2.wait_for_transition")
                .ok_or("missing retained-witness wait")?;
            assert_eq!(
                wait["params"]["arguments"]["wait_for_millis"],
                json!(1),
                "retained-witness recovery read must not inherit the ordinary 120-second wait"
            );
        }
        let mut original = None;
        for call in calls.iter().filter(|value| {
            value["params"]["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("watchdog."))
        }) {
            let reference = &call["params"]["arguments"]["payload"]["operation"];
            assert_eq!(reference["operation_id"], PENDING_OPERATION_ID);
            assert_eq!(
                reference["payload_digest"],
                self.durable
                    .operation_payload_digest(PENDING_OPERATION_ID)?
            );
            if let Some(first) = original {
                assert_eq!(reference, first, "reconcile must keep the lookup identity");
            } else {
                original = Some(reference);
            }
        }
        Ok(())
    }
}

fn set_state(frame: &mut Value, state: &str) {
    frame["payload"]["result"]["status"] = json!(state);
    frame["payload"]["operation"]["state"] = json!(state);
}

#[path = "runtime_v3_recovery_diagnostics_test.rs"]
mod diagnostics;

#[path = "runtime_v3_lifecycle_recovery_negative_test.rs"]
mod negative;
#[path = "runtime_v3_lifecycle_recovery_runner_test.rs"]
mod runner;
#[path = "runtime_v3_lifecycle_recovery_terminal_test.rs"]
mod terminal;
