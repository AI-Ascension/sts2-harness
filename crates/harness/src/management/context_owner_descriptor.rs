// SPDX-License-Identifier: MIT

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextBindingDescriptor {
    pub schema_version: String,
    pub binding_id: String,
    pub version: u64,
    pub digest: String,
    pub context_ref: String,
    pub node_kinds: Vec<String>,
    pub sources: Vec<ContextBindingSource>,
    pub operations: Vec<ContextBindingOperation>,
    pub effective_limits: ContextEffectiveLimits,
    pub continuity: ContextBindingContinuity,
    pub grants: ContextBindingGrants,
    pub state: ContextBindingState,
}

impl ContextBindingDescriptor {
    pub fn seal(mut self) -> Result<Self, ManagementError> {
        self.digest.clear();
        let bytes = serde_json::to_vec(&self).map_err(|error| {
            ManagementError::invalid("context_binding_encode", error.to_string())
        })?;
        self.digest = sha256_hex(bytes);
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), ManagementError> {
        if self.schema_version != CONTEXT_OWNER_BINDING_SCHEMA_VERSION
            || self.version == 0
            || self.node_kinds.is_empty()
            || self.node_kinds.len() > MAX_CONTEXT_NODE_KINDS
            || self.sources.len() > MAX_CONTEXT_SOURCES
            || self.operations.len() > MAX_CONTEXT_OPERATIONS
        {
            return Err(ManagementError::invalid(
                "context_binding_descriptor_invalid",
                "context binding descriptor is outside its bounds",
            ));
        }
        for (field, value) in [
            ("context_binding_id", self.binding_id.as_str()),
            ("context_ref", self.context_ref.as_str()),
        ] {
            validate_identifier(field, value)?;
        }
        validate_digest("context_binding_digest", &self.digest)?;
        let mut seen = std::collections::BTreeSet::new();
        for kind in &self.node_kinds {
            validate_identifier("context_binding_node_kind", kind)?;
            if !seen.insert(kind) {
                return Err(ManagementError::invalid(
                    "context_binding_duplicate_node_kind",
                    "context binding node kinds must be unique",
                ));
            }
        }
        let mut seen_sources = std::collections::BTreeSet::new();
        for source in &self.sources {
            validate_identifier("context_binding_source_id", &source.source_id)?;
            if source.version == 0 {
                return Err(ManagementError::invalid(
                    "context_binding_source_version",
                    "context binding source version must be positive",
                ));
            }
            validate_digest("context_binding_source_digest", &source.digest)?;
            if !seen_sources.insert((&source.source_id, source.version)) {
                return Err(ManagementError::invalid(
                    "context_binding_duplicate_source",
                    "context binding sources must be unique",
                ));
            }
        }
        let mut seen_operations = std::collections::BTreeSet::new();
        for operation in &self.operations {
            if !seen_operations.insert(operation) {
                return Err(ManagementError::invalid(
                    "context_binding_duplicate_operation",
                    "context binding operations must be unique",
                ));
            }
        }
        validate_limits(&self.effective_limits)?;
        validate_grants(&self.grants)?;
        if matches!(self.state, ContextBindingState::Available)
            && self.operations.is_empty()
            && self.grants.control
        {
            return Err(ManagementError::invalid(
                "context_binding_control_without_operations",
                "a controllable binding must advertise at least one operation",
            ));
        }
        // Only hash after every nested field has been validated and bounded.
        let expected = self.clone().seal()?.digest;
        if expected != self.digest {
            return Err(ManagementError::conflict(
                "context_binding_digest_mismatch",
                "context binding descriptor digest does not match its immutable fields",
            ));
        }
        Ok(())
    }

    /// Returns whether this immutable descriptor can be used by the named
    /// workflow node kind. A disabled or stale descriptor is never a live
    /// binding, even when its metadata remains discoverable.
    pub fn supports(&self, context_ref: &str, node_kind: &str) -> bool {
        self.context_ref == context_ref
            && self.node_kinds.iter().any(|kind| kind == node_kind)
            && matches!(self.state, ContextBindingState::Available)
    }
}
