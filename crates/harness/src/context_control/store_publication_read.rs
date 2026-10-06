// SPDX-License-Identifier: MIT

// Read and verify owner-publication metadata and activation links.

impl ContextControlStore {
    pub fn list_draft_publications(
        &self,
        owner_id: &str,
        actor_subject: &str,
    ) -> Result<Vec<DurableContextOwnerPublication>, DurableControlStoreError> {
        validate_identity(owner_id)?;
        validate_identity(actor_subject)?;
        self.authenticate_publication_key()?;
        self.verify_connection_owner()?;
        let owner = index_digest(&self.key, OWNER_INDEX_DOMAIN, &[&self.run_id, owner_id]);
        let actor = index_digest(
            &self.key,
            ACTOR_INDEX_DOMAIN,
            &[&self.run_id, owner_id, actor_subject],
        );
        let mut statement = self.connection.prepare(
            "SELECT source_id, source_version, request_digest, source_digest, receipt_envelope,
                    receipt_envelope_digest, owner_index_digest, actor_index_digest, request_index_digest
             FROM context_control_owner_publications WHERE run_id = ?1 AND owner_index_digest = ?2
               AND actor_index_digest = ?3 ORDER BY published_at DESC, source_id LIMIT 17",
        ).map_err(|_| DurableControlStoreError::Sqlite)?;
        let rows = statement
            .query_map(params![self.run_id, hex(&owner), hex(&actor)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let mut publications = Vec::new();
        for row in rows {
            let (
                source_id,
                source_version,
                request_digest,
                source_digest,
                envelope,
                envelope_digest,
                owner_hex,
                actor_hex,
                request_hex,
            ) = row.map_err(|_| DurableControlStoreError::Sqlite)?;
            if publications.len() == MAX_OWNER_PUBLICATIONS {
                return Err(DurableControlStoreError::TooLarge);
            }
            let indices = PublicationIndices {
                owner: unhex32(&owner_hex)?,
                actor: unhex32(&actor_hex)?,
                request: unhex32(&request_hex)?,
                source: [0; 32],
            };
            publications.push(decode_publication(
                &self.key,
                &self.run_id,
                owner_id,
                actor_subject,
                indices,
                (
                    source_id,
                    source_version,
                    request_digest,
                    source_digest,
                    envelope,
                    envelope_digest,
                ),
                None,
            )?);
        }
        Ok(publications)
    }

    pub fn load_publication_source(
        &self,
        owner_id: &str,
        actor_subject: &str,
        source_id: &str,
        version: u64,
        source_digest: &str,
    ) -> Result<
        Option<(DurableContextOwnerPublication, DurableContextSourceSnapshot)>,
        DurableControlStoreError,
    > {
        let Some(publication) = self
            .list_draft_publications(owner_id, actor_subject)?
            .into_iter()
            .find(|value| {
                value.receipt.source_id == source_id
                    && value.receipt.source_version == version
                    && value.receipt.source_digest == source_digest
            })
        else {
            return Ok(None);
        };
        let source = self.load_context_source(source_id, version, source_digest)?;
        Ok(source.map(|source| (publication, source)))
    }

    pub fn load_active_publication_link(
        &self,
        owner_id: &str,
        actor_subject: &str,
        source_id: &str,
        version: u64,
        source_digest: &str,
        active_revision_id: &str,
    ) -> Result<Option<DurableActiveContextPublicationLink>, DurableControlStoreError> {
        validate_identity(owner_id)?;
        validate_identity(actor_subject)?;
        self.authenticate_publication_key()?;
        self.verify_connection_owner()?;
        let owner = hex(&index_digest(
            &self.key,
            OWNER_INDEX_DOMAIN,
            &[&self.run_id, owner_id],
        ));
        let actor = hex(&index_digest(
            &self.key,
            ACTOR_INDEX_DOMAIN,
            &[&self.run_id, owner_id, actor_subject],
        ));
        let row = self.connection.query_row(
            "SELECT source_id, source_version, source_digest, owner_index_digest, actor_index_digest,
                    request_index_digest, request_digest, publication_receipt_envelope_digest,
                    control_receipt_envelope_digest, active_revision_id, activated_at, envelope,
                    envelope_digest
             FROM context_control_active_publication_links WHERE run_id = ?1",
            [self.run_id.as_str()],
            |row| Ok((
                row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?,
                row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
                row.get::<_, String>(6)?, row.get::<_, String>(7)?, row.get::<_, String>(8)?,
                row.get::<_, String>(9)?, row.get::<_, i64>(10)?, row.get::<_, Vec<u8>>(11)?,
                row.get::<_, String>(12)?,
            )),
        ).optional().map_err(|_| DurableControlStoreError::Sqlite)?;
        let Some((
            stored_source_id,
            stored_version,
            stored_source_digest,
            stored_owner,
            stored_actor,
            request_index,
            request_digest,
            publication_digest,
            control_digest,
            revision,
            activated_at,
            envelope,
            envelope_digest,
        )) = row
        else {
            return Ok(None);
        };
        let source_version_i64 =
            i64::try_from(version).map_err(|_| DurableControlStoreError::Corrupt)?;
        if stored_source_id != source_id
            || stored_version != source_version_i64
            || stored_source_digest != source_digest
            || stored_owner != owner
            || stored_actor != actor
            || revision != active_revision_id
        {
            return Err(DurableControlStoreError::ActivePublicationConflict);
        }
        if digest(&envelope) != envelope_digest || envelope.len() > MAX_OWNER_RECEIPT_BYTES + 40 {
            return Err(DurableControlStoreError::Corrupt);
        }
        let indices = PublicationIndices {
            owner: unhex32(&owner)?,
            actor: unhex32(&actor)?,
            request: unhex32(&request_index)?,
            source: [0; 32],
        };
        let aad = link_aad(LinkAad {
            run_id: &self.run_id,
            owner_id,
            source_id,
            source_version: version,
            source_digest,
            indices,
            request_digest: &request_digest,
            publication_receipt_digest: &publication_digest,
            control_receipt_digest: &control_digest,
            active_revision_id,
            activated_at: u64::try_from(activated_at)
                .map_err(|_| DurableControlStoreError::Corrupt)?,
        })?;
        let plaintext = self.decrypt_with_aad(&envelope, &aad)?;
        let link: DurableActiveContextPublicationLink =
            serde_json::from_slice(&plaintext).map_err(|_| DurableControlStoreError::Decode)?;
        if link.source_id != source_id
            || link.source_version != version
            || link.source_digest != source_digest
            || link.active_revision_id != active_revision_id
            || link.request_digest != request_digest
            || link.activated_at as i64 != activated_at
            || link.control_receipt_envelope_digest != control_digest
            || link.owner_index_digest != owner
            || link.actor_index_digest != actor
            || link.request_index_digest != request_index
            || link.publication_receipt_envelope_digest != publication_digest
        {
            return Err(DurableControlStoreError::Corrupt);
        }
        let publication_row = self.connection.query_row(
            "SELECT source_digest, request_digest, receipt_envelope, receipt_envelope_digest
             FROM context_control_owner_publications
             WHERE run_id = ?1 AND source_id = ?2 AND source_version = ?3
               AND owner_index_digest = ?4 AND actor_index_digest = ?5 AND request_index_digest = ?6",
            params![self.run_id, source_id, source_version_i64, owner, actor, request_index],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?, row.get::<_, String>(3)?)),
        ).optional().map_err(|_| DurableControlStoreError::Sqlite)?
            .ok_or(DurableControlStoreError::Corrupt)?;
        if publication_row.0 != source_digest
            || publication_row.1 != request_digest
            || publication_row.3 != publication_digest
            || digest(&publication_row.2) != publication_digest
        {
            return Err(DurableControlStoreError::Corrupt);
        }
        let control_row = self
            .connection
            .query_row(
                "SELECT envelope FROM context_control_owner_receipts
             WHERE run_id = ?1 AND owner_id = ?2 AND envelope_digest = ?3",
                params![self.run_id, owner_id, control_digest],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?
            .ok_or(DurableControlStoreError::Corrupt)?;
        if digest(&control_row) != control_digest {
            return Err(DurableControlStoreError::Corrupt);
        }
        Ok(Some(link))
    }

    fn authenticate_publication_key(&self) -> Result<(), DurableControlStoreError> {
        let row = self
            .connection
            .query_row(
                "SELECT envelope, envelope_digest FROM context_control_journal WHERE run_id = ?1",
                [self.run_id.as_str()],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?
            .ok_or(DurableControlStoreError::Missing)?;
        if row.0.len() > super::store_types::MAX_JOURNAL_BYTES + 40 || digest(&row.0) != row.1 {
            return Err(DurableControlStoreError::Corrupt);
        }
        super::decrypt_with_key(&self.key, &row.0, super::store_types::AAD).map(|_| ())
    }
}

fn validate_lookup(
    owner_id: &str,
    actor: &str,
    lookup: &ContextOwnerDraftPublicationLookupRequest,
) -> Result<(), DurableControlStoreError> {
    validate_identity(owner_id)?;
    validate_identity(actor)?;
    if lookup.schema_version != crate::management::CONTEXT_OWNER_PUBLICATION_LOOKUP_SCHEMA_VERSION
        || lookup.request.expected_boundary.run_id.is_empty()
    {
        return Err(DurableControlStoreError::ScopeMismatch);
    }
    lookup
        .request
        .digest()
        .map(|_| ())
        .map_err(|_| DurableControlStoreError::ScopeMismatch)
}
