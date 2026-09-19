// SPDX-License-Identifier: MIT

//! Recomputation check for the committed System One live-exchange evidence file.
//!
//! That file is a record rather than a fixture: it is the only place a real exchange is written
//! down. Its decision, its digests and its withdrawn claims are therefore re-derived here from the
//! committed request and response. The offline suite cannot see a defect of this kind, because it
//! builds every response in process and the rationale is consistent with it by construction; this
//! is the check that would have caught the one that reached the record (#305).

use super::*;
use sha2::{Digest as _, Sha256};

/// The committed evidence file, resolved from this crate so the check reads the repository copy.
const EVIDENCE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/evidence/system-one-live-exchange-20260918.json"
);

/// The issue that records the corrections this file carries.
const CORRECTION_ISSUE: &str = "https://github.com/AI-Ascension/sts2-harness/issues/305";

/// Reads the committed evidence file.
fn evidence() -> Value {
    let raw = std::fs::read_to_string(EVIDENCE).expect("the evidence file is committed");
    serde_json::from_str(&raw).expect("the evidence file is JSON")
}

/// The published canonicalization: compact JSON, object keys sorted, array order preserved.
///
/// This is `serde_json`'s own `Value` encoding, which is why the check can be a plain re-serialization
/// rather than a second, hand-written canonical form that could disagree with the first.
fn sha256_hex(value: &Value) -> String {
    let canonical = serde_json::to_string(value).expect("a parsed value re-serializes");
    let digest = Sha256::digest(canonical.as_bytes());
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[test]
fn the_published_decision_is_what_the_mapper_derives_from_the_published_response() {
    let file = evidence();
    let options: Vec<String> = file["provider_request"]["questions"]["action"]["criteria"]
        .as_object()
        .map(|criteria| criteria.keys().cloned().collect())
        .unwrap_or_default();
    assert_eq!(options.len(), 5, "the presented option set is recorded");

    let recomputed = decision::map_decision(
        &file["provider_response"],
        ACTION_QUESTION,
        &options,
        decision::DEFAULT_CONFIDENCE_GATE,
    )
    .expect("the committed response maps to a decision");
    assert_eq!(
        file["bridge_decision"], recomputed,
        "the stored decision must be the mapper's output for the stored response"
    );
}

#[test]
fn the_published_digests_recompute_from_the_committed_objects() {
    let file = evidence();
    let canonicalization = file["digests"]["canonicalization"]
        .as_str()
        .expect("the canonicalization is stated");
    assert!(
        canonicalization.contains("compact JSON") && canonicalization.contains("sorted"),
        "the digest pair is only checkable if its canonical form is named"
    );
    assert_eq!(
        file["digests"]["provider_request_sha256"],
        json!(sha256_hex(&file["provider_request"]))
    );
    assert_eq!(
        file["digests"]["provider_response_sha256"],
        json!(sha256_hex(&file["provider_response"]))
    );
}

#[test]
fn withdrawn_claims_stay_visible_with_the_issue_that_records_them() {
    let file = evidence();
    let withdrawn = file["withdrawn_claims"]
        .as_array()
        .expect("withdrawn claims are recorded");
    // The unreproducible response digest, the rationale that no input produces, and the run-2 claim
    // with no artifact. Deleting them quietly would hide why the record changed.
    assert_eq!(withdrawn.len(), 3);
    for claim in withdrawn {
        assert_eq!(claim["issue"], json!(CORRECTION_ISSUE));
        assert!(
            claim["reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty()),
            "every withdrawn claim carries its reason"
        );
    }
}
