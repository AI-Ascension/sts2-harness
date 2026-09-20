// SPDX-License-Identifier: MIT

//! In-memory and unavailable inference-profile revision journals.
//!
//! The memory journal is the deterministic component-test implementation of the
//! compare-and-swap contract; the unavailable one refuses every read and write
//! when an owner is composed without a journal.

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::super::contract::{InferenceProfileDescriptor, validate_identifier};
use super::super::store::StoreError;
use super::{
    InferenceProfileRevisionJournal, RevisionAppendOutcome, request_error, unavailable_error,
    unknown, validate_baseline,
};

/// In-memory journal for deterministic component tests.
#[derive(Default)]
pub struct MemoryInferenceProfileRevisionJournal {
    state: Mutex<JournalState>,
}

#[derive(Default)]
struct JournalState {
    /// Accepted revisions per profile, in acceptance order.
    revisions: BTreeMap<String, Vec<InferenceProfileDescriptor>>,
    heads: BTreeMap<String, String>,
    mutations: BTreeMap<(String, String), (String, InferenceProfileDescriptor)>,
}

impl MemoryInferenceProfileRevisionJournal {
    /// A journal whose accepted head for `descriptor.profile_id` is
    /// `descriptor`.
    pub fn seeded(descriptor: InferenceProfileDescriptor) -> Self {
        let journal = Self::default();
        {
            let mut state = journal.lock();
            let profile_id = descriptor.profile_id.clone();
            state
                .heads
                .insert(profile_id.clone(), descriptor.digest.clone());
            state.revisions.insert(profile_id, vec![descriptor]);
        }
        journal
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, JournalState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn head_of(state: &JournalState, profile_id: &str) -> Option<InferenceProfileDescriptor> {
        let digest = state.heads.get(profile_id)?;
        state
            .revisions
            .get(profile_id)?
            .iter()
            .find(|descriptor| &descriptor.digest == digest)
            .cloned()
    }

    /// Records `baseline` as the first accepted revision when none is held.
    fn seed(state: &mut JournalState, baseline: &InferenceProfileDescriptor) {
        let profile_id = baseline.profile_id.clone();
        if state.heads.contains_key(&profile_id) {
            return;
        }
        state
            .heads
            .insert(profile_id.clone(), baseline.digest.clone());
        state.revisions.insert(profile_id, vec![baseline.clone()]);
    }
}

impl InferenceProfileRevisionJournal for MemoryInferenceProfileRevisionJournal {
    fn head(&self, profile_id: &str) -> Result<Option<InferenceProfileDescriptor>, StoreError> {
        validate_identifier("profile_id", profile_id).map_err(request_error)?;
        Ok(Self::head_of(&self.lock(), profile_id))
    }

    fn history(&self, profile_id: &str) -> Result<Vec<InferenceProfileDescriptor>, StoreError> {
        validate_identifier("profile_id", profile_id).map_err(request_error)?;
        Ok(self
            .lock()
            .revisions
            .get(profile_id)
            .cloned()
            .unwrap_or_default())
    }

    fn append(
        &self,
        profile_id: &str,
        baseline: &InferenceProfileDescriptor,
        expected_revision_digest: &str,
        client_mutation_id: &str,
        candidate: &InferenceProfileDescriptor,
    ) -> Result<RevisionAppendOutcome, StoreError> {
        validate_identifier("profile_id", profile_id).map_err(request_error)?;
        validate_identifier("client_mutation_id", client_mutation_id).map_err(request_error)?;
        validate_baseline(profile_id, baseline, candidate)?;
        let mut state = self.lock();
        Self::seed(&mut state, baseline);
        let current = Self::head_of(&state, profile_id).ok_or_else(unknown)?;
        if let Some((digest, recorded)) = state
            .mutations
            .get(&(profile_id.to_owned(), client_mutation_id.to_owned()))
        {
            return Ok(if digest == &candidate.digest {
                RevisionAppendOutcome::Replayed(Box::new(recorded.clone()))
            } else {
                RevisionAppendOutcome::Conflict(Box::new(current))
            });
        }
        if current.digest != expected_revision_digest {
            return Ok(RevisionAppendOutcome::Conflict(Box::new(current)));
        }
        if state
            .revisions
            .get(profile_id)
            .is_some_and(|held| held.iter().any(|item| item.version == candidate.version))
        {
            return Err(StoreError::new(
                "inference_profile_revision_duplicate",
                "the revision journal already holds this profile version",
            ));
        }
        state
            .revisions
            .entry(profile_id.to_owned())
            .or_default()
            .push(candidate.clone());
        state
            .heads
            .insert(profile_id.to_owned(), candidate.digest.clone());
        state.mutations.insert(
            (profile_id.to_owned(), client_mutation_id.to_owned()),
            (candidate.digest.clone(), candidate.clone()),
        );
        Ok(RevisionAppendOutcome::Adopted(Box::new(candidate.clone())))
    }
}

/// Refuses every read and write when no journal is attached to this owner.
pub struct UnavailableInferenceProfileRevisionJournal;

impl InferenceProfileRevisionJournal for UnavailableInferenceProfileRevisionJournal {
    fn head(&self, _profile_id: &str) -> Result<Option<InferenceProfileDescriptor>, StoreError> {
        Err(unavailable_error())
    }

    fn history(&self, _profile_id: &str) -> Result<Vec<InferenceProfileDescriptor>, StoreError> {
        Err(unavailable_error())
    }

    fn append(
        &self,
        _profile_id: &str,
        _baseline: &InferenceProfileDescriptor,
        _expected_revision_digest: &str,
        _client_mutation_id: &str,
        _candidate: &InferenceProfileDescriptor,
    ) -> Result<RevisionAppendOutcome, StoreError> {
        Err(unavailable_error())
    }
}
