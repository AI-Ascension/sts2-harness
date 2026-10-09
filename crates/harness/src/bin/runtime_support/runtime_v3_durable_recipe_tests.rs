// SPDX-License-Identifier: MIT

use super::*;
use std::cell::Cell;
use sts2_harness::{
    ActionKind, Decision, DecisionInput, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeObservation, EpisodeStage, ExecutionFingerprint, ExecutionLineage, ExecutionStore,
    ExecutionStoreConfig, ModelExecutionId, RecipeInvocationStatus,
};

use super::super::super::decision_admission::DecisionAdmission;

#[path = "runtime_v3_durable_recipe_owner_input_test_support.rs"]
mod owner_input_test_support;
use owner_input_test_support::mapped_input;

fn durable_fixture_with_store(
    store: ExecutionStore,
) -> Result<(DurableHandle, ExecutionLineage, ExecutionFingerprint), Box<dyn std::error::Error>> {
    durable_fixture_with_config(store, "e".repeat(64))
}

fn durable_fixture_with_config(
    mut store: ExecutionStore,
    config_digest: String,
) -> Result<(DurableHandle, ExecutionLineage, ExecutionFingerprint), Box<dyn std::error::Error>> {
    let lineage = ExecutionLineage::new(
        "run-runtime-map-1",
        "episode-runtime-map-1",
        "attempt-runtime-map-1",
        "trajectory-runtime-map-1",
    )?;
    let fingerprint = ExecutionFingerprint::new(
        "seed-runtime-map-1",
        "a".repeat(64),
        "b".repeat(64),
        config_digest.clone(),
        "d".repeat(64),
    )?;
    store.start_episode(&lineage, &fingerprint)?;
    let durable = DurableHandle::from_store_for_lifecycle_test(
        store,
        lineage.clone(),
        fingerprint.clone(),
        String::from("model-revision-1"),
        config_digest,
    )?;
    Ok((durable, lineage, fingerprint))
}

fn durable_fixture() -> Result<DurableHandle, Box<dyn std::error::Error>> {
    Ok(durable_fixture_with_store(ExecutionStore::open_in_memory()?)?.0)
}

fn map_response(map_instance_id: &str) -> Value {
    serde_json::json!({
        "protocol_version":"runtime-map-v1",
        "schema_digest":sts2_harness::RUNTIME_MAP_SCHEMA_DIGEST,
        "provenance":{"artifact":"sts2-protocol/runtime-map-v1",
            "source":"schemas/runtime-map-v1.schema.json","generator":"hand-authored"},
        "correlation_id":"3","instance_id":"instance-1",
        "session_id":"gateway-session-1","lease_id":"lease-1","lease_epoch":1,
        "generation":1,"kind":"snapshot_response","timeout":null,
        "snapshot":{
            "state_id":"state-1","generation":1,
            "schema_version":"visible-map-v1","projection_version":"runtime-map-v1",
            "game_build":"build","mod_version":"mod","map_instance_id":map_instance_id,
            "act_id":1,"scope_id":"scope-1","availability":"available",
            "completeness":"complete","freshness":"current","reason":null,
            "nodes":[
                {"id":"start","row":0,"column":0,"category":"start","visited":true},
                {"id":"next","row":1,"column":0,"category":"monster","visited":false}
            ],
            "edges":[{"from":"start","to":"next"}],
            "position":{"kind":"current","node_id":"start"},"history":["start"],
            "terminal_node_ids":["next"],
            "bindings":[{"graph_node_id":"next","host_action_id":"move-1",
                "action":{"kind":"select_map_node","node_id":"next"}}]
        }
    })
}

fn map_actions() -> Result<EpisodeLegalActionSet, Box<dyn std::error::Error>> {
    Ok(EpisodeLegalActionSet::new(
        "state-1",
        1,
        vec![EpisodeLegalAction::new(
            "move-1",
            ActionKind::SelectMapNode,
        )?],
    )?)
}

fn invocation_binding(durable: &DurableHandle) -> Result<RecipeInvocationBinding, String> {
    let execution_id = ModelExecutionId::new(1)
        .ok_or_else(|| String::from("model execution identity must be nonzero"))?;
    map_invocation_binding(durable, execution_id, "state-1", 1)
}

fn invocation_receipt(
    durable: &DurableHandle,
) -> Result<Option<sts2_harness::RecipeInvocationReceipt>, String> {
    durable.map_invocation_receipt(&invocation_binding(durable)?)
}

fn invocation_status(durable: &DurableHandle) -> Result<RecipeInvocationStatus, String> {
    invocation_receipt(durable)?
        .map(|receipt| receipt.status())
        .ok_or_else(|| String::from("runtime map receipt is missing"))
}

fn collect_with_intent_check(
    durable: &DurableHandle,
    actions: &EpisodeLegalActionSet,
    response: Value,
    reads: &Cell<u8>,
) -> Result<Value, String> {
    let binding = invocation_binding(durable)?;
    collect_map_snapshot(
        durable,
        "state-1",
        1,
        ModelExecutionId::new(1).ok_or_else(|| String::from("execution identity is zero"))?,
        actions,
        || {
            reads.set(reads.get() + 1);
            let receipt = durable
                .map_invocation_receipt(&binding)?
                .ok_or_else(|| String::from("map read started without durable intent"))?;
            if receipt.status() != RecipeInvocationStatus::IntentRecorded {
                return Err(String::from("map read started without durable intent"));
            }
            Ok(response)
        },
    )
}

fn no_context_input() -> Result<DecisionInput, Box<dyn std::error::Error>> {
    let observation = EpisodeObservation::new(
        "state-1",
        1,
        EpisodeStage::Map,
        true,
        false,
        true,
        serde_json::json!({
            "state_id":"state-1","generation":1,"visible_seed":null,
            "player":{"hp":10,"max_hp":10,"energy":3,"gold":0,
                "hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state":{"state":"map","node_id":"start","options":["next"]},
            "legal_actions":[{"action_id":"move-1",
                "action":{"kind":"select_map_node","node_id":"next"}}]
        }),
    )?;
    Ok(DecisionInput::new(
        ModelExecutionId::new(1).ok_or("execution identity must be nonzero")?,
        observation,
        map_actions()?,
        "map receipt test",
        Vec::new(),
    ))
}

fn no_map_combat_input() -> Result<DecisionInput, Box<dyn std::error::Error>> {
    let observation = EpisodeObservation::new(
        "combat-1",
        1,
        EpisodeStage::Combat,
        true,
        false,
        true,
        serde_json::json!({
            "state_id":"combat-1","generation":1,"visible_seed":"seed",
            "player":{"hp":10,"max_hp":10,"energy":3,"gold":0,
                "hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state":{"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions":[{"action_id":"end-turn",
                "action":{"kind":"end_turn"}}]
        }),
    )?;
    let actions = EpisodeLegalActionSet::new(
        "combat-1",
        1,
        vec![EpisodeLegalAction::new("end-turn", ActionKind::EndTurn)?],
    )?;
    Ok(DecisionInput::new(
        ModelExecutionId::new(1).ok_or("execution identity must be nonzero")?,
        observation,
        actions,
        "approved worker combat",
        Vec::new(),
    ))
}

#[test]
fn approved_worker_reference_without_map_context_admits_before_provider_io()
-> Result<(), Box<dyn std::error::Error>> {
    let durable = durable_fixture_with_config(
        ExecutionStore::open_in_memory()?,
        String::from("config:selected"),
    )?
    .0;
    let input = no_map_combat_input()?;
    assert!(input.accepted_map_snapshot_digest().is_none());
    let base_fingerprint = super::super::support::decision_input_digest(&input)?;
    assert_eq!(
        finalize_decision_context(&durable, &input, &base_fingerprint)?,
        base_fingerprint
    );
    let lineage = durable.lifecycle_lineage();
    let receipt_exists = durable
        .store
        .try_borrow_mut()?
        .recipe_invocation_exists_for_execution(&lineage, input.execution_id)?;
    assert!(!receipt_exists);

    // Stop at durable admission: no DecisionSource or provider transport is invoked.
    let token = match durable.decision_admission_with_reuse(&input)? {
        DecisionAdmission::Fresh(token) => token,
        DecisionAdmission::Reused(_) => return Err("new execution was unexpectedly reused".into()),
    };
    let stored = durable.store.try_borrow()?.decision("model-execution-1")?;
    assert_eq!(stored.reference.input_fingerprint, base_fingerprint);
    assert_eq!(stored.reference.config_digest, "config:selected");
    assert!(
        durable
            .store
            .try_borrow()?
            .provider_reservation("provider-reservation-model-execution-1")
            .is_ok()
    );
    drop(token);
    Ok(())
}

#[test]
fn durable_intent_precedes_the_one_typed_map_read() -> Result<(), Box<dyn std::error::Error>> {
    let durable = durable_fixture()?;
    let actions = map_actions()?;
    let reads = Cell::new(0_u8);
    let response = collect_with_intent_check(&durable, &actions, map_response("map-1"), &reads)?;

    assert_eq!(reads.get(), 1);
    assert_eq!(response["kind"], "snapshot_response");
    assert_eq!(
        invocation_status(&durable)?,
        RecipeInvocationStatus::ResponseValidated
    );
    Ok(())
}

#[test]
fn owner_context_absence_keeps_the_receipt_unfinalized() -> Result<(), Box<dyn std::error::Error>> {
    let durable = durable_fixture()?;
    let actions = map_actions()?;
    let input = no_context_input()?;
    assert_eq!(
        finalize_decision_context(&durable, &input, &"f".repeat(64))?,
        "f".repeat(64)
    );
    collect_with_intent_check(&durable, &actions, map_response("map-1"), &Cell::new(0))?;
    assert!(finalize_decision_context(&durable, &input, &"f".repeat(64)).is_err());
    assert_eq!(
        invocation_status(&durable)?,
        RecipeInvocationStatus::ResponseValidated
    );
    Ok(())
}

#[test]
fn owner_context_finalizes_before_reservation_and_completed_retry_reuses()
-> Result<(), Box<dyn std::error::Error>> {
    let durable = durable_fixture()?;
    let actions = map_actions()?;
    let reads = Cell::new(0_u8);
    let response = map_response("map-1");
    collect_with_intent_check(&durable, &actions, response.clone(), &reads)?;
    let input = mapped_input(response)?;
    let base_digest = super::super::support::decision_input_digest(&input)?;
    let owner_digest = input
        .accepted_map_snapshot_digest()
        .ok_or("runner input omitted its accepted map context")?;
    let expected_fingerprint = map_decision_input_digest(&base_digest, owner_digest)?;
    let token = match durable.decision_admission_with_reuse(&input)? {
        DecisionAdmission::Fresh(token) => token,
        DecisionAdmission::Reused(_) => return Err("first admission unexpectedly reused".into()),
    };
    let receipt = invocation_receipt(&durable)?.ok_or("receipt disappeared after admission")?;
    assert_eq!(receipt.status(), RecipeInvocationStatus::ContextValidated);
    assert_eq!(
        receipt.decision_input_digest(),
        Some(expected_fingerprint.as_str())
    );
    let stored = durable.store.try_borrow()?.decision("model-execution-1")?;
    assert_eq!(stored.reference.input_fingerprint, expected_fingerprint);
    let decision = Decision::Action {
        action_id: String::from("move-1"),
        rationale: String::from("synthetic map receipt test"),
        confidence: Some(90),
    };
    durable.complete_decision(&token, &decision)?;
    match durable.decision_admission_with_reuse(&input)? {
        DecisionAdmission::Reused(reused) => assert_eq!(reused, decision),
        DecisionAdmission::Fresh(_) => return Err("completed decision was not reused".into()),
    }
    assert_eq!(reads.get(), 1);
    Ok(())
}

#[test]
fn changed_valid_map_snapshot_fails_before_provider_reservation()
-> Result<(), Box<dyn std::error::Error>> {
    let durable = durable_fixture()?;
    let actions = map_actions()?;
    collect_with_intent_check(&durable, &actions, map_response("map-1"), &Cell::new(0))?;
    let changed_input = mapped_input(map_response("map-2"))?;
    assert!(
        durable
            .decision_admission_with_reuse(&changed_input)
            .is_err()
    );
    assert_eq!(
        invocation_status(&durable)?,
        RecipeInvocationStatus::ResponseValidated
    );
    assert!(
        durable
            .store
            .try_borrow()?
            .provider_reservation("provider-reservation-model-execution-1")
            .is_err()
    );
    Ok(())
}

#[test]
fn file_backed_response_receipt_cannot_be_replayed_after_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!(
        "sts2-runtime-map-restart-{}-{}.sqlite3",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let (durable, lineage, fingerprint) = durable_fixture_with_store(ExecutionStore::open(
            ExecutionStoreConfig::new(path.clone()),
        )?)?;
        let actions = map_actions()?;
        let reads = Cell::new(0_u8);
        let response = map_response("map-1");
        collect_with_intent_check(&durable, &actions, response.clone(), &reads)?;
        durable.close()?;
        drop(durable);

        let store = ExecutionStore::open(ExecutionStoreConfig::new(path.clone()))?;
        let reopened = DurableHandle::from_store_for_lifecycle_test(
            store,
            lineage,
            fingerprint,
            String::from("model-revision-1"),
            "e".repeat(64),
        )?;
        let input = mapped_input(response.clone())?;
        assert!(reopened.decision_admission_with_reuse(&input).is_err());
        assert_eq!(
            invocation_status(&reopened)?,
            RecipeInvocationStatus::ResponseValidated
        );
        assert!(
            reopened
                .store
                .try_borrow()?
                .provider_reservation("provider-reservation-model-execution-1")
                .is_err()
        );
        let second = collect_map_snapshot(
            &reopened,
            "state-1",
            1,
            ModelExecutionId::new(1).ok_or("execution identity must be nonzero")?,
            &actions,
            || {
                reads.set(reads.get() + 1);
                Ok(response)
            },
        );
        assert!(second.is_err());
        assert_eq!(reads.get(), 1);
        reopened.close()?;
        Ok::<(), Box<dyn std::error::Error>>(())
    })();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    result
}

#[path = "runtime_v3_durable_recipe_causality_tests.rs"]
mod causality_tests;
