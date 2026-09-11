// SPDX-License-Identifier: MIT

impl DurableMemoryStore {
    fn aad(&self, entry: &MemoryEntry) -> Vec<u8> {
        serde_json::to_vec(&(&self.scope, &entry.entry_id, entry.version, &entry.sha256))
            .unwrap_or_default()
    }

    fn admit_loaded(
        corpus: &mut MemoryCorpus,
        loaded: Vec<MemoryEntry>,
    ) -> Result<(), MemoryError> {
        for entry in loaded {
            let reference = entry.reference();
            match corpus.admit(entry) {
                Ok(_) => {}
                Err(MemoryError::Revoked) => {
                    corpus.revoked.insert(reference);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn load_revocation_ledger(&self, corpus: &mut MemoryCorpus) -> Result<(), MemoryError> {
        let mut statement = self
            .connection
            .prepare("SELECT entry_id, version, sha256, epoch FROM memory_revocations")
            .map_err(|_| MemoryError::Unsupported)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    u64::try_from(row.get::<_, i64>(1)?).map_err(|_| {
                        rusqlite::Error::IntegralValueOutOfRange(1, 0)
                    })?,
                    row.get::<_, String>(2)?,
                    u64::try_from(row.get::<_, i64>(3)?).map_err(|_| {
                        rusqlite::Error::IntegralValueOutOfRange(3, 0)
                    })?,
                ))
            })
            .map_err(|_| MemoryError::Unsupported)?;
        for row in rows {
            let (entry_id, version, sha256, epoch) =
                row.map_err(|_| MemoryError::Unsupported)?;
            let reference = MemoryRef::new(entry_id, version, sha256);
            if !reference.valid() || epoch == 0 {
                return Err(MemoryError::InvalidEntry);
            }
            corpus.revoked.insert(reference);
            corpus.revocation_epoch = corpus.revocation_epoch.max(epoch);
        }
        if corpus.revoked.len() > MAX_REVOKED_REFS {
            return Err(MemoryError::Capacity);
        }
        Ok(())
    }

    pub fn revoke(&mut self, roots: &[MemoryRef], epoch: u64) -> Result<(), MemoryError> {
        if roots.is_empty()
            || roots.len() > 16
            || roots.iter().any(|reference| !reference.valid())
            || epoch == 0
            || roots.iter().any(|reference| {
                self.connection
                    .query_row(
                        "SELECT 1 FROM memory_entries WHERE entry_id = ?1 AND version = ?2 AND sha256 = ?3",
                        params![reference.entry_id, reference.version as i64, reference.sha256],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()
                    .ok()
                    .flatten()
                    .is_none()
            })
        {
            return Err(MemoryError::InvalidEntry);
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| MemoryError::PublicationFailed)?;
        for root in roots {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO memory_revocations (entry_id, version, sha256, epoch) VALUES (?1, ?2, ?3, ?4)",
                    params![root.entry_id, root.version as i64, root.sha256, epoch as i64],
                )
                .map_err(|_| MemoryError::PublicationFailed)?;
        }
        transaction.commit().map_err(|_| MemoryError::PublicationFailed)
    }

    pub fn purge_revoked(&mut self) -> Result<usize, MemoryError> {
        let corpus = self.load_corpus()?;
        let rows = {
            let mut statement = self
                .connection
                .prepare("SELECT metadata, entry_id, version, sha256 FROM memory_entries")
                .map_err(|_| MemoryError::Unsupported)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, String>(1)?,
                        u64::try_from(row.get::<_, i64>(2)?).map_err(|_| {
                            rusqlite::Error::IntegralValueOutOfRange(2, 0)
                        })?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|_| MemoryError::Unsupported)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|_| MemoryError::Unsupported)?
        };
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| MemoryError::PublicationFailed)?;
        let mut purged = 0;
        for (metadata, entry_id, version, sha256) in rows {
            let entry: MemoryEntry = serde_json::from_slice(&metadata).map_err(|_| MemoryError::InvalidEntry)?;
            let reference = MemoryRef::new(&entry_id, version, &sha256);
            if corpus.revoked.contains(&reference)
                || corpus
                    .entry(&reference)
                    .is_some_and(|item| item.status == EntryStatus::Revoked)
            {
                let tombstone = serde_json::to_vec(&MemoryEntry { status: EntryStatus::Revoked, ..entry })
                    .map_err(|_| MemoryError::Unsupported)?;
                transaction
                    .execute(
                        "UPDATE memory_entries SET metadata = ?1, ciphertext = X'', protected = 0 WHERE entry_id = ?2 AND version = ?3 AND sha256 = ?4",
                        params![tombstone, entry_id, version as i64, sha256],
                    )
                    .map_err(|_| MemoryError::PublicationFailed)?;
                purged += 1;
            }
        }
        transaction.commit().map_err(|_| MemoryError::PublicationFailed)?;
        Ok(purged)
    }

    pub fn storage_contains_plaintext(&self, needle: &[u8]) -> Result<bool, MemoryError> {
        if needle.is_empty() {
            return Ok(false);
        }
        let mut statement = self
            .connection
            .prepare("SELECT metadata, ciphertext FROM memory_entries")
            .map_err(|_| MemoryError::Unsupported)?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)))
            .map_err(|_| MemoryError::Unsupported)?;
        for row in rows {
            let (metadata, ciphertext) = row.map_err(|_| MemoryError::Unsupported)?;
            if metadata.windows(needle.len()).any(|window| window == needle)
                || ciphertext.windows(needle.len()).any(|window| window == needle)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
