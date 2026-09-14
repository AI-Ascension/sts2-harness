// SPDX-License-Identifier: MIT

//! Machine-readable effective-limit classification for harness-owned policy values.
//!
//! A portable JSON Schema ceiling is a syntax bound, not an execution promise; an absent field is
//! never unlimited. [`EffectiveLimitRecord::admit_authorized`] authenticates the complete record
//! against the derivation of the validated trusted capability descriptor.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Shared record schema for both the context-memory and provider-session surfaces.
pub const EFFECTIVE_LIMIT_RECORD_SCHEMA: &str = "ascension.harness.effective-limits.v1";

/// How a published ceiling relates to the portable policy-schema ceiling.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitClass {
    /// The portable policy-schema ceiling and this profile's executable ceiling are identical.
    SchemaExecutableEqual,
    /// The portable policy schema intentionally admits larger values than this profile executes.
    SchemaBroaderThanExecutable,
    /// The ceiling is selected per corpus/profile and published as the executable ceiling.
    ProfileSelected,
    /// A runtime or transport guard with no portable policy field.
    RuntimeGuard,
}

impl LimitClass {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::SchemaExecutableEqual => "schema_executable_equal",
            Self::SchemaBroaderThanExecutable => "schema_broader_than_executable",
            Self::ProfileSelected => "profile_selected",
            Self::RuntimeGuard => "runtime_guard",
        }
    }
}

/// Machine-readable reason a value is not executable on the selected owner/profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// The value is schema-valid for this surface but exceeds the executable ceiling.
    EffectiveLimitExceeded,
    /// The selected profile disables the surface, so no advertised value executes.
    Disabled,
    /// The field is absent from the published record; absent is not unlimited.
    FieldNotAdvertised,
    /// The capability descriptor does not match trusted owner/profile/revision pins.
    DescriptorStale,
    /// The capability descriptor fails its own integrity digest.
    DescriptorTampered,
    /// The record was published for a different surface or owner revision.
    ProfileMismatch,
    /// No consumer pin is recorded for the repository.
    ConsumerNotRecorded,
    /// The recorded consumer pin has not adopted this effective-limit revision.
    ConsumerPinNotAdopted,
}

impl UnavailableReason {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::EffectiveLimitExceeded => "effective_limit_exceeded",
            Self::Disabled => "disabled",
            Self::FieldNotAdvertised => "field_not_advertised",
            Self::DescriptorStale => "descriptor_stale",
            Self::DescriptorTampered => "descriptor_tampered",
            Self::ProfileMismatch => "profile_mismatch",
            Self::ConsumerNotRecorded => "consumer_not_recorded",
            Self::ConsumerPinNotAdopted => "consumer_pin_not_adopted",
        }
    }
}

/// One classified value. `executable_ceiling` is authoritative; the two schema ceilings are
/// published only to make an intentional divergence visible.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitRow {
    /// Field name in the matching capability descriptor's `effective_limits` object.
    pub field: String,
    pub class: LimitClass,
    /// Portable policy-schema maximum; `None` when no policy field exists for the value.
    pub policy_schema_ceiling: Option<u64>,
    /// Maximum the capability schema permits this descriptor to publish.
    pub capabilities_schema_ceiling: u64,
    /// Effective ceiling for the selected owner/profile.
    pub executable_ceiling: u64,
    /// Owner validator that enforces the executable ceiling.
    pub validator: String,
}

impl LimitRow {
    #[must_use]
    pub fn new(
        field: impl Into<String>,
        class: LimitClass,
        policy_schema_ceiling: Option<u64>,
        capabilities_schema_ceiling: u64,
        executable_ceiling: u64,
        validator: impl Into<String>,
    ) -> Self {
        Self {
            field: field.into(),
            class,
            policy_schema_ceiling,
            capabilities_schema_ceiling,
            executable_ceiling,
            validator: validator.into(),
        }
    }

    /// A portable policy field that executes only up to the selected profile ceiling.
    pub fn policy(
        field: impl Into<String>,
        policy_schema_ceiling: u64,
        capabilities_schema_ceiling: u64,
        executable_ceiling: u64,
        validator: impl Into<String>,
    ) -> Self {
        let class = if policy_schema_ceiling == executable_ceiling {
            LimitClass::SchemaExecutableEqual
        } else {
            LimitClass::SchemaBroaderThanExecutable
        };
        Self::new(
            field,
            class,
            Some(policy_schema_ceiling),
            capabilities_schema_ceiling,
            executable_ceiling,
            validator,
        )
    }

    /// A parsed ceiling selected per corpus/profile; the portable policy schema has no such field.
    pub fn profile_selected(
        field: impl Into<String>,
        schema_ceiling: u64,
        executable_ceiling: u64,
        validator: impl Into<String>,
    ) -> Self {
        Self::new(
            field,
            LimitClass::ProfileSelected,
            None,
            schema_ceiling,
            executable_ceiling,
            validator,
        )
    }

    /// A runtime or transport guard with no portable policy field.
    pub fn runtime_guard(
        field: impl Into<String>,
        capabilities_schema_ceiling: u64,
        executable_ceiling: u64,
        validator: impl Into<String>,
    ) -> Self {
        Self::new(
            field,
            LimitClass::RuntimeGuard,
            None,
            capabilities_schema_ceiling,
            executable_ceiling,
            validator,
        )
    }

    fn validate(&self) -> Result<(), LimitRecordError> {
        if !valid_token(&self.field) || self.validator.trim().is_empty() {
            return Err(LimitRecordError::InvalidRecord);
        }
        if self.executable_ceiling == 0
            || self.capabilities_schema_ceiling < self.executable_ceiling
        {
            return Err(LimitRecordError::InvalidRecord);
        }
        match (self.class, self.policy_schema_ceiling) {
            (LimitClass::SchemaExecutableEqual, Some(policy))
                if policy == self.executable_ceiling
                    && self.capabilities_schema_ceiling == self.executable_ceiling => {}
            (LimitClass::SchemaBroaderThanExecutable, Some(policy))
                if policy > self.executable_ceiling
                    && policy >= self.capabilities_schema_ceiling => {}
            (LimitClass::ProfileSelected | LimitClass::RuntimeGuard, None) => {}
            _ => return Err(LimitRecordError::InvalidRecord),
        }
        Ok(())
    }
}

/// Owner-published classification of every advertised value for one surface.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveLimitRecord {
    pub schema: String,
    pub surface: String,
    pub owner: String,
    pub owner_revision: String,
    pub capability_schema: String,
    pub capability_descriptor_sha256: String,
    /// The selected profile currently advertises at least one executable operation/value.
    pub enabled: bool,
    pub rows: Vec<LimitRow>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitRecordError {
    InvalidRecord,
    DuplicateField,
}

impl EffectiveLimitRecord {
    /// Validate the record before any value from it is trusted. Returns [`LimitRecordError`] for a
    /// malformed record, row, or field set.
    pub fn validate(&self) -> Result<(), LimitRecordError> {
        if self.schema != EFFECTIVE_LIMIT_RECORD_SCHEMA
            || !valid_token(&self.surface)
            || !valid_token(&self.owner)
            || !valid_token(&self.owner_revision)
            || !valid_token(&self.capability_schema)
            || !valid_sha256(&self.capability_descriptor_sha256)
            || self.rows.is_empty()
        {
            return Err(LimitRecordError::InvalidRecord);
        }
        let fields = self
            .rows
            .iter()
            .map(|row| row.field.as_str())
            .collect::<BTreeSet<_>>();
        if fields.len() != self.rows.len() {
            return Err(LimitRecordError::DuplicateField);
        }
        for row in &self.rows {
            row.validate()?;
        }
        Ok(())
    }

    #[must_use]
    pub fn row(&self, field: &str) -> Option<&LimitRow> {
        self.rows.iter().find(|row| row.field == field)
    }

    #[must_use]
    pub fn executable_ceiling(&self, field: &str) -> Option<u64> {
        self.row(field).map(|row| row.executable_ceiling)
    }

    /// Selected-profile admissibility for an already-validated record. Returns
    /// [`UnavailableReason`] when disabled, absent, or above the executable ceiling.
    pub fn admit(&self, field: &str, requested: u64) -> Result<(), UnavailableReason> {
        if !self.enabled {
            return Err(UnavailableReason::Disabled);
        }
        let row = self
            .row(field)
            .ok_or(UnavailableReason::FieldNotAdvertised)?;
        if requested > row.executable_ceiling {
            return Err(UnavailableReason::EffectiveLimitExceeded);
        }
        Ok(())
    }

    /// Authenticate the complete record against the record derived from the trusted capability
    /// descriptor. `trusted` must never be derived from, or be a clone of, the record being checked.
    ///
    /// # Errors
    ///
    /// Returns [`UnavailableReason`] on a malformed, mismatched, stale, or tampered record.
    pub fn authenticate(&self, trusted: &EffectiveLimitRecord) -> Result<(), UnavailableReason> {
        trusted
            .validate()
            .map_err(|_| UnavailableReason::DescriptorTampered)?;
        if self.surface != trusted.surface || self.owner_revision != trusted.owner_revision {
            return Err(UnavailableReason::ProfileMismatch);
        }
        if self.capability_descriptor_sha256 != trusted.capability_descriptor_sha256 {
            return Err(UnavailableReason::DescriptorStale);
        }
        if self != trusted {
            return Err(UnavailableReason::DescriptorTampered);
        }
        Ok(())
    }

    /// Admit a value only after authenticating the complete record against the trusted derivation.
    ///
    /// # Errors
    ///
    /// Returns [`UnavailableReason`] for an invalid, mismatched, stale, or oversized request.
    pub fn admit_authorized(
        &self,
        trusted: &EffectiveLimitRecord,
        field: &str,
        requested: u64,
    ) -> Result<(), UnavailableReason> {
        self.authenticate(trusted)?;
        trusted.admit(field, requested)
    }
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
