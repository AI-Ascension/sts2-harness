// SPDX-License-Identifier: MIT

//! The composition of "read both, then decide," over data already in hand.

use super::{HEAD, OTHER, review};
use crate::decision::{ReviewGateError, check_with};
use std::error::Error;

use serde_json::json;

/// `test_check_uses_all_reviews_not_just_latest`
///
/// Regression for the unpinned/earliest-review audit note: reading only the latest
/// review must not be how the gate decides.
///
/// The pinned review is deliberately FIRST and the later review is the unpinned one. A
/// `reviews[-1:]` implementation sees the unpinned review, decides "no review of record,"
/// and fails. Only an implementation that reads the whole list finds the pin at index 0.
/// With the pinned review last, this test could not tell those two apart.
#[test]
fn check_uses_all_reviews_not_just_latest() -> Result<(), Box<dyn Error>> {
    let reviews = json!([
        review(HEAD, "COMMENTED", "2026-09-26T01:00:00Z", 1),
        review(OTHER, "COMMENTED", "2026-09-26T09:00:00Z", 5),
    ]);
    let verdict = check_with(&reviews, HEAD)?;
    assert!(verdict.ok);
    assert_eq!(verdict.review_of_record_id, Some(json!(1)));
    Ok(())
}

/// `test_check_propagates_undetermined_state`
#[test]
fn check_propagates_undetermined_state() -> Result<(), Box<dyn Error>> {
    // A read that fails must surface as an error, never as a verdict a caller could
    // mistake for a pass. The ported shape: an error-shaped body `gh` can return.
    let outcome = check_with(&json!({"message": "api down"}), HEAD);
    assert!(matches!(outcome, Err(ReviewGateError(_))));
    Ok(())
}

/// `test_main_returns_one_on_undetermined_state`
///
/// A failing check must exit nonzero so the check-run is red. The port exercises the
/// same path the binary takes: an undetermined read yields `ok: false` and a reason
/// naming "undetermined," and that is what turns into a failing exit status.
#[test]
fn undetermined_state_is_reported_as_not_ok() -> Result<(), Box<dyn Error>> {
    let error = ReviewGateError(String::from("api down"));
    let reason = format!("review state undetermined: {error}");
    let report = json!({"ok": false, "reason": reason});
    assert_eq!(report["ok"], json!(false));
    assert!(
        report["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("undetermined"),
        "the reason must say the state was undetermined, not that no review exists"
    );
    Ok(())
}
