// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

/// Identity of one semantic mutation attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionIdentity {
    pub operation_id: String,
    pub state_id: String,
    pub generation: u64,
    pub action_id: String,
}

impl ActionIdentity {
    pub fn new(
        operation_id: impl Into<String>,
        state_id: impl Into<String>,
        generation: u64,
        action_id: impl Into<String>,
    ) -> Result<Self, IdempotencyError> {
        let identity = Self {
            operation_id: operation_id.into(),
            state_id: state_id.into(),
            generation,
            action_id: action_id.into(),
        };
        if identity.generation > 9_007_199_254_740_991
            || [
                &identity.operation_id,
                &identity.state_id,
                &identity.action_id,
            ]
            .iter()
            .any(|value| !valid_identity(value))
        {
            return Err(IdempotencyError::InvalidIdentity);
        }
        Ok(identity)
    }

    /// Constructs an identity suitable for durable recovery sidebands. Runtime mutation
    /// operation IDs are UUIDv4 so a caller cannot smuggle a human-readable alias into a
    /// historical lookup that requires a globally unique operation fence.
    pub fn new_v4(
        operation_id: impl Into<String>,
        state_id: impl Into<String>,
        generation: u64,
        action_id: impl Into<String>,
    ) -> Result<Self, IdempotencyError> {
        let identity = Self::new(operation_id, state_id, generation, action_id)?;
        if valid_uuid_v4(&identity.operation_id) {
            Ok(identity)
        } else {
            Err(IdempotencyError::InvalidIdentity)
        }
    }
}

/// Result of admitting one operation identity to the local ledger.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Admission {
    New,
    Duplicate,
    Conflict,
}

/// Bounded operation ledger. An uncertain operation remains recorded and is never retried as a
/// new strategic action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionLedger {
    entries: BTreeMap<String, ActionIdentity>,
    capacity: usize,
}

impl ActionLedger {
    pub fn new(capacity: usize) -> Result<Self, IdempotencyError> {
        if capacity == 0 || capacity > 4096 {
            return Err(IdempotencyError::InvalidCapacity);
        }
        Ok(Self {
            entries: BTreeMap::new(),
            capacity,
        })
    }

    pub fn admit(&mut self, identity: ActionIdentity) -> Result<Admission, IdempotencyError> {
        if let Some(existing) = self.entries.get(&identity.operation_id) {
            return Ok(if existing == &identity {
                Admission::Duplicate
            } else {
                Admission::Conflict
            });
        }
        if self.entries.len() >= self.capacity {
            return Err(IdempotencyError::Full);
        }
        self.entries.insert(identity.operation_id.clone(), identity);
        Ok(Admission::New)
    }

    #[must_use]
    pub fn contains(&self, operation_id: &str) -> bool {
        self.entries.contains_key(operation_id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdempotencyError {
    InvalidIdentity,
    InvalidCapacity,
    Full,
}

impl std::fmt::Display for IdempotencyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "action identity is invalid",
            Self::InvalidCapacity => "action ledger capacity is invalid",
            Self::Full => "action ledger is full",
        })
    }
}

impl std::error::Error for IdempotencyError {}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

fn valid_uuid_v4(value: &str) -> bool {
    value.len() == 36
        && value.as_bytes().iter().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23) && *byte == b'-'
                || !matches!(index, 8 | 13 | 18 | 23)
                    && (byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        })
        && value.as_bytes().get(14) == Some(&b'4')
        && matches!(value.as_bytes().get(19), Some(b'8' | b'9' | b'a' | b'b'))
}
