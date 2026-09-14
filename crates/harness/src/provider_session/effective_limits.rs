// SPDX-License-Identifier: MIT

use super::{
    MAX_CANDIDATES, MAX_COMPLETED_TURNS, MAX_DEPENDENCIES, MAX_EVENTS, MAX_FRAME_BYTES,
    MAX_HISTORY_BYTES, MAX_HISTORY_TTL_SECONDS, MAX_JSON_DEPTH, MAX_MAINTENANCE_JOBS,
    MAX_METHOD_BYTES, MAX_OPERATIONS, MAX_OUTPUT_SCHEMA_BYTES, MAX_PREPARED, MAX_PREPARED_BYTES,
    MAX_SESSION_ITEMS, MAX_SUFFIX_BYTES, NativeCapabilities,
    SESSION_POLICY_SCHEMA_MAX_COMPLETED_TURNS, SESSION_POLICY_SCHEMA_MAX_HISTORY_TTL_SECONDS,
};
use crate::effective_limits::{
    EFFECTIVE_LIMIT_RECORD_SCHEMA, EffectiveLimitRecord, LimitRow, UnavailableReason,
};

const SESSION_POLICY_LIMIT_VALIDATOR: &str =
    "ProviderSessionPolicy::validate_schema+ProviderSessionBroker::new";

impl NativeCapabilities {
    /// Publish the schema-valid versus executable classification for every advertised value.
    ///
    /// A policy value can be valid against the portable schema and still not execute on the
    /// selected adapter profile. Consumers must read the executable ceiling from this record
    /// instead of copying a schema maximum into a form.
    #[must_use]
    pub fn effective_limit_record(&self) -> EffectiveLimitRecord {
        let limits = &self.effective_limits;
        let rows = vec![
            LimitRow::policy(
                "max_completed_turns",
                SESSION_POLICY_SCHEMA_MAX_COMPLETED_TURNS as u64,
                MAX_COMPLETED_TURNS as u64,
                limits.max_completed_turns as u64,
                SESSION_POLICY_LIMIT_VALIDATOR,
            ),
            LimitRow::policy(
                "max_history_ttl_seconds",
                SESSION_POLICY_SCHEMA_MAX_HISTORY_TTL_SECONDS,
                MAX_HISTORY_TTL_SECONDS,
                limits.max_history_ttl_seconds,
                SESSION_POLICY_LIMIT_VALIDATOR,
            ),
            LimitRow::runtime_guard(
                "max_session_items",
                MAX_SESSION_ITEMS as u64,
                limits.max_session_items as u64,
                "ProviderSessionBroker",
            ),
            LimitRow::runtime_guard(
                "max_dependencies",
                MAX_DEPENDENCIES as u64,
                limits.max_dependencies as u64,
                "ProviderSessionBroker",
            ),
            LimitRow::runtime_guard(
                "max_events",
                MAX_EVENTS as u64,
                limits.max_events as u64,
                "ProviderSessionBroker",
            ),
            LimitRow::runtime_guard(
                "max_operations",
                MAX_OPERATIONS as u64,
                limits.max_operations as u64,
                "ProviderSessionBroker",
            ),
            LimitRow::runtime_guard(
                "max_prepared",
                MAX_PREPARED as u64,
                limits.max_prepared as u64,
                "ProviderSessionBroker",
            ),
            LimitRow::runtime_guard(
                "max_candidates",
                MAX_CANDIDATES as u64,
                limits.max_candidates as u64,
                "ProviderSessionBroker",
            ),
            LimitRow::runtime_guard(
                "max_maintenance_jobs",
                MAX_MAINTENANCE_JOBS as u64,
                limits.max_maintenance_jobs as u64,
                "ProviderSessionBroker",
            ),
            LimitRow::runtime_guard(
                "max_frame_bytes",
                MAX_FRAME_BYTES as u64,
                limits.max_frame_bytes as u64,
                "NativeTransport",
            ),
            LimitRow::runtime_guard(
                "max_history_bytes",
                MAX_HISTORY_BYTES as u64,
                limits.max_history_bytes as u64,
                "ProviderSessionBroker",
            ),
            LimitRow::runtime_guard(
                "max_prepared_bytes",
                MAX_PREPARED_BYTES as u64,
                limits.max_prepared_bytes as u64,
                "ProviderSessionBroker",
            ),
            LimitRow::runtime_guard(
                "max_suffix_bytes",
                MAX_SUFFIX_BYTES as u64,
                limits.max_suffix_bytes as u64,
                "NativeTransport",
            ),
            LimitRow::runtime_guard(
                "max_output_schema_bytes",
                MAX_OUTPUT_SCHEMA_BYTES as u64,
                limits.max_output_schema_bytes as u64,
                "NativeTransport",
            ),
            LimitRow::runtime_guard(
                "max_method_bytes",
                MAX_METHOD_BYTES as u64,
                limits.max_method_bytes as u64,
                "NativeFrame parse",
            ),
            LimitRow::runtime_guard(
                "max_json_depth",
                MAX_JSON_DEPTH as u64,
                limits.max_json_depth as u64,
                "NativeFrame parse",
            ),
        ];
        EffectiveLimitRecord {
            schema: EFFECTIVE_LIMIT_RECORD_SCHEMA.to_owned(),
            surface: "provider-session".to_owned(),
            owner: self.binding.owner.clone(),
            owner_revision: self.binding.owner_revision.clone(),
            capability_schema: self.schema.clone(),
            capability_descriptor_sha256: self.binding.descriptor_sha256.clone(),
            enabled: !self.enabled_methods.is_empty(),
            rows,
        }
    }

    /// Selected-profile admission for a session policy value. Schema validity is checked separately
    /// by [`ProviderSessionPolicy::validate_schema`]; this answers only whether the selected
    /// adapter profile can execute it.
    ///
    /// # Errors
    ///
    /// Returns [`UnavailableReason`] when the profile is disabled, the field is not advertised, or
    /// the value exceeds the executable ceiling.
    pub fn admit_policy_value(&self, field: &str, requested: u64) -> Result<(), UnavailableReason> {
        self.effective_limit_record().admit(field, requested)
    }

    /// Admit a value only after authenticating the record against the derivation of this trusted
    /// capability descriptor.
    ///
    /// # Errors
    ///
    /// Returns [`UnavailableReason`] when the record was tampered, is stale, targets another
    /// surface, or the value exceeds the executable ceiling.
    pub fn admit_authorized_record(
        &self,
        record: &EffectiveLimitRecord,
        field: &str,
        requested: u64,
    ) -> Result<(), UnavailableReason> {
        record.admit_authorized(&self.effective_limit_record(), field, requested)
    }
}
