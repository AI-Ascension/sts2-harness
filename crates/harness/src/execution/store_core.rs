// SPDX-License-Identifier: MIT

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use super::schema;
pub(crate) use super::store_core_helpers::{
    append_event, attempt_fingerprint, configure, create_parent, ensure_current_lineage,
    has_store_schema, insert_attempt, verify_contract,
};
use super::types::{
    AttemptKind, ExecutionFingerprint, ExecutionLineage, ExecutionStoreConfig, ExecutionStoreError,
    StorePragmas, StoredEpisode,
};
/// Single-owner durable harness execution state. The connection is intentionally synchronous and
/// non-cloneable: one process owns this store, while other components use explicit ports.
pub struct ExecutionStore {
    pub(crate) connection: Connection,
    pub(crate) config: ExecutionStoreConfig,
    pub(crate) closed: bool,
    pub(crate) read_only: bool,
    incarnation: Arc<()>,
}

impl ExecutionStore {
    pub fn open(config: ExecutionStoreConfig) -> Result<Self, ExecutionStoreError> {
        config.validate()?;
        create_parent(&config.path)?;
        let existing = config.path != Path::new(":memory:") && config.path.is_file();
        if existing && fs::metadata(&config.path).map_or(true, |metadata| metadata.len() == 0) {
            return Err(ExecutionStoreError::Corrupt);
        }
        let mut connection = Connection::open(&config.path).map_err(schema::map_sqlite)?;
        if existing && !has_store_schema(&connection)? {
            return Err(ExecutionStoreError::Corrupt);
        }
        configure(&mut connection, &config)?;
        schema::migrate(&mut connection)?;
        verify_contract(&connection, &config, false)?;
        let store = Self {
            connection,
            config,
            closed: false,
            read_only: false,
            incarnation: Arc::new(()),
        };
        store.integrity_check()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, ExecutionStoreError> {
        Self::open(ExecutionStoreConfig::new(":memory:"))
    }

    /// Opens existing state without migration or creation. This is suitable for read-only status
    /// paths that must not turn a missing database into a new epoch of execution.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, ExecutionStoreError> {
        let path = path.as_ref().to_path_buf();
        if !path.is_file() {
            return Err(ExecutionStoreError::Missing);
        }
        let config = ExecutionStoreConfig::new(path);
        let connection = Connection::open_with_flags(
            &config.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .map_err(schema::map_sqlite)?;
        let version = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
            .map_err(schema::map_sqlite)?;
        if version != schema::CURRENT_SCHEMA_VERSION {
            return Err(ExecutionStoreError::Incompatible);
        }
        verify_contract(&connection, &config, true)?;
        let store = Self {
            connection,
            config,
            closed: false,
            read_only: true,
            incarnation: Arc::new(()),
        };
        store.integrity_check()?;
        Ok(store)
    }

    #[must_use]
    pub fn config(&self) -> &ExecutionStoreConfig {
        &self.config
    }

    /// Returns the SQLite library actually linked into the harness process.
    #[must_use]
    pub fn sqlite_version() -> &'static str {
        rusqlite::version()
    }

    pub fn pragmas(&self) -> Result<StorePragmas, ExecutionStoreError> {
        self.ensure_open()?;
        let journal_mode = self
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .map_err(schema::map_sqlite)?;
        let synchronous = self
            .connection
            .query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0))
            .map_err(schema::map_sqlite)?;
        let foreign_keys = self
            .connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .map_err(schema::map_sqlite)?;
        Ok(StorePragmas {
            journal_mode,
            synchronous,
            foreign_keys: foreign_keys != 0,
        })
    }

    pub fn integrity_check(&self) -> Result<(), ExecutionStoreError> {
        self.ensure_open()?;
        let result = self
            .connection
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .map_err(schema::map_sqlite)?;
        if result != "ok" {
            return Err(ExecutionStoreError::Corrupt);
        }
        let mut statement = self
            .connection
            .prepare("PRAGMA foreign_key_check")
            .map_err(schema::map_sqlite)?;
        let mut rows = statement.query([]).map_err(schema::map_sqlite)?;
        if rows.next().map_err(schema::map_sqlite)?.is_some() {
            return Err(ExecutionStoreError::Corrupt);
        }
        Ok(())
    }

    pub fn close(&mut self) -> Result<(), ExecutionStoreError> {
        self.ensure_open()?;
        if !self.read_only {
            self.connection
                .execute_batch("PRAGMA wal_checkpoint(PASSIVE)")
                .map_err(schema::map_sqlite)?;
        }
        self.closed = true;
        Ok(())
    }

    pub fn start_episode(
        &mut self,
        lineage: &ExecutionLineage,
        fingerprint: &ExecutionFingerprint,
    ) -> Result<StoredEpisode, ExecutionStoreError> {
        self.ensure_open()?;
        lineage.validate()?;
        fingerprint.validate()?;
        let now = now_millis();
        let tx = schema::transaction(&mut self.connection)?;
        let existing_run = tx
            .query_row(
                "SELECT seed, build_digest, state_digest, config_digest, provider_digest
                 FROM runs WHERE run_id = ?1",
                [lineage.run_id.as_str()],
                |row| {
                    ExecutionFingerprint::new(
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    )
                    .map_err(|_| rusqlite::Error::InvalidQuery)
                },
            )
            .optional()
            .map_err(schema::map_sqlite)?;
        if existing_run.as_ref().is_some_and(|old| old != fingerprint) {
            return Err(ExecutionStoreError::Conflict);
        }
        if existing_run.is_none() {
            tx.execute(
                "INSERT INTO runs(run_id, seed, build_digest, state_digest, config_digest,
                 provider_digest, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    lineage.run_id,
                    fingerprint.seed,
                    fingerprint.build_digest,
                    fingerprint.state_digest,
                    fingerprint.config_digest,
                    fingerprint.provider_digest,
                    now
                ],
            )
            .map_err(schema::map_sqlite)?;
        }
        let existing_episode = tx
            .query_row(
                "SELECT current_attempt_id, current_trajectory_id FROM episodes
                 WHERE episode_id = ?1",
                [lineage.episode_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(schema::map_sqlite)?;
        if let Some((attempt_id, trajectory_id)) = existing_episode {
            if attempt_id != lineage.attempt_id || trajectory_id != lineage.trajectory_id {
                return Err(ExecutionStoreError::Conflict);
            }
            let old = attempt_fingerprint(&tx, &lineage.attempt_id)?;
            if old != *fingerprint {
                return Err(ExecutionStoreError::Conflict);
            }
            tx.commit().map_err(schema::map_sqlite)?;
            return self.load_episode(&lineage.episode_id);
        }
        tx.execute(
            "INSERT INTO episodes(episode_id, run_id, current_attempt_id,
             current_trajectory_id, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)",
            params![
                lineage.episode_id,
                lineage.run_id,
                lineage.attempt_id,
                lineage.trajectory_id,
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        insert_attempt(&tx, lineage, fingerprint, AttemptKind::Initial, None, now)?;
        tx.execute(
            "INSERT INTO trajectories(trajectory_id, run_id, episode_id, attempt_id,
             provenance_ref, provenance_digest, created_at) VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5)",
            params![
                lineage.trajectory_id,
                lineage.run_id,
                lineage.episode_id,
                lineage.attempt_id,
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        append_event(&tx, "episode", &lineage.episode_id, "active", None, now)?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.load_episode(&lineage.episode_id)
    }

    pub(crate) fn ensure_open(&self) -> Result<(), ExecutionStoreError> {
        if self.closed {
            Err(ExecutionStoreError::Persistence(String::from(
                "execution store is closed",
            )))
        } else {
            Ok(())
        }
    }

    pub(crate) fn now() -> i64 {
        now_millis()
    }

    pub(crate) fn incarnation(&self) -> &Arc<()> {
        &self.incarnation
    }
}

fn now_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

impl Drop for ExecutionStore {
    fn drop(&mut self) {
        if !self.closed && !self.read_only {
            let _ = self
                .connection
                .execute_batch("PRAGMA wal_checkpoint(PASSIVE)");
        }
    }
}
