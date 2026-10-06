// SPDX-License-Identifier: MIT

// Publication index derivation, framing, AAD, and stored-row validation.

fn existing_request(
    transaction: &Transaction<'_>,
    key: &[u8; 32],
    run_id: &str,
    owner_id: &str,
    actor_subject: &str,
    indices: PublicationIndices,
) -> Result<Option<DurableContextOwnerPublication>, DurableControlStoreError> {
    let row = transaction.query_row(
        "SELECT source_id, source_version, request_digest, source_digest, receipt_envelope,
                receipt_envelope_digest FROM context_control_owner_publications
         WHERE run_id = ?1 AND owner_index_digest = ?2 AND actor_index_digest = ?3 AND request_index_digest = ?4",
        params![run_id, hex(&indices.owner), hex(&indices.actor), hex(&indices.request)], publication_row,
    ).optional().map_err(|_| DurableControlStoreError::Sqlite)?;
    row.map(|row| decode_publication(key, run_id, owner_id, actor_subject, indices, row, None))
        .transpose()
}

type PublicationRow = (String, i64, String, String, Vec<u8>, String);

fn publication_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PublicationRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn decode_publication(
    key: &[u8; 32],
    run_id: &str,
    owner_id: &str,
    actor_subject: &str,
    indices: PublicationIndices,
    row: PublicationRow,
    expected_digest: Option<&str>,
) -> Result<DurableContextOwnerPublication, DurableControlStoreError> {
    let (source_id, source_version, request_digest, source_digest, envelope, envelope_digest) = row;
    if envelope.len() > MAX_OWNER_RECEIPT_BYTES + 40
        || digest(&envelope) != envelope_digest
        || expected_digest.is_some_and(|value| value != request_digest)
    {
        return Err(DurableControlStoreError::Corrupt);
    }
    let source_version =
        u64::try_from(source_version).map_err(|_| DurableControlStoreError::Corrupt)?;
    let aad = receipt_aad_values(
        run_id,
        owner_id,
        &source_id,
        source_version,
        indices,
        &request_digest,
        &source_digest,
    )?;
    let plaintext = super::decrypt_with_key(key, &envelope, &aad)?;
    if plaintext.len() > MAX_OWNER_RECEIPT_BYTES {
        return Err(DurableControlStoreError::TooLarge);
    }
    let receipt: ContextOwnerDraftPublicationReceipt =
        serde_json::from_slice(&plaintext).map_err(|_| DurableControlStoreError::Decode)?;
    if receipt.owner_id != owner_id
        || receipt.workflow_run_id != run_id
        || receipt.actor_subject != actor_subject
        || receipt.source_id != source_id
        || receipt.source_version != source_version
        || receipt.source_digest != source_digest
        || receipt.request_digest != request_digest
    {
        return Err(DurableControlStoreError::Corrupt);
    }
    Ok(DurableContextOwnerPublication {
        receipt,
        receipt_envelope_digest: envelope_digest,
        owner_index_digest: hex(&indices.owner),
        actor_index_digest: hex(&indices.actor),
        request_index_digest: hex(&indices.request),
    })
}

fn insert_publication_source(
    transaction: &Transaction<'_>,
    run_id: &str,
    key: &[u8; 32],
    source: &DurableContextSourceSnapshot,
) -> Result<(), DurableControlStoreError> {
    validate_source_id(&source.source_id, source.version, &source.digest)?;
    let plaintext =
        serde_json::to_vec(&source.document).map_err(|_| DurableControlStoreError::Encode)?;
    if plaintext.is_empty()
        || plaintext.len() > MAX_CONTEXT_SOURCE_BYTES
        || digest(&plaintext) != source.digest
    {
        return Err(DurableControlStoreError::SourceConflict);
    }
    let count = transaction
        .query_row(
            "SELECT COUNT(*) FROM context_control_context_sources WHERE run_id = ?1",
            [run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| DurableControlStoreError::Sqlite)?;
    if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_OWNER_PUBLICATIONS {
        return Err(DurableControlStoreError::PublicationCapacity);
    }
    let source_version =
        i64::try_from(source.version).map_err(|_| DurableControlStoreError::TooLarge)?;
    let exists = transaction.query_row(
        "SELECT 1 FROM context_control_context_sources WHERE run_id=?1 AND source_id=?2 AND version=?3",
        params![run_id, source.source_id, source_version], |row| row.get::<_, i64>(0),
    ).optional().map_err(|_| DurableControlStoreError::Sqlite)?;
    if exists.is_some() {
        return Err(DurableControlStoreError::SourceConflict);
    }
    let aad = source_aad(run_id, &source.source_id, source.version);
    let envelope = super::encrypt_with_key(key, &plaintext, &aad)?;
    let envelope_digest = digest(&envelope);
    transaction
        .execute(
            "INSERT INTO context_control_context_sources
         (run_id, source_id, version, source_digest, envelope, envelope_digest)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                run_id,
                source.source_id,
                source_version,
                source.digest,
                envelope,
                envelope_digest
            ],
        )
        .map_err(|_| DurableControlStoreError::Sqlite)?;
    Ok(())
}

fn validate_receipt(
    owner_id: &str,
    actor: &str,
    request: &ContextOwnerDraftPublicationRequest,
    receipt: &ContextOwnerDraftPublicationReceipt,
    source: &DurableContextSourceSnapshot,
    state_bytes: &[u8],
) -> Result<(), DurableControlStoreError> {
    request
        .validate()
        .map_err(|_| DurableControlStoreError::ScopeMismatch)?;
    let request_digest = request
        .digest()
        .map_err(|_| DurableControlStoreError::ScopeMismatch)?;
    if request.expected_boundary.run_id != receipt.workflow_run_id
        || receipt.owner_id != owner_id
        || receipt.actor_subject != actor
        || receipt.request_id != request.request_id
        || receipt.request_digest != request_digest
        || receipt.draft_id != request.draft_id
        || receipt.draft_version != request.expected_draft_version
        || receipt.base_revision_id != request.expected_base_revision_id
        || receipt.expected_owner_state_version != request.expected_owner_state_version
        || receipt.expected_owner_state_version.checked_add(1)
            != Some(receipt.resulting_owner_state_version)
        || receipt.binding.owner_id != owner_id
        || receipt.binding.workflow_run_id != receipt.workflow_run_id
        || receipt.binding.binding_id != request.expected_binding_id
        || receipt.binding.binding_digest != request.expected_binding_digest
        || receipt.boundary != request.expected_boundary
        || receipt.binding.boundary != receipt.boundary
        || receipt.source_id != source.source_id
        || receipt.source_version != source.version
        || receipt.source_digest != source.digest
        || receipt.expires_at <= receipt.published_at
        || receipt.expires_at == u64::MAX
        || source.document.draft.draft_id != request.draft_id
        || source.document.draft.version != request.expected_draft_version
        || source.document.draft.base_revision_id != request.expected_base_revision_id
        || state_bytes.is_empty()
        || receipt.schema_version != crate::management::CONTEXT_OWNER_PUBLICATION_SCHEMA_VERSION
    {
        return Err(DurableControlStoreError::ScopeMismatch);
    }
    Ok(())
}

fn receipt_aad_values(
    run_id: &str,
    owner_id: &str,
    source_id: &str,
    source_version: u64,
    indices: PublicationIndices,
    request_digest: &str,
    source_digest: &str,
) -> Result<Vec<u8>, DurableControlStoreError> {
    let mut aad = RECEIPT_AAD_DOMAIN.to_vec();
    append_lp(&mut aad, run_id)?;
    append_lp(&mut aad, owner_id)?;
    append_lp(&mut aad, source_id)?;
    aad.extend_from_slice(&source_version.to_be_bytes());
    aad.extend_from_slice(&indices.owner);
    aad.extend_from_slice(&indices.actor);
    aad.extend_from_slice(&indices.request);
    append_digest(&mut aad, request_digest)?;
    append_digest(&mut aad, source_digest)?;
    Ok(aad)
}

fn receipt_aad(
    run_id: &str,
    receipt: &ContextOwnerDraftPublicationReceipt,
    indices: PublicationIndices,
) -> Result<Vec<u8>, DurableControlStoreError> {
    receipt_aad_values(
        run_id,
        &receipt.owner_id,
        &receipt.source_id,
        receipt.source_version,
        indices,
        &receipt.request_digest,
        &receipt.source_digest,
    )
}

struct LinkAad<'a> {
    run_id: &'a str,
    owner_id: &'a str,
    source_id: &'a str,
    source_version: u64,
    source_digest: &'a str,
    indices: PublicationIndices,
    request_digest: &'a str,
    publication_receipt_digest: &'a str,
    control_receipt_digest: &'a str,
    active_revision_id: &'a str,
    activated_at: u64,
}

fn link_aad(input: LinkAad<'_>) -> Result<Vec<u8>, DurableControlStoreError> {
    let LinkAad {
        run_id,
        owner_id,
        source_id,
        source_version,
        source_digest,
        indices,
        request_digest,
        publication_receipt_digest,
        control_receipt_digest,
        active_revision_id,
        activated_at,
    } = input;
    let mut aad = LINK_AAD_DOMAIN.to_vec();
    append_lp(&mut aad, run_id)?;
    append_lp(&mut aad, owner_id)?;
    append_lp(&mut aad, source_id)?;
    aad.extend_from_slice(&source_version.to_be_bytes());
    append_digest(&mut aad, source_digest)?;
    aad.extend_from_slice(&indices.owner);
    aad.extend_from_slice(&indices.actor);
    aad.extend_from_slice(&indices.request);
    append_digest(&mut aad, request_digest)?;
    append_digest(&mut aad, publication_receipt_digest)?;
    append_digest(&mut aad, control_receipt_digest)?;
    append_lp(&mut aad, active_revision_id)?;
    aad.extend_from_slice(&activated_at.to_be_bytes());
    Ok(aad)
}

fn authenticate_transaction_key(
    transaction: &Transaction<'_>,
    key: &[u8; 32],
    run_id: &str,
) -> Result<(), DurableControlStoreError> {
    let row = transaction
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_journal WHERE run_id=?1",
            [run_id],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|_| DurableControlStoreError::Sqlite)?
        .ok_or(DurableControlStoreError::Missing)?;
    if row.0.len() > super::store_types::MAX_JOURNAL_BYTES + 40 || digest(&row.0) != row.1 {
        return Err(DurableControlStoreError::Corrupt);
    }
    super::decrypt_with_key(key, &row.0, super::store_types::AAD).map(|_| ())
}

fn owner_state_aad(run_id: &str, owner_id: &str) -> Result<Vec<u8>, DurableControlStoreError> {
    let mut aad = b"ascension.context-control.owner-state.v1\0".to_vec();
    append_lp(&mut aad, run_id)?;
    append_lp(&mut aad, owner_id)?;
    Ok(aad)
}
