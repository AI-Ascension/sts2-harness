// SPDX-License-Identifier: MIT

//! Recomputation of the committed System One live-exchange evidence.
//!
//! `docs/evidence/system-one-live-exchange-20260918.json` is a transcription of one operator-run
//! exchange. Nothing recomputed a stored rationale or a stored digest from that file, which is how
//! a decision belonging to a second call stayed filed beside the first call's response (#305). The
//! offline suite could not see it either: every fixture case is built in process, so the emitted
//! rationale is consistent with its own response by construction. These tests recompute both from
//! the committed evidence instead.

use super::*;
use sha2::{Digest, Sha256};

const ARTIFACT: &str =
    include_str!("../../../../../docs/evidence/system-one-live-exchange-20260918.json");
const RESPONSE_BODY: &str = include_str!(
    "../../../../../docs/evidence/system-one-live-exchange-20260918.response-body.json"
);
const REPORT: &str =
    include_str!("../../../../../docs/evidence/system-one-live-exchange-20260918.md");

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    sts2_harness::hex_bytes(hasher.finalize())
}

/// The digests the report publishes, in the order it publishes them: request, then response.
fn published_digests() -> Vec<String> {
    REPORT
        .split("sha256:")
        .skip(1)
        .filter_map(|tail| tail.get(..64))
        .filter(|value| value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_filed_decision_is_derived_from_the_filed_response() {
    let artifact: Value = serde_json::from_str(ARTIFACT).expect("the evidence artifact parses");
    let response = artifact
        .get("provider_response")
        .expect("provider_response");
    let options: Vec<String> = artifact["bridge_request"]["legal_action_ids"]
        .as_array()
        .expect("legal_action_ids")
        .iter()
        .map(|value| value.as_str().expect("an action identifier").to_owned())
        .collect();
    let mapped = map_decision(response, "action", &options, DEFAULT_CONFIDENCE_GATE)
        .expect("the filed response decides");

    assert_eq!(
        mapped.get("rationale"),
        artifact["bridge_decision"].get("rationale"),
        "the filed rationale must be the one this bridge derives from the filed response"
    );
    assert_eq!(
        mapped.get("decision"),
        artifact["bridge_decision"].get("decision"),
        "the filed decision must be the one this bridge derives from the filed response"
    );
}

#[test]
fn the_published_digests_reproduce_from_the_committed_evidence() {
    let digests = published_digests();
    assert_eq!(
        digests.len(),
        2,
        "the report publishes one request/response digest pair"
    );

    let artifact: Value = serde_json::from_str(ARTIFACT).expect("the evidence artifact parses");
    let request =
        serde_json::to_string(&artifact["provider_request"]).expect("the request re-serializes");
    assert_eq!(
        sha256_hex(request.as_bytes()),
        digests[0],
        "the committed request must hash to the published request digest"
    );

    assert!(
        !RESPONSE_BODY.ends_with("\n\n"),
        "the transported body ends with exactly one newline"
    );
    assert!(
        !RESPONSE_BODY.trim_end_matches('\n').contains('\n'),
        "the transported body is one compact line"
    );
    assert_eq!(
        sha256_hex(RESPONSE_BODY.as_bytes()),
        digests[1],
        "the committed transported body must hash to the published response digest"
    );

    let transported: Value =
        serde_json::from_str(RESPONSE_BODY).expect("the transported body parses");
    assert_eq!(
        transported, artifact["provider_response"],
        "the transported body must carry the same exchange as the artifact"
    );
}
