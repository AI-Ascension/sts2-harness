// SPDX-License-Identifier: MIT

//! The decision rule, over reviews already read.

use super::{HEAD, OTHER, review, review_at};
use crate::decision::evaluate;
use std::error::Error;

use serde_json::json;

/// `test_no_reviews_at_all_fails`
#[test]
fn no_reviews_at_all_fails() -> Result<(), Box<dyn Error>> {
    let verdict = evaluate(&json!([]), Some(HEAD))?;
    assert!(!verdict.ok);
    assert!(verdict.reason.contains("no reviews at all"));
    assert_eq!(verdict.head_sha, HEAD);
    assert_eq!(verdict.review_of_record_id, None);
    Ok(())
}

/// `test_review_pinned_to_head_passes`
#[test]
fn review_pinned_to_head_passes() -> Result<(), Box<dyn Error>> {
    let verdict = evaluate(&json!([review_at(HEAD, 1)]), Some(HEAD))?;
    assert!(verdict.ok);
    assert_eq!(verdict.review_of_record_id, Some(json!(1)));
    Ok(())
}

/// `test_review_pinned_to_other_sha_fails`
#[test]
fn review_pinned_to_other_sha_fails() -> Result<(), Box<dyn Error>> {
    let verdict = evaluate(&json!([review_at(OTHER, 1)]), Some(HEAD))?;
    assert!(!verdict.ok);
    assert!(verdict.reason.contains("no review pinned to head"));
    assert_eq!(verdict.review_of_record_id, None);
    Ok(())
}

/// `test_mixed_reviews_passes_only_via_pinned_one`
#[test]
fn mixed_reviews_passes_only_via_pinned_one() -> Result<(), Box<dyn Error>> {
    let reviews = json!([review_at(OTHER, 1), review_at(HEAD, 2)]);
    let verdict = evaluate(&reviews, Some(HEAD))?;
    assert!(verdict.ok);
    assert_eq!(verdict.review_of_record_id, Some(json!(2)));
    Ok(())
}

/// `test_earliest_pinned_review_is_the_record`
#[test]
fn earliest_pinned_review_is_the_record() -> Result<(), Box<dyn Error>> {
    let reviews = json!([
        review(HEAD, "APPROVED", "2026-09-26T09:00:00Z", 5),
        review(HEAD, "COMMENTED", "2026-09-26T01:00:00Z", 1),
        review(HEAD, "CHANGES_REQUESTED", "2026-09-26T05:00:00Z", 3),
    ]);
    let verdict = evaluate(&reviews, Some(HEAD))?;
    assert!(verdict.ok);
    assert_eq!(verdict.review_of_record_id, Some(json!(1)));
    assert!(verdict.reason.contains("3 review(s) pinned"));
    Ok(())
}

/// `test_all_states_count_when_pinned`
///
/// A `COMMENT` review is a review. The gate's job is to prove a human looked at
/// the exact head, not to re-litigate the verdict they left.
#[test]
fn all_states_count_when_pinned() -> Result<(), Box<dyn Error>> {
    for state in [
        "APPROVED",
        "CHANGES_REQUESTED",
        "COMMENTED",
        "DISMISSED",
        "PENDING",
    ] {
        let reviews = json!([review(HEAD, state, "2026-09-26T01:00:00Z", 1)]);
        let verdict = evaluate(&reviews, Some(HEAD))?;
        assert!(verdict.ok, "state {state} must count as a review of record");
    }
    Ok(())
}

/// `test_short_sha_is_not_a_pin`
#[test]
fn short_sha_is_not_a_pin() -> Result<(), Box<dyn Error>> {
    // A malformed review is not a hard error here: a single odd review should not
    // crash the gate. It also must never satisfy the pin.
    let reviews = json!([review_at("aaaaaaa", 1)]);
    let verdict = evaluate(&reviews, Some(HEAD))?;
    assert!(
        !verdict.ok,
        "a short review SHA is a prefix, not an identity"
    );
    Ok(())
}

/// `test_missing_head_sha_raises`
#[test]
fn missing_head_sha_raises() -> Result<(), Box<dyn Error>> {
    assert!(evaluate(&json!([]), None).is_err());
    assert!(evaluate(&json!([]), Some("")).is_err());
    assert!(evaluate(&json!([]), Some("abc")).is_err());
    Ok(())
}

/// `test_reviews_not_an_array_raises`
#[test]
fn reviews_not_an_array_raises() -> Result<(), Box<dyn Error>> {
    for body in [json!({}), json!("[]"), json!(null), json!(7)] {
        assert!(
            evaluate(&body, Some(HEAD)).is_err(),
            "a non-array review payload is undetermined, not empty: {body}"
        );
    }
    Ok(())
}

/// `test_malformed_review_entry_raises`
#[test]
fn malformed_review_entry_raises() -> Result<(), Box<dyn Error>> {
    assert!(evaluate(&json!(["not-an-object"]), Some(HEAD)).is_err());
    Ok(())
}

/// `test_empty_payload_is_not_a_pass`
#[test]
fn empty_payload_is_not_a_pass() -> Result<(), Box<dyn Error>> {
    let verdict = evaluate(&json!([]), Some(HEAD))?;
    assert!(!verdict.ok);
    Ok(())
}
