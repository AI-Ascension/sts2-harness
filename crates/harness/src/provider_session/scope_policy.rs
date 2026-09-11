// SPDX-License-Identifier: MIT

use super::common::{
    MAX_COMPLETED_TURNS, MAX_HISTORY_TTL_SECONDS, SESSION_POLICY_SCHEMA, SessionError, digest,
    valid_digest, valid_id,
};
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

    pub fn validate(&self) -> Result<(), SessionError> {
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
            || !(1..=MAX_COMPLETED_TURNS).contains(&self.max_completed_turns)
            || !(1..=MAX_HISTORY_TTL_SECONDS).contains(&self.history_ttl_seconds)
            || self.epoch == 0
        {
            return Err(SessionError::InvalidPolicy);
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
