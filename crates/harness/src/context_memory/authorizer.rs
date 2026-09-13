// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum MemoryRole {
    Read,
    Search,
    Generate,
    Review,
    Select,
    Policy,
    Revoke,
    Control,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryAuthorizer {
    grants: BTreeMap<String, BTreeSet<MemoryRole>>,
}

impl MemoryAuthorizer {
    pub fn new() -> Self {
        Self {
            grants: BTreeMap::new(),
        }
    }

    pub fn grant(
        &mut self,
        principal: impl Into<String>,
        role: MemoryRole,
    ) -> Result<(), MemoryError> {
        let principal = principal.into();
        if !valid_id(&principal) {
            return Err(MemoryError::PermissionDenied);
        }
        self.grants.entry(principal).or_default().insert(role);
        Ok(())
    }

    pub fn check(&self, principal: &str, role: MemoryRole) -> Result<(), MemoryError> {
        if self
            .grants
            .get(principal)
            .is_some_and(|roles| roles.contains(&role))
        {
            Ok(())
        } else {
            Err(MemoryError::PermissionDenied)
        }
    }
}

impl Default for MemoryAuthorizer {
    fn default() -> Self {
        Self::new()
    }
}
