// SPDX-License-Identifier: MIT

//! Every branch the reference implementation's own suite pins, ported.
//!
//! The reference is `AI-Ascension/.github`'s `tests/test_review_gate.py`. Each test
//! below names the reference case it ports, so a divergence is a one-line lookup
//! rather than a re-derivation.
//!
//! The suite is split by the half of the tool it exercises: `decision` covers the
//! rule that reads reviews, `runner` covers the `gh` subprocess seam that reads the
//! API, and `check` covers the composition of the two.

mod check;
mod decision;
mod runner;

use serde_json::Value;

pub(super) const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(super) const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// A review as the API returns it: an identity, a state, the commit it is pinned
/// to, and when it was submitted.
pub(super) fn review(commit_id: &str, state: &str, submitted: &str, id: u64) -> Value {
    serde_json::json!({
        "id": id,
        "state": state,
        "commit_id": commit_id,
        "submitted_at": submitted,
        "user": {"login": "CompleteDotTech"},
    })
}

/// A `COMMENTED` review, which the rule counts exactly like `APPROVED`.
pub(super) fn review_at(commit_id: &str, id: u64) -> Value {
    review(commit_id, "COMMENTED", "2026-09-26T01:00:00Z", id)
}
