// SPDX-License-Identifier: MIT

// A small durable corpus adapter used by offline tests.  Metadata is stored separately from
// encrypted source bytes; no plaintext token index, WAL copy or provider callback is created.

use chacha20poly1305::{XChaCha20Poly1305, XNonce, aead::{Aead, KeyInit, Payload}};
use rusqlite::{Connection, OptionalExtension, params};

pub const MEMORY_STORE_SCHEMA: &str = "ascension.context-memory.sqlite.v1";

pub struct DurableMemoryStore {
    connection: Connection,
    scope: MemoryScope,
    key: [u8; 32],
}

impl DurableMemoryStore {
    pub fn open(
        path: &str,
        scope: MemoryScope,
        key: [u8; 32],
    ) -> Result<Self, MemoryError> {
        if !scope.valid() {
            return Err(MemoryError::InvalidScope);
        }
        if key.iter().all(|byte| *byte == 0) {
            return Err(MemoryError::PermissionDenied);
        }
        let connection = Connection::open(path).map_err(|_| MemoryError::Unsupported)?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS memory_entries (
                   entry_id TEXT NOT NULL,
                   version INTEGER NOT NULL,
                   sha256 TEXT NOT NULL,
                   metadata BLOB NOT NULL,
                   ciphertext BLOB NOT NULL,
                   protected INTEGER NOT NULL,
                   PRIMARY KEY (entry_id, version)
                 );
                 CREATE TABLE IF NOT EXISTS memory_revocations (
                   entry_id TEXT NOT NULL,
                   version INTEGER NOT NULL,
                   sha256 TEXT NOT NULL,
                   epoch INTEGER NOT NULL,
                   PRIMARY KEY (entry_id, version, sha256)
                 );",
            )
            .map_err(|_| MemoryError::Unsupported)?;
        Ok(Self {
            connection,
            scope,
            key,
        })
    }

    fn nonce(reference: &MemoryRef) -> XNonce {
        let digest = sha256_hex(format!("{}:{}", reference.sha256, reference.version));
        let mut bytes = [0_u8; 24];
        for (index, pair) in digest.as_bytes().chunks_exact(2).take(24).enumerate() {
            bytes[index] = u8::from_str_radix(std::str::from_utf8(pair).unwrap_or("00"), 16)
                .unwrap_or(0);
        }
        *XNonce::from_slice(&bytes)
    }

    fn encrypt(&self, entry: &MemoryEntry) -> Result<Vec<u8>, MemoryError> {
        let cipher = XChaCha20Poly1305::new((&self.key).into());
        cipher
            .encrypt(
                &Self::nonce(&entry.reference()),
                Payload {
                    msg: &entry.content,
                    aad: &self.aad(entry),
                },
            )
            .map_err(|_| MemoryError::Unsupported)
    }

    fn decrypt(&self, entry: &MemoryEntry, ciphertext: &[u8]) -> Result<Vec<u8>, MemoryError> {
        let cipher = XChaCha20Poly1305::new((&self.key).into());
        cipher
            .decrypt(
                &Self::nonce(&entry.reference()),
                Payload {
                    msg: ciphertext,
                    aad: &self.aad(entry),
                },
            )
            .map_err(|_| MemoryError::InvalidEntry)
    }

    pub fn publish(&mut self, entry: MemoryEntry) -> Result<AdmissionOutcome, MemoryError> {
        entry.validate()?;
        if entry.scope != self.scope {
            return Err(MemoryError::InvalidScope);
        }
        for parent in &entry.parents {
            let parent_row: Option<Vec<u8>> = self
                .connection
                .query_row(
                    "SELECT metadata FROM memory_entries WHERE entry_id = ?1 AND version = ?2 AND sha256 = ?3",
                    params![parent.entry_id, parent.version as i64, parent.sha256],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| MemoryError::Unsupported)?;
            let parent_entry: MemoryEntry = parent_row
                .as_deref()
                .map(serde_json::from_slice)
                .transpose()
                .map_err(|_| MemoryError::InvalidEntry)?
                .ok_or(MemoryError::MissingParent)?;
            if parent_entry.status != EntryStatus::Admitted {
                return Err(MemoryError::Revoked);
            }
            let revoked: Option<i64> = self
                .connection
                .query_row(
                    "SELECT 1 FROM memory_revocations WHERE entry_id = ?1 AND version = ?2 AND sha256 = ?3",
                    params![parent.entry_id, parent.version as i64, parent.sha256],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| MemoryError::Unsupported)?;
            if revoked.is_some() {
                return Err(MemoryError::Revoked);
            }
            if parent_entry.scope != self.scope {
                return Err(MemoryError::CrossScopeParent);
            }
            if parent_entry.observed_seq > entry.observed_seq
                || parent_entry.admitted_seq > entry.admitted_seq
                || parent_entry.lineage_depth.saturating_add(1) != entry.lineage_depth
            {
                return Err(MemoryError::FutureParent);
            }
        }
        let reference = entry.reference();
        let existing: Option<(Vec<u8>, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT metadata, ciphertext FROM memory_entries WHERE entry_id = ?1 AND version = ?2",
                params![reference.entry_id, reference.version as i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| MemoryError::Unsupported)?;
        if let Some((metadata, ciphertext)) = existing {
            let old: MemoryEntry = serde_json::from_slice(&metadata).map_err(|_| MemoryError::InvalidEntry)?;
            if old.scope != self.scope {
                return Err(MemoryError::InvalidScope);
            }
            let old_content = self.decrypt(&old, &ciphertext)?;
            return if old_content == entry.content {
                Ok(AdmissionOutcome::Duplicate)
            } else {
                Err(MemoryError::Conflict)
            };
        }
        let metadata = serde_json::to_vec(&entry).map_err(|_| MemoryError::Unsupported)?;
        let ciphertext = self.encrypt(&entry)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| MemoryError::PublicationFailed)?;
        transaction
            .execute(
                    "INSERT INTO memory_entries (entry_id, version, sha256, metadata, ciphertext, protected) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![reference.entry_id, reference.version as i64, reference.sha256, metadata, ciphertext, entry.protected as i64],
            )
            .map_err(|_| MemoryError::PublicationFailed)?;
        transaction.commit().map_err(|_| MemoryError::PublicationFailed)?;
        Ok(AdmissionOutcome::Inserted)
    }

    pub fn load_corpus(&self) -> Result<MemoryCorpus, MemoryError> {
        let mut corpus = MemoryCorpus::with_limits(self.scope.clone(), MAX_ENTRIES_PER_RUN, MAX_CORPUS_BYTES)?;
        self.load_revocation_ledger(&mut corpus)?;
        let mut statement = self
            .connection
            .prepare("SELECT metadata, ciphertext, protected FROM memory_entries")
            .map_err(|_| MemoryError::Unsupported)?;
        let rows = statement
            .query_map([], |row| {
                let metadata: Vec<u8> = row.get(0)?;
                let ciphertext: Vec<u8> = row.get(1)?;
                let protected: i64 = row.get(2)?;
                Ok((metadata, ciphertext, protected != 0))
            })
            .map_err(|_| MemoryError::Unsupported)?;
        let mut loaded = Vec::new();
        for row in rows {
            let (metadata, ciphertext, protected) = row.map_err(|_| MemoryError::Unsupported)?;
            let mut entry: MemoryEntry = serde_json::from_slice(&metadata).map_err(|_| MemoryError::InvalidEntry)?;
            if entry.status == EntryStatus::Revoked {
                corpus.revoked.insert(entry.reference());
                continue;
            }
            if entry.status != EntryStatus::Admitted {
                continue;
            }
            entry.content = self.decrypt(&entry, &ciphertext)?;
            entry.protected = protected;
            loaded.push(entry);
        }
        loaded.sort_by_key(|entry| (entry.admitted_seq, entry.observed_seq, entry.entry_id.clone()));
        Self::admit_loaded(&mut corpus, loaded)?;
        Ok(corpus)
    }

}
