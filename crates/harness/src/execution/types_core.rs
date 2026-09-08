// SPDX-License-Identifier: MIT

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde_json::Value;
use sha2::Digest;
use std::fmt;
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
pub const MAX_ORIGINAL_CONTEXT_BYTES: usize = 8 * 1024;

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
        && format!("{:x}", sha2::Sha256::digest(raw)) == expected_digest
        && serde_json::from_slice::<Value>(raw)
            .ok()
            .is_some_and(|value| value.is_array())
}

/// The recovery wire contract currently carries a closed host/lease context. Keep the retained
/// representation bounded and structurally strict before it can be used for a sideband request.
pub(crate) fn valid_original_context_raw(raw: &[u8]) -> bool {
    if raw.is_empty() || raw.len() > MAX_ORIGINAL_CONTEXT_BYTES {
        return false;
    }
    let Ok(object) = unique_context_object(raw) else {
        return false;
    };
    let expected = [
        "deployment_id",
        "instance_id",
        "instance_incarnation",
        "boot_id",
        "authority_generation",
        "lease_id",
        "lease_epoch",
    ];
    object.len() == expected.len()
        && expected.iter().all(|key| object.contains_key(*key))
        && object
            .get("deployment_id")
            .and_then(Value::as_str)
            .is_some_and(valid_context_uuid)
        && object
            .get("instance_id")
            .and_then(Value::as_str)
            .is_some_and(valid_context_uuid)
        && object
            .get("instance_incarnation")
            .and_then(Value::as_str)
            .is_some_and(valid_context_uuid_v4)
        && object
            .get("boot_id")
            .and_then(Value::as_str)
            .is_some_and(valid_context_uuid_v4)
        && object
            .get("lease_id")
            .and_then(Value::as_str)
            .is_some_and(valid_context_uuid_v4)
        && object
            .get("authority_generation")
            .and_then(Value::as_u64)
            .is_some_and(valid_context_integer)
        && object
            .get("lease_epoch")
            .and_then(Value::as_u64)
            .is_some_and(valid_context_integer)
}

fn unique_context_object(raw: &[u8]) -> Result<serde_json::Map<String, Value>, ()> {
    struct ContextVisitor;

    impl<'de> Visitor<'de> for ContextVisitor {
        type Value = serde_json::Map<String, Value>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a closed JSON object without duplicate members")
        }

        fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut object = serde_json::Map::new();
            while let Some(key) = access.next_key::<String>()? {
                if object.contains_key(&key) {
                    return Err(de::Error::custom("duplicate original context member"));
                }
                object.insert(key, access.next_value()?);
            }
            Ok(object)
        }
    }

    let mut deserializer = serde_json::Deserializer::from_slice(raw);
    let object = deserializer
        .deserialize_map(ContextVisitor)
        .map_err(|_| ())?;
    deserializer.end().map_err(|_| ())?;
    Ok(object)
}

fn valid_context_uuid(value: &str) -> bool {
    value.len() == 36
        && value.as_bytes().iter().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23) && *byte == b'-'
                || !matches!(index, 8 | 13 | 18 | 23)
                    && (byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        })
        && matches!(value.as_bytes().get(19), Some(b'8' | b'9' | b'a' | b'b'))
}

fn valid_context_uuid_v4(value: &str) -> bool {
    valid_context_uuid(value) && value.as_bytes().get(14) == Some(&b'4')
}

fn valid_context_integer(value: u64) -> bool {
    (1..=9_007_199_254_740_991).contains(&value)
}

fn distinct(values: &[&String]) -> Result<(), ()> {
    for (index, value) in values.iter().enumerate() {
        if values[..index].contains(value) {
            return Err(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MAX_ORIGINAL_CONTEXT_BYTES, valid_original_context_raw};

    const VALID: &[u8] = br#"{"deployment_id":"33333333-3333-4333-8333-333333333333","instance_id":"44444444-4444-4444-8444-444444444444","instance_incarnation":"55555555-5555-4555-8555-555555555555","boot_id":"66666666-6666-4666-8666-666666666666","authority_generation":1,"lease_id":"77777777-7777-4777-8777-777777777777","lease_epoch":1}"#;

    #[test]
    fn original_context_is_closed_bounded_and_duplicate_free() {
        assert!(valid_original_context_raw(VALID));
        assert!(!valid_original_context_raw(
            br#"{"deployment_id":"33333333-3333-4333-8333-333333333333","deployment_id":"33333333-3333-4333-8333-333333333333","instance_id":"44444444-4444-4444-8444-444444444444","instance_incarnation":"55555555-5555-4555-8555-555555555555","boot_id":"66666666-6666-4666-8666-666666666666","authority_generation":1,"lease_id":"77777777-7777-4777-8777-777777777777","lease_epoch":1}"#
        ));
        assert!(!valid_original_context_raw(
            br#"{"deployment_id":"33333333-3333-4333-8333-333333333333","instance_id":"44444444-4444-4444-8444-444444444444","instance_incarnation":"55555555-5555-4555-8555-555555555555","boot_id":"66666666-6666-4666-8666-666666666666","authority_generation":1,"lease_id":"77777777-7777-4777-8777-777777777777","lease_epoch":1,"extra":true}"#
        ));
        let mut oversized = VALID.to_vec();
        oversized.resize(MAX_ORIGINAL_CONTEXT_BYTES + 1, b' ');
        assert!(!valid_original_context_raw(&oversized));
    }
}
