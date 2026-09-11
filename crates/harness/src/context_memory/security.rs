// SPDX-License-Identifier: MIT

// Scoped least-privilege grants and inert source envelopes for provider input.

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ScopedGrant {
    scope: MemoryScope,
    role: MemoryRole,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopedMemoryAuthorizer {
    grants: BTreeMap<String, BTreeSet<ScopedGrant>>,
}

impl ScopedMemoryAuthorizer {
    pub fn new() -> Self {
        Self {
            grants: BTreeMap::new(),
        }
    }

    pub fn grant(
        &mut self,
        principal: impl Into<String>,
        scope: MemoryScope,
        role: MemoryRole,
    ) -> Result<(), MemoryError> {
        let principal = principal.into();
        if !valid_id(&principal) || !scope.valid() {
            return Err(MemoryError::PermissionDenied);
        }
        self.grants
            .entry(principal)
            .or_default()
            .insert(ScopedGrant { scope, role });
        Ok(())
    }

    pub fn check(
        &self,
        principal: &str,
        scope: &MemoryScope,
        role: MemoryRole,
    ) -> Result<(), MemoryError> {
        if self.grants.get(principal).is_some_and(|grants| {
            grants.iter().any(|grant| grant.scope == *scope && grant.role == role)
        }) {
            Ok(())
        } else {
            Err(MemoryError::PermissionDenied)
        }
    }
}

impl Default for ScopedMemoryAuthorizer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InertSourceEnvelope {
    pub schema: String,
    pub role: String,
    pub source: MemoryRef,
    pub content: String,
}

pub fn inert_source_envelope(
    source: MemoryRef,
    bytes: &[u8],
) -> Result<InertSourceEnvelope, MemoryError> {
    if !source.valid() {
        return Err(MemoryError::InvalidEntry);
    }
    let content = std::str::from_utf8(bytes).map_err(|_| MemoryError::InvalidEntry)?;
    if content.len() > MAX_SOURCE_BYTES {
        return Err(MemoryError::Capacity);
    }
    Ok(InertSourceEnvelope {
        schema: "ascension.context-memory.source-envelope.v1".to_owned(),
        role: "data".to_owned(),
        source,
        content: content.to_owned(),
    })
}
