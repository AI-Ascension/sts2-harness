// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::workflow::{
    ArtifactError, CatalogError, CatalogInsert, CompilerId, DecodeError, DecoderLimits,
    DiagnosticCode, Digest, MAX_SAFE_INTEGER, ProducerId, Revision, SemanticVersion,
    ValidationError, WorkflowArtifact, WorkflowCatalog, WorkflowDefinition, canonical_json_bytes,
    decode_json, decode_strict, semantic_diff, validate_definition,
};

const VALID: &[u8] = include_bytes!("../../../conformance/workflow-v1/valid-strict.json");
const DUPLICATE: &[u8] =
    include_bytes!("../../../conformance/workflow-v1/invalid-duplicate-key.json");
const UNKNOWN_NODE: &[u8] =
    include_bytes!("../../../conformance/workflow-v1/invalid-unknown-node.json");
const SCHEMA: &str = include_str!("../../../schemas/workflow-v1.schema.json");

fn valid_definition() -> Result<WorkflowDefinition, DecodeError> {
    decode_strict(VALID)
}

fn diagnostic_code(result: Result<Value, DecodeError>) -> Option<DiagnosticCode> {
    result.err().map(|error| error.diagnostic().code)
}

#[test]
fn raw_decoder_rejects_duplicate_non_finite_float_and_unsafe_numbers() {
    assert_eq!(
        diagnostic_code(decode_json(DUPLICATE)),
        Some(DiagnosticCode::DuplicateKey)
    );
    assert_eq!(
        diagnostic_code(decode_json(br#"{"value":NaN}"#)),
        Some(DiagnosticCode::NonFinite)
    );
    assert_eq!(
        diagnostic_code(decode_json(br#"{"value":Infinity}"#)),
        Some(DiagnosticCode::NonFinite)
    );
    assert_eq!(
        diagnostic_code(decode_json(br#"{"value":-Infinity}"#)),
        Some(DiagnosticCode::NonFinite)
    );
    assert_eq!(
        diagnostic_code(decode_json(br#"{"value":1.5}"#)),
        Some(DiagnosticCode::FloatNotAllowed)
    );
    assert_eq!(
        diagnostic_code(decode_json(br#"{"value":9007199254740992}"#)),
        Some(DiagnosticCode::UnsafeInteger)
    );
}

#[test]
fn raw_decoder_applies_byte_depth_string_and_collection_limits() {
    let limits = DecoderLimits {
        max_bytes: 5,
        ..DecoderLimits::default()
    };
    assert_eq!(
        diagnostic_code(sts2_harness::workflow::decode_strict_with_limits(
            br#"{"a":1}"#,
            limits
        )),
        Some(DiagnosticCode::CollectionTooLarge)
    );

    let limits = DecoderLimits {
        max_depth: 1,
        ..DecoderLimits::default()
    };
    assert_eq!(
        diagnostic_code(sts2_harness::workflow::decode_strict_with_limits(
            br#"{"a":{"b":true}}"#,
            limits
        )),
        Some(DiagnosticCode::DepthExceeded)
    );

    let limits = DecoderLimits {
        max_string_bytes: 2,
        ..DecoderLimits::default()
    };
    assert_eq!(
        diagnostic_code(sts2_harness::workflow::decode_strict_with_limits(
            br#"{"a":"abc"}"#,
            limits
        )),
        Some(DiagnosticCode::StringTooLong)
    );

    let limits = DecoderLimits {
        max_array_items: 1,
        ..DecoderLimits::default()
    };
    assert_eq!(
        diagnostic_code(sts2_harness::workflow::decode_strict_with_limits(
            br#"[true,false]"#,
            limits
        )),
        Some(DiagnosticCode::CollectionTooLarge)
    );
}

#[test]
fn typed_identifiers_versions_digests_and_revisions_are_bounded() {
    assert!(SemanticVersion::new("1.2.3").is_ok());
    assert!(SemanticVersion::new("01.2.3").is_err());
    assert!(SemanticVersion::new("1.2").is_err());
    assert!(Revision::new(1).is_ok());
    assert!(Revision::new(0).is_err());
    assert!(Revision::new(MAX_SAFE_INTEGER + 1).is_err());
    assert!(Digest::new("a".repeat(64)).is_ok());
    assert!(Digest::new("A".repeat(64)).is_err());
}

#[test]
fn valid_vector_has_typed_nodes_and_local_references() {
    let result = valid_definition();
    assert!(result.is_ok());
    let Ok(definition) = result else {
        return;
    };
    assert!(validate_definition(&definition).is_ok());
    let graph = &definition.graphs[0];
    assert_eq!(
        graph.nodes[0].kind(),
        sts2_harness::workflow::NodeKind::Observe
    );
    assert_eq!(
        graph.nodes[1].kind(),
        sts2_harness::workflow::NodeKind::Decide
    );
    assert_eq!(
        graph.nodes[1].output_type("proposal"),
        Some(sts2_harness::workflow::ValueType::DecisionProposal)
    );
    assert_eq!(graph.nodes[3].output_type("proposal"), None);
}

#[test]
fn unknown_node_and_unknown_config_fields_fail_before_validation() {
    let unknown = decode_strict::<WorkflowDefinition>(UNKNOWN_NODE);
    assert!(matches!(
        unknown,
        Err(DecodeError::Schema(diagnostic)) if diagnostic.code == DiagnosticCode::UnknownEnum
    ));

    let mut value: Value = match serde_json::from_slice(VALID) {
        Ok(value) => value,
        Err(_) => return,
    };
    value["graphs"][0]["nodes"][0]["config"]["unexpected"] = json!(true);
    let bytes = match serde_json::to_vec(&value) {
        Ok(bytes) => bytes,
        Err(_) => return,
    };
    let result = decode_strict::<WorkflowDefinition>(&bytes);
    assert!(matches!(
        result,
        Err(DecodeError::Schema(diagnostic)) if diagnostic.code == DiagnosticCode::UnknownField
    ));
}

#[test]
fn local_reference_validation_rejects_missing_nodes_and_cycles() {
    let mut value: Value = match serde_json::from_slice(VALID) {
        Ok(value) => value,
        Err(_) => return,
    };
    value["graphs"][0]["nodes"][2]["config"]["proposal_from"]["node_id"] = json!("missing");
    let bytes = match serde_json::to_vec(&value) {
        Ok(bytes) => bytes,
        Err(_) => return,
    };
    let decoded = decode_strict::<WorkflowDefinition>(&bytes);
    assert!(decoded.is_ok());
    if let Ok(definition) = decoded {
        let result = validate_definition(&definition);
        assert!(result.is_err());
        if let Err(error) = result {
            assert!(error.contains(DiagnosticCode::MissingReference));
        }
    }

    let mut cycle: Value = match serde_json::from_slice(VALID) {
        Ok(value) => value,
        Err(_) => return,
    };
    if let Some(edges) = cycle["graphs"][0]["edges"].as_array_mut() {
        edges.push(json!({
            "from": "execute",
            "to": "observe",
            "on": "unavailable",
            "priority": 0
        }));
    }
    let bytes = match serde_json::to_vec(&cycle) {
        Ok(bytes) => bytes,
        Err(_) => return,
    };
    let decoded = decode_strict::<WorkflowDefinition>(&bytes);
    assert!(decoded.is_ok());
    if let Ok(definition) = decoded {
        let result = validate_definition(&definition);
        assert!(result.is_err());
        if let Err(error) = result {
            assert!(error.contains(DiagnosticCode::GraphCycle));
        }
    }
}

#[test]
fn canonicalization_sorts_keys_preserves_arrays_and_excludes_annotations() {
    let left = canonical_json_bytes(&json!({"b": 1, "a": [true, null]}));
    let right = canonical_json_bytes(&json!({"a": [true, null], "b": 1}));
    assert_eq!(left, right);

    let result = valid_definition();
    assert!(result.is_ok());
    let Ok(mut annotated) = result else {
        return;
    };
    let original_digest = match annotated.semantic_digest() {
        Ok(value) => value,
        Err(_) => return,
    };
    if let Some(value) = annotated.annotations.as_mut() {
        value.summary = "changed".to_owned();
    }
    let annotated_digest = match annotated.semantic_digest() {
        Ok(value) => value,
        Err(_) => return,
    };
    assert_eq!(original_digest, annotated_digest);
    let original = match valid_definition() {
        Ok(value) => value,
        Err(_) => return,
    };
    let diff = match semantic_diff(&original, &annotated) {
        Ok(value) => value,
        Err(_) => return,
    };
    assert!(!diff.executable_changed);
    assert!(diff.annotations_changed);
}

#[test]
fn canonical_vector_matches_the_checked_in_semantic_digest() {
    let definition = match valid_definition() {
        Ok(value) => value,
        Err(_) => return,
    };
    let vectors: Value = match serde_json::from_str(include_str!(
        "../../../contract-artifact/workflow-v1/canonical-vectors.json"
    )) {
        Ok(value) => value,
        Err(_) => return,
    };
    let expected = vectors["vectors"][0]["semantic_digest"].as_str();
    let actual = definition.semantic_digest().ok();
    assert_eq!(actual.as_ref().map(Digest::as_str), expected);
}

#[test]
fn artifact_catalog_is_digest_bound_and_immutable() {
    let producer = match ProducerId::new("sts2-harness") {
        Ok(value) => value,
        Err(_) => return,
    };
    let compiler = match CompilerId::new("workflow-compiler.v1") {
        Ok(value) => value,
        Err(_) => return,
    };
    let artifact_result = WorkflowArtifact::from_json(VALID, producer.clone(), compiler.clone());
    assert!(artifact_result.is_ok());
    let Ok(artifact) = artifact_result else {
        return;
    };
    let digest = artifact.semantic_digest().clone();
    assert!(artifact.consume(&digest).is_ok());
    assert!(matches!(
        artifact.consume(&Digest::sha256(b"other")),
        Err(ArtifactError::DigestMismatch)
    ));

    let mut catalog = WorkflowCatalog::default();
    assert_eq!(
        catalog.publish(artifact.clone()),
        Ok(CatalogInsert::Inserted)
    );
    assert_eq!(catalog.publish(artifact), Ok(CatalogInsert::AlreadyPresent));

    let mut changed: Value = match serde_json::from_slice(VALID) {
        Ok(value) => value,
        Err(_) => return,
    };
    changed["limits"]["max_steps"] = json!(31);
    let changed_bytes = match serde_json::to_vec(&changed) {
        Ok(bytes) => bytes,
        Err(_) => return,
    };
    let changed_artifact = match WorkflowArtifact::from_json(&changed_bytes, producer, compiler) {
        Ok(value) => value,
        Err(_) => return,
    };
    assert_eq!(
        catalog.publish(changed_artifact),
        Err(CatalogError::ImmutableConflict)
    );
}

#[test]
fn schema_and_manifest_are_checked_in_as_contract_artifacts() {
    let schema: Value = match serde_json::from_str(SCHEMA) {
        Ok(value) => value,
        Err(_) => return,
    };
    assert_eq!(schema["$id"], "urn:ascension:workflow:v1");
    assert_eq!(schema["properties"]["graphs"]["maxItems"], json!(32));
    let manifest: Value = match serde_json::from_str(include_str!(
        "../../../contract-artifact/workflow-v1/manifest.json"
    )) {
        Ok(value) => value,
        Err(_) => return,
    };
    assert_eq!(manifest["contract_version"], "ascension.workflow/v1");
    assert_eq!(
        manifest["semantic_hash"]["excluded_fields"],
        json!(["annotations"])
    );
}

#[test]
fn schema_accepts_positive_vector_and_rejects_unknown_node() {
    let schema_value: Value = match serde_json::from_str(SCHEMA) {
        Ok(value) => value,
        Err(_) => return,
    };
    let compiled = jsonschema::validator_for(&schema_value);
    assert!(compiled.is_ok());
    let Ok(compiled) = compiled else {
        return;
    };
    let mut positive: Value = match serde_json::from_slice(VALID) {
        Ok(value) => value,
        Err(_) => return,
    };
    let negative: Value = match serde_json::from_slice(UNKNOWN_NODE) {
        Ok(value) => value,
        Err(_) => return,
    };
    assert!(compiled.is_valid(&positive));
    assert!(!compiled.is_valid(&negative));

    positive["graphs"][0]["guards"] = json!([{
        "id": "guard.ready",
        "expression": {
            "kind": "equal",
            "value": {
                "left": "state.ready",
                "right": {"kind": "boolean", "value": true}
            }
        }
    }]);
    assert!(compiled.is_valid(&positive));
    positive["graphs"][0]["guards"][0]["expression"]["unexpected"] = json!(true);
    assert!(!compiled.is_valid(&positive));
}

#[test]
fn invalid_validation_error_has_stable_diagnostic_shape() {
    let result = valid_definition();
    assert!(result.is_ok());
    let Ok(definition) = result else {
        return;
    };
    let error: Result<(), ValidationError> = {
        let mut changed = definition;
        changed.limits.max_steps = 0;
        validate_definition(&changed)
    };
    assert!(error.is_err());
    if let Err(error) = error {
        assert!(error.contains(DiagnosticCode::InvalidLimit));
        assert!(
            error
                .diagnostics()
                .iter()
                .all(|diagnostic| { diagnostic.location.path.starts_with('$') })
        );
    }
}
