// SPDX-License-Identifier: MIT

use std::cell::Cell;
use std::error::Error;
use std::fs;

use serde_json::{Value, json};
use sts2_harness::{
    ActionKind, EpisodeLegalAction, EpisodeLegalActionSet, ExecutionFingerprint, ExecutionLineage,
    ExecutionStore, ExecutionStoreConfig, ModelExecutionId, RecipeInvocationStatus,
};

use super::super::DurableHandle;
use super::{collect_map_snapshot_for_port, map_invocation_binding};

// Synthetic collector inputs exercise error mapping, not native owner provenance.
fn durable_fixture() -> Result<DurableHandle, Box<dyn Error>> {
    let lineage = ExecutionLineage::new(
        "run-map-error-test",
        "episode-map-error-test",
        "attempt-map-error-test",
        "trajectory-map-error-test",
    )?;
    let fingerprint = ExecutionFingerprint::new(
        "seed-map-error-test",
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
    )?;
    let mut store = ExecutionStore::open_in_memory()?;
    store.start_episode(&lineage, &fingerprint)?;
    Ok(DurableHandle::from_store_for_test(
        store,
        lineage,
        fingerprint,
    )?)
}

fn actions() -> Result<EpisodeLegalActionSet, Box<dyn Error>> {
    Ok(EpisodeLegalActionSet::new(
        "state-1",
        1,
        vec![EpisodeLegalAction::new(
            "move-1",
            ActionKind::SelectMapNode,
        )?],
    )?)
}

fn response() -> Value {
    json!({
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
            "game_build":"build","mod_version":"mod","map_instance_id":"map-1",
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

fn collect<F>(
    durable: &DurableHandle,
    actions: &EpisodeLegalActionSet,
    read: F,
) -> Result<Value, sts2_harness::PortError>
where
    F: FnOnce() -> Result<Value, String>,
{
    collect_map_snapshot_for_port(
        durable,
        "state-1",
        1,
        ModelExecutionId::new(1)
            .ok_or_else(|| sts2_harness::PortError::new("fixture", "zero execution id", false))?,
        actions,
        read,
    )
}

fn assert_intent_only(durable: &DurableHandle) -> Result<(), Box<dyn Error>> {
    let binding = map_invocation_binding(
        durable,
        ModelExecutionId::new(1).ok_or("execution identity must be nonzero")?,
        "state-1",
        1,
    )?;
    let receipt = durable
        .map_invocation_receipt(&binding)?
        .ok_or("map invocation receipt is missing")?;
    assert_eq!(receipt.status(), RecipeInvocationStatus::IntentRecorded);
    assert!(receipt.typed_result_digest().is_none());
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
fn owner_snapshot_refusal_is_invalid_and_keeps_only_the_intent() -> Result<(), Box<dyn Error>> {
    let durable = durable_fixture()?;
    let actions = actions()?;
    let reads = Cell::new(0_u8);
    let mut invalid = response();
    invalid["snapshot"]["state_id"] = json!("wrong-map-state");
    let error = collect(&durable, &actions, || {
        reads.set(reads.get() + 1);
        Ok(invalid)
    })
    .err()
    .ok_or("owner-invalid snapshot must fail closed")?;
    assert_eq!(error.code(), "map_snapshot_invalid");
    assert_eq!(reads.get(), 1);
    assert_intent_only(&durable)?;

    let retry = collect(&durable, &actions, || {
        reads.set(reads.get() + 1);
        Ok(response())
    })
    .err()
    .ok_or("pending intent must not repeat the map read")?;
    assert_eq!(retry.code(), "map_snapshot_failed");
    assert_eq!(reads.get(), 1);
    assert_intent_only(&durable)?;
    Ok(())
}

#[test]
fn transport_and_identity_failures_keep_the_generic_code() -> Result<(), Box<dyn Error>> {
    let durable = durable_fixture()?;
    let actions = actions()?;
    let reads = Cell::new(0_u8);
    let transport = collect(&durable, &actions, || {
        reads.set(reads.get() + 1);
        Err(String::from("synthetic map read failure"))
    })
    .err()
    .ok_or("read failure must refuse")?;
    assert_eq!(transport.code(), "map_snapshot_failed");
    assert_eq!(reads.get(), 1);
    assert_intent_only(&durable)?;

    for (field, value) in [
        ("provenance", json!({"artifact":"wrong"})),
        ("protocol_version", json!("other-profile")),
        ("schema_digest", json!("wrong-schema")),
        ("correlation_id", json!("4")),
        ("instance_id", json!("foreign-instance")),
        ("session_id", json!("foreign-gateway-session")),
        ("lease_id", json!("foreign-lease")),
        ("lease_epoch", json!(2)),
        ("generation", json!(2)),
    ] {
        let durable = durable_fixture()?;
        let mut foreign = response();
        foreign[field] = value;
        let identity = collect(&durable, &actions, || Ok(foreign))
            .err()
            .ok_or("mismatched response envelope or identity must refuse")?;
        assert_eq!(identity.code(), "map_snapshot_failed");
        assert_intent_only(&durable)?;
    }
    Ok(())
}

#[test]
fn readonly_store_failure_is_generic_and_does_not_read() -> Result<(), Box<dyn Error>> {
    let path = std::env::temp_dir().join(format!(
        "sts2-runtime-map-error-readonly-{}-{}.sqlite3",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let lineage = ExecutionLineage::new(
            "run-map-error-readonly",
            "episode-map-error-readonly",
            "attempt-map-error-readonly",
            "trajectory-map-error-readonly",
        )?;
        let fingerprint = ExecutionFingerprint::new(
            "seed-map-error-readonly",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
        )?;
        let mut store = ExecutionStore::open(ExecutionStoreConfig::new(path.clone()))?;
        store.start_episode(&lineage, &fingerprint)?;
        store.close()?;
        drop(store);
        let store = ExecutionStore::open_read_only(&path)?;
        let durable = DurableHandle::from_store_for_test(store, lineage, fingerprint)?;
        let actions = actions()?;
        let reads = Cell::new(0_u8);
        let error = collect(&durable, &actions, || {
            reads.set(reads.get() + 1);
            Ok(response())
        })
        .err()
        .ok_or("read-only store must reject durable intent")?;
        assert_eq!(error.code(), "map_snapshot_failed");
        assert_eq!(reads.get(), 0);
        let binding = map_invocation_binding(
            &durable,
            ModelExecutionId::new(1).ok_or("execution identity must be nonzero")?,
            "state-1",
            1,
        )?;
        assert!(durable.map_invocation_receipt(&binding)?.is_none());
        assert!(
            durable
                .store
                .try_borrow()?
                .provider_reservation("provider-reservation-model-execution-1")
                .is_err()
        );
        durable.close()?;
        Ok::<(), Box<dyn Error>>(())
    })();
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(format!("{}-wal", path.display()));
    let _ = fs::remove_file(format!("{}-shm", path.display()));
    result
}
