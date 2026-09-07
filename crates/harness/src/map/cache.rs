// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

pub const MAP_CACHE_MAX_ENTRIES: usize = 256;
pub const MAP_CACHE_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const MAP_CACHE_MAX_IN_FLIGHT: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct TopologyCacheKey {
    pub projection_version: String,
    pub scope: String,
    pub snapshot_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct NavigationCacheKey {
    pub projection_version: String,
    pub scope: String,
    pub snapshot_digest: String,
    pub generation: u64,
    pub catalog_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct AnalysisCacheKey {
    pub snapshot_digest: String,
    pub analysis_version: String,
    pub evaluator_version: String,
    pub public_state_digest: String,
    pub objective_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct RenderCacheKey {
    pub analysis_digest: String,
    pub renderer_version: String,
    pub settings_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CacheEntry<V> {
    value: V,
    bytes: usize,
}

/// A deterministic bounded cache. It has no authority methods: a digest or a
/// cache hit cannot authorize a lease, an action, or a provider call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedCache<K, V> {
    entries: BTreeMap<K, CacheEntry<V>>,
    order: VecDeque<K>,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
    max_in_flight: usize,
    in_flight: usize,
}

impl<K: Ord + Clone, V: Clone> BoundedCache<K, V> {
    pub fn new(
        max_entries: usize,
        max_bytes: usize,
        max_in_flight: usize,
    ) -> Result<Self, CacheError> {
        if max_entries == 0
            || max_entries > MAP_CACHE_MAX_ENTRIES
            || max_bytes == 0
            || max_bytes > MAP_CACHE_MAX_BYTES
            || max_in_flight == 0
            || max_in_flight > MAP_CACHE_MAX_IN_FLIGHT
        {
            return Err(CacheError::InvalidLimits);
        }
        Ok(Self {
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            max_entries,
            max_bytes,
            max_in_flight,
            in_flight: 0,
        })
    }

    pub fn get(&self, key: &K) -> Option<V> {
        self.entries.get(key).map(|entry| entry.value.clone())
    }

    pub fn insert(&mut self, key: K, value: V, bytes: usize) -> Result<(), CacheError> {
        if bytes > self.max_bytes {
            return Err(CacheError::ValueTooLarge);
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
            self.order.retain(|candidate| candidate != &key);
        }
        self.entries
            .insert(key.clone(), CacheEntry { value, bytes });
        self.order.push_back(key);
        self.bytes = self.bytes.saturating_add(bytes);
        self.evict();
        Ok(())
    }

    fn evict(&mut self) {
        while self.entries.len() > self.max_entries || self.bytes > self.max_bytes {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
        }
    }

    pub fn begin_in_flight(&mut self) -> Result<InFlightGuard<'_, K, V>, CacheError> {
        if self.in_flight >= self.max_in_flight {
            return Err(CacheError::InFlightLimit);
        }
        self.in_flight += 1;
        Ok(InFlightGuard {
            cache: self,
            done: false,
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    #[must_use]
    pub const fn in_flight(&self) -> usize {
        self.in_flight
    }
}

pub struct InFlightGuard<'a, K, V> {
    cache: &'a mut BoundedCache<K, V>,
    done: bool,
}

impl<K: Ord + Clone, V: Clone> InFlightGuard<'_, K, V> {
    pub fn finish(mut self) {
        self.done = true;
        self.cache.in_flight = self.cache.in_flight.saturating_sub(1);
    }
}

impl<K, V> Drop for InFlightGuard<'_, K, V> {
    fn drop(&mut self) {
        if !self.done {
            self.cache.in_flight = self.cache.in_flight.saturating_sub(1);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheError {
    InvalidLimits,
    ValueTooLarge,
    InFlightLimit,
}

impl fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLimits => "invalid cache limits",
            Self::ValueTooLarge => "cache value exceeds byte bound",
            Self::InFlightLimit => "cache in-flight limit reached",
        })
    }
}

impl std::error::Error for CacheError {}
