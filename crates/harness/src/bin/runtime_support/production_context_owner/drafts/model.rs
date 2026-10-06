// SPDX-License-Identifier: MIT

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerDraftState {
    schema_version: String,
    owner_id: String,
    workflow_run_id: String,
    next_revision: u64,
    next_preview: u64,
    next_item_version: u64,
    drafts: BTreeMap<String, StoredDraft>,
    revisions: BTreeMap<String, ContextOwnerRevisionEnvelope>,
    previews: BTreeMap<String, ContextOwnerPreviewEnvelope>,
    receipts: BTreeMap<String, BTreeMap<String, StoredReceipt>>,
    authored_items: BTreeMap<String, AuthoredItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDraft {
    envelope: ContextOwnerDraftEnvelope,
    base_source: Option<SourceIdentity>,
    authored_item_keys: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceIdentity {
    source_id: String,
    version: u64,
    digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredItem {
    item: ContextItem,
    source: SourceIdentity,
    created_by: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReceipt {
    payload_digest: String,
    receipt: ContextOwnerMutationReceipt,
}

#[derive(Clone)]
struct EligibleItem {
    item: ContextItem,
    source: SourceIdentity,
}

impl OwnerDraftState {
    fn empty(owner_id: &str, workflow_run_id: &str) -> Self {
        Self {
            schema_version: OWNER_STATE_SCHEMA.to_owned(),
            owner_id: owner_id.to_owned(),
            workflow_run_id: workflow_run_id.to_owned(),
            next_revision: 1,
            next_preview: 1,
            next_item_version: 1,
            drafts: BTreeMap::new(),
            revisions: BTreeMap::new(),
            previews: BTreeMap::new(),
            receipts: BTreeMap::new(),
            authored_items: BTreeMap::new(),
        }
    }

    fn validate(&self, owner_id: &str, workflow_run_id: &str) -> Result<(), ManagementError> {
        if self.schema_version != OWNER_STATE_SCHEMA
            || self.owner_id != owner_id
            || self.workflow_run_id != workflow_run_id
            || self.drafts.len() > MAX_DRAFTS
            || self.revisions.len() > MAX_REVISIONS
            || self.previews.len() > MAX_PREVIEWS
            || self.authored_items.len() > MAX_AUTHORED_ITEMS
            || self
                .receipts
                .values()
                .any(|actor_receipts| actor_receipts.len() > MAX_RECEIPTS_PER_ACTOR)
        {
            return Err(state_corrupt());
        }
        for (key, stored) in &self.drafts {
            let envelope = &stored.envelope;
            if key != &envelope.draft.draft_id
                || envelope.schema_version != CONTEXT_OWNER_DRAFT_SCHEMA_VERSION
                || envelope.actor_subject.is_empty()
                || envelope.binding.owner_id != owner_id
                || envelope.binding.workflow_run_id != workflow_run_id
                || envelope.draft.schema != sts2_harness::context_control::CONTEXT_DRAFT_SCHEMA
                || envelope.draft.version == 0
                || stored.authored_item_keys.len() > MAX_AUTHORED_ITEMS
            {
                return Err(state_corrupt());
            }
            for authored_key in &stored.authored_item_keys {
                let authored = self
                    .authored_items
                    .get(authored_key)
                    .ok_or_else(state_corrupt)?;
                if authored.created_by != envelope.actor_subject
                    || stored.base_source.as_ref() != Some(&authored.source)
                {
                    return Err(state_corrupt());
                }
            }
        }
        for (key, revision) in &self.revisions {
            if key != &revision.revision_id
                || revision.schema_version != CONTEXT_OWNER_REVISION_SCHEMA_VERSION
                || revision.actor_subject.is_empty()
                || revision.binding.owner_id != owner_id
                || revision.binding.workflow_run_id != workflow_run_id
                || revision.draft.schema != sts2_harness::context_control::CONTEXT_DRAFT_SCHEMA
                || revision.draft.version == 0
            {
                return Err(state_corrupt());
            }
        }
        for (key, preview) in &self.previews {
            if key != &preview.preview_id
                || preview.schema_version != CONTEXT_OWNER_PREVIEW_SCHEMA_VERSION
                || preview.actor_subject.is_empty()
                || preview.binding.owner_id != owner_id
                || preview.binding.workflow_run_id != workflow_run_id
                || preview.expires_at <= preview.created_at
            {
                return Err(state_corrupt());
            }
        }
        for (actor, receipts) in &self.receipts {
            if actor.is_empty() {
                return Err(state_corrupt());
            }
            for (request_id, stored) in receipts {
                let receipt = &stored.receipt;
                if request_id != &receipt.request_id
                    || receipt.schema_version != CONTEXT_OWNER_MUTATION_RECEIPT_SCHEMA_VERSION
                    || receipt.actor_subject != *actor
                    || receipt.owner_id != owner_id
                    || receipt.workflow_run_id != workflow_run_id
                    || receipt.payload_digest != stored.payload_digest
                {
                    return Err(state_corrupt());
                }
            }
        }
        for (key, authored) in &self.authored_items {
            if key != &item_key(&authored.item.reference)
                || !authored.item.reference.valid()
                || authored.item.bytes.is_empty()
                || sts2_harness::sha256_hex(&authored.item.bytes) != authored.item.reference.sha256
                || authored.created_by.is_empty()
                || authored.item.expires_at == 0
            {
                return Err(state_corrupt());
            }
        }
        Ok(())
    }

    fn encode(&self) -> Result<Vec<u8>, ManagementError> {
        let bytes = serde_json::to_vec(self).map_err(|error| {
            ManagementError::unavailable("context_owner_state_encode", error.to_string())
        })?;
        if bytes.is_empty()
            || bytes.len() > sts2_harness::context_control::MAX_OWNER_CONTEXT_STATE_BYTES
        {
            return Err(ManagementError::unavailable(
                "context_owner_state_too_large",
                "owner draft history reached its bounded durable capacity",
            ));
        }
        Ok(bytes)
    }
}
