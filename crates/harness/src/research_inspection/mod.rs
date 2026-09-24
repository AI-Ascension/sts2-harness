// SPDX-License-Identifier: MIT

//! Explicitly scoped research inspection of hidden checkpoint state (issue #129).
//!
//! Ordinary gameplay, provider and tool lanes carry only public observation and
//! keyed checkpoint handles (`crate::checkpoint_projection`). A researcher
//! sometimes needs more than that: the hidden ordered piles, unrevealed
//! assignments and RNG streams a capture recorded. That is a *different*
//! permission, so it is admitted separately, bound to one exact checkpoint and
//! branch, and limited to an explicitly enumerated field group at the time of
//! approval.
//!
//! Nothing here reads, simulates, restores or mutates anything. The module fixes
//! the source-only contract: which fields exist, which of them a grant may
//! reach, how a read reports coverage instead of inventing a value, and how a
//! paged read is bounded. The private read adapter that talks to a verified
//! native capture stays with the native owner.

mod error;
mod field;
mod read;
mod scope;

pub use error::ResearchInspectionError;
pub use field::{FieldAvailability, MAX_FIELD_REF_BYTES, ResearchFieldGroup, ResearchFieldRef};
pub use read::{
    AdmittedResearchRead, MAX_RESEARCH_PAGE_ITEMS, ResearchFieldReport, ResearchPageReport,
    ResearchReadRequest, admit_research_read,
};
pub use scope::{
    MAX_RESEARCH_GRANTS, MAX_RESEARCH_IDENTITY_BYTES, RESEARCH_INSPECTION_SCHEMA_VERSION,
    ResearchInspectionGrant, ResearchScopeRefusal, ResearchTargetBinding, is_research_identity,
};
