// SPDX-License-Identifier: MIT

use std::fs;

use serde_json::{Value, json};
use sha2::Digest;
use sts2_harness::{
    Decision, EpisodeRunner, EpisodeRunnerConfig, EpisodeRunnerError, EpisodeStage,
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, OperationState, RecoveryController,
    StabilityBarrier,
};

use super::super::super::durable::DurableHandle;
use super::super::reconnect_support::*;
use super::*;

#[path = "runtime_v3_lifecycle_recovery_runner_script.rs"]
mod script;

const ACTION_ID: &str = "combat.end-turn-pending";

pub(super) fn run_in_process(mode: &str) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let authority = recovery_authority();
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;

    let action = json!({"kind": "end_turn"});
    let actions = json!([{"action_id": ACTION_ID, "action": action}]);
    let catalog_raw = serde_json::to_vec(&actions)?;
    let catalog_digest = format!("{:x}", sha2::Sha256::digest(&catalog_raw));
    let canonical = serde_json::to_vec(&json!({"action": action, "action_id": ACTION_ID}))?;
    let canonical_b64 = encode_base64(&canonical).trim_end_matches('=').to_owned();

    let mut config = super::config(listener.local_addr()?.to_string());
    config.gateway_token = format!("fixture-{mode}");
    config.instance_id = authority.instance_id.clone();
    config.lease_id = authority.lease_id.clone();
    config.lease_epoch = authority.lease_epoch;
    config.recovery_environment = RecoveryEnvironment::new().0;
    config.mcp_binary = script::runner_script(&fixture, mode, &catalog_digest, &canonical_b64)?;
    let lineage = ExecutionLineage::new("run-1", "episode-1", "attempt-inprocess", "trajectory-1")?;
    let fingerprint = ExecutionFingerprint::new("seed", "build", "state", "config", "provider")?;
    let mut store = ExecutionStore::open_in_memory()?;
    store.start_episode(&lineage, &fingerprint)?;
    let durable = DurableHandle::from_store_for_test(store, lineage, fingerprint)?;
    let mut port =
        RuntimeV3Port::new_with_store(config, TelemetryHandle::disabled(), durable.clone())?;
    let gateway_authority = authority.clone();
    let mut source = CountingSource {
        calls: 0,
        decision: Decision::Action {
            action_id: ACTION_ID.to_owned(),
            rationale: String::from("one real dispatch before recovery"),
            confidence: Some(100),
        },
    };
    let runner = EpisodeRunner::new(EpisodeRunnerConfig::new(
        2,
        StabilityBarrier::new(1, 1)?,
        RecoveryController::new(1)?,
        "in-process recovery boundary",
        Vec::new(),
    )?);

    std::thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
        let gateway = scope.spawn(move || super::runner_gateway(listener, &gateway_authority));
        if mode == "settled" {
            let mut completions = 0;
            let result = runner.run_with_completion(&mut port, &mut source, |port, _, report| {
                completions += 1;
                port.complete_durable(report).map_err(|error| {
                    sts2_harness::PortError::new("completion_failed", error, false)
                })
            });
            let report = result?;
            assert_eq!(report.terminal_stage(), EpisodeStage::Victory);
            assert_eq!(report.transitions(), 1);
            assert_eq!(report.recoveries(), 1);
            assert_eq!(completions, 1);
            assert!(durable.is_completed()?);
        } else {
            let result = runner.run(&mut port, &mut source);
            assert_eq!(result, Err(EpisodeRunnerError::UncertainMutation));
        }
        assert_eq!(
            source.calls, 1,
            "recovery must not request a replacement decision"
        );
        assert!(port.released, "runner did not release its allocated lease");
        gateway.join().map_err(|_| "fake gateway panicked")??;
        Ok(())
    })?;

    let requests: Vec<Value> = fs::read_to_string(fixture.0.join("requests"))?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let names: Vec<&str> = requests
        .iter()
        .filter_map(|request| request["params"]["name"].as_str())
        .collect();
    let expected = match mode {
        "settled" => vec![
            "sts2.observe",
            "sts2.legal_actions",
            "sts2.dispatch_action",
            "watchdog.operation_lookup",
            "watchdog.operation_reconcile",
            "sts2.wait_for_transition",
        ],
        "not-found" => vec![
            "sts2.observe",
            "sts2.legal_actions",
            "sts2.dispatch_action",
            "watchdog.operation_lookup",
        ],
        _ => vec![
            "sts2.observe",
            "sts2.legal_actions",
            "sts2.dispatch_action",
            "watchdog.operation_lookup",
            "watchdog.operation_reconcile",
        ],
    };
    assert_eq!(names, expected);
    assert_eq!(
        names
            .iter()
            .filter(|name| **name == "sts2.dispatch_action")
            .count(),
        1
    );
    assert_eq!(
        names
            .iter()
            .filter(|name| **name == "sts2.wait_for_transition")
            .count(),
        usize::from(mode == "settled")
    );
    if mode == "settled" {
        let wait = requests
            .iter()
            .find(|request| request["params"]["name"] == "sts2.wait_for_transition")
            .ok_or("missing historical witness wait")?;
        assert_eq!(
            wait["params"]["arguments"]["wait_for_millis"],
            json!(1),
            "historical settlement must use the retained-witness deadline"
        );
    }
    let dispatch = requests
        .iter()
        .find(|request| request["params"]["name"] == "sts2.dispatch_action")
        .ok_or("missing real dispatch request")?;
    let operation_id = dispatch["params"]["arguments"]["operation_id"]
        .as_str()
        .ok_or("dispatch omitted dynamic operation id")?;
    assert!(uuid::Uuid::parse_str(operation_id).is_ok());
    let state = durable.operation_state(operation_id)?;
    if mode == "settled" {
        assert_eq!(state, OperationState::Reconciled);
    } else {
        assert!(
            state.is_unresolved(),
            "invalid sideband state was closed: {state:?}"
        );
    }
    Ok(())
}
