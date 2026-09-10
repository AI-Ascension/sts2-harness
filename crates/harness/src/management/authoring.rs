// SPDX-License-Identifier: MIT

//! Harness-owned Studio authoring persistence.
//!
//! The authoring store contains semantic candidates and inert layout only. It
//! never reads or mutates workflow runtime tables.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::Value;

use super::contract_authoring::{StudioDefinitionRecord, StudioDraftRecord};
use super::store::StoreError;

#[path = "authoring_sqlite.rs"]
mod sqlite;
#[path = "authoring_sqlite_support.rs"]
mod sqlite_support;
#[path = "authoring_validation.rs"]
mod validation;

use validation::{
    conflict_draft, draft_record, mutation_digest, validate_draft, validate_id, validate_payload,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublishResult {
    Published(StudioDefinitionRecord),
    AlreadyPublished(StudioDefinitionRecord),
    Conflict(StudioDraftRecord),
}

pub trait AuthoringStore: Send + Sync {
    fn list_definitions(&self) -> Result<Vec<StudioDefinitionRecord>, StoreError>;

    fn get_draft(&self, draft_id: &str) -> Result<Option<StudioDraftRecord>, StoreError>;

    fn create_draft(
        &self,
        draft: StudioDraftRecord,
        mutation_id: &str,
    ) -> Result<StudioDraftRecord, StoreError>;

    fn save_draft(
        &self,
        draft_id: &str,
        expected_revision: u64,
        expected_etag: &str,
        mutation_id: &str,
        document: Value,
        layout: Value,
    ) -> Result<StudioDraftRecord, StoreError>;

    fn publish_draft(
        &self,
        draft_id: &str,
        expected_revision: u64,
        expected_etag: &str,
        expected_definition_digest: &str,
    ) -> Result<PublishResult, StoreError>;
}

#[derive(Default)]
pub struct MemoryAuthoringStore {
    state: Mutex<AuthoringState>,
}

#[derive(Default)]
struct AuthoringState {
    drafts: BTreeMap<String, StudioDraftRecord>,
    definitions: BTreeMap<String, StudioDefinitionRecord>,
    mutations: BTreeMap<(String, String), MutationRecord>,
}

#[derive(Clone)]
struct MutationRecord {
    digest: String,
    draft: StudioDraftRecord,
}

impl MemoryAuthoringStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, AuthoringState>, StoreError> {
        self.state.lock().map_err(|_| {
            StoreError::new(
                "authoring_store_poisoned",
                "authoring store lock is poisoned",
            )
        })
    }
}

impl AuthoringStore for MemoryAuthoringStore {
    fn list_definitions(&self) -> Result<Vec<StudioDefinitionRecord>, StoreError> {
        Ok(self.lock()?.definitions.values().cloned().collect())
    }

    fn get_draft(&self, draft_id: &str) -> Result<Option<StudioDraftRecord>, StoreError> {
        validate_id("draft_id", draft_id)?;
        Ok(self.lock()?.drafts.get(draft_id).cloned())
    }

    fn create_draft(
        &self,
        draft: StudioDraftRecord,
        mutation_id: &str,
    ) -> Result<StudioDraftRecord, StoreError> {
        validate_draft(&draft)?;
        validate_id("client_mutation_id", mutation_id)?;
        let digest = mutation_digest(&draft.document, &draft.layout)?;
        let mut state = self.lock()?;
        let mutation_key = (draft.draft_id.clone(), mutation_id.to_owned());
        if let Some(existing) = state.drafts.get(&draft.draft_id).cloned() {
            if state
                .mutations
                .get(&mutation_key)
                .is_some_and(|previous| previous.digest == digest)
            {
                return Ok(state
                    .mutations
                    .get(&mutation_key)
                    .map_or(existing, |previous| previous.draft.clone()));
            }
            return Ok(conflict_draft(existing));
        }
        state.drafts.insert(draft.draft_id.clone(), draft.clone());
        state.mutations.insert(
            mutation_key,
            MutationRecord {
                digest,
                draft: draft.clone(),
            },
        );
        Ok(draft)
    }

    fn save_draft(
        &self,
        draft_id: &str,
        expected_revision: u64,
        expected_etag: &str,
        mutation_id: &str,
        document: Value,
        layout: Value,
    ) -> Result<StudioDraftRecord, StoreError> {
        validate_id("draft_id", draft_id)?;
        validate_id("client_mutation_id", mutation_id)?;
        validate_payload(&document, &layout)?;
        let digest = mutation_digest(&document, &layout)?;
        let mut state = self.lock()?;
        let mutation_key = (draft_id.to_owned(), mutation_id.to_owned());
        let current = state
            .drafts
            .get(draft_id)
            .cloned()
            .ok_or_else(|| StoreError::new("draft_not_found", "Studio draft was not found"))?;
        if let Some(previous) = state.mutations.get(&mutation_key) {
            if previous.digest == digest {
                return Ok(previous.draft.clone());
            }
            return Ok(conflict_draft(current));
        }
        if current.revision != expected_revision || current.etag != expected_etag {
            return Ok(conflict_draft(current));
        }
        let revision = current.revision.checked_add(1).ok_or_else(|| {
            StoreError::new("draft_revision_overflow", "draft revision overflowed")
        })?;
        let next = draft_record(
            &current.draft_id,
            &current.definition_id,
            revision,
            document,
            layout,
        )?;
        state.drafts.insert(draft_id.to_owned(), next.clone());
        state.mutations.insert(
            mutation_key,
            MutationRecord {
                digest,
                draft: next.clone(),
            },
        );
        Ok(next)
    }

    fn publish_draft(
        &self,
        draft_id: &str,
        expected_revision: u64,
        expected_etag: &str,
        expected_definition_digest: &str,
    ) -> Result<PublishResult, StoreError> {
        validate_id("draft_id", draft_id)?;
        validation::validate_digest_text(expected_definition_digest)?;
        let mut state = self.lock()?;
        let current = state
            .drafts
            .get(draft_id)
            .cloned()
            .ok_or_else(|| StoreError::new("draft_not_found", "Studio draft was not found"))?;
        if current.revision != expected_revision || current.etag != expected_etag {
            return Ok(PublishResult::Conflict(conflict_draft(current)));
        }
        let digest = validation::definition_digest(&current.document)?;
        if digest != expected_definition_digest {
            return Ok(PublishResult::Conflict(conflict_draft(current)));
        }
        if let Some(existing) = state.definitions.get(&digest).cloned() {
            return Ok(PublishResult::AlreadyPublished(existing));
        }
        let definition = validation::published_definition(&current, &digest)?;
        state.definitions.insert(digest, definition.clone());
        Ok(PublishResult::Published(definition))
    }
}

pub(crate) fn new_draft(
    draft_id: &str,
    definition_id: &str,
    document: Value,
    layout: Value,
) -> Result<StudioDraftRecord, StoreError> {
    draft_record(draft_id, definition_id, 0, document, layout)
}

pub(crate) fn with_conflict(current: StudioDraftRecord) -> StudioDraftRecord {
    conflict_draft(current)
}
