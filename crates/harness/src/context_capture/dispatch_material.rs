// SPDX-License-Identifier: MIT

//! Immutable prepared application input and its observable manifest.
//!
//! A prepared input is the application-controlled material an adapter must write unchanged.  Its
//! manifest is a canonical digest over the ordered components, so the approved record and a
//! recording write port compare one shared value instead of two independent serializations.

use super::dispatch_error::DispatchError;
use super::dispatch_support::{AdapterSupport, EffectiveContextClaim, adapter_support};
use super::{
    CaptureBoundary, CaptureComponent, CaptureComponentKind, MAX_CAPTURE_BYTES, valid_identity,
};
use sha2::{Digest, Sha256};

/// Maximum number of byte components in one prepared input.
pub const MAX_PREPARED_COMPONENTS: usize = 8;

/// Maximum application bytes in one prepared input.
pub const MAX_PREPARED_BYTES: usize = MAX_CAPTURE_BYTES;

/// Maximum length of a component media type.
pub const MAX_MEDIA_TYPE_BYTES: usize = 128;

const MANIFEST_DOMAIN: &[u8] = b"ascension.prepared-dispatch-manifest.v1\0";
const MATERIAL_DOMAIN: &[u8] = b"ascension.prepared-dispatch-material.v1\0";

fn absorb(hasher: &mut Sha256, field: &[u8]) {
    hasher.update((field.len() as u64).to_be_bytes());
    hasher.update(field);
}

/// One ordered manifest entry.  The approved input and a consumer verifying a recording port both
/// build these, so the two manifests are directly comparable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryManifestEntry {
    /// Application component kind.
    pub kind: CaptureComponentKind,
    /// Position of this component within the boundary.
    pub ordinal: u16,
    /// Media type of the exact bytes.
    pub media_type: String,
    /// Exact byte length of the component.
    pub observed_bytes: usize,
    /// Lowercase hexadecimal digest of the component bytes.
    pub sha256: String,
}

/// Canonical manifest digest over one binding and its ordered entries.
pub fn manifest_sha256(
    adapter_id: &str,
    execution_id: &str,
    attempt_id: Option<&str>,
    boundary: CaptureBoundary,
    entries: &[BoundaryManifestEntry],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(MANIFEST_DOMAIN);
    absorb(&mut hasher, adapter_id.as_bytes());
    absorb(&mut hasher, execution_id.as_bytes());
    absorb(
        &mut hasher,
        attempt_id.unwrap_or("<unavailable>").as_bytes(),
    );
    absorb(&mut hasher, boundary.as_str().as_bytes());
    hasher.update((entries.len() as u64).to_be_bytes());
    for entry in entries {
        hasher.update(entry.ordinal.to_be_bytes());
        absorb(&mut hasher, entry.kind.as_str().as_bytes());
        absorb(&mut hasher, entry.media_type.as_bytes());
        hasher.update((entry.observed_bytes as u64).to_be_bytes());
        absorb(&mut hasher, entry.sha256.as_bytes());
    }
    crate::hex_bytes(hasher.finalize())
}

/// Canonical digest over the exact approved material itself: the ordered component bytes.
pub fn material_sha256(chunks: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(MATERIAL_DOMAIN);
    hasher.update((chunks.len() as u64).to_be_bytes());
    for chunk in chunks {
        hasher.update((chunk.len() as u64).to_be_bytes());
        hasher.update(chunk);
    }
    crate::hex_bytes(hasher.finalize())
}

/// One exact byte component of an approved prepared input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedBoundaryComponent {
    /// Application component kind.
    pub kind: CaptureComponentKind,
    /// Position of this component within the boundary.
    pub ordinal: u16,
    /// Media type of the exact bytes.
    pub media_type: String,
    /// Exact byte length of the component.
    pub observed_bytes: usize,
    /// Lowercase hexadecimal digest of the component bytes.
    pub sha256: String,
    bytes: Vec<u8>,
}

impl PreparedBoundaryComponent {
    /// The exact bytes an adapter must write.  These are application bytes only and never
    /// provider-internal content.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The manifest entry a recording write port must reproduce.
    pub fn entry(&self) -> BoundaryManifestEntry {
        BoundaryManifestEntry {
            kind: self.kind,
            ordinal: self.ordinal,
            media_type: self.media_type.clone(),
            observed_bytes: self.observed_bytes,
            sha256: self.sha256.clone(),
        }
    }
}

/// Immutable application-boundary material prepared for one reserved invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedApplicationInput {
    /// Adapter id the material was prepared for.
    pub adapter_id: String,
    /// Execution identity of the invocation that owns this material.
    pub execution_id: String,
    /// Attempt identity within the execution, when the caller reserved one.
    pub attempt_id: Option<String>,
    /// Exact application boundary the adapter writes.
    pub boundary: CaptureBoundary,
    /// Digest of the ordered component bytes.
    pub approved_material_sha256: String,
    /// Digest of the ordered manifest.
    pub manifest_sha256: String,
    components: Vec<PreparedBoundaryComponent>,
}

impl PreparedApplicationInput {
    /// Prepares immutable bytes for one advertised exact adapter.
    ///
    /// # Errors
    ///
    /// Returns [`DispatchError::UnsupportedAdapter`] when the adapter has no exact application
    /// boundary, and binding or material errors for invalid identities, unordered ordinals,
    /// empty media types and oversized components.
    pub fn prepare(
        adapter_id: &str,
        execution_id: &str,
        attempt_id: Option<&str>,
        components: &[CaptureComponent<'_>],
    ) -> Result<Self, DispatchError> {
        let AdapterSupport::Exact { boundary } = adapter_support(adapter_id) else {
            return Err(DispatchError::UnsupportedAdapter);
        };
        if !valid_identity(adapter_id)
            || !valid_identity(execution_id)
            || attempt_id.is_some_and(|attempt| !valid_identity(attempt))
        {
            return Err(DispatchError::InvalidBinding);
        }
        if components.is_empty() || components.len() > MAX_PREPARED_COMPONENTS {
            return Err(DispatchError::InvalidMaterial);
        }
        let mut prepared = Vec::with_capacity(components.len());
        let mut total_bytes = 0usize;
        let mut previous_ordinal: Option<u16> = None;
        for component in components {
            if component.bytes.len() > MAX_CAPTURE_BYTES {
                return Err(DispatchError::TooLarge);
            }
            if component.media_type.is_empty() || component.media_type.len() > MAX_MEDIA_TYPE_BYTES
            {
                return Err(DispatchError::InvalidMaterial);
            }
            if previous_ordinal.is_some_and(|previous| component.ordinal <= previous) {
                return Err(DispatchError::InvalidMaterial);
            }
            total_bytes = total_bytes.saturating_add(component.bytes.len());
            if total_bytes > MAX_PREPARED_BYTES {
                return Err(DispatchError::TooLarge);
            }
            previous_ordinal = Some(component.ordinal);
            prepared.push(PreparedBoundaryComponent {
                kind: component.kind,
                ordinal: component.ordinal,
                media_type: component.media_type.to_owned(),
                observed_bytes: component.bytes.len(),
                sha256: crate::sha256_hex(component.bytes),
                bytes: component.bytes.to_vec(),
            });
        }
        let entries: Vec<BoundaryManifestEntry> = prepared
            .iter()
            .map(PreparedBoundaryComponent::entry)
            .collect();
        let chunks: Vec<&[u8]> = prepared
            .iter()
            .map(PreparedBoundaryComponent::bytes)
            .collect();
        Ok(Self {
            adapter_id: adapter_id.to_owned(),
            execution_id: execution_id.to_owned(),
            attempt_id: attempt_id.map(str::to_owned),
            boundary,
            approved_material_sha256: material_sha256(&chunks),
            manifest_sha256: manifest_sha256(
                adapter_id,
                execution_id,
                attempt_id,
                boundary,
                &entries,
            ),
            components: prepared,
        })
    }

    /// Ordered exact components of this prepared input.
    pub fn components(&self) -> &[PreparedBoundaryComponent] {
        &self.components
    }

    /// Number of exact components.
    pub fn component_count(&self) -> usize {
        self.components.len()
    }

    /// Exact application bytes prepared for this input.
    pub fn approved_bytes(&self) -> usize {
        self.components
            .iter()
            .map(|component| component.observed_bytes)
            .sum()
    }

    /// Ordered manifest entries of this prepared input.
    pub fn entries(&self) -> Vec<BoundaryManifestEntry> {
        self.components
            .iter()
            .map(PreparedBoundaryComponent::entry)
            .collect()
    }

    /// Recomputes both digests from the retained bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DispatchError::InvalidMaterial`] when a retained digest no longer describes the
    /// retained bytes.
    pub fn verify(&self) -> Result<(), DispatchError> {
        let entries = self.entries();
        let chunks: Vec<&[u8]> = self
            .components
            .iter()
            .map(PreparedBoundaryComponent::bytes)
            .collect();
        if self.approved_material_sha256 != material_sha256(&chunks)
            || self.manifest_sha256
                != manifest_sha256(
                    &self.adapter_id,
                    &self.execution_id,
                    self.attempt_id.as_deref(),
                    self.boundary,
                    &entries,
                )
        {
            return Err(DispatchError::InvalidMaterial);
        }
        Ok(())
    }

    /// Exactness a consumer may publish for this input.
    pub fn claim(&self) -> EffectiveContextClaim {
        adapter_support(&self.adapter_id).claim()
    }
}
