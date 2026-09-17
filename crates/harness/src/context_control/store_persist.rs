// SPDX-License-Identifier: MIT

use super::*;

impl ContextControlStore {
    pub(super) fn persist_inner(
        &mut self,
        authority: &ControlAuthority,
        mode: StoreMode,
        receipt_record: Option<&DurableContextOwnerControlReceipt>,
        source_activation: Option<&DurableActiveContextSource>,
    ) -> Result<(), DurableControlStoreError> {
        let journal = authority
            .export_journal()
            .map_err(|_| DurableControlStoreError::Encode)?;
        if journal.len() > MAX_JOURNAL_BYTES {
            return Err(DurableControlStoreError::TooLarge);
        }
        let encrypted = self.encrypt(&journal)?;
        let journal_digest = digest(&encrypted);
        if self.failpoint == Some(DurableStoreFailpoint::BeforeJournalWrite) {
            self.failpoint = None;
            return Err(DurableControlStoreError::Failpoint);
        }
        let state = authority.state();
        if state.boundary.run_id != self.run_id {
            return Err(DurableControlStoreError::ScopeMismatch);
        }
        if source_activation
            .is_some_and(|source| source.active_revision_id != state.active_revision_id)
        {
            return Err(DurableControlStoreError::SourceConflict);
        }
        let receipt_envelope = receipt_record
            .map(|record| prepare_owner_receipt(self, record, state))
            .transpose()?;
        self.claim_owner()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        Self::verify_owner(&transaction, &self.run_id, &self.owner_token)?;
        persist_owner_receipt(&transaction, &self.run_id, &self.key, receipt_envelope)?;
        if let Some(source) = source_activation {
            persist_active_source(&transaction, &self.run_id, source)?;
        }
        transaction
            .execute(
                "INSERT INTO context_control_journal
                    (run_id, envelope, envelope_digest, management_active, active_revision_id,
                     pause_latched, controller_epoch, plan_epoch, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(run_id) DO UPDATE SET
                    envelope = excluded.envelope,
                    envelope_digest = excluded.envelope_digest,
                    management_active = excluded.management_active,
                    active_revision_id = excluded.active_revision_id,
                    pause_latched = excluded.pause_latched,
                    controller_epoch = excluded.controller_epoch,
                    plan_epoch = excluded.plan_epoch,
                    updated_at = excluded.updated_at",
                params![
                    self.run_id,
                    encrypted,
                    journal_digest,
                    mode.as_i64(),
                    state.active_revision_id,
                    i64::from(state.pause_latched),
                    state.boundary.controller_epoch as i64,
                    state.plan_epoch as i64,
                    now_seconds(),
                ],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        insert_outbox(&transaction, &self.run_id, authority.events())?;
        if self.failpoint == Some(DurableStoreFailpoint::BeforeCommit) {
            self.failpoint = None;
            return Err(DurableControlStoreError::Failpoint);
        }
        transaction
            .commit()
            .map_err(|_| DurableControlStoreError::Sqlite)
    }
}
