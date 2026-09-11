// SPDX-License-Identifier: MIT

// Finite retention accounting for source bodies, derived artifacts and cleanup work.

pub const MEMORY_RETENTION_SCHEMA: &str = "ascension.context-memory.retention.v1";
pub const MAX_RETENTION_RESOURCES: usize = 512;
pub const MAX_RETENTION_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionKind {
    SourceBody,
    SummaryBody,
    Snippet,
    Index,
    Cache,
    Job,
    Backup,
    Temp,
    Tombstone,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RetainedResource {
    id: String,
    root: MemoryRef,
    kind: RetentionKind,
    bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetentionInventory {
    max_resources: usize,
    max_bytes: usize,
    resources: BTreeMap<String, RetainedResource>,
    tombstones: BTreeSet<MemoryRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionSnapshot {
    pub schema: String,
    pub resource_count: usize,
    pub retained_bytes: usize,
    pub cleanup_backlog: usize,
    pub tombstone_count: usize,
}

impl RetentionInventory {
    pub fn new(max_resources: usize, max_bytes: usize) -> Result<Self, MemoryError> {
        if max_resources == 0
            || max_resources > MAX_RETENTION_RESOURCES
            || max_bytes == 0
            || max_bytes > MAX_RETENTION_BYTES
        {
            return Err(MemoryError::Capacity);
        }
        Ok(Self {
            max_resources,
            max_bytes,
            resources: BTreeMap::new(),
            tombstones: BTreeSet::new(),
        })
    }

    pub fn retain(
        &mut self,
        id: impl Into<String>,
        root: MemoryRef,
        kind: RetentionKind,
        bytes: usize,
    ) -> Result<(), MemoryError> {
        let id = id.into();
        if !valid_id(&id) || !root.valid() || bytes > MAX_SOURCE_BYTES * 4 {
            return Err(MemoryError::Capacity);
        }
        if let Some(existing) = self.resources.get(&id) {
            if existing.root == root && existing.kind == kind && existing.bytes == bytes {
                return Ok(());
            }
            return Err(MemoryError::Conflict);
        }
        if self.resources.len() >= self.max_resources
            || self
                .retained_bytes()
                .saturating_add(bytes)
                > self.max_bytes
        {
            return Err(MemoryError::Capacity);
        }
        self.resources.insert(
            id.clone(),
            RetainedResource {
                id,
                root,
                kind,
                bytes,
            },
        );
        Ok(())
    }

    pub fn mark_revoked(&mut self, roots: &[MemoryRef]) -> Result<(), MemoryError> {
        if roots.is_empty() || roots.iter().any(|root| !root.valid()) {
            return Err(MemoryError::InvalidEntry);
        }
        self.tombstones.extend(roots.iter().cloned());
        Ok(())
    }

    pub fn cleanup_revoked(&mut self) -> usize {
        let before = self.resources.len();
        self.resources
            .retain(|_, resource| !self.tombstones.contains(&resource.root));
        before.saturating_sub(self.resources.len())
    }

    pub fn retained_bytes(&self) -> usize {
        self.resources.values().map(|resource| resource.bytes).sum()
    }

    pub fn snapshot(&self) -> RetentionSnapshot {
        RetentionSnapshot {
            schema: MEMORY_RETENTION_SCHEMA.to_owned(),
            resource_count: self.resources.len(),
            retained_bytes: self.retained_bytes(),
            cleanup_backlog: self
                .resources
                .values()
                .filter(|resource| self.tombstones.contains(&resource.root))
                .count(),
            tombstone_count: self.tombstones.len(),
        }
    }

    pub fn resource_ids(&self) -> impl Iterator<Item = &str> {
        self.resources.values().map(|resource| resource.id.as_str())
    }
}
