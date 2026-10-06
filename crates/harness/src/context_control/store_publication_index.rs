// SPDX-License-Identifier: MIT

// Owner-scoped publication indexes and binary framing shared by encrypted records.

impl ContextControlStore {
    pub fn draft_publication_source_id(
        &self,
        owner_id: &str,
        actor_subject: &str,
        request: &ContextOwnerDraftPublicationRequest,
    ) -> Result<String, DurableControlStoreError> {
        self.draft_publication_identity(owner_id, actor_subject, request)
            .map(|(source_id, _)| source_id)
    }

    /// Returns the deterministic owner-issued source identity and the exact framed request
    /// digest without exposing the store key or keyed indexes.
    pub fn draft_publication_identity(
        &self,
        owner_id: &str,
        actor_subject: &str,
        request: &ContextOwnerDraftPublicationRequest,
    ) -> Result<(String, String), DurableControlStoreError> {
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
        let source_id = publication_source_id(publication_indices(
            &self.key,
            &self.run_id,
            owner_id,
            actor_subject,
            request,
        ));
        Ok((source_id, request_digest))
    }
}

fn publication_indices(
    key: &[u8; 32],
    run_id: &str,
    owner_id: &str,
    actor: &str,
    request: &ContextOwnerDraftPublicationRequest,
) -> PublicationIndices {
    let request_id = request.request_id.as_str();
    let owner = index_digest(key, OWNER_INDEX_DOMAIN, &[run_id, owner_id]);
    let actor_index = index_digest(key, ACTOR_INDEX_DOMAIN, &[run_id, owner_id, actor]);
    let request_index = index_digest(
        key,
        REQUEST_INDEX_DOMAIN,
        &[run_id, owner_id, actor, request_id],
    );
    let source = index_digest(
        key,
        SOURCE_ID_DOMAIN,
        &[run_id, owner_id, actor, request_id],
    );
    PublicationIndices {
        owner,
        actor: actor_index,
        request: request_index,
        source,
    }
}

fn index_digest(key: &[u8; 32], domain: &[u8], fields: &[&str]) -> [u8; 32] {
    let subkey = hmac_sha256(key, INDEX_KEY_DOMAIN, b"");
    hmac_sha256(&subkey, domain, &frame(fields))
}

fn publication_source_id(indices: PublicationIndices) -> String {
    format!("ownerpub.{}", hex(&indices.source))
}

fn validate_source_id(
    source_id: &str,
    version: u64,
    source_digest: &str,
) -> Result<(), DurableControlStoreError> {
    if !source_id.starts_with("ownerpub.")
        || source_id.len() != 73
        || version != 1
        || source_id[9..].bytes().any(|byte| digit(byte).is_none())
    {
        return Err(DurableControlStoreError::InvalidSourceId);
    }
    validate_digest(source_digest)
}

fn validate_identity(value: &str) -> Result<(), DurableControlStoreError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_alphanumeric() || (i > 0 && b"._:-".contains(&b)))
    {
        return Err(DurableControlStoreError::ScopeMismatch);
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), DurableControlStoreError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(DurableControlStoreError::Corrupt);
    }
    Ok(())
}

fn append_lp(frame: &mut Vec<u8>, value: &str) -> Result<(), DurableControlStoreError> {
    let bytes = value.as_bytes();
    frame.extend_from_slice(
        &u64::try_from(bytes.len())
            .map_err(|_| DurableControlStoreError::TooLarge)?
            .to_be_bytes(),
    );
    frame.extend_from_slice(bytes);
    Ok(())
}

fn append_digest(frame: &mut Vec<u8>, value: &str) -> Result<(), DurableControlStoreError> {
    frame.extend_from_slice(&unhex32(value)?);
    Ok(())
}

fn frame(fields: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for field in fields {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field.as_bytes());
    }
    bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex32(value: &str) -> Result<[u8; 32], DurableControlStoreError> {
    validate_digest(value)?;
    let mut out = [0; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = digit(pair[0]).ok_or(DurableControlStoreError::Corrupt)?;
        let low = digit(pair[1]).ok_or(DurableControlStoreError::Corrupt)?;
        out[index] = high << 4 | low;
    }
    Ok(out)
}

fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
