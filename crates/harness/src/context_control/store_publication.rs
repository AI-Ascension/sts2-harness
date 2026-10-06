// SPDX-License-Identifier: MIT

//! Encrypted, run-scoped immutable draft publications and activation links.

use super::super::store_render_sources::source_aad;
use super::super::store_schema::digest;
use super::super::store_types::{
    DurableActiveContextPublicationLink, DurableActiveContextSource,
    DurableContextOwnerPublication, DurableContextOwnerPublicationWrite,
    DurableContextSourceSnapshot, DurableControlStoreError, DurableStoreFailpoint,
    MAX_CONTEXT_SOURCE_BYTES, MAX_OWNER_PUBLICATIONS, MAX_OWNER_RECEIPT_BYTES,
};
use super::ContextControlStore;
use crate::checkpoint_projection::hmac_sha256;
use crate::management::{
    ContextOwnerDraftPublicationLookupRequest, ContextOwnerDraftPublicationReceipt,
    ContextOwnerDraftPublicationRequest,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

const INDEX_KEY_DOMAIN: &[u8] = b"ascension.context-control.draft-publication.index-key.v1\0";
const OWNER_INDEX_DOMAIN: &[u8] = b"ascension.context-control.draft-publication.owner-index.v1\0";
const ACTOR_INDEX_DOMAIN: &[u8] = b"ascension.context-control.draft-publication.actor-index.v1\0";
const REQUEST_INDEX_DOMAIN: &[u8] =
    b"ascension.context-control.draft-publication.request-index.v1\0";
const SOURCE_ID_DOMAIN: &[u8] = b"ascension.context-control.draft-publication.source-id.v1\0";
const RECEIPT_AAD_DOMAIN: &[u8] = b"ascension.context-control.draft-publication-receipt.v1\0";
const LINK_AAD_DOMAIN: &[u8] = b"ascension.context-control.active-publication-link.v1\0";

#[derive(Clone, Copy)]
struct PublicationIndices {
    owner: [u8; 32],
    actor: [u8; 32],
    request: [u8; 32],
    source: [u8; 32],
}

impl ContextControlStore {
    /// Returns only the exact same-actor receipt. Authentication is checked before keyed lookup,
    /// so a wrong store key cannot turn a populated index into an empty result.
    pub fn recover_draft_publication(
        &self,
        owner_id: &str,
        actor_subject: &str,
        lookup: &ContextOwnerDraftPublicationLookupRequest,
    ) -> Result<Option<DurableContextOwnerPublication>, DurableControlStoreError> {
        validate_lookup(owner_id, actor_subject, lookup)?;
        if lookup.request.expected_boundary.run_id != self.run_id {
            return Err(DurableControlStoreError::ScopeMismatch);
        }
        self.authenticate_publication_key()?;
        self.verify_connection_owner()?;
        let request_digest = lookup
            .request
            .digest()
            .map_err(|_| DurableControlStoreError::ScopeMismatch)?;
        let indices = publication_indices(
            &self.key,
            &self.run_id,
            owner_id,
            actor_subject,
            &lookup.request,
        );
        let row = self
            .connection
            .query_row(
                "SELECT source_id, source_version, request_digest, source_digest,
                        receipt_envelope, receipt_envelope_digest
                 FROM context_control_owner_publications
                 WHERE run_id = ?1 AND owner_index_digest = ?2
                   AND actor_index_digest = ?3 AND request_index_digest = ?4",
                params![
                    self.run_id,
                    hex(&indices.owner),
                    hex(&indices.actor),
                    hex(&indices.request)
                ],
                publication_row,
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let Some(row) = row else {
            return Ok(None);
        };
        if row.2 != request_digest {
            return Err(DurableControlStoreError::PublicationConflict);
        }
        decode_publication(
            &self.key,
            &self.run_id,
            owner_id,
            actor_subject,
            indices,
            row,
            Some(&request_digest),
        )
        .map(Some)
    }

    /// Writes source bytes, encrypted receipt, and the owner-state CAS version in one immediate
    /// transaction. Exact replay and changed-payload conflict are resolved before live CAS reads.
    pub fn publish_draft_source(
        &mut self,
        write: DurableContextOwnerPublicationWrite<'_>,
    ) -> Result<DurableContextOwnerPublication, DurableControlStoreError> {
        let DurableContextOwnerPublicationWrite {
            owner_id,
            actor_subject,
            request,
            receipt,
            source,
            expected_owner_state_bytes,
            configured_source_count,
        } = write;
        validate_identity(owner_id)?;
        validate_identity(actor_subject)?;
        request
            .validate()
            .map_err(|_| DurableControlStoreError::ScopeMismatch)?;
        if request.expected_boundary.run_id != self.run_id {
            return Err(DurableControlStoreError::ScopeMismatch);
        }
        let request_digest = request
            .digest()
            .map_err(|_| DurableControlStoreError::ScopeMismatch)?;
        let indices =
            publication_indices(&self.key, &self.run_id, owner_id, actor_subject, request);
        let run_id = self.run_id.clone();
        let key = self.key;
        let owner_token = self.owner_token.clone();
        self.claim_owner()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        Self::verify_owner(&transaction, &run_id, &owner_token)?;
        authenticate_transaction_key(&transaction, &key, &run_id)?;
        if let Some(existing) = existing_request(
            &transaction,
            &key,
            &run_id,
            owner_id,
            actor_subject,
            indices,
        )? {
            if existing.receipt.request_digest != request_digest {
                return Err(DurableControlStoreError::PublicationConflict);
            }
            transaction
                .commit()
                .map_err(|_| DurableControlStoreError::Sqlite)?;
            return Ok(existing);
        }
        let dynamic_source_count = transaction
            .query_row(
                "SELECT COUNT(*) FROM context_control_owner_publications WHERE run_id = ?1",
                [run_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let dynamic_source_count = usize::try_from(dynamic_source_count).unwrap_or(usize::MAX);
        if configured_source_count > MAX_OWNER_PUBLICATIONS
            || configured_source_count.saturating_add(dynamic_source_count)
                >= MAX_OWNER_PUBLICATIONS
        {
            return Err(DurableControlStoreError::PublicationCapacity);
        }
        validate_receipt(
            owner_id,
            actor_subject,
            request,
            receipt,
            source,
            expected_owner_state_bytes,
        )?;
        if expected_owner_state_bytes.is_empty()
            || expected_owner_state_bytes.len()
                > super::super::store_types::MAX_OWNER_CONTEXT_STATE_BYTES
        {
            return Err(DurableControlStoreError::TooLarge);
        }
        if receipt.source_id != publication_source_id(indices) {
            return Err(DurableControlStoreError::ScopeMismatch);
        }

        let current = transaction
            .query_row(
                "SELECT owner_id, record_version, envelope, envelope_digest
                 FROM context_control_owner_state WHERE run_id = ?1",
                [run_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?
            .ok_or(DurableControlStoreError::OwnerContextConflict)?;
        let (stored_owner, stored_version, state_envelope, state_digest) = current;
        if stored_owner != owner_id {
            return Err(DurableControlStoreError::ScopeMismatch);
        }
        let expected = i64::try_from(receipt.expected_owner_state_version)
            .map_err(|_| DurableControlStoreError::TooLarge)?;
        let resulting = i64::try_from(receipt.resulting_owner_state_version)
            .map_err(|_| DurableControlStoreError::TooLarge)?;
        let published_at =
            i64::try_from(receipt.published_at).map_err(|_| DurableControlStoreError::TooLarge)?;
        let expires_at =
            i64::try_from(receipt.expires_at).map_err(|_| DurableControlStoreError::TooLarge)?;
        let source_version = i64::try_from(receipt.source_version)
            .map_err(|_| DurableControlStoreError::TooLarge)?;
        if stored_version != expected
            || state_envelope.len() > super::super::store_types::MAX_OWNER_CONTEXT_STATE_BYTES + 40
            || digest(&state_envelope) != state_digest
        {
            return Err(DurableControlStoreError::OwnerContextConflict);
        }
        let state_plaintext =
            super::decrypt_with_key(&key, &state_envelope, &owner_state_aad(&run_id, owner_id)?)?;
        if state_plaintext != expected_owner_state_bytes {
            return Err(DurableControlStoreError::OwnerContextConflict);
        }
        insert_publication_source(&transaction, &run_id, &key, source)?;
        let receipt_plaintext =
            serde_json::to_vec(receipt).map_err(|_| DurableControlStoreError::Encode)?;
        if receipt_plaintext.len() > MAX_OWNER_RECEIPT_BYTES {
            return Err(DurableControlStoreError::TooLarge);
        }
        let envelope = super::encrypt_with_key(
            &key,
            &receipt_plaintext,
            &receipt_aad(&run_id, receipt, indices)?,
        )?;
        let envelope_digest = digest(&envelope);
        let expected_next = receipt
            .expected_owner_state_version
            .checked_add(1)
            .ok_or(DurableControlStoreError::TooLarge)?;
        if receipt.resulting_owner_state_version != expected_next {
            return Err(DurableControlStoreError::OwnerContextConflict);
        }
        let changed = transaction
            .execute(
                "UPDATE context_control_owner_state SET record_version = ?1, updated_at = ?2
                 WHERE run_id = ?3 AND owner_id = ?4 AND record_version = ?5",
                params![resulting, published_at, run_id, owner_id, stored_version],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        if changed != 1 {
            return Err(DurableControlStoreError::OwnerContextConflict);
        }
        transaction
            .execute(
                "INSERT INTO context_control_owner_publications
                 (run_id, source_id, source_version, owner_index_digest, actor_index_digest,
                  request_index_digest, request_digest, source_digest, expected_owner_state_version,
                  resulting_owner_state_version, published_at, expires_at, receipt_envelope,
                  receipt_envelope_digest)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    run_id,
                    receipt.source_id,
                    source_version,
                    hex(&indices.owner),
                    hex(&indices.actor),
                    hex(&indices.request),
                    request_digest,
                    receipt.source_digest,
                    expected,
                    resulting,
                    published_at,
                    expires_at,
                    envelope,
                    envelope_digest
                ],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        if self.failpoint == Some(DurableStoreFailpoint::BeforeCommit) {
            self.failpoint = None;
            return Err(DurableControlStoreError::Failpoint);
        }
        transaction
            .commit()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        Ok(DurableContextOwnerPublication {
            receipt: receipt.clone(),
            receipt_envelope_digest: envelope_digest,
            owner_index_digest: hex(&indices.owner),
            actor_index_digest: hex(&indices.actor),
            request_index_digest: hex(&indices.request),
        })
    }
}

include!("store_publication_index.rs");
include!("store_publication_read.rs");
include!("store_publication_link.rs");
include!("store_publication_crypto.rs");
