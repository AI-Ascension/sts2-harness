// SPDX-License-Identifier: MIT

use super::NativeCapabilities;
use super::common::{
    MAX_COMPLETED_TURNS, MAX_HISTORY_TTL_SECONDS, SESSION_POLICY_SCHEMA,
    SESSION_POLICY_SCHEMA_MAX_COMPLETED_TURNS, SESSION_POLICY_SCHEMA_MAX_HISTORY_TTL_SECONDS,
    SessionError, digest, valid_digest, valid_id,
};
use super::policy_migration::{SessionPolicyLimitViolation, TTL_LIMIT, TURN_LIMIT};
use crate::effective_limits::UnavailableReason;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionScope {
    pub project_id: String,
    pub run_id: String,
    pub episode_id: String,
    pub agent_id: String,
}

impl SessionScope {
    pub fn new(
        project_id: impl Into<String>,
        run_id: impl Into<String>,
        episode_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Result<Self, SessionError> {
        let value = Self {
            project_id: project_id.into(),
            run_id: run_id.into(),
            episode_id: episode_id.into(),
            agent_id: agent_id.into(),
        };
        if value.valid() {
            Ok(value)
        } else {
            Err(SessionError::InvalidScope)
        }
    }

    #[must_use]
    pub fn valid(&self) -> bool {
        valid_id(&self.project_id)
            && valid_id(&self.run_id)
            && valid_id(&self.episode_id)
            && valid_id(&self.agent_id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSessionMode {
    Disabled,
    FixtureOnly,
    InspectOnly,
    Enabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuityMode {
    StrictReviewed,
    ObservedPersistent,
}

impl ProviderSessionPolicy {
    /// Limits this saved policy asks for that the selected profile cannot execute.
    ///
    /// Only advertised limits are considered: an absent row is reported by
    /// [`Self::admit_for_profile`] as not advertised rather than being treated as a violation.
    #[must_use]
    pub fn capability_limit_violations(
        &self,
        capabilities: &NativeCapabilities,
    ) -> Vec<SessionPolicyLimitViolation> {
        let record = capabilities.effective_limit_record();
        let mut violations = Vec::new();
        for (field, requested) in [
            (TURN_LIMIT, self.max_completed_turns as u64),
            (TTL_LIMIT, self.history_ttl_seconds),
        ] {
            if let Some(effective) = record.executable_ceiling(field)
                && requested > effective
            {
                violations.push(SessionPolicyLimitViolation {
                    limit: field.to_owned(),
                    requested,
                    effective,
                });
            }
        }
        violations
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransformPolicy {
    DenyAndFence,
    ExplicitBoundedObserved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicy {
    pub schema: String,
    pub policy_id: String,
    pub scope: SessionScope,
    pub version: u64,
    pub mode: ProviderSessionMode,
    pub continuity: ContinuityMode,
    pub credential_realm_ref: String,
    pub profile_sha256: String,
    pub cross_scope_fork: bool,
    pub reconnect_resumes_gameplay: bool,
    pub compaction_generation_permission_required: bool,
    pub max_completed_turns: usize,
    pub history_ttl_seconds: u64,
    pub automatic_transform_policy: TransformPolicy,
    pub epoch: u64,
}

impl ProviderSessionPolicy {
    #[must_use]
    pub fn disabled(scope: SessionScope) -> Self {
        Self {
            schema: SESSION_POLICY_SCHEMA.to_owned(),
            policy_id: "provider-session-disabled".to_owned(),
            scope,
            version: 1,
            mode: ProviderSessionMode::Disabled,
            continuity: ContinuityMode::StrictReviewed,
            credential_realm_ref: "none".to_owned(),
            profile_sha256: digest(b"disabled"),
            cross_scope_fork: false,
            reconnect_resumes_gameplay: false,
            compaction_generation_permission_required: true,
            max_completed_turns: 1,
            history_ttl_seconds: 3600,
            automatic_transform_policy: TransformPolicy::DenyAndFence,
            epoch: 1,
        }
    }

    /// Validate the portable policy contract. The profile-specific runtime ceiling is checked
    /// separately against the selected capability descriptor.
    pub fn validate_schema(&self) -> Result<(), SessionError> {
        if self.schema != SESSION_POLICY_SCHEMA
            || !valid_id(&self.policy_id)
            || !self.scope.valid()
            || self.version == 0
            || !valid_id(&self.credential_realm_ref)
            || (!matches!(self.mode, ProviderSessionMode::Disabled)
                && self.credential_realm_ref == "none")
            || !valid_digest(&self.profile_sha256)
            || self.cross_scope_fork
            || self.reconnect_resumes_gameplay
            || !self.compaction_generation_permission_required
            || !(1..=SESSION_POLICY_SCHEMA_MAX_COMPLETED_TURNS).contains(&self.max_completed_turns)
            || !(1..=SESSION_POLICY_SCHEMA_MAX_HISTORY_TTL_SECONDS)
                .contains(&self.history_ttl_seconds)
            || self.epoch == 0
        {
            return Err(SessionError::InvalidPolicy);
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), SessionError> {
        self.validate_schema()?;
        if self.max_completed_turns > MAX_COMPLETED_TURNS
            || self.history_ttl_seconds > MAX_HISTORY_TTL_SECONDS
        {
            return Err(SessionError::InvalidPolicy);
        }
        Ok(())
    }

    /// Saved-policy admission for the **selected** adapter profile.
    ///
    /// Schema validity ([`Self::validate_schema`]) is the portable-contract check. This answers
    /// the separate question the effective-limit record exists for: can the selected profile
    /// actually execute these saved values? The two outcomes stay distinguishable, and a value is
    /// never clamped to fit — an unsupported policy is refused with a precise reason so a caller
    /// can retain it for inspection and propose a bounded migration instead of silently changing
    /// semantics.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyAdmissionError::Schema`] when the saved policy is not portable-valid, or
    /// [`PolicyAdmissionError::Profile`] when it is schema-valid but the selected profile cannot
    /// execute it (disabled surface, unadvertised field, or above the executable ceiling).
    pub fn admit_for_profile(
        &self,
        capabilities: &NativeCapabilities,
    ) -> Result<(), PolicyAdmissionError> {
        self.validate_schema()
            .map_err(PolicyAdmissionError::Schema)?;
        for (field, requested) in [
            ("max_completed_turns", self.max_completed_turns as u64),
            ("max_history_ttl_seconds", self.history_ttl_seconds),
        ] {
            capabilities
                .admit_policy_value(field, requested)
                .map_err(PolicyAdmissionError::Profile)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn allows_execution(&self) -> bool {
        matches!(
            self.mode,
            ProviderSessionMode::FixtureOnly | ProviderSessionMode::Enabled
        )
    }
}

/// Why a saved provider-session policy was not admitted for the selected profile.
///
/// The variants are deliberately distinct: a portable-contract failure is not the same thing as a
/// selected profile being unable to execute a perfectly valid policy, and neither may be reported
/// as a silent clamp or a generic invalid-policy error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyAdmissionError {
    /// The saved policy does not satisfy the portable policy contract.
    Schema(SessionError),
    /// The policy is schema-valid but the selected profile cannot execute it.
    Profile(UnavailableReason),
}

impl PolicyAdmissionError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Schema(_) => "provider_session_policy_schema_invalid",
            Self::Profile(reason) => reason.code(),
        }
    }
}
