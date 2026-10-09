// SPDX-License-Identifier: MIT

use rusqlite::{Connection, params};
use sts2_harness::{
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreConfig, ResumeState,
};

#[test]
fn version_eight_migration_installs_recipe_receipt_schema_and_constraints()
-> Result<(), Box<dyn std::error::Error>> {
    let path = super::database_path("recipe-migration");
    let mut store = ExecutionStore::open(ExecutionStoreConfig::new(&path))?;
    let lineage = ExecutionLineage::new(
        "migration-run",
        "migration-episode",
        "migration-attempt",
        "migration-trajectory",
    )?;
    let fingerprint = ExecutionFingerprint::new(
        "migration-seed",
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
    )?;
    store.start_episode(&lineage, &fingerprint)?;
    store.close()?;
    drop(store);

    // The sole v9 schema addition is this table; removing it restores the v8 schema.
    {
        let raw = Connection::open(&path)?;
        raw.execute_batch("DROP TABLE recipe_map_invocations; PRAGMA user_version = 8;")?;
    }
    let mut migrated = ExecutionStore::open(ExecutionStoreConfig::new(&path))?;
    assert!(matches!(
        migrated.resume_episode(&lineage.episode_id, &fingerprint)?,
        ResumeState::Ready { .. }
    ));
    migrated.close()?;
    drop(migrated);

    let raw = Connection::open(&path)?;
    raw.execute_batch("PRAGMA foreign_keys = ON")?;
    let version: i32 = raw.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    assert_eq!(version, 9);

    let tables: i64 = raw.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN
         ('workflow_events', 'worker_control', 'worker_control_boots', 'worker_handoffs',
          'recipe_map_invocations')",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(tables, 5);

    let column_info = {
        let mut statement = raw.prepare("PRAGMA table_info(recipe_map_invocations)")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(5)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let columns = column_info
        .iter()
        .map(|(name, _, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(",");
    const EXPECTED_COLUMNS: &str = concat!(
        "run_id,episode_id,attempt_id,trajectory_id,model_execution_id,recipe_id,",
        "recipe_revision,operation,runtime_config_digest,instance_id,gateway_session_id,",
        "mcp_session_id,lease_id,lease_epoch,state_id,generation,runtime_map_profile,",
        "schema_digest,rpc_correlation_id,request_digest,typed_result_digest,",
        "owner_snapshot_digest,decision_input_digest,state,created_at,updated_at"
    );
    assert_eq!(columns, EXPECTED_COLUMNS);

    let primary_key = column_info
        .iter()
        .filter(|(_, _, order)| *order > 0)
        .map(|(name, _, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(",");
    const EXPECTED_PRIMARY_KEY: &str = concat!(
        "run_id,episode_id,attempt_id,trajectory_id,model_execution_id,recipe_id,",
        "recipe_revision,operation"
    );
    assert_eq!(primary_key, EXPECTED_PRIMARY_KEY);

    let foreign_keys = {
        let mut statement = raw.prepare("PRAGMA foreign_key_list(recipe_map_invocations)")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    assert!(foreign_keys.contains(&(
        String::from("episodes"),
        String::from("episode_id"),
        String::from("episode_id"),
    )));
    assert!(foreign_keys.contains(&(
        String::from("attempts"),
        String::from("attempt_id"),
        String::from("attempt_id"),
    )));

    let recipe_error = insert_receipt(&raw, &lineage, &lineage.attempt_id, "runtime.invalid", None)
        .expect_err("an unsupported recipe id must be rejected");
    assert_constraint_violation(recipe_error, "recipe id CHECK");
    let foreign_key_error = insert_receipt(
        &raw,
        &lineage,
        "missing-attempt",
        "runtime.map-context",
        None,
    )
    .expect_err("an unknown attempt must be rejected");
    assert_constraint_violation(foreign_key_error, "attempt foreign key");
    let state_error = insert_receipt(
        &raw,
        &lineage,
        &lineage.attempt_id,
        "runtime.map-context",
        Some("f".repeat(64)),
    )
    .expect_err("intent rows cannot contain response digests");
    assert_constraint_violation(state_error, "receipt-state CHECK");

    assert_eq!(
        insert_receipt(
            &raw,
            &lineage,
            &lineage.attempt_id,
            "runtime.map-context",
            None
        )?,
        1
    );
    let duplicate = insert_receipt(
        &raw,
        &lineage,
        &lineage.attempt_id,
        "runtime.map-context",
        None,
    )
    .expect_err("the exact invocation primary key cannot be inserted twice");
    assert_constraint_violation(duplicate, "invocation primary key");

    let rows: i64 = raw.query_row("SELECT COUNT(*) FROM recipe_map_invocations", [], |row| {
        row.get(0)
    })?;
    assert_eq!(rows, 1);
    drop(raw);
    super::remove_database(&path);
    Ok(())
}

fn insert_receipt(
    connection: &Connection,
    lineage: &ExecutionLineage,
    attempt_id: &str,
    recipe_id: &str,
    typed_result_digest: Option<String>,
) -> Result<usize, rusqlite::Error> {
    connection.execute(
        "INSERT INTO recipe_map_invocations(
            run_id, episode_id, attempt_id, trajectory_id, model_execution_id, recipe_id,
            recipe_revision, operation, runtime_config_digest, instance_id, gateway_session_id,
            mcp_session_id, lease_id, lease_epoch, state_id, generation, runtime_map_profile,
            schema_digest, rpc_correlation_id, request_digest, typed_result_digest,
            owner_snapshot_digest, decision_input_digest, state, created_at, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, 'model-execution-1', ?5, 1, 'map_snapshot', ?6, 'instance-1',
            'gateway-session-1', 'mcp-session-1', 'lease-1', 1, 'state-1', 1,
            'runtime-map-v1', ?7, '3', ?8, ?9, NULL, NULL, 'intent_recorded', 1, 1
         )",
        params![
            lineage.run_id,
            lineage.episode_id,
            attempt_id,
            lineage.trajectory_id,
            recipe_id,
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "d".repeat(64),
            "e".repeat(64),
            typed_result_digest,
        ],
    )
}

fn assert_constraint_violation(error: rusqlite::Error, contract: &str) {
    assert!(
        matches!(
            &error,
            rusqlite::Error::SqliteFailure(details, _)
                if details.code == rusqlite::ErrorCode::ConstraintViolation
        ),
        "{contract} was not enforced: {error}"
    );
}
