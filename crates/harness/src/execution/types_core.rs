// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::path::PathBuf;

use super::error::ExecutionStoreError;
use std::time::Duration;

pub const RECOVERY_CONTRACT_VERSION: &str = "watchdog-recovery-v1";
/// SHA-256 of the installed recovery-v1 frame schema artifact. Consumers that decode sideband
/// frames must pin this value in their release/config record before becoming mutation-ready.
pub const RECOVERY_SCHEMA_DIGEST: &str =
    "fb934d3157485aaf6e13e6ebbb213ec8a14c7fc6f5eeebc06b7a22c1f0009217";
pub const MAX_ID_BYTES: usize = 512;
pub const MAX_REFERENCE_BYTES: usize = 512;
pub const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const MAX_CATALOG_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionStoreConfig {
    pub path: PathBuf,
    pub busy_timeout: Duration,
    pub max_payload_bytes: usize,
    pub recovery_schema_digest: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorePragmas {
    pub journal_mode: String,
    pub synchronous: i64,
    pub foreign_keys: bool,
}

impl ExecutionStoreConfig {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            busy_timeout: Duration::from_millis(5_000),
            max_payload_bytes: MAX_PAYLOAD_BYTES,
            recovery_schema_digest: Some(String::from(RECOVERY_SCHEMA_DIGEST)),
        }
    }

    #[must_use]
    pub fn with_recovery_schema_digest(mut self, digest: impl Into<String>) -> Self {
        self.recovery_schema_digest = Some(digest.into());
        self
    }

    #[must_use]
    pub fn with_approved_recovery_schema(mut self) -> Self {
        self.recovery_schema_digest = Some(String::from(RECOVERY_SCHEMA_DIGEST));
        self
    }

    #[must_use]
    pub fn with_busy_timeout(mut self, timeout: Duration) -> Self {
        self.busy_timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_max_payload_bytes(mut self, maximum: usize) -> Self {
        self.max_payload_bytes = maximum;
        self
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        if self.path.as_os_str().is_empty()
            || self.busy_timeout.is_zero()
            || self.busy_timeout > Duration::from_secs(30)
            || self.max_payload_bytes == 0
            || self.max_payload_bytes > MAX_PAYLOAD_BYTES
            || self
                .recovery_schema_digest
                .as_deref()
                .is_some_and(|digest| !valid_digest(digest))
        {
            return Err(ExecutionStoreError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl Default for ExecutionStoreConfig {
    fn default() -> Self {
        Self::new("harness-execution.sqlite3")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionLineage {
    pub run_id: String,
    pub episode_id: String,
    pub attempt_id: String,
    pub trajectory_id: String,
}

impl ExecutionLineage {
    pub fn new(
        run_id: impl Into<String>,
        episode_id: impl Into<String>,
        attempt_id: impl Into<String>,
        trajectory_id: impl Into<String>,
    ) -> Result<Self, ExecutionStoreError> {
        let lineage = Self {
            run_id: run_id.into(),
            episode_id: episode_id.into(),
            attempt_id: attempt_id.into(),
            trajectory_id: trajectory_id.into(),
        };
        if [
            &lineage.run_id,
            &lineage.episode_id,
            &lineage.attempt_id,
            &lineage.trajectory_id,
        ]
        .iter()
        .any(|value| !valid_id(value))
            || distinct(&[
                &lineage.run_id,
                &lineage.episode_id,
                &lineage.attempt_id,
                &lineage.trajectory_id,
            ])
            .is_err()
        {
            return Err(ExecutionStoreError::InvalidIdentity);
        }
        Ok(lineage)
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        if [
            &self.run_id,
            &self.episode_id,
            &self.attempt_id,
            &self.trajectory_id,
        ]
        .iter()
        .any(|value| !valid_id(value))
            || distinct(&[
                &self.run_id,
                &self.episode_id,
                &self.attempt_id,
                &self.trajectory_id,
            ])
            .is_err()
        {
            return Err(ExecutionStoreError::InvalidIdentity);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionFingerprint {
    pub seed: String,
    pub build_digest: String,
    pub state_digest: String,
    pub config_digest: String,
    pub provider_digest: String,
}

impl ExecutionFingerprint {
    pub fn new(
        seed: impl Into<String>,
        build_digest: impl Into<String>,
        state_digest: impl Into<String>,
        config_digest: impl Into<String>,
        provider_digest: impl Into<String>,
    ) -> Result<Self, ExecutionStoreError> {
        let fingerprint = Self {
            seed: seed.into(),
            build_digest: build_digest.into(),
            state_digest: state_digest.into(),
            config_digest: config_digest.into(),
            provider_digest: provider_digest.into(),
        };
        if [
            &fingerprint.seed,
            &fingerprint.build_digest,
            &fingerprint.state_digest,
            &fingerprint.config_digest,
            &fingerprint.provider_digest,
        ]
        .iter()
        .any(|value| !valid_reference(value))
        {
            return Err(ExecutionStoreError::InvalidFingerprint);
        }
        Ok(fingerprint)
    }

    /// Validates a fingerprint assembled by a caller that used the public fields directly.
    /// Constructors are preferred, but the fields remain public so adapters can deserialize an
    /// already-approved release record without duplicating this type.
    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        if [
            &self.seed,
            &self.build_digest,
            &self.state_digest,
            &self.config_digest,
            &self.provider_digest,
        ]
        .iter()
        .any(|value| !valid_reference(value))
        {
            return Err(ExecutionStoreError::InvalidFingerprint);
        }
        Ok(())
    }
}

pub(crate) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

pub(crate) fn valid_reference(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_REFERENCE_BYTES && !value.chars().any(char::is_control)
}

pub(crate) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Validates retained legal-action bytes without reconstructing them from a semantic value.
/// Persistent rows are bounded before this helper is called; the JSON parse only establishes the
/// required array root and rejects malformed bytes.
pub(crate) fn valid_catalog_raw(raw: &[u8], expected_digest: &str) -> bool {
    raw.len() <= MAX_CATALOG_BYTES
        && !raw.is_empty()
        && crate::sha256_hex(raw) == expected_digest
        && serde_json::from_slice::<Value>(raw)
            .ok()
            .is_some_and(|value| value.is_array())
}

fn distinct(values: &[&String]) -> Result<(), ()> {
    for (index, value) in values.iter().enumerate() {
        if values[..index].contains(value) {
            return Err(());
        }
    }
    Ok(())
}
