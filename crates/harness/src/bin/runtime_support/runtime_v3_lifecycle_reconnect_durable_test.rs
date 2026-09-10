// SPDX-License-Identifier: MIT

use sha2::Digest;
use sts2_harness::{
    ActionIdentity, ActionKind, Decision, DecisionInput, DecisionSource, EpisodeLegalAction,
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, ModelExecutionId,
};

use super::super::durable::{DurableHandle, OperationCatalogEvidence};
use super::reconnect_support::*;
use super::*;

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
    let catalog_raw = parsed.catalog_raw.clone();
    let observation = port.install(parsed)?;
    durable.refresh_resume_boundary()?;
    let divergent = synthetic_observation(
        observation.state_id(),
        observation.generation(),
        "combat",
        json!([{"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}]),
    )?;
    assert!(
        durable
            .verify_resume_boundary_with_catalog(&divergent, &catalog_raw)
            .is_err()
    );
    durable.verify_resume_boundary_with_catalog(&observation, &catalog_raw)?;
    let identity = ActionIdentity::new(
        SETTLED_OPERATION_ID,
        observation.state_id(),
        observation.generation(),
        action.action_id(),
    )?;
    let receipt = port.dispatch_action(&identity, &action)?;
    assert_eq!(receipt.status(), sts2_harness::DispatchStatus::Settled);
    assert_eq!(
        durable.operation_state(SETTLED_OPERATION_ID)?,
        sts2_harness::OperationState::Settled
    );
    durable.refresh_resume_boundary()?;
    durable.verify_resume_boundary_with_catalog(
        receipt
            .after()
            .ok_or("settled receipt omitted observation")?,
        b"[ ]",
    )?;

    let input = DecisionInput::new(
        ModelExecutionId::new(1).ok_or("execution identity")?,
        observation.clone(),
        port.current_actions.clone().ok_or("missing catalog")?,
        "synthetic lifecycle",
        Vec::new(),
    );
    let reservation = match durable.decision_admission_with_reuse(&input)? {
        super::super::decision_admission::DecisionAdmission::Fresh(reservation) => reservation,
        super::super::decision_admission::DecisionAdmission::Reused(_) => {
            return Err("new decision unexpectedly reused a result".into());
        }
    };
    let decision = Decision::Action {
        action_id: action.action_id().to_owned(),
        rationale: String::from("synthetic provider decision"),
        confidence: Some(90),
    };
    durable.complete_decision(&reservation, &decision)?;
    let mut source = CountingSource {
        calls: 0,
        decision: Decision::Wait {
            rationale: String::from("provider must not be called"),
        },
    };
    let mut recorder = super::super::recording::DecisionRecorder::with_durable(
        &mut source,
        TelemetryHandle::disabled(),
        durable.clone(),
    );
    assert_eq!(recorder.decide(&input)?, decision);
    drop(recorder);
    assert_eq!(source.calls, 0);
    assert!(matches!(
        durable.decision_admission_with_reuse(&input)?,
        super::super::decision_admission::DecisionAdmission::Reused(_)
    ));

    let pending_action = EpisodeLegalAction::new("combat.end-turn-pending", ActionKind::EndTurn)?;
    let recovery_context = recovery_original_context();
    let pending_catalog = json!([{
        "action_id": "combat.end-turn-pending",
        "action": {"kind": "end_turn"}
    }]);
    let pending_input = json!({
        "state_id": PENDING_STATE_ID,
        "generation": observation.generation(),
        "legal_actions": pending_catalog.clone()
    });
    let pending_catalog_raw = serde_json::to_vec(
        pending_input
            .get("legal_actions")
            .ok_or("pending catalog omitted from operation input")?,
    )?;
    durable.operation_intent_with_catalog(
        PENDING_OPERATION_ID,
        PENDING_STATE_ID,
        observation.generation(),
        &pending_action,
        &json!({"kind":"end_turn"}),
        OperationCatalogEvidence {
            input: &pending_input,
            raw: &pending_catalog_raw,
            original_context: Some(&recovery_context),
        },
    )?;
    let pending_digest = durable.operation_payload_digest(PENDING_OPERATION_ID)?;
    let canonical_action =
        wire::canonical_action_bytes(pending_action.action_id(), &json!({"kind":"end_turn"}))?;
    let canonical_json_b64 = encode_base64(&canonical_action);
    let catalog_digest = format!(
        "{:x}",
        sha2::Sha256::digest(serde_json::to_vec(&pending_catalog)?)
    );
    durable.operation_dispatched(PENDING_OPERATION_ID, &pending_digest)?;
    drop(port);

    let recovery_environment = RecoveryEnvironment::new();
    let mut resumed_config = config("127.0.0.1:15525".into());
    resumed_config.recovery_environment = recovery_environment.0;
    resumed_config.mcp_binary = recovery_settled_script(
        &fixture,
        PENDING_OPERATION_ID,
        PENDING_STATE_ID,
        observation.generation(),
        &pending_digest,
        &catalog_digest,
        &canonical_json_b64,
    )?;
    let mut resumed = RuntimeV3Port::new_with_store(
        resumed_config,
        TelemetryHandle::disabled(),
        durable.clone(),
    )?;
    enable_historical_recovery(&mut resumed)?;
    resumed.allocated = true;
    let mut mcp = McpProcess::spawn(&resumed.config)?;
    wire::initialize_mcp(&mut mcp)?;
    resumed.mcp = Some(mcp);
    resumed.reconcile_pending_operations()?;
    assert_eq!(
        durable.operation_state(PENDING_OPERATION_ID)?,
        sts2_harness::OperationState::Reconciled
    );
    let resumed_observation = resumed.observe()?;
    assert_eq!(resumed_observation.generation(), 1);

    let terminal = synthetic_observation("victory-2", 2, "victory", json!([]))?;
    let terminal_catalog_raw = serde_json::to_vec(&json!([]))?;
    durable.checkpoint_raw(&terminal, &terminal_catalog_raw)?;
    durable.complete_observation(&terminal)?;
    assert!(durable.decision_admission_with_reuse(&input).is_err());
    resumed.mcp.as_mut().ok_or("missing MCP")?.close()?;
    Ok(())
}
