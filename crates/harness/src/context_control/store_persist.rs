// SPDX-License-Identifier: MIT

use super::publication::{ActivePublicationLinkWrite, persist_active_publication_link};
use super::*;

impl ContextControlStore {
    pub(super) fn persist_inner(
        &mut self,
        authority: &ControlAuthority,
        mode: StoreMode,
        receipt_record: Option<&DurableContextOwnerControlReceipt>,
        source_activation: Option<&DurableActiveContextSource>,
        publication: Option<&DurableContextOwnerPublication>,
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
        if let Some(publication) = publication {
            let Some(source) = source_activation else {
                return Err(DurableControlStoreError::ActivePublicationConflict);
            };
            if source.source_id != publication.receipt.source_id
                || source.version != publication.receipt.source_version
                || source.digest != publication.receipt.source_digest
                || source.active_revision_id != state.active_revision_id
                || receipt_record.is_none_or(|record| {
                    !matches!(
                        &record.command,
                        crate::management::ContextControlCommand::Commit {
                            preview_manifest_digest,
                            approved_manifest_digest,
                            ..
                        } if preview_manifest_digest == &source.digest
                            && approved_manifest_digest == &source.digest
                    )
                })
            {
                return Err(DurableControlStoreError::ActivePublicationConflict);
            }
        }
        let receipt_envelope = receipt_record
            .map(|record| prepare_owner_receipt(self, record, state))
            .transpose()?;
        self.claim_owner()?;
        let key = &self.key;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        Self::verify_owner(&transaction, &self.run_id, &self.owner_token)?;
        let control_receipt_digest =
            persist_owner_receipt(&transaction, &self.run_id, &self.key, receipt_envelope)?;
        if let Some(source) = source_activation {
            persist_active_source(&transaction, &self.run_id, source)?;
            if let Some(publication) = publication {
                let control_receipt_digest = control_receipt_digest
                    .as_deref()
                    .ok_or(DurableControlStoreError::OwnerReceiptConflict)?;
                persist_active_publication_link(
                    &transaction,
                    ActivePublicationLinkWrite {
                        run_id: &self.run_id,
                        key,
                        source,
                        publication,
                        control_receipt_envelope_digest: control_receipt_digest,
                        active_revision_id: &state.active_revision_id,
                        activated_at: u64::try_from(now_seconds())
                            .map_err(|_| DurableControlStoreError::TooLarge)?,
                    },
                )?;
            } else {
                transaction
                    .execute(
                        "DELETE FROM context_control_active_publication_links WHERE run_id = ?1",
                        [self.run_id.as_str()],
                    )
                    .map_err(|_| DurableControlStoreError::Sqlite)?;
            }
        } else {
            // A control commit without a source activation retires the prior source pointer when
            // it belongs to an older revision. Pause and resume keep the same revision, so their
            // source and publication link remain active.
            transaction
                .execute(
                    "DELETE FROM context_control_active_context_source
                     WHERE run_id = ?1 AND active_revision_id <> ?2",
                    params![self.run_id.as_str(), state.active_revision_id.as_str()],
                )
                .map_err(|_| DurableControlStoreError::Sqlite)?;
            transaction
                .execute(
                    "DELETE FROM context_control_active_publication_links
                     WHERE run_id = ?1 AND active_revision_id <> ?2",
                    params![self.run_id.as_str(), state.active_revision_id.as_str()],
                )
                .map_err(|_| DurableControlStoreError::Sqlite)?;
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
