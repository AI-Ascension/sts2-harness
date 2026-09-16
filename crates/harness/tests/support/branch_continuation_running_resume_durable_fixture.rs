// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn seed_execution_boundary(
    root: &Path,
    pending_unknown: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let fingerprint = ExecutionFingerprint::new(
        "seed:selected",
        "build:selected",
        "state:selected",
        "config:selected",
        "provider:selected",
    )?;
    let lineage = ExecutionLineage::new(RUN_ID, EPISODE_ID, ATTEMPT_ID, TRAJECTORY_ID)?;
    let catalog = json!([{
        "action_id":"combat.end-turn",
        "action":{"kind":"end_turn"}
    }]);
    let catalog_raw = serde_json::to_vec(&catalog)?;
    let observation = json!({
        "state_id":"state:selected",
        "generation":3,
        "visible_seed":"seed:selected",
        "player":{
            "hp":50,"max_hp":50,"energy":3,"gold":10,
            "hand":[],"deck":[],"discard":[],"exhaust":[]
        },
        "state":{"state":"combat","turn_index":4,"enemies":[]},
        "legal_actions":catalog
    });
    let observation_raw = serde_json::to_vec(&observation)?;
    let mut store =
        ExecutionStore::open(ExecutionStoreConfig::new(root.join("execution.sqlite3")))?;
    store.start_episode(&lineage, &fingerprint)?;
    let checkpoint = Checkpoint::new_with_catalog(
        lineage.clone(),
        0,
        "state:selected",
        3,
        fingerprint,
        observation_raw,
        CatalogEvidence::new(
            sts2_harness::sha256_hex(&catalog_raw),
            Some(catalog_raw.clone()),
        ),
    )?;
    store.save_checkpoint(&checkpoint)?;
    if pending_unknown {
        seed_unknown_operation(&mut store, lineage, catalog_raw)?;
    }
    store.close()?;
    Ok(())
}

fn seed_unknown_operation(
    store: &mut ExecutionStore,
    lineage: ExecutionLineage,
    catalog_raw: Vec<u8>,
) -> Result<(), Box<dyn std::error::Error>> {
    let action_payload =
        br#"{"action":{"kind":"end_turn"},"action_id":"combat.end-turn"}"#.to_vec();
    let intent = OperationIntent::new_with_action_and_catalog(
        lineage,
        "00000000-0000-4000-8000-000000000099",
        "state:selected",
        3,
        "combat.end-turn",
        "end_turn",
        action_payload.clone(),
        sts2_harness::sha256_hex(&action_payload),
        sts2_harness::sha256_hex(b"durable-input"),
        Some(sts2_harness::sha256_hex(&catalog_raw)),
        Some(catalog_raw),
    )?;
    store.record_operation_intent(&intent)?;
    store.mark_operation_dispatched(&intent.operation_id, &intent.payload_digest)?;
    store.record_operation_result(
        &intent.operation_id,
        &intent.payload_digest,
        OperationState::Unknown,
        Some("unknown-receipt"),
        Some("unknown-response-digest"),
    )?;
    Ok(())
}
