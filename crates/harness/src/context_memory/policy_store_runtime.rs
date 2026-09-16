// SPDX-License-Identifier: MIT

use super::*;

impl PolicyStore {
    pub fn open_private(
        path: &Path,
        key: [u8; 32],
        scope: MemoryScope,
        consent: PolicyStoreConsent,
    ) -> Result<Self, PolicyOwnerError> {
        validate_open(&key, &consent)?;
        let (connection, guard) =
            crate::context_memory::private_sqlite::PrivateSqliteGuard::open(path, 32 * 1024 * 1024)
                .map_err(|_| PolicyOwnerError::Unavailable)?;
        Self::initialize(path, key, scope, connection, Some(guard))
    }

    pub(super) fn initialize(
        path: &Path,
        key: [u8; 32],
        scope: MemoryScope,
        connection: Connection,
        private_guard: Option<crate::context_memory::private_sqlite::PrivateSqliteGuard>,
    ) -> Result<Self, PolicyOwnerError> {
        let mut store = Self {
            connection,
            path: path.to_owned(),
            private_guard,
            key,
            scope,
            epoch: 0,
            authority_lease_active: false,
            failpoint: None,
        };
        store.verify_path()?;
        store
            .connection
            .busy_timeout(std::time::Duration::from_secs(1))
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        check_pages(&store.connection)?;
        let existing = has_schema(&store.connection)?;
        if existing {
            check_schema(&store.connection)?;
        }
        store
            .connection
            .execute_batch(
                "PRAGMA page_size=4096; PRAGMA max_page_count=8192;
             PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA temp_store=MEMORY;",
            )
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        store.claim(existing)?;
        Ok(store)
    }

    pub(super) fn verify_path(&self) -> Result<(), PolicyOwnerError> {
        if let Some(guard) = &self.private_guard {
            guard.verify().map_err(|_| PolicyOwnerError::Unavailable)
        } else {
            check_file(&self.path)
        }
    }

    /// Opens an immediate SQLite transaction and leaves it active while a
    /// lookup-authority guard holds this store mutex. The caller must end the
    /// lease before releasing that mutex.
    pub fn begin_authority_lease(&mut self) -> Result<PolicyJournal, PolicyOwnerError> {
        if self.authority_lease_active {
            return Err(PolicyOwnerError::Unavailable);
        }
        self.verify_path()?;
        check_pages(&self.connection)?;
        check_schema(&self.connection)?;
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        self.authority_lease_active = true;
        let result = (|| {
            check_schema(&self.connection)?;
            let (epoch, envelope) = read_envelope(&self.connection)?;
            if epoch != self.epoch {
                return Err(PolicyOwnerError::OwnerFenced);
            }
            let journal = decrypt(&self.key, &self.scope, &envelope)?;
            if journal.store_epoch != epoch {
                return Err(PolicyOwnerError::Corrupt);
            }
            Ok(journal)
        })();
        if result.is_err() {
            self.end_authority_lease();
        }
        result
    }

    pub fn end_authority_lease(&mut self) {
        if self.authority_lease_active {
            let _ = self.connection.execute_batch("ROLLBACK");
            self.authority_lease_active = false;
        }
    }
}
