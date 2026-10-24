// SPDX-License-Identifier: MIT

use rusqlite::Connection;

use super::*;
use crate::execution::types::{ExecutionFingerprint, ExecutionLineage, ExecutionStoreConfig};
use crate::identity::ModelExecutionId;
use crate::recipe::contract_v2::{RecipeDefinitionV2, RecipeOperationV2, admit_recipe_v2};

fn seeded_store() -> Result<(ExecutionStore, ExecutionLineage), Box<dyn std::error::Error>> {
    let mut store = ExecutionStore::open_in_memory()?;
    let lineage = ExecutionLineage::new(
        "run-receipt-1",
        "episode-receipt-1",
        "attempt-receipt-1",
        "trajectory-receipt-1",
    )?;
    let fingerprint = ExecutionFingerprint::new(
        "seed-receipt-1",
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
    )?;
    store.start_episode(&lineage, &fingerprint)?;
    Ok((store, lineage))
}

fn make_binding(
    lineage: ExecutionLineage,
    execution: u64,
    state_id: &str,
) -> Result<RecipeInvocationBinding, Box<dyn std::error::Error>> {
    let recipe = admit_recipe_v2(RecipeDefinitionV2::new(
        "runtime.map-context",
        1,
        RecipeOperationV2::MapSnapshot {},
    ))?;
    let execution_id = ModelExecutionId::new(execution)
        .ok_or_else(|| std::io::Error::other("test execution id must be nonzero"))?;
    Ok(RecipeInvocationBinding::new(
        lineage,
        execution_id,
        &recipe,
        RecipeInvocationContext {
            runtime_config_digest: "e".repeat(64),
            instance_id: String::from("instance-1"),
            gateway_session_id: String::from("gateway-session-1"),
            mcp_session_id: String::from("mcp-session-1"),
            lease_id: String::from("lease-1"),
            lease_epoch: 1,
            state_id: state_id.to_owned(),
            generation: 1,
        },
    )?)
}

fn seed_existing_operation(
    connection: &Connection,
    lineage: &ExecutionLineage,
) -> Result<(), rusqlite::Error> {
    connection.execute(
        "INSERT INTO operations(
            operation_id, run_id, episode_id, attempt_id, trajectory_id, state_id,
            generation, action_id, payload_digest, input_digest, state, result_ref,
            result_digest, created_at, updated_at
         ) VALUES ('operation-preserved', ?1, ?2, ?3, ?4, 'state-1', 1, 'action-1',
                   ?5, ?6, 'prepared', NULL, NULL, 1, 1)",
        rusqlite::params![
            lineage.run_id,
            lineage.episode_id,
            lineage.attempt_id,
            lineage.trajectory_id,
            "a".repeat(64),
            "b".repeat(64),
        ],
    )?;
    Ok(())
}

fn seed_existing_decision_and_checkpoint(
    connection: &Connection,
    lineage: &ExecutionLineage,
) -> Result<(), rusqlite::Error> {
    connection.execute(
        "INSERT INTO decisions(
            execution_id, run_id, episode_id, attempt_id, trajectory_id, input_fingerprint,
            model_revision, config_digest, state, result_ref, result_digest,
            provider_reservation_id, created_at, updated_at
         ) VALUES ('model-execution-88', ?1, ?2, ?3, ?4, ?5, 'model-rev', ?6,
                   'pending', NULL, NULL, NULL, 1, 1)",
        rusqlite::params![
            lineage.run_id,
            lineage.episode_id,
            lineage.attempt_id,
            lineage.trajectory_id,
            "c".repeat(64),
            "d".repeat(64),
        ],
    )?;
    connection.execute(
        "INSERT INTO checkpoints(
            episode_id, attempt_id, sequence, state_id, generation, seed, build_digest,
            state_digest, config_digest, provider_digest, observation, legal_actions_digest,
            created_at
         ) VALUES (?1, ?2, 1, 'state-1', 1, 'seed', ?3, ?4, ?5, ?6, X'7B7D', ?7, 1)",
        rusqlite::params![
            lineage.episode_id,
            lineage.attempt_id,
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
            "e".repeat(64),
        ],
    )?;
    Ok(())
}

fn seed_existing_rows(
    connection: &Connection,
    lineage: &ExecutionLineage,
) -> Result<(), rusqlite::Error> {
    seed_existing_operation(connection, lineage)?;
    seed_existing_decision_and_checkpoint(connection, lineage)
}

fn supersede_lineage(
    store: &mut ExecutionStore,
    lineage: &ExecutionLineage,
) -> Result<(), rusqlite::Error> {
    store.connection.execute(
        "UPDATE episodes SET current_attempt_id = 'attempt-receipt-next',
         current_trajectory_id = 'trajectory-receipt-next' WHERE episode_id = ?1",
        [&lineage.episode_id],
    )?;
    Ok(())
}

#[test]
fn transitions_are_monotonic_idempotent_and_binding_fenced()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut store, lineage) = seeded_store()?;
    let binding = make_binding(lineage.clone(), 1, "state-1")?;
    let changed_binding = make_binding(lineage, 1, "state-2")?;

    assert!(store.record_recipe_invocation_intent(&binding)?);
    assert!(!store.record_recipe_invocation_intent(&binding)?);
    assert_eq!(
        store.record_recipe_invocation_response(&binding, &"f".repeat(64)),
        Ok(())
    );
    assert_eq!(
        store.record_recipe_invocation_response(&binding, &"0".repeat(64)),
        Err(ExecutionStoreError::Conflict)
    );
    assert_eq!(
        store.record_recipe_invocation_intent(&changed_binding),
        Err(ExecutionStoreError::Conflict)
    );
    assert_eq!(
        store.finalize_recipe_invocation(&binding, &"1".repeat(64), &"2".repeat(64)),
        Ok(())
    );
    assert_eq!(
        store.finalize_recipe_invocation(&binding, &"1".repeat(64), &"2".repeat(64)),
        Ok(())
    );
    assert_eq!(
        store.finalize_recipe_invocation(&binding, &"3".repeat(64), &"2".repeat(64)),
        Err(ExecutionStoreError::Conflict)
    );
    let state = store.connection.query_row(
        "SELECT state FROM recipe_map_invocations WHERE model_execution_id = ?1",
        [binding.model_execution_id()],
        |row| row.get::<_, String>(0),
    )?;
    assert_eq!(state, "context_validated");
    let receipt = store
        .recipe_invocation_receipt(&binding)?
        .ok_or("finalized receipt was not readable")?;
    let typed_digest = "f".repeat(64);
    let owner_digest = "1".repeat(64);
    let decision_digest = "2".repeat(64);
    assert_eq!(receipt.status(), RecipeInvocationStatus::ContextValidated);
    assert_eq!(receipt.typed_result_digest(), Some(typed_digest.as_str()));
    assert_eq!(receipt.owner_snapshot_digest(), Some(owner_digest.as_str()));
    assert_eq!(
        receipt.decision_input_digest(),
        Some(decision_digest.as_str())
    );
    Ok(())
}

#[test]
fn superseded_lineage_cannot_advance_response_or_context_receipts()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut response_store, response_lineage) = seeded_store()?;
    let response_binding = make_binding(response_lineage.clone(), 1, "state-1")?;
    response_store.record_recipe_invocation_intent(&response_binding)?;
    supersede_lineage(&mut response_store, &response_lineage)?;
    assert_eq!(
        response_store.record_recipe_invocation_response(&response_binding, &"f".repeat(64)),
        Err(ExecutionStoreError::Conflict)
    );
    assert_eq!(
        response_store.recipe_invocation_receipt(&response_binding),
        Err(ExecutionStoreError::Conflict)
    );

    let (mut context_store, context_lineage) = seeded_store()?;
    let context_binding = make_binding(context_lineage.clone(), 1, "state-1")?;
    context_store.record_recipe_invocation_intent(&context_binding)?;
    context_store.record_recipe_invocation_response(&context_binding, &"f".repeat(64))?;
    supersede_lineage(&mut context_store, &context_lineage)?;
    assert_eq!(
        context_store.finalize_recipe_invocation(
            &context_binding,
            &"1".repeat(64),
            &"2".repeat(64),
        ),
        Err(ExecutionStoreError::Conflict)
    );
    Ok(())
}

#[test]
fn receipt_capacity_fails_closed_without_eviction() -> Result<(), Box<dyn std::error::Error>> {
    let (mut store, lineage) = seeded_store()?;
    insert_capacity_rows(&mut store, &lineage)?;
    let next = make_binding(lineage, 5000, "state-1")?;
    assert_eq!(
        store.record_recipe_invocation_intent(&next),
        Err(ExecutionStoreError::Capacity)
    );
    let count =
        store
            .connection
            .query_row("SELECT COUNT(*) FROM recipe_map_invocations", [], |row| {
                row.get::<_, i64>(0)
            })?;
    assert_eq!(count, 4096);
    Ok(())
}

fn insert_capacity_rows(
    store: &mut ExecutionStore,
    lineage: &ExecutionLineage,
) -> Result<(), rusqlite::Error> {
    let sql = "
        WITH RECURSIVE seq(value) AS (
            SELECT 1 UNION ALL SELECT value + 1 FROM seq WHERE value < 4096
        )
        INSERT INTO recipe_map_invocations(
            run_id, episode_id, attempt_id, trajectory_id, model_execution_id,
            recipe_id, recipe_revision, operation, runtime_config_digest, instance_id,
            gateway_session_id, mcp_session_id, lease_id, lease_epoch, state_id, generation,
            runtime_map_profile, schema_digest, rpc_correlation_id, request_digest,
            typed_result_digest, owner_snapshot_digest, decision_input_digest, state,
            created_at, updated_at
        )
        SELECT ?1, ?2, ?3, ?4, 'model-execution-' || value,
            'runtime.map-context', 1, 'map_snapshot', ?5, 'instance-1',
            'gateway-session-1', 'mcp-session-1', 'lease-1', 1, 'state-1', 1,
            'runtime-map-v1', ?6, '3', printf('%064d', value),
            NULL, NULL, NULL, 'intent_recorded', 0, 0
        FROM seq";
    store.connection.execute(
        sql,
        rusqlite::params![
            lineage.run_id,
            lineage.episode_id,
            lineage.attempt_id,
            lineage.trajectory_id,
            "e".repeat(64),
            crate::RUNTIME_MAP_SCHEMA_DIGEST,
        ],
    )?;
    Ok(())
}

fn make_v7_file(path: &std::path::Path) -> Result<ExecutionLineage, Box<dyn std::error::Error>> {
    let mut store = ExecutionStore::open(ExecutionStoreConfig::new(path))?;
    let lineage = ExecutionLineage::new(
        "run-migrate-1",
        "episode-migrate-1",
        "attempt-migrate-1",
        "trajectory-migrate-1",
    )?;
    let fingerprint = ExecutionFingerprint::new(
        "seed-migrate-1",
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
    )?;
    store.start_episode(&lineage, &fingerprint)?;
    seed_existing_rows(&store.connection, &lineage)?;
    store.close()?;
    drop(store);
    let connection = Connection::open(path)?;
    connection.execute_batch(
        "DROP TABLE recipe_map_invocations;
         DROP TABLE worker_handoffs;
         DROP TABLE worker_control_boots;
         DROP TABLE worker_control;
         PRAGMA user_version = 7;",
    )?;
    Ok(lineage)
}

fn assert_v7_open_migrates(
    path: &std::path::Path,
    lineage: &ExecutionLineage,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut migrated = ExecutionStore::open(ExecutionStoreConfig::new(path))?;
    let version = migrated
        .connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))?;
    assert_eq!(version, 9);
    for name in [
        "worker_control",
        "worker_handoffs",
        "recipe_map_invocations",
    ] {
        let exists = migrated.connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get::<_, i64>(0),
        )?;
        assert_eq!(exists, 1, "missing migrated table {name}");
    }
    assert_preserved_migration_rows(&migrated.connection, lineage)?;
    migrated.close()?;
    Ok(())
}

fn assert_preserved_migration_rows(
    connection: &Connection,
    lineage: &ExecutionLineage,
) -> Result<(), Box<dyn std::error::Error>> {
    for (query, identity) in [
        (
            "SELECT COUNT(*) FROM episodes WHERE episode_id = ?1",
            lineage.episode_id.as_str(),
        ),
        (
            "SELECT COUNT(*) FROM operations WHERE operation_id = ?1",
            "operation-preserved",
        ),
        (
            "SELECT COUNT(*) FROM decisions WHERE execution_id = ?1",
            "model-execution-88",
        ),
    ] {
        let count = connection.query_row(query, [identity], |row| row.get::<_, i64>(0))?;
        assert_eq!(count, 1);
    }
    let checkpoint = connection.query_row(
        "SELECT COUNT(*) FROM checkpoints WHERE episode_id = ?1 AND sequence = 1",
        [&lineage.episode_id],
        |row| row.get::<_, i64>(0),
    )?;
    assert_eq!(checkpoint, 1);
    Ok(())
}

fn remove_test_store(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

#[test]
fn v7_upgrade_runs_worker_then_receipt_migration_on_normal_open()
-> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!(
        "sts2-recipe-receipt-v7-{}-{}.sqlite3",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let lineage = make_v7_file(&path)?;
        assert_v7_open_migrates(&path, &lineage)
    })();
    remove_test_store(&path);
    result
}

#[test]
fn v8_v9_and_future_version_migrations_are_explicit() -> Result<(), Box<dyn std::error::Error>> {
    let (mut store, lineage) = seeded_store()?;
    seed_existing_rows(&store.connection, &lineage)?;
    store.connection.execute_batch(
        "DROP TABLE recipe_map_invocations;
         PRAGMA user_version = 8;",
    )?;
    schema::migrate(&mut store.connection)?;
    let version = store
        .connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))?;
    assert_eq!(version, 9);
    let preserved = store.connection.query_row(
        "SELECT COUNT(*) FROM operations WHERE operation_id = 'operation-preserved'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    assert_eq!(preserved, 1);
    schema::migrate(&mut store.connection)?;
    store
        .connection
        .execute_batch("PRAGMA user_version = 10;")?;
    assert_eq!(
        schema::migrate(&mut store.connection),
        Err(ExecutionStoreError::Incompatible)
    );
    Ok(())
}
