// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::super::schema;
use super::super::store_core::ExecutionStore;
use super::super::types::ExecutionStoreError;
use super::super::workflow_types::{WorkflowDefinition, WorkflowPlan};

impl ExecutionStore {
    /// Registers an immutable workflow definition. Repeating the exact same identity and bytes is
    /// idempotent; changing either the identity's content or the definition bytes is rejected.
    pub fn register_workflow_definition(
        &mut self,
        definition: &WorkflowDefinition,
    ) -> Result<(), ExecutionStoreError> {
        self.ensure_open()?;
        definition.validate()?;
        let tx = schema::transaction(&mut self.connection)?;
        let existing = tx
            .query_row(
                "SELECT definition_digest, definition FROM workflow_definitions
                 WHERE workflow_id = ?1",
                [definition.id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()
            .map_err(schema::map_sqlite)?;
        if let Some((digest, bytes)) = existing {
            if digest != definition.digest || bytes != definition.bytes {
                return Err(ExecutionStoreError::Conflict);
            }
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(());
        }
        tx.execute(
            "INSERT INTO workflow_definitions
             (workflow_id, definition_digest, definition, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                definition.id.as_str(),
                definition.digest,
                definition.bytes,
                ExecutionStore::now()
            ],
        )
        .map_err(schema::map_sqlite)?;
        tx.commit().map_err(schema::map_sqlite)
    }

    /// Registers an immutable plan after its definition has been admitted.
    pub fn register_workflow_plan(
        &mut self,
        plan: &WorkflowPlan,
    ) -> Result<(), ExecutionStoreError> {
        self.ensure_open()?;
        plan.validate()?;
        let tx = schema::transaction(&mut self.connection)?;
        let definition_exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workflow_definitions WHERE workflow_id = ?1)",
                [plan.workflow_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(schema::map_sqlite)?
            != 0;
        if !definition_exists {
            return Err(ExecutionStoreError::Missing);
        }
        let existing = tx
            .query_row(
                "SELECT workflow_id, plan_digest, plan FROM workflow_plans WHERE plan_id = ?1",
                [plan.id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(schema::map_sqlite)?;
        if let Some((workflow_id, digest, bytes)) = existing {
            if workflow_id != plan.workflow_id.as_str()
                || digest != plan.digest
                || bytes != plan.bytes
            {
                return Err(ExecutionStoreError::Conflict);
            }
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(());
        }
        tx.execute(
            "INSERT INTO workflow_plans
             (plan_id, workflow_id, plan_digest, plan, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                plan.id.as_str(),
                plan.workflow_id.as_str(),
                plan.digest,
                plan.bytes,
                ExecutionStore::now()
            ],
        )
        .map_err(schema::map_sqlite)?;
        tx.commit().map_err(schema::map_sqlite)
    }
}
