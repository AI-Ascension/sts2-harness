// SPDX-License-Identifier: MIT

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextBindingCatalog {
    pub schema_version: String,
    pub owner_id: String,
    pub owner_version: String,
    pub catalog_digest: String,
    pub descriptors: Vec<ContextBindingDescriptor>,
}

impl ContextBindingCatalog {
    /// Seals the existing v1 catalog encoding without changing its descriptors.
    /// Call `validate` separately; a self-consistent digest is not owner authority.
    pub fn seal(mut self) -> Result<Self, ManagementError> {
        self.catalog_digest =
            catalog_digest(&self.owner_id, &self.owner_version, &self.descriptors)?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), ManagementError> {
        if self.schema_version != CONTEXT_OWNER_CATALOG_SCHEMA_VERSION
            || self.descriptors.len() > MAX_CONTEXT_BINDINGS
        {
            return Err(ManagementError::invalid(
                "context_binding_catalog_invalid",
                "context binding catalog is outside its bounds",
            ));
        }
        validate_identifier("context_owner_id", &self.owner_id)?;
        validate_identifier("context_owner_version", &self.owner_version)?;
        validate_digest("context_catalog_digest", &self.catalog_digest)?;
        let mut identities = std::collections::BTreeSet::new();
        for descriptor in &self.descriptors {
            descriptor.validate()?;
            if !identities.insert((&descriptor.binding_id, descriptor.version)) {
                return Err(ManagementError::conflict(
                    "context_binding_duplicate",
                    "context binding IDs and versions must be unique",
                ));
            }
        }
        let expected = catalog_digest(&self.owner_id, &self.owner_version, &self.descriptors)?;
        if expected != self.catalog_digest {
            return Err(ManagementError::conflict(
                "context_catalog_digest_mismatch",
                "context binding catalog digest does not match its descriptors",
            ));
        }
        Ok(())
    }

    pub fn descriptor_for(
        &self,
        context_ref: &str,
        node_kind: &str,
    ) -> Result<&ContextBindingDescriptor, ManagementError> {
        validate_identifier("context_ref", context_ref)?;
        validate_identifier("context_node_kind", node_kind)?;
        let mut matches = self
            .descriptors
            .iter()
            .filter(|descriptor| descriptor.supports(context_ref, node_kind));
        let Some(descriptor) = matches.next() else {
            return Err(ManagementError::capability(
                "context_binding_unsupported",
                "context owner catalog does not advertise a usable binding for this node",
            ));
        };
        if matches.next().is_some() {
            return Err(ManagementError::conflict(
                "context_binding_ambiguous",
                "context owner catalog advertises multiple bindings for this node",
            ));
        }
        Ok(descriptor)
    }
}
