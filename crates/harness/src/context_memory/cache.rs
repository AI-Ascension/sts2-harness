// SPDX-License-Identifier: MIT

pub const MEMORY_CACHE_SCHEMA: &str = "ascension.context-memory.cache.v1";
pub const MAX_CACHE_ENTRIES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
struct CachedRetrieval {
    corpus_generation: u64,
    projection_generation: u64,
    revocation_epoch: u64,
    response: RetrievalResponse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetrievalCache {
    entries: BTreeMap<String, CachedRetrieval>,
    max_entries: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheLookup {
    Hit(Box<RetrievalResponse>),
    Miss,
    Invalidated,
}

impl RetrievalCache {
    pub fn new(max_entries: usize) -> Result<Self, MemoryError> {
        if max_entries == 0 || max_entries > MAX_CACHE_ENTRIES {
            return Err(MemoryError::Capacity);
        }
        Ok(Self {
            entries: BTreeMap::new(),
            max_entries,
        })
    }

    fn key(query: &MemoryQuery) -> String {
        sha256_hex(serde_json::to_vec(query).unwrap_or_default())
    }

    pub fn put(&mut self, query: &MemoryQuery, response: RetrievalResponse) {
        if self.entries.len() >= self.max_entries
            && !self.entries.contains_key(&Self::key(query))
            && let Some(oldest) = self.entries.keys().next().cloned()
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(
            Self::key(query),
            CachedRetrieval {
                corpus_generation: response.corpus_generation,
                projection_generation: response.projection_generation,
                revocation_epoch: response.revocation_epoch,
                response,
            },
        );
    }

    pub fn get(&self, query: &MemoryQuery, corpus: &MemoryCorpus) -> CacheLookup {
        let Some(cached) = self.entries.get(&Self::key(query)) else {
            return CacheLookup::Miss;
        };
        if cached.corpus_generation != query.corpus_generation
            || cached.projection_generation != corpus.projection_generation
            || cached.revocation_epoch != corpus.revocation_epoch
            || !corpus.projection_healthy
        {
            return CacheLookup::Invalidated;
        }
        CacheLookup::Hit(Box::new(cached.response.clone()))
    }

    pub fn invalidate_all(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
