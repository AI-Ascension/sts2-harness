// SPDX-License-Identifier: MIT

//! Raw and semantic legal-catalog identity kept in distinct versioned fields.
//!
//! The existing raw catalog digest is a byte-level proof over the exact host-emitted legal-action
//! array. It must survive transport, checkpoint records, rebinding, and redaction unchanged. A
//! semantic catalog identity is a separate value with its own versioned namespace, derived from
//! stable engine command, parameter, and target identities. Keeping them distinct means a rebind or
//! projection can never silently rewrite the raw proof, and neither value can be mistaken for the
//! other.

use std::fmt;

/// Serialized prefix of the existing raw legal-action array digest.
pub const RAW_CATALOG_PREFIX: &str = "sha256:";
/// Serialized prefix of the new semantic catalog identity.
pub const SEMANTIC_CATALOG_PREFIX: &str = "asc-catalog:v1:sha256:";
/// Maximum accepted semantic catalog schema identifier length.
pub const MAX_CATALOG_SCHEMA_BYTES: usize = 128;

/// Rejection reasons for catalog identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogError {
    /// The raw digest is not `sha256:` followed by 64 lowercase hex characters.
    InvalidRawDigest,
    /// The semantic digest is not `asc-catalog:v1:sha256:` followed by 64 lowercase hex characters.
    InvalidSemanticDigest,
    /// The semantic catalog schema identifier is empty or too long.
    InvalidSchema,
    /// A caller tried to replace the raw digest instead of preserving it.
    RawDigestRewritten,
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRawDigest => "raw catalog digest is invalid",
            Self::InvalidSemanticDigest => "semantic catalog digest is invalid",
            Self::InvalidSchema => "semantic catalog schema is invalid",
            Self::RawDigestRewritten => "raw catalog digest must be preserved unchanged",
        })
    }
}

impl std::error::Error for CatalogError {}

/// The existing raw legal-action array digest, preserved verbatim.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RawCatalogDigest(String);

impl RawCatalogDigest {
    /// Parses a raw digest without reinterpreting its meaning.
    pub fn parse(value: &str) -> Result<Self, CatalogError> {
        if !valid_body(value, RAW_CATALOG_PREFIX) {
            return Err(CatalogError::InvalidRawDigest);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the serialized digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A separate, versioned semantic catalog identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SemanticCatalogDigest(String);

impl SemanticCatalogDigest {
    /// Parses a semantic catalog identity.
    pub fn parse(value: &str) -> Result<Self, CatalogError> {
        if !valid_body(value, SEMANTIC_CATALOG_PREFIX) {
            return Err(CatalogError::InvalidSemanticDigest);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the serialized digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Both catalog identities for one boundary, in distinct versioned fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogIdentity {
    /// Raw host-emitted array digest; never rewritten by this type.
    pub raw: RawCatalogDigest,
    /// Semantic catalog identity with its own namespace.
    pub semantic: SemanticCatalogDigest,
    /// Versioned schema/contract identifier for the semantic identity.
    pub semantic_schema: String,
}

impl CatalogIdentity {
    /// Creates an identity, validating both namespaces and the schema label.
    pub fn new(
        raw: RawCatalogDigest,
        semantic: SemanticCatalogDigest,
        semantic_schema: &str,
    ) -> Result<Self, CatalogError> {
        if semantic_schema.is_empty()
            || semantic_schema.len() > MAX_CATALOG_SCHEMA_BYTES
            || semantic_schema.contains('\0')
        {
            return Err(CatalogError::InvalidSchema);
        }
        Ok(Self {
            raw,
            semantic,
            semantic_schema: semantic_schema.to_owned(),
        })
    }

    /// Confirms that a stored raw digest is exactly the one carried here.
    pub fn preserves_raw(&self, stored: &RawCatalogDigest) -> bool {
        self.raw == *stored
    }

    /// Refuses a rebind that would change the raw digest.
    pub fn rebind(&self, stored_raw: &RawCatalogDigest) -> Result<Self, CatalogError> {
        if !self.preserves_raw(stored_raw) {
            return Err(CatalogError::RawDigestRewritten);
        }
        Ok(self.clone())
    }

    /// Reports whether the two identities occupy distinct namespaces.
    #[must_use]
    pub fn namespaces_are_distinct(&self) -> bool {
        !self.raw.as_str().starts_with(SEMANTIC_CATALOG_PREFIX)
            && !self.semantic.as_str().starts_with(RAW_CATALOG_PREFIX)
    }
}

fn valid_body(value: &str, prefix: &str) -> bool {
    let Some(hex) = value.strip_prefix(prefix) else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
