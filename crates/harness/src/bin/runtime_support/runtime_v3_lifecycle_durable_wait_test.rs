// SPDX-License-Identifier: MIT

use sts2_harness::{
    ActionIdentity, BarrierPort, Decision, DecisionInput, DecisionSource, DispatchStatus,
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreConfig, ModelExecutionId,
    OperationState, WaitOutcome,
};

use super::super::durable::DurableHandle;
use super::reconnect_support::{CountingSource, Fixture, SETTLED_OPERATION_ID, reply};
use super::*;

fn response(id: u64, value: Value) -> String {
    reply(json!({"jsonrpc":"2.0", "id":id,
        "result":{"content":[{"type":"text", "text":value.to_string()}]}}))
}

fn wait_script(fixture: &Fixture, variant: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut waited: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
    ))?;
    waited["kind"] = json!("wait_response");
    waited["correlation_id"] = json!("2");
    waited["operation_id"] = json!(SETTLED_OPERATION_ID);
    waited["wait_outcome"] = json!("same_state_mutation");
    waited["legal_actions"] = json!([
        {"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}
    ]);
    let mut unknown = waited.clone();
    unknown["kind"] = json!("dispatch_action_response");
    unknown["correlation_id"] = json!("1");
    unknown["generation"] = json!(0);
    unknown["status"] = json!("unknown");
    unknown["error_code"] = json!("settlement_unproven");
    for field in [
        "state_id",
        "observation",
        "legal_actions",
        "transition",
        "wait_outcome",
    ] {
        unknown[field] = Value::Null;
    }
    match variant {
        "wrong_operation" => waited["operation_id"] = json!("different-operation"),
        "wrong_generation" => waited["transition"]["from_generation"] = json!(99),
        "missing_witness" => waited["transition"] = Value::Null,
        "timeout" => {
            waited = unknown.clone();
            waited["kind"] = json!("wait_response");
            waited["correlation_id"] = json!("2");
            waited["wait_outcome"] = json!("timeout");
        }
        "settled" => {}
        _ => return Err("unknown synthetic wait variant".into()),
    }
    let mut repeat = waited.clone();
    repeat["correlation_id"] = json!("3");
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
    fixture.script(&format!(
        "cd '{}' || exit 1\n{}{}{}{}{}",
        fixture.0.display(),
        reply(json!({"jsonrpc":"2.0", "id":1, "result":{}})),
        reply(json!({"jsonrpc":"2.0", "id":2,
            "result":{"revision":"runtime-v3-gameplay-mcp", "tools":tools}})),
        response(1, unknown),
        response(2, waited),
        response(3, repeat),
    ))
}

fn fixture_port(
    fixture: &Fixture,
    variant: &str,
) -> Result<(RuntimeV3Port, DurableHandle), Box<dyn std::error::Error>> {
    let mut config = config("127.0.0.1:15525".into());
    config.mcp_binary = wait_script(fixture, variant)?;
    let lineage = ExecutionLineage::new(
        config.run_id.clone(),
        config.episode_id.clone(),
        "attempt-synthetic",
        config.trajectory_id.clone(),
    )?;
    let fingerprint = ExecutionFingerprint::new(
        "synthetic-seed",
        "synthetic-build",
        "synthetic-state",
        "synthetic-config",
        "synthetic-provider",
    )?;
    let mut store = ExecutionStore::open(
        ExecutionStoreConfig::new(fixture.0.join("execution.sqlite3"))
            .with_approved_recovery_schema(),
    )?;
    store.start_episode(&lineage, &fingerprint)?;
    let durable = DurableHandle::from_store_for_test(store, lineage, fingerprint)?;
    let mut port =
        RuntimeV3Port::new_with_store(config, TelemetryHandle::disabled(), durable.clone())?;
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
    let identity = ActionIdentity::new(
        SETTLED_OPERATION_ID,
        observation.state_id(),
        observation.generation(),
        action.action_id(),
    )?;
    assert_eq!(
        port.dispatch_action(&identity, &action)?.status(),
        DispatchStatus::Unknown
    );
    Ok((port, durable))
}

#[test]
fn durable_wait_settlement_allows_the_next_provider_decision()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let (mut port, durable) = fixture_port(&fixture, "settled")?;
    let sample = port.wait_for_transition(SETTLED_OPERATION_ID, 100)?;
    assert_eq!(sample.outcome(), WaitOutcome::SameStateMutation);
    assert_eq!(port.wait_for_transition(SETTLED_OPERATION_ID, 100)?, sample);
    let input = DecisionInput::new(
        ModelExecutionId::new(2).ok_or("execution identity")?,
        sample
            .observation()
            .ok_or("missing settled observation")?
            .clone(),
        port.current_actions
            .clone()
            .ok_or("missing successor catalog")?,
        "synthetic next decision after authoritative wait",
        Vec::new(),
    );
    let mut source = CountingSource {
        calls: 0,
        decision: Decision::Wait {
            rationale: "synthetic decision".into(),
        },
    };
    let mut recorder = super::super::recording::DecisionRecorder::with_durable(
        &mut source,
        TelemetryHandle::disabled(),
        durable.clone(),
    );
    let next = recorder.decide(&input);
    drop(recorder);
    assert!(
        next.is_ok(),
        "authoritative wait must unblock decision admission: {next:?}"
    );
    assert_eq!(source.calls, 1);
    assert!(matches!(
        durable.operation_state(SETTLED_OPERATION_ID)?,
        OperationState::Settled | OperationState::Reconciled
    ));
    drop(port);
    drop(durable);
    let reopened = ExecutionStore::open(
        ExecutionStoreConfig::new(fixture.0.join("execution.sqlite3"))
            .with_approved_recovery_schema(),
    )?;
    assert!(reopened.pending_operations("episode-1")?.is_empty());
    let requests = std::fs::read_to_string(fixture.0.join("requests"))?;
    assert_eq!(requests.matches("sts2.dispatch_action").count(), 1);
    assert_eq!(requests.matches("sts2.wait_for_transition").count(), 2);
    Ok(())
}

#[test]
fn invalid_or_unresolved_wait_does_not_clear_durable_uncertainty()
-> Result<(), Box<dyn std::error::Error>> {
    for variant in [
        "wrong_operation",
        "wrong_generation",
        "missing_witness",
        "timeout",
    ] {
        let fixture = Fixture::new()?;
        let (mut port, durable) = fixture_port(&fixture, variant)?;
        let result = port.wait_for_transition(SETTLED_OPERATION_ID, 1);
        if variant == "timeout" {
            // A bounded wait may time out or see the synthetic child exhaust its replies;
            // neither outcome supplies authority to resolve the original mutation.
            if let Ok(sample) = result {
                assert_eq!(sample.outcome(), WaitOutcome::Timeout);
            }
        } else {
            assert!(result.is_err(), "{variant} must reject its invalid witness");
        }
        assert_eq!(
            durable.operation_state(SETTLED_OPERATION_ID)?,
            OperationState::Unknown
        );
    }
    Ok(())
}
