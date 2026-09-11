// SPDX-License-Identifier: MIT

// Serialized corpus access for race tests.  The lock is the only coordination primitive here;
// it does not start workers, providers or gameplay actions.

use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct ConcurrentMemoryCorpus {
    inner: Arc<Mutex<MemoryCorpus>>,
}

impl ConcurrentMemoryCorpus {
    pub fn new(corpus: MemoryCorpus) -> Self {
        Self {
            inner: Arc::new(Mutex::new(corpus)),
        }
    }

    pub fn admit(&self, entry: MemoryEntry) -> Result<AdmissionOutcome, MemoryError> {
        self.inner
            .lock()
            .map_err(|_| MemoryError::Unsupported)?
            .admit(entry)
    }

    pub fn revoke(
        &self,
        roots: &[MemoryRef],
        created_at: impl Into<String>,
    ) -> Result<RevocationRecord, MemoryError> {
        self.inner
            .lock()
            .map_err(|_| MemoryError::Unsupported)?
            .revoke(roots, created_at)
    }

    pub fn retrieve(
        &self,
        query: &MemoryQuery,
        now: &str,
    ) -> Result<RetrievalResponse, MemoryError> {
        self.inner
            .lock()
            .map_err(|_| MemoryError::Unsupported)?
            .retrieve(query, now)
    }

    pub fn cleanup_revoked(&self) -> Result<usize, MemoryError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| MemoryError::Unsupported)?
            .cleanup_revoked())
    }

    pub fn with_corpus<T>(
        &self,
        operation: impl FnOnce(&MemoryCorpus) -> T,
    ) -> Result<T, MemoryError> {
        let corpus = self.inner.lock().map_err(|_| MemoryError::Unsupported)?;
        Ok(operation(&corpus))
    }
}
