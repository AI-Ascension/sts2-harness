// SPDX-License-Identifier: MIT

use super::*;
use std::sync::MutexGuard;
use sts2_harness::context_memory::{
    MemoryCorpus,
    policy_owner::{LookupPolicyAuthorityGuard, LookupPolicySnapshot},
};
use sts2_harness::game_information::LookupError;

impl RuntimeGameInformationOwner {
    pub(in crate::runtime_support::runtime_v3) fn management_lookup_snapshot(
        &self,
        bearer: Option<&str>,
        expected: Option<&ActivePolicyBinding>,
    ) -> Result<LookupPolicySnapshot, PolicyOwnerError> {
        let snapshot = self.owner.lookup_snapshot(
            self.management_access(bearer, &self.selector_grant_id),
            expected,
        )?;
        let store = self
            .corpus_store
            .lock()
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        let persisted = store.load_corpus()?;
        if !same_corpus_snapshot(&persisted, &snapshot.corpus) {
            return Err(PolicyOwnerError::OwnerFenced);
        }
        Ok(snapshot)
    }

    pub(in crate::runtime_support::runtime_v3) fn lookup_snapshot(
        &self,
        expected: Option<&ActivePolicyBinding>,
    ) -> Result<LookupPolicySnapshot, PolicyOwnerError> {
        let store = self
            .corpus_store
            .lock()
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        let persisted = store.load_corpus()?;
        let snapshot = self
            .owner
            .lookup_snapshot(self.access(&self.selector_grant_id), expected)?;
        if !same_corpus_snapshot(&persisted, &snapshot.corpus) {
            return Err(PolicyOwnerError::OwnerFenced);
        }
        Ok(snapshot)
    }

    pub(in crate::runtime_support::runtime_v3) fn lock_lookup_snapshot(
        &self,
        expected: &ActivePolicyBinding,
    ) -> Result<RuntimeLookupAuthorityGuard<'_>, PolicyOwnerError> {
        let corpus_store = self
            .corpus_store
            .lock()
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        let persisted = corpus_store.load_corpus()?;
        let policy = self
            .owner
            .lock_lookup_snapshot(self.access(&self.selector_grant_id), expected)?;
        if !same_corpus_snapshot(&persisted, &policy.snapshot().corpus) {
            return Err(PolicyOwnerError::OwnerFenced);
        }
        Ok(RuntimeLookupAuthorityGuard {
            // The policy journal and authority locks are released before the corpus lock.
            policy,
            _corpus_store: corpus_store,
        })
    }

    /// Authorizes one bounded external call at admission, releases every owner/store lock while
    /// it runs, then discards its result if current authority changed before return.
    pub(in crate::runtime_support::runtime_v3) fn call_with_lookup_revalidation<T>(
        &self,
        expected: &ActivePolicyBinding,
        call: impl FnOnce() -> Result<T, LookupError>,
    ) -> Result<T, LookupError> {
        self.lookup_snapshot(Some(expected))
            .map_err(|_| LookupError::Scope)?;
        let result = call();
        let postflight = self.lookup_snapshot(Some(expected));
        match (result, postflight) {
            (Ok(value), Ok(_)) => Ok(value),
            (Err(error), Ok(_)) => Err(error),
            (_, Err(_)) => Err(LookupError::Scope),
        }
    }
}

/// Holds the selected-policy and corpus-store leases through one short local validation or write.
pub(in crate::runtime_support::runtime_v3) struct RuntimeLookupAuthorityGuard<'a> {
    policy: LookupPolicyAuthorityGuard<'a>,
    _corpus_store: MutexGuard<'a, DurableMemoryStore>,
}

impl RuntimeLookupAuthorityGuard<'_> {
    pub(in crate::runtime_support::runtime_v3) fn snapshot(&self) -> &LookupPolicySnapshot {
        self.policy.snapshot()
    }
}

fn same_corpus_snapshot(left: &MemoryCorpus, right: &MemoryCorpus) -> bool {
    fn encoded(corpus: &MemoryCorpus) -> Option<Vec<u8>> {
        let entries = corpus
            .entries()
            .map(|entry| (entry, entry.is_protected()))
            .collect::<Vec<_>>();
        serde_json::to_vec(&(
            corpus.scope(),
            corpus.generation(),
            corpus.projection_generation(),
            corpus.revocation_epoch(),
            corpus.enabled(),
            entries,
        ))
        .ok()
    }

    encoded(left)
        .zip(encoded(right))
        .is_some_and(|(left, right)| left == right)
}
