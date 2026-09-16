// SPDX-License-Identifier: MIT

use super::state::ControlState;
use super::store::{ContextControlStore, decrypt_with_key};
use super::store_schema::digest;
use super::store_types::{
    DurableContextOwnerControlReceipt, DurableControlStoreError, MAX_OWNER_RECEIPT_BYTES,
};
use rusqlite::{OptionalExtension, Transaction, params};

pub(super) struct PreparedOwnerReceipt {
    owner_id: String,
    command_digest: String,
    idempotency_digest: String,
    envelope: Vec<u8>,
    plaintext: Vec<u8>,
}

pub(super) fn prepare_owner_receipt(
    store: &ContextControlStore,
    record: &DurableContextOwnerControlReceipt,
    state: &ControlState,
) -> Result<PreparedOwnerReceipt, DurableControlStoreError> {
    if record.owner_id.is_empty()
        || record.actor_subject.is_empty()
        || record.binding.owner_id != record.owner_id
        || record.binding.workflow_run_id != state.boundary.run_id
        || record.receipt.owner_id != record.owner_id
        || record.receipt.invocation_id != record.binding.invocation_id
        || record.receipt.binding_id != record.binding.binding_id
        || record.receipt.binding_digest != record.binding.binding_digest
    {
        return Err(DurableControlStoreError::ScopeMismatch);
    }
    if record.receipt.boundary != state.boundary
        || record.receipt.plan_epoch != state.plan_epoch
        || record.receipt.controller_epoch != state.boundary.controller_epoch
    {
        return Err(DurableControlStoreError::ScopeMismatch);
    }
    if matches!(
        &record.command,
        crate::management::ContextControlCommand::Commit { .. }
    ) && record.receipt.revision_id.as_deref() != Some(state.active_revision_id.as_str())
    {
        return Err(DurableControlStoreError::ScopeMismatch);
    }
    record
        .receipt
        .validate_for(&record.binding, &record.command)
        .map_err(|_| DurableControlStoreError::Corrupt)?;
    let plaintext = serde_json::to_vec(record).map_err(|_| DurableControlStoreError::Encode)?;
    if plaintext.len() > MAX_OWNER_RECEIPT_BYTES {
        return Err(DurableControlStoreError::TooLarge);
    }
    let command_digest = serialized_digest(&record.command)?;
    let aad = owner_receipt_aad(
        &record.binding.workflow_run_id,
        &record.owner_id,
        &command_digest,
    )?;
    let envelope = store.encrypt_with_aad(&plaintext, &aad)?;
    Ok(PreparedOwnerReceipt {
        owner_id: record.owner_id.clone(),
        command_digest,
        idempotency_digest: idempotency_digest(&record.command)?,
        envelope,
        plaintext,
    })
}

pub(super) fn persist_owner_receipt(
    transaction: &Transaction<'_>,
    run_id: &str,
    key: &[u8; 32],
    receipt: Option<PreparedOwnerReceipt>,
) -> Result<(), DurableControlStoreError> {
    let Some(receipt) = receipt else {
        return Ok(());
    };
    let existing_command = transaction
        .query_row(
            "SELECT command_digest, idempotency_digest, envelope, envelope_digest
             FROM context_control_owner_receipts
             WHERE run_id = ?1 AND owner_id = ?2 AND command_digest = ?3",
            params![run_id, receipt.owner_id, receipt.command_digest],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|_| DurableControlStoreError::Sqlite)?;
    let existing_idempotency = transaction
        .query_row(
            "SELECT command_digest, idempotency_digest, envelope, envelope_digest
             FROM context_control_owner_receipts
             WHERE run_id = ?1 AND owner_id = ?2 AND idempotency_digest = ?3",
            params![run_id, receipt.owner_id, receipt.idempotency_digest],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|_| DurableControlStoreError::Sqlite)?;
    if let (Some(command), Some(idempotency)) = (&existing_command, &existing_idempotency)
        && command.0 != idempotency.0
    {
        return Err(DurableControlStoreError::OwnerReceiptConflict);
    }
    if let Some((existing_command_digest, existing_idempotency_digest, envelope, envelope_digest)) =
        existing_command.or(existing_idempotency)
    {
        if existing_command_digest != receipt.command_digest
            || existing_idempotency_digest != receipt.idempotency_digest
            || digest(&envelope) != envelope_digest
        {
            return Err(DurableControlStoreError::OwnerReceiptConflict);
        }
        let aad = owner_receipt_aad(run_id, &receipt.owner_id, &existing_command_digest)?;
        let plaintext = decrypt_with_key(key, &envelope, &aad)?;
        if plaintext.len() > MAX_OWNER_RECEIPT_BYTES {
            return Err(DurableControlStoreError::TooLarge);
        }
        let existing: DurableContextOwnerControlReceipt =
            serde_json::from_slice(&plaintext).map_err(|_| DurableControlStoreError::Decode)?;
        let bytes = serde_json::to_vec(&existing).map_err(|_| DurableControlStoreError::Encode)?;
        if bytes != receipt.plaintext {
            return Err(DurableControlStoreError::OwnerReceiptConflict);
        }
    } else {
        let envelope_digest = digest(&receipt.envelope);
        transaction
            .execute(
                "INSERT INTO context_control_owner_receipts
                    (run_id, owner_id, command_digest, idempotency_digest,
                     envelope, envelope_digest)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    run_id,
                    receipt.owner_id,
                    receipt.command_digest,
                    receipt.idempotency_digest,
                    receipt.envelope,
                    envelope_digest,
                ],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
    }
    Ok(())
}

impl ContextControlStore {
    /// Reads exact historical owner receipt evidence without claiming or verifying the
    /// single-writer authority fence. Callers still authorize the actor and validate scope.
    pub fn lookup_owner_control_receipt(
        &self,
        owner_id: &str,
        actor_subject: &str,
        command: &crate::management::ContextControlCommand,
    ) -> Result<Option<DurableContextOwnerControlReceipt>, DurableControlStoreError> {
        let command_digest = serialized_digest(command)?;
        let idempotency_digest = idempotency_digest(command)?;
        let Some((stored_idempotency_digest, envelope, envelope_digest)) = self
            .connection
            .query_row(
                "SELECT idempotency_digest, envelope, envelope_digest
                 FROM context_control_owner_receipts
                 WHERE run_id = ?1 AND owner_id = ?2 AND command_digest = ?3",
                params![self.run_id, owner_id, command_digest],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?
        else {
            return Ok(None);
        };
        if envelope.len() > MAX_OWNER_RECEIPT_BYTES + 40 || digest(&envelope) != envelope_digest {
            return Err(DurableControlStoreError::Corrupt);
        }
        if stored_idempotency_digest != idempotency_digest {
            return Err(DurableControlStoreError::Corrupt);
        }
        let aad = owner_receipt_aad(&self.run_id, owner_id, &command_digest)?;
        let plaintext = self.decrypt_with_aad(&envelope, &aad)?;
        if plaintext.len() > MAX_OWNER_RECEIPT_BYTES {
            return Err(DurableControlStoreError::TooLarge);
        }
        let record: DurableContextOwnerControlReceipt =
            serde_json::from_slice(&plaintext).map_err(|_| DurableControlStoreError::Decode)?;
        if record.owner_id != owner_id
            || record.binding.owner_id != owner_id
            || record.binding.workflow_run_id != self.run_id
            || record.command != *command
            || record.receipt.owner_id != owner_id
            || record
                .receipt
                .validate_for(&record.binding, command)
                .is_err()
        {
            return Err(DurableControlStoreError::Corrupt);
        }
        if record.actor_subject != actor_subject {
            return Ok(None);
        }
        Ok(Some(record))
    }
}

fn serialized_digest<T: serde::Serialize>(value: &T) -> Result<String, DurableControlStoreError> {
    serde_json::to_vec(value)
        .map(|bytes| digest(&bytes))
        .map_err(|_| DurableControlStoreError::Encode)
}

fn idempotency_digest(
    command: &crate::management::ContextControlCommand,
) -> Result<String, DurableControlStoreError> {
    let key = match command {
        crate::management::ContextControlCommand::Pause {
            idempotency_key, ..
        }
        | crate::management::ContextControlCommand::Commit {
            idempotency_key, ..
        }
        | crate::management::ContextControlCommand::Resume {
            idempotency_key, ..
        } => idempotency_key,
    };
    Ok(digest(key.as_bytes()))
}

fn owner_receipt_aad(
    run_id: &str,
    owner_id: &str,
    command_digest: &str,
) -> Result<Vec<u8>, DurableControlStoreError> {
    let mut aad = Vec::new();
    for component in [
        b"ascension.context-control.owner-receipt.v1\0".as_slice(),
        run_id.as_bytes(),
        owner_id.as_bytes(),
        command_digest.as_bytes(),
    ] {
        let length =
            u64::try_from(component.len()).map_err(|_| DurableControlStoreError::TooLarge)?;
        aad.extend_from_slice(&length.to_be_bytes());
        aad.extend_from_slice(component);
    }
    Ok(aad)
}
