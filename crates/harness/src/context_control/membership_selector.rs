// SPDX-License-Identifier: MIT

//! The owner-configured half of a per-invocation membership policy.
//!
//! Everything here is independent of the invocation identity; [`bind`](ContextMembershipSelector::bind)
//! mints the finished, versioned policy for one invocation of one draft revision.

use super::{
    CONTEXT_MEMBERSHIP_POLICY_SCHEMA, ContextMembershipBroaderScope, ContextMembershipError,
    ContextMembershipPolicy, ContextModelView, MembershipDisposition,
};
use crate::context_control::types::ContextItemRef;
use crate::sha256_hex;
use serde::{Deserialize, Serialize};

/// The owner-configured half of a membership policy.
///
/// A selector is everything that does not depend on the invocation identity: the disposition, the
/// node's own overrides, pin inheritance, wider scope, and the model view.
/// [`ContextMembershipSelector::bind`] mints the versioned policy for one invocation, so a node
/// default or override cannot silently carry another invocation's identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextMembershipSelector {
    pub disposition: MembershipDisposition,
    #[serde(default)]
    pub overrides: Vec<ContextItemRef>,
    #[serde(default)]
    pub inherit_pins: bool,
    #[serde(default)]
    pub broader_scope: ContextMembershipBroaderScope,
    pub model_view: ContextModelView,
}

impl ContextMembershipSelector {
    /// An `include` selector that keeps the draft selection and a visible observation.
    #[must_use]
    pub fn include() -> Self {
        Self {
            disposition: MembershipDisposition::Include,
            overrides: Vec::new(),
            inherit_pins: false,
            broader_scope: ContextMembershipBroaderScope::default(),
            model_view: ContextModelView::visible(),
        }
    }

    /// Mints the versioned policy for one invocation of one draft revision.
    #[must_use]
    pub fn bind(&self, invocation_id: &str, base_revision_id: &str) -> ContextMembershipPolicy {
        ContextMembershipPolicy {
            schema: CONTEXT_MEMBERSHIP_POLICY_SCHEMA.to_owned(),
            invocation_id: invocation_id.to_owned(),
            base_revision_id: base_revision_id.to_owned(),
            disposition: self.disposition,
            overrides: self.overrides.clone(),
            inherit_pins: self.inherit_pins,
            broader_scope: self.broader_scope.clone(),
            model_view: self.model_view,
        }
    }

    /// Stable digest over the canonical encoding of this selector.
    ///
    /// The owner records this digest on the render source identity, so the live fence re-derives
    /// the identity from the current configuration before and after inference and refuses when the
    /// digest moved — a selector changed mid-inference is refused exactly like a changed source.
    /// The digest is computed once per resolution, not re-encoded inside the comparison.
    pub fn digest(&self) -> Result<String, ContextMembershipError> {
        let encoded = serde_json::to_vec(self).map_err(|_| ContextMembershipError::Encode)?;
        Ok(sha256_hex(&encoded))
    }
}
