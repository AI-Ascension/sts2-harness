// SPDX-License-Identifier: MIT

//! Adapter boundary support for prepared application input.
//!
//! A consumer may publish only the exactness recorded here.  Every claim describes
//! application-controlled bytes that the harness itself assembles before a write; no claim
//! describes provider-internal conversation, hidden context or an effective provider window.

use super::CaptureBoundary;

/// The strongest exactness a consumer may publish for one prepared input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectiveContextClaim {
    /// The harness assembles these application bytes itself, so approved material can be compared
    /// byte-for-byte with what a recording write port observed.
    ExactApplicationBoundary,
    /// No exact application boundary is modelled.  Only bounded metadata may be published, and
    /// never an exact effective provider context.
    Unsupported,
}

impl EffectiveContextClaim {
    /// Stable wire label for the claim.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExactApplicationBoundary => "exact_application_boundary",
            Self::Unsupported => "unsupported",
        }
    }

    /// Provider internals stay outside this contract for every claim, including the exact one.
    pub const fn covers_provider_internal_context(self) -> bool {
        false
    }
}

/// What the harness knows about one adapter boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterSupport {
    /// The harness assembles and writes the exact application bytes at this boundary.
    Exact { boundary: CaptureBoundary },
    /// No exact application boundary is modelled for this adapter.
    Unsupported { reason: &'static str },
}

impl AdapterSupport {
    /// Exactness a consumer may publish for this adapter.
    pub const fn claim(self) -> EffectiveContextClaim {
        match self {
            Self::Exact { .. } => EffectiveContextClaim::ExactApplicationBoundary,
            Self::Unsupported { .. } => EffectiveContextClaim::Unsupported,
        }
    }

    /// The exact boundary, or `None` when the adapter may not claim exactness.
    pub const fn exact_boundary(self) -> Option<CaptureBoundary> {
        match self {
            Self::Exact { boundary } => Some(boundary),
            Self::Unsupported { .. } => None,
        }
    }
}

/// Every adapter this crate advertises as exact, with the boundary the harness writes.
///
/// The Astra bridge writes the final child input through `PreparedAstraInput` (stdin, output
/// schema and argv/cwd configuration) and the Ollama bridge writes the final serialized HTTP body
/// through `PreparedOllamaInput`, so both are exact application boundaries.  No other adapter id
/// may be advertised as exact, and native or provider-owned internals are never included.
pub const ADVERTISED_EXACT_ADAPTERS: [(&str, CaptureBoundary); 2] = [
    ("exo", CaptureBoundary::ExoSessionRequest),
    ("ollama", CaptureBoundary::HttpBody),
];

/// Resolve one adapter id.  An unknown id is unsupported rather than optimistically exact.
pub fn adapter_support(adapter_id: &str) -> AdapterSupport {
    for (id, boundary) in ADVERTISED_EXACT_ADAPTERS {
        if id == adapter_id {
            return AdapterSupport::Exact { boundary };
        }
    }
    AdapterSupport::Unsupported {
        reason: "no exact application boundary is modelled for this adapter",
    }
}
