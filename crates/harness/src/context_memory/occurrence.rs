// SPDX-License-Identifier: MIT

// Content-addressed blob reuse with separately addressable event occurrences.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryOccurrenceStore {
    scope: MemoryScope,
    blobs: BTreeMap<String, Vec<u8>>,
    occurrences: BTreeMap<String, Vec<MemoryRef>>,
    max_blobs: usize,
    max_occurrences: usize,
    occurrence_count: usize,
}

impl MemoryOccurrenceStore {
    pub fn new(
        scope: MemoryScope,
        max_blobs: usize,
        max_occurrences: usize,
    ) -> Result<Self, MemoryError> {
        if !scope.valid()
            || max_blobs == 0
            || max_blobs > MAX_ENTRIES_PER_RUN
            || max_occurrences == 0
            || max_occurrences > MAX_ENTRIES_PER_RUN
        {
            return Err(MemoryError::Capacity);
        }
        Ok(Self {
            scope,
            blobs: BTreeMap::new(),
            occurrences: BTreeMap::new(),
            max_blobs,
            max_occurrences,
            occurrence_count: 0,
        })
    }

    pub fn insert(&mut self, entry: &MemoryEntry) -> Result<MemoryRef, MemoryError> {
        entry.validate()?;
        if entry.scope != self.scope {
            return Err(MemoryError::InvalidScope);
        }
        let reference = entry.reference();
        if !self.blobs.contains_key(&entry.sha256) && self.blobs.len() >= self.max_blobs {
            return Err(MemoryError::Capacity);
        }
        let existing = self
            .occurrences
            .get(&entry.source_record_id)
            .is_some_and(|refs| refs.contains(&reference));
        if !existing && self.occurrence_count >= self.max_occurrences {
            return Err(MemoryError::Capacity);
        }
        if let Some(existing_bytes) = self.blobs.get(&entry.sha256) {
            if existing_bytes != &entry.content {
                return Err(MemoryError::Conflict);
            }
        } else {
            self.blobs.insert(entry.sha256.clone(), entry.content.clone());
        }
        if !existing {
            self.occurrences
                .entry(entry.source_record_id.clone())
                .or_default()
                .push(reference.clone());
            self.occurrence_count = self.occurrence_count.saturating_add(1);
        }
        Ok(reference)
    }

    pub fn blob_count(&self) -> usize {
        self.blobs.len()
    }

    pub fn occurrence_count(&self) -> usize {
        self.occurrence_count
    }

    pub fn occurrences(&self, source_record_id: &str) -> &[MemoryRef] {
        self.occurrences
            .get(source_record_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn read(&self, reference: &MemoryRef) -> Result<&[u8], MemoryError> {
        self.blobs
            .get(&reference.sha256)
            .map(Vec::as_slice)
            .ok_or(MemoryError::MissingParent)
    }
}
