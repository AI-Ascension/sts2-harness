// SPDX-License-Identifier: MIT

//! The decision half of the gate: what counts as a review of record.
//!
//! Split out of `main.rs` so the policy and the transport move independently. The
//! policy here is pure and network-free -- it takes reviews and a head SHA and returns a
//! verdict -- which is what makes every branch in it testable without a subprocess. The
//! only impure part of the tool is the `gh api` seam in `main.rs`.
//!
//! See `main.rs` for why every undetermined state is a failure rather than a pass.

use super::FULL_SHA_LENGTH;

/// The review state could not be determined; the gate must fail, not pass.
#[derive(Debug)]
pub(crate) struct ReviewGateError(pub(crate) String);

impl std::fmt::Display for ReviewGateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ReviewGateError {}

pub(crate) fn is_full_sha(value: &str) -> bool {
    value.len() == FULL_SHA_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn require_full_sha(
    value: Option<&str>,
    field: &str,
) -> Result<String, ReviewGateError> {
    match value {
        Some(text) if is_full_sha(text) => Ok(text.to_owned()),
        _ => Err(ReviewGateError(format!(
            "{field} is missing or not a full {FULL_SHA_LENGTH}-character commit SHA"
        ))),
    }
}

/// A review's `commit_id` when it is a well-formed full SHA, else `None`.
///
/// A malformed review entry is *not* a hard error: a single odd review should not
/// crash the gate, but it must never satisfy the pin either. It simply cannot count
/// as a review of record. Structural errors in the collection as a whole are caught
/// by [`validate_reviews`].
pub(crate) fn review_commit(review: &serde_json::Value) -> Option<&str> {
    let value = review.get("commit_id")?.as_str()?;
    is_full_sha(value).then_some(value)
}

/// Fail closed unless `reviews` is a well-formed array of review objects.
pub(crate) fn validate_reviews(
    reviews: &serde_json::Value,
) -> Result<&Vec<serde_json::Value>, ReviewGateError> {
    let entries = reviews
        .as_array()
        .ok_or_else(|| ReviewGateError("reviews response is not an array".to_owned()))?;
    for (index, review) in entries.iter().enumerate() {
        if !review.is_object() {
            return Err(ReviewGateError(format!(
                "reviews[{index}] is not an object"
            )));
        }
    }
    Ok(entries)
}

/// The gate's verdict.
pub(crate) struct Verdict {
    pub(crate) ok: bool,
    pub(crate) reason: String,
    pub(crate) head_sha: String,
    pub(crate) review_of_record_id: Option<serde_json::Value>,
}

/// Decide whether `reviews` contains a review of record for `head_sha`.
///
/// `ok` is true only when at least one review is pinned to the exact head SHA. Any
/// input the function cannot interpret returns an error; callers must treat that as
/// a failure.
pub(crate) fn evaluate(
    reviews: &serde_json::Value,
    head_sha: Option<&str>,
) -> Result<Verdict, ReviewGateError> {
    let head = require_full_sha(head_sha, "head SHA")?;
    let entries = validate_reviews(reviews)?;

    let mut pinned: Vec<&serde_json::Value> = entries
        .iter()
        .filter(|review| review_commit(review) == Some(head.as_str()))
        .collect();

    if !pinned.is_empty() {
        // Report the earliest pinned review as the review of record, matching how the
        // audit reads the thread. The decision depends only on the existence of a pin,
        // not on this choice.
        pinned.sort_by_key(|review| {
            (
                review
                    .get("submitted_at")
                    .map_or(String::new(), ToString::to_string),
                review.get("id").map_or(String::new(), ToString::to_string),
            )
        });
        let record = pinned
            .first()
            .copied()
            .ok_or_else(|| ReviewGateError("a pinned review set was empty".to_owned()))?;
        let id = record.get("id").cloned();
        let state = record
            .get("state")
            .map_or(String::from("(none)"), ToString::to_string);
        return Ok(Verdict {
            ok: true,
            reason: format!(
                "{} review(s) pinned to head {head}; review of record id={} state={state}",
                pinned.len(),
                id.as_ref()
                    .map_or(String::from("(none)"), ToString::to_string)
            ),
            head_sha: head,
            review_of_record_id: id,
        });
    }

    // No review is pinned to the head. Distinguish "no reviews at all" from "reviews
    // exist but none pinned" -- both fail, but the reason differs.
    let reason = if entries.is_empty() {
        format!("no review of record: head {head} has no reviews at all")
    } else {
        let mut observed: Vec<String> = entries
            .iter()
            .map(|review| {
                review_commit(review)
                    .map_or_else(|| String::from("(missing commit_id)"), ToString::to_string)
            })
            .collect();
        observed.sort();
        observed.dedup();
        format!(
            "no review pinned to head {head}: {} review(s) present, pinned to {}",
            entries.len(),
            observed.join(", ")
        )
    };
    Ok(Verdict {
        ok: false,
        reason,
        head_sha: head,
        review_of_record_id: None,
    })
}

/// The decision half of the tool's `check`, over data already read.
///
/// Split out so the composition of "read both, then decide" can be tested without a
/// subprocess, while the undetermined read itself is exercised through the `gh` seam.
pub(crate) fn check_with(
    reviews: &serde_json::Value,
    head: &str,
) -> Result<Verdict, ReviewGateError> {
    validate_reviews(reviews)?;
    evaluate(reviews, Some(head))
}
