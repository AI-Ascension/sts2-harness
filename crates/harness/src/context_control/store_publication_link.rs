// SPDX-License-Identifier: MIT

// Atomically persist authenticated active-publication links.

pub(super) struct ActivePublicationLinkWrite<'a> {
    pub(super) run_id: &'a str,
    pub(super) key: &'a [u8; 32],
    pub(super) source: &'a DurableActiveContextSource,
    pub(super) publication: &'a DurableContextOwnerPublication,
    pub(super) control_receipt_envelope_digest: &'a str,
    pub(super) active_revision_id: &'a str,
    pub(super) activated_at: u64,
}

pub(super) fn persist_active_publication_link(
    transaction: &Transaction<'_>,
    write: ActivePublicationLinkWrite<'_>,
) -> Result<(), DurableControlStoreError> {
    let ActivePublicationLinkWrite {
        run_id,
        key,
        source,
        publication,
        control_receipt_envelope_digest,
        active_revision_id,
        activated_at,
    } = write;
    validate_source_id(&source.source_id, source.version, &source.digest)?;
    validate_digest(control_receipt_envelope_digest)?;
    let source_version =
        i64::try_from(source.version).map_err(|_| DurableControlStoreError::TooLarge)?;
    let activated_at_i64 =
        i64::try_from(activated_at).map_err(|_| DurableControlStoreError::TooLarge)?;
    if source.active_revision_id != active_revision_id
        || source.source_id != publication.receipt.source_id
        || source.version != publication.receipt.source_version
        || source.digest != publication.receipt.source_digest
    {
        return Err(DurableControlStoreError::ActivePublicationConflict);
    }
    let stored = transaction
        .query_row(
            "SELECT owner_index_digest, actor_index_digest, request_index_digest, request_digest,
                    source_digest, receipt_envelope_digest
             FROM context_control_owner_publications
             WHERE run_id = ?1 AND source_id = ?2 AND source_version = ?3",
            params![run_id, source.source_id, source_version],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|_| DurableControlStoreError::Sqlite)?
        .ok_or(DurableControlStoreError::PublicationMissing)?;
    if stored.0 != publication.owner_index_digest
        || stored.1 != publication.actor_index_digest
        || stored.2 != publication.request_index_digest
        || stored.3 != publication.receipt.request_digest
        || stored.4 != source.digest
        || stored.5 != publication.receipt_envelope_digest
    {
        return Err(DurableControlStoreError::ActivePublicationConflict);
    }
    let indices = PublicationIndices {
        owner: unhex32(&publication.owner_index_digest)?,
        actor: unhex32(&publication.actor_index_digest)?,
        request: unhex32(&publication.request_index_digest)?,
        source: [0; 32],
    };
    let link = DurableActiveContextPublicationLink {
        source_id: source.source_id.clone(),
        source_version: source.version,
        source_digest: source.digest.clone(),
        owner_index_digest: publication.owner_index_digest.clone(),
        actor_index_digest: publication.actor_index_digest.clone(),
        request_index_digest: publication.request_index_digest.clone(),
        request_digest: publication.receipt.request_digest.clone(),
        publication_receipt_envelope_digest: publication.receipt_envelope_digest.clone(),
        control_receipt_envelope_digest: control_receipt_envelope_digest.to_owned(),
        active_revision_id: active_revision_id.to_owned(),
        activated_at,
    };
    let aad = link_aad(LinkAad {
        run_id,
        owner_id: &publication.receipt.owner_id,
        source_id: &source.source_id,
        source_version: source.version,
        source_digest: &source.digest,
        indices,
        request_digest: &publication.receipt.request_digest,
        publication_receipt_digest: &publication.receipt_envelope_digest,
        control_receipt_digest: control_receipt_envelope_digest,
        active_revision_id,
        activated_at,
    })?;
    let plaintext = serde_json::to_vec(&link).map_err(|_| DurableControlStoreError::Encode)?;
    if plaintext.len() > MAX_OWNER_RECEIPT_BYTES {
        return Err(DurableControlStoreError::TooLarge);
    }
    let envelope = super::encrypt_with_key(key, &plaintext, &aad)?;
    let envelope_digest = digest(&envelope);
    transaction.execute(
        "INSERT INTO context_control_active_publication_links
         (run_id, source_id, source_version, source_digest, owner_index_digest, actor_index_digest,
          request_index_digest, request_digest, publication_receipt_envelope_digest,
          control_receipt_envelope_digest, active_revision_id, activated_at, envelope, envelope_digest)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(run_id) DO UPDATE SET source_id=excluded.source_id, source_version=excluded.source_version,
          source_digest=excluded.source_digest, owner_index_digest=excluded.owner_index_digest,
          actor_index_digest=excluded.actor_index_digest, request_index_digest=excluded.request_index_digest,
          request_digest=excluded.request_digest, publication_receipt_envelope_digest=excluded.publication_receipt_envelope_digest,
          control_receipt_envelope_digest=excluded.control_receipt_envelope_digest,
          active_revision_id=excluded.active_revision_id, activated_at=excluded.activated_at,
          envelope=excluded.envelope, envelope_digest=excluded.envelope_digest",
        params![run_id, source.source_id, source_version, source.digest,
            publication.owner_index_digest, publication.actor_index_digest, publication.request_index_digest,
            publication.receipt.request_digest, publication.receipt_envelope_digest,
            control_receipt_envelope_digest, active_revision_id, activated_at_i64, envelope, envelope_digest],
    ).map_err(|_| DurableControlStoreError::Sqlite)?;
    Ok(())
}
