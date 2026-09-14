// SPDX-License-Identifier: MIT

//! Producer/consumer pin and digest conformance matrix for published effective limits.
//!
//! The harness owns the executable ceilings, the context-console facade forwards them, and Studio
//! presents them. A consumer on an older capability revision cannot discover the executable
//! ceiling and therefore must not present a value as supported. Producer digests are recomputed
//! from repository bytes here; consumer digests are recorded from an exact revision and re-verified
//! by that consumer's own checkout, because the repositories are separate.
//!
//! The matrix is data plus a fail-closed validator: producer drift, a tampered or absent artifact,
//! an unknown consumer surface, a stale adoption label, and a workflow pin that disagrees with the
//! recorded pin all reject before any limit is authorized.

mod matrix;
mod types;

pub use matrix::PinMatrixError;
pub use types::{
    Adoption, ArtifactMode, ConsumerArtifact, ConsumerPin, ConsumerSurface,
    EFFECTIVE_LIMIT_PIN_MATRIX_SCHEMA, HarnessCiPin, MEMORY_CAPABILITIES_SCHEMA_PATH,
    MEMORY_POLICY_SCHEMA_PATH, PinMatrix, ProducerArtifact, ProducerPins, ProducerSurface,
    RevisionSource, SESSION_CAPABILITIES_SCHEMA_PATH, SESSION_POLICY_SCHEMA_PATH,
    STUDIO_CONTRACT_WORKFLOW_PATH,
};
