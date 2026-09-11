// SPDX-License-Identifier: MIT

//! Bounded Phase 3 memory policy owned by the harness.
//!
//! The implementation is split into bounded source files for review and repository policy; the
//! includes intentionally share one module namespace so private content never crosses a public
//! wire type.

mod context_memory_impl {
    include!("context_memory/validation.rs");
    include!("context_memory/strict_json.rs");
    include!("context_memory/migration.rs");
    include!("context_memory/core.rs");
    include!("context_memory/errors.rs");
    include!("context_memory/corpus_data.rs");
    include!("context_memory/corpus.rs");
    include!("context_memory/retrieval.rs");
    include!("context_memory/extraction.rs");
    include!("context_memory/retrieval_helpers.rs");
    include!("context_memory/retrieval_types.rs");
    include!("context_memory/review.rs");
    include!("context_memory/jobs.rs");
    include!("context_memory/summary_store.rs");
    include!("context_memory/manifest.rs");
    include!("context_memory/selection.rs");
    include!("context_memory/approval.rs");
    include!("context_memory/capabilities.rs");
    include!("context_memory/map.rs");
    include!("context_memory/lifecycle.rs");
    include!("context_memory/cache.rs");
    include!("context_memory/controls.rs");
    include!("context_memory/resume.rs");
    include!("context_memory/artifacts.rs");
    include!("context_memory/usage.rs");
    include!("context_memory/evaluation.rs");
    include!("context_memory/retention.rs");
    include!("context_memory/peer.rs");
    include!("context_memory/persistent.rs");
    include!("context_memory/persistent_helpers.rs");
    include!("context_memory/occurrence.rs");
    include!("context_memory/concurrency.rs");
    include!("context_memory/security.rs");
    include!("context_memory/tests.rs");
}

pub use context_memory_impl::*;
