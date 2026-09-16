// SPDX-License-Identifier: MIT

use super::*;
use serde_json::json;

const EXPECTED_VALID_VECTORS: &[&str] = &[
    "LBR-VALID-DISCOVERY-INITIAL",
    "LBR-VALID-DISCOVERY-OBSERVED",
    "LBR-VALID-REOBSERVE-REQUIRED",
    "LBR-VALID-REOBSERVE-EXHAUSTED",
    "LBR-VALID-REOBSERVE-UNAVAILABLE",
    "LBR-VALID-REOBSERVED",
    "LBR-VALID-BINDING-IDENTITY-INPUT",
    "LBR-VALID-BINDING-IDENTITY-EPOCH",
];

const EXPECTED_INVALID_VECTORS: &[&str] = &[
    "LBR-INVALID-WRONG-SCOPE",
    "LBR-INVALID-WRONG-INSTANCE",
    "LBR-INVALID-FORGED-BINDING-ID",
    "LBR-INVALID-MIXED-BINDING",
    "LBR-INVALID-STALE-OBSERVATION",
    "LBR-INVALID-MISSING-CAPABILITY",
    "LBR-INVALID-SUPERSEDES-MISMATCH",
    "LBR-INVALID-UNKNOWN-VERSION",
    "LBR-INVALID-REOBSERVE-UNAVAILABLE-SHAPE",
    "LBR-INVALID-EXHAUSTED-WITH-OBSERVATION",
    "LBR-INVALID-STATE-SHAPE",
    "LBR-INVALID-DUPLICATE-KEY",
];

#[test]
fn all_shared_lookup_binding_vectors_are_consumed_at_the_boundary() -> Result<(), String> {
    let root = artifact_root();
    let conformance = read_json(&root.join("conformance.json"))?;
    let case = read_json(&root.join("conformance/cases/game-information-lookup-binding-v1.json"))?;
    let valid = case["valid_vectors"]
        .as_array()
        .ok_or_else(|| String::from("valid vector index is missing"))?;
    let invalid = case["invalid_vectors"]
        .as_array()
        .ok_or_else(|| String::from("invalid vector index is missing"))?;
    let mut valid_ids = valid
        .iter()
        .map(|vector| {
            vector["id"]
                .as_str()
                .ok_or_else(|| String::from("valid vector id is invalid"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut invalid_ids = invalid
        .iter()
        .map(|vector| {
            vector["id"]
                .as_str()
                .ok_or_else(|| String::from("invalid vector id is invalid"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    valid_ids.sort_unstable();
    invalid_ids.sort_unstable();
    let mut expected_valid = EXPECTED_VALID_VECTORS.to_vec();
    let mut expected_invalid = EXPECTED_INVALID_VECTORS.to_vec();
    expected_valid.sort_unstable();
    expected_invalid.sort_unstable();
    assert_eq!(valid_ids, expected_valid);
    assert_eq!(invalid_ids, expected_invalid);
    assert_eq!(conformance["valid_vectors"], json!(EXPECTED_VALID_VECTORS));
    assert_eq!(
        conformance["invalid_vectors"],
        json!(EXPECTED_INVALID_VECTORS)
    );
    let context = &case["context"];
    for vector in valid {
        let id = vector["id"]
            .as_str()
            .ok_or_else(|| String::from("valid vector id is invalid"))?;
        if let Some(relative) = vector["response"].as_str() {
            let golden_path = relative
                .strip_prefix("artifacts/game-information-lookup-binding-v1/")
                .ok_or_else(|| String::from("golden path is outside the pinned artifact"))?;
            let bytes = std::fs::read(root.join(golden_path))
                .map_err(|error| format!("read {id} golden: {error}"))?;
            let value = decode_lookup_binding_response(&bytes)
                .map_err(|error| format!("{id} strict decode: {error}"))?;
            let correlation = value["correlation_id"]
                .as_str()
                .ok_or_else(|| String::from("golden correlation is invalid"))?;
            let session = vector_session(context)?;
            let decoded = session
                .decode(&bytes, correlation, session.observation.as_ref())
                .map_err(|error| format!("{id} boundary validation: {error}"))?;
            if id == "LBR-VALID-REOBSERVE-UNAVAILABLE" {
                assert_eq!(
                    decoded.error,
                    Some(LookupBindingError::ReobserveUnavailable),
                    "{id}"
                );
            }
        } else if let Some(relative) = vector["fixture"].as_str() {
            let fixture = read_json(&root.join(relative))?;
            let canonical = fixture["canonical_json"]
                .as_str()
                .ok_or_else(|| String::from("identity canonical JSON is invalid"))?;
            assert_eq!(
                serde_json::to_string(&fixture["identity_input"])
                    .map_err(|error| format!("{id} identity JSON: {error}"))?,
                canonical,
                "{id} canonical bytes"
            );
            assert_eq!(
                crate::sha256_hex(canonical.as_bytes()),
                fixture["binding_id"]
                    .as_str()
                    .ok_or_else(|| String::from("identity digest is invalid"))?,
                "{id} digest"
            );
        } else {
            return Err(format!("{id} has neither response nor identity fixture"));
        }
    }

    for vector in invalid {
        let id = vector["id"]
            .as_str()
            .ok_or_else(|| String::from("invalid vector id is invalid"))?;
        let expected = expected_error(
            vector["expected_error"]
                .as_str()
                .ok_or_else(|| String::from("invalid expected error is missing"))?,
        )?;
        let fixture_path = vector["fixture"]
            .as_str()
            .ok_or_else(|| String::from("invalid vector fixture path is missing"))?;
        let fixture = read_json(&root.join(fixture_path))?;
        let result = if let Some(raw) = fixture["raw"].as_str() {
            decode_lookup_binding_response(raw.as_bytes()).map(|_| ())
        } else {
            let document = &fixture["document"];
            let bytes = serde_json::to_vec(document)
                .map_err(|error| format!("{id} vector JSON: {error}"))?;
            let correlation = document["correlation_id"]
                .as_str()
                .ok_or_else(|| String::from("vector document correlation is invalid"))?;
            let session = vector_session(&fixture["context"])?;
            session
                .decode(&bytes, correlation, session.observation.as_ref())
                .map(|_| ())
        };
        assert_eq!(result, Err(expected), "{id}");
    }
    Ok(())
}
