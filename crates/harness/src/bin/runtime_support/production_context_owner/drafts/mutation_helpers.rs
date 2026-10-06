// SPDX-License-Identifier: MIT

struct MutationReceiptInput<'a> {
    actor: &'a AuthContext,
    workflow_run_id: &'a str,
    binding: &'a ContextOwnerBinding,
    operation: &'a str,
    request_id: String,
    payload_digest: String,
    result: ContextOwnerMutationResult,
    now: u64,
}

struct AuthoredItemInput<'a> {
    actor: &'a AuthContext,
    item_id: &'a str,
    kind: &'a str,
    bytes: &'a [u8],
    expires_at: u64,
    source: SourceIdentity,
}

impl Owner {
    fn new_revision(
        &self,
        state: &mut OwnerDraftState,
        actor: &AuthContext,
        binding: &ContextOwnerBinding,
        draft: &ContextDraft,
        now: u64,
        retention_expires_at: Option<u64>,
    ) -> Result<ContextOwnerRevisionEnvelope, ManagementError> {
        if state.revisions.len() >= MAX_REVISIONS {
            return Err(owner_capacity());
        }
        let sequence = state.next_revision;
        state.next_revision = sequence.checked_add(1).ok_or_else(owner_capacity)?;
        Ok(ContextOwnerRevisionEnvelope {
            schema_version: CONTEXT_OWNER_REVISION_SCHEMA_VERSION.to_owned(),
            revision_id: format!("revision.{sequence:016x}"),
            actor_subject: actor.subject.clone(),
            binding: binding.clone(),
            created_at: now,
            retention_expires_at,
            draft: draft.clone(),
        })
    }

    fn new_receipt(&self, input: MutationReceiptInput<'_>) -> ContextOwnerMutationReceipt {
        let MutationReceiptInput {
            actor,
            workflow_run_id,
            binding,
            operation,
            request_id,
            payload_digest,
            result,
            now,
        } = input;
        ContextOwnerMutationReceipt {
            schema_version: CONTEXT_OWNER_MUTATION_RECEIPT_SCHEMA_VERSION.to_owned(),
            owner_id: self.configuration.owner_id.clone(),
            workflow_run_id: workflow_run_id.to_owned(),
            actor_subject: actor.subject.clone(),
            binding_id: binding.binding_id.clone(),
            binding_digest: binding.binding_digest.clone(),
            invocation_id: binding.invocation_id.clone(),
            boundary: binding.boundary.clone(),
            operation: operation.to_owned(),
            request_id,
            payload_digest,
            result,
            created_at: now,
        }
    }

    fn create_authored_item(
        &self,
        state: &mut OwnerDraftState,
        draft: &mut StoredDraft,
        input: AuthoredItemInput<'_>,
    ) -> Result<ContextItemRef, ManagementError> {
        let AuthoredItemInput {
            actor,
            item_id,
            kind,
            bytes,
            expires_at,
            source,
        } = input;
        validate_identifier("context_item_id", item_id)?;
        validate_identifier("context_item_kind", kind)?;
        if bytes.is_empty()
            || bytes.len() > MAX_NOTE_BYTES.max(MAX_OBJECTIVE_BYTES)
            || expires_at == 0
            || state.authored_items.len() >= MAX_AUTHORED_ITEMS
        {
            return Err(owner_capacity());
        }
        let version = state.next_item_version;
        state.next_item_version = version.checked_add(1).ok_or_else(owner_capacity)?;
        let reference = ContextItemRef {
            item_id: item_id.to_owned(),
            version,
            sha256: sts2_harness::sha256_hex(bytes),
        };
        let item = ContextItem {
            reference: reference.clone(),
            kind: kind.to_owned(),
            bytes: bytes.to_vec(),
            protected: false,
            expires_at,
        };
        let key = item_key(&reference);
        state.authored_items.insert(
            key.clone(),
            AuthoredItem {
                item,
                source,
                created_by: actor.subject.clone(),
            },
        );
        if !draft.authored_item_keys.contains(&key) {
            draft.authored_item_keys.push(key);
        }
        Ok(reference)
    }
}
