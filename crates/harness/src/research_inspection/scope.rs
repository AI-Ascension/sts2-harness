// SPDX-License-Identifier: MIT

//! The deployment-supplied research grant and the target it binds.
//!
//! A grant is admitted by the operator, not by the request. It names one exact
//! checkpoint, run and branch, one consumer lane, and the field groups that lane
//! may read; a read that names a different checkpoint, branch or group is
//! refused. Because the scope lives in the grant rather than in the request, a
//! gameplay lane cannot escalate by asking for a different visibility parameter.
//!
//! Revocation is monotonic: a revoked grant is refused, and re-admitting the
//! same identity cannot bring it back, so a cached or replayed request cannot
//! outlive the approval that authorized it.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{ResearchFieldGroup, ResearchInspectionError};

/// The research-inspection schema version this harness implements.
pub const RESEARCH_INSPECTION_SCHEMA_VERSION: &str = "ascension.research-inspection/v1";

/// Maximum field groups one grant may admit.
pub const MAX_RESEARCH_GRANTS: usize = 5;

/// Maximum bytes of a research identity component.
pub const MAX_RESEARCH_IDENTITY_BYTES: usize = 128;

/// The exact checkpoint, run and branch a grant binds to.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchTargetBinding {
    /// The exact checkpoint the research read may inspect.
    pub checkpoint_id: String,
    /// The run the checkpoint belongs to.
    pub run_id: String,
    /// The branch the checkpoint belongs to; never another branch's state.
    pub branch_id: String,
}

impl ResearchTargetBinding {
    /// Validates every identity component's shape.
    pub fn validate(&self) -> Result<(), ResearchInspectionError> {
        for value in [&self.checkpoint_id, &self.run_id, &self.branch_id] {
            if !is_research_identity(value) {
                return Err(ResearchInspectionError::InvalidScope);
            }
        }
        Ok(())
    }
}

/// Whether a value is a usable research identity component.
///
/// Portable identifiers only: no path separator, no URL scheme, no host path and
/// no whitespace, so a grant cannot name a file or an external object.
#[must_use]
pub fn is_research_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_RESEARCH_IDENTITY_BYTES
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains("://")
        && !value.contains("..")
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
}

/// Why a research grant could not be admitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResearchScopeRefusal {
    /// The target binding was unusable.
    InvalidTarget,
    /// The consumer lane identity was unusable.
    InvalidConsumer,
    /// The grant admitted no field groups, or more than the declared maximum.
    InvalidGroups,
}

/// An operator-admitted, revocable scope for one research consumer lane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResearchInspectionGrant {
    research_id: String,
    target: ResearchTargetBinding,
    consumer_lane: String,
    groups: BTreeSet<ResearchFieldGroup>,
    revoked: bool,
}

impl ResearchInspectionGrant {
    /// Admits a grant for one consumer lane, over explicit field groups only.
    ///
    /// Refuses an unusable target, an unusable consumer identity, and an empty or
    /// oversized group set. The groups are never widened here: what the operator
    /// admitted is exactly what a read may reach.
    pub fn admit(
        research_id: impl Into<String>,
        target: ResearchTargetBinding,
        consumer_lane: impl Into<String>,
        groups: impl IntoIterator<Item = ResearchFieldGroup>,
    ) -> Result<Self, ResearchScopeRefusal> {
        let research_id = research_id.into();
        let consumer_lane = consumer_lane.into();
        if !is_research_identity(&research_id) || target.validate().is_err() {
            return Err(ResearchScopeRefusal::InvalidTarget);
        }
        if !is_research_identity(&consumer_lane) {
            return Err(ResearchScopeRefusal::InvalidConsumer);
        }
        let groups: BTreeSet<ResearchFieldGroup> = groups.into_iter().collect();
        if groups.is_empty() || groups.len() > MAX_RESEARCH_GRANTS {
            return Err(ResearchScopeRefusal::InvalidGroups);
        }
        Ok(Self {
            research_id,
            target,
            consumer_lane,
            groups,
            revoked: false,
        })
    }

    /// The admitted research identity.
    #[must_use]
    pub fn research_id(&self) -> &str {
        &self.research_id
    }

    /// The exact checkpoint, run and branch this grant binds.
    #[must_use]
    pub fn target(&self) -> &ResearchTargetBinding {
        &self.target
    }

    /// The one consumer lane this grant was admitted for.
    #[must_use]
    pub fn consumer_lane(&self) -> &str {
        &self.consumer_lane
    }

    /// Whether this grant has been revoked.
    #[must_use]
    pub const fn is_revoked(&self) -> bool {
        self.revoked
    }

    /// Whether `group` is inside the admitted scope.
    #[must_use]
    pub fn admits(&self, group: ResearchFieldGroup) -> bool {
        !self.revoked && self.groups.contains(&group)
    }

    /// The admitted groups, in stable label order.
    #[must_use]
    pub fn groups(&self) -> Vec<ResearchFieldGroup> {
        self.groups.iter().copied().collect()
    }

    /// Revokes this grant. Revocation is monotonic and never fails.
    pub fn revoke(&mut self) {
        self.revoked = true;
    }

    /// Checks that a read names this grant's exact target and consumer lane.
    ///
    /// Refusing here is what keeps one branch's hidden state out of another
    /// branch's read: the target is compared component by component, so a
    /// different checkpoint, run or branch is `WrongTarget`, and a lane that is
    /// not the admitted consumer is `RevokedScope` rather than a silent success.
    pub fn authorizes(
        &self,
        target: &ResearchTargetBinding,
        consumer_lane: &str,
    ) -> Result<(), ResearchInspectionError> {
        if self.revoked || consumer_lane != self.consumer_lane {
            return Err(ResearchInspectionError::RevokedScope);
        }
        if target != &self.target {
            return Err(ResearchInspectionError::WrongTarget);
        }
        Ok(())
    }
}
